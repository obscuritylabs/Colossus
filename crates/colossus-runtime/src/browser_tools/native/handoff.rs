use super::{lifecycle, types::*};
use crate::{Runtime, prelude::*};
use colossus_browser::NativeBrowserHandoff;
use colossus_contracts::{BrowserAction, BrowserControlState, BrowserSessionId};
use colossus_ports::{BrowserDriverControl, BrowserNativeHandoffRequest};

impl Runtime {
    /// Transfer one live human page to its registered application/conversation run.
    /// Renderer input supplies no actor, control grant, native receipt or document authority.
    pub async fn handoff_native_browser(
        &self,
        role: &RuntimeNativeBrowserAuthority,
        session: &BrowserSessionId,
        run_id: &str,
        lease_ms: u32,
    ) -> Result<RuntimeNativeBrowserGranted, RuntimeNativeBrowserError> {
        let browser = self
            .browser
            .as_ref()
            .ok_or(RuntimeNativeBrowserError::Unavailable)?
            .clone();
        let entry = lifecycle::entry(&browser, role, session)?;
        let run = browser
            .runs
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
            .get(run_id)
            .cloned()
            .ok_or(RuntimeNativeBrowserError::Denied)?;
        if run.actor.binding != entry.binding
            || run.context.session_id.as_deref() != Some(&entry.conversation_id)
            || run.control.is_cancelled()
            || !run
                .context
                .offered_tools
                .iter()
                .any(|tool| tool == "browser.open")
        {
            return Err(RuntimeNativeBrowserError::Denied);
        }
        let gateway = self.gateway.clone();
        let (mut sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut guard = lifecycle::WorkerGuard::new(browser.clone(), entry.clone());
            let mut pending = Box::pin(handoff_owned(&browser, &gateway, &entry, &run, lease_ms));
            let result = tokio::select! {
                result=&mut pending=>result,
                ()=sender.closed()=>{lifecycle::fence_entry(&entry);pending.await}
            };
            match result {
                Ok(granted) => {
                    if sender.send(Ok(granted)).is_err() {
                        let _ = lifecycle::close_entry(&browser, &entry).await;
                    }
                }
                Err(error) => {
                    let fenced = entry.view.lock().map_or(true, |view| view.fenced);
                    if fenced {
                        let _ = lifecycle::close_entry(&browser, &entry).await;
                    }
                    let _ = sender.send(Err(if fenced {
                        RuntimeNativeBrowserError::OutcomeUnknown
                    } else {
                        error
                    }));
                }
            }
            guard.finish();
        });
        receiver
            .await
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
    }
}

async fn handoff_owned(
    browser: &Arc<super::super::RuntimeBrowserTools>,
    gateway: &Arc<EffectGateway>,
    entry: &Arc<Entry>,
    run: &super::super::service::BrowserRun,
    lease_ms: u32,
) -> Result<RuntimeNativeBrowserGranted, RuntimeNativeBrowserError> {
    let _operation = entry.operation.lock().await;
    if entry.control.is_cancelled() {
        return Err(RuntimeNativeBrowserError::Denied);
    }
    let original = entry
        .request
        .lock()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
        .clone()
        .ok_or(RuntimeNativeBrowserError::Stale)?;
    browser
        .run(&run.context)
        .map_err(|_| RuntimeNativeBrowserError::Denied)?;
    let mut request = effect_request(
        Actor {
            actor_type: ActorType::Application,
            id: run.actor.binding.application_id.clone(),
        },
        "browser.open",
        format!("browser:{}", original.session_id.as_str()),
        json!({"operation":"native_handoff","session_id":original.session_id,"lease_ms":lease_ms,"allowed_origins":original.options.allowed_origins}),
    );
    request.capabilities = vec!["browser.open".into()];
    request.context = run.context.clone();
    let result = StdMutex::new(None);
    gateway
        .execute(
            request,
            &HandoffEffect {
                browser,
                entry,
                run,
                original: &original,
                lease_ms,
                result: &result,
            },
        )
        .await
        .map_err(|_| {
            if entry.view.lock().map_or(true, |view| view.fenced) {
                RuntimeNativeBrowserError::OutcomeUnknown
            } else {
                RuntimeNativeBrowserError::Denied
            }
        })?;
    browser
        .run(&run.context)
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
    if entry.control.is_cancelled() {
        return Err(RuntimeNativeBrowserError::OutcomeUnknown);
    }
    result
        .into_inner()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
        .ok_or(RuntimeNativeBrowserError::OutcomeUnknown)
}

struct HandoffEffect<'a> {
    browser: &'a super::super::RuntimeBrowserTools,
    entry: &'a Entry,
    run: &'a super::super::service::BrowserRun,
    original: &'a colossus_ports::BrowserDriverOpenRequest,
    lease_ms: u32,
    result: &'a StdMutex<Option<RuntimeNativeBrowserGranted>>,
}
#[async_trait]
impl EffectExecutor for HandoffEffect<'_> {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let known = |_: RuntimeNativeBrowserError| {
            ExecutionError::Failed("native browser handoff denied".into())
        };
        let unknown = |_: RuntimeNativeBrowserError| {
            ExecutionError::OutcomeUnknown("native browser handoff unknown".into())
        };
        if request.action != "browser.open"
            || self.entry.control.is_cancelled()
            || request.content["operation"] != "native_handoff"
            || request.content["session_id"] != json!(self.original.session_id)
            || request.capabilities != ["browser.open"]
            || request.actor.actor_type != ActorType::Application
            || request.actor.id != self.run.actor.binding.application_id
            || request.context != self.run.context
            || !permit.obligations().require_post_effect
            || permit.obligations().max_output_bytes
                < u64::from(
                    self.browser
                        .coordinator
                        .capabilities()
                        .limits
                        .max_observation_bytes,
                )
        {
            return Err(known(RuntimeNativeBrowserError::Denied));
        }
        self.browser
            .run(&request.context)
            .map_err(|_| known(RuntimeNativeBrowserError::Denied))?;
        for origin in &self.original.options.allowed_origins {
            if colossus_policy::http_transport_authority_match(
                permit.obligations(),
                origin.as_str(),
            )
            .map_err(|_| known(RuntimeNativeBrowserError::Denied))?
            .is_none()
            {
                return Err(known(RuntimeNativeBrowserError::Denied));
            }
        }
        let summary = self
            .browser
            .coordinator
            .get(&self.entry.binding, &self.original.session_id)
            .map_err(|error| known(error.into()))?;
        if summary.control != BrowserControlState::Human || summary.control_generation != 0 {
            return Err(known(RuntimeNativeBrowserError::Stale));
        }
        let expected = summary
            .tabs
            .iter()
            .find(|tab| summary.selected_tab_id.as_ref() == Some(&tab.tab_id))
            .map(|tab| colossus_contracts::BrowserTarget {
                tab_id: tab.tab_id.clone(),
                document_id: tab.document_id.clone(),
            })
            .ok_or_else(|| known(RuntimeNativeBrowserError::Stale))?;
        let (client, prior, mut configure, digest) = {
            let mut view = self
                .entry
                .view
                .lock()
                .map_err(|_| unknown(RuntimeNativeBrowserError::OutcomeUnknown))?;
            if view.fenced || view.closed || view.agent_generation.is_some() {
                return Err(known(RuntimeNativeBrowserError::Stale));
            }
            let tuple = (
                view.client
                    .clone()
                    .ok_or_else(|| known(RuntimeNativeBrowserError::Stale))?,
                view.lease
                    .ok_or_else(|| known(RuntimeNativeBrowserError::Stale))?,
                view.configure
                    .clone()
                    .ok_or_else(|| known(RuntimeNativeBrowserError::Stale))?,
                view.digest
                    .ok_or_else(|| known(RuntimeNativeBrowserError::Stale))?,
            );
            // After this point no failure or cancellation can restore human input.
            view.fenced = true;
            tuple
        };
        let fenced = client
            .fence_human(prior)
            .await
            .map_err(|error| unknown(error.into()))?;
        if fenced.prior_lease != prior || fenced.state.target.tab_id != expected.tab_id {
            return Err(unknown(RuntimeNativeBrowserError::Stale));
        }
        if self.entry.control.is_cancelled() {
            return Err(unknown(RuntimeNativeBrowserError::Denied));
        }
        let control =
            BrowserDriverControl::new(self.entry.control.clone(), self.run.control.clone());
        let confirmed = self
            .browser
            .native
            .driver
            .confirm_native_handoff(
                BrowserNativeHandoffRequest {
                    binding: self.entry.binding.clone(),
                    session_id: self.original.session_id.clone(),
                    expected_target: expected.clone(),
                    confirmed_target: fenced.state.target.clone(),
                    native_document_generation: fenced.native_document_generation,
                },
                &control,
            )
            .await
            .map_err(|error| unknown(error.into()))?;
        if confirmed.tab_id != fenced.state.target.tab_id
            || confirmed.document_id != fenced.state.target.document_id
        {
            return Err(unknown(RuntimeNativeBrowserError::Stale));
        }
        self.browser
            .run(&request.context)
            .map_err(|_| unknown(RuntimeNativeBrowserError::Denied))?;
        if self.entry.control.is_cancelled() {
            return Err(unknown(RuntimeNativeBrowserError::Denied));
        }
        let lease = self
            .browser
            .coordinator
            .grant_control_from_native_handoff(
                &self.run.actor,
                &self.original.session_id,
                self.lease_ms,
                NativeBrowserHandoff {
                    expected_control_generation: summary.control_generation,
                    expected_target: expected,
                    confirmed_tab: confirmed.clone(),
                },
            )
            .map_err(|error| unknown(error.into()))?;
        let target = colossus_contracts::BrowserTarget {
            tab_id: confirmed.tab_id,
            document_id: confirmed.document_id,
        };
        // The first protected observation binds native controller/document to the
        // CAS grant under this same handoff permit; no page bytes are released here.
        let observed = self
            .browser
            .coordinator
            .execute(
                &self.run.actor,
                &lease,
                &target,
                BrowserAction::Snapshot {
                    max_nodes: self
                        .browser
                        .coordinator
                        .capabilities()
                        .limits
                        .max_snapshot_nodes,
                },
                &self.entry.control,
            )
            .await
            .map_err(super::super::service::execution_error)?;
        if self.entry.control.is_cancelled() {
            return Err(unknown(RuntimeNativeBrowserError::Denied));
        }
        configure.target = colossus_contracts::BrowserTarget {
            tab_id: observed.tab.tab_id,
            document_id: observed.tab.document_id,
        };
        configure.control_generation = lease.control_generation;
        configure.viewport_generation = configure
            .viewport_generation
            .checked_add(1)
            .ok_or_else(|| unknown(RuntimeNativeBrowserError::LimitExceeded))?;
        let next = client
            .configure(configure.clone())
            .await
            .map_err(|error| unknown(error.into()))?;
        if !configure.accepts(next) {
            return Err(unknown(RuntimeNativeBrowserError::Stale));
        }
        self.browser
            .run(&request.context)
            .map_err(|_| unknown(RuntimeNativeBrowserError::Denied))?;
        if self.entry.control.is_cancelled() {
            return Err(unknown(RuntimeNativeBrowserError::Denied));
        }
        self.browser
            .remember_session(
                &self.run.actor,
                super::super::service::BrowserSessionAuthority {
                    lease: lease.clone(),
                    origins: self.original.options.allowed_origins.clone(),
                },
            )
            .map_err(|_| unknown(RuntimeNativeBrowserError::Denied))?;
        {
            let mut view = self
                .entry
                .view
                .lock()
                .map_err(|_| unknown(RuntimeNativeBrowserError::OutcomeUnknown))?;
            if view.closed {
                return Err(unknown(RuntimeNativeBrowserError::Stale));
            }
            view.agent_generation = Some(lease.control_generation);
            view.configure = Some(configure.clone());
            view.lease = Some(next);
        }
        configure.viewport_generation = configure
            .viewport_generation
            .checked_add(1)
            .ok_or_else(|| unknown(RuntimeNativeBrowserError::LimitExceeded))?;
        let value =
            json!({"session_id":lease.session_id,"control_generation":lease.control_generation});
        let bytes = serde_json::to_vec(&value)
            .map_err(|_| unknown(RuntimeNativeBrowserError::OutcomeUnknown))?;
        if bytes.len() as u64 > permit.obligations().max_output_bytes {
            return Err(unknown(RuntimeNativeBrowserError::LimitExceeded));
        }
        *self
            .result
            .lock()
            .map_err(|_| unknown(RuntimeNativeBrowserError::OutcomeUnknown))? =
            Some(RuntimeNativeBrowserGranted {
                control: lease,
                configure,
                enrollment_digest: digest,
            });
        Ok(QuarantinedEffectResult {
            bytes,
            media_type: "application/json".into(),
            effect_succeeded: true,
        })
    }
}
