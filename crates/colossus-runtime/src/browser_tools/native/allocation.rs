use super::{lifecycle, types::*};
use crate::{Runtime, prelude::*};
use colossus_browser_presentation::Configure;
use colossus_contracts::{
    BrowserMode, BrowserOpenOptions, BrowserScope, BrowserSessionBinding, BrowserSessionId,
};
use std::sync::atomic::Ordering;

impl Runtime {
    /// Seal a native role after the managed worker verifies its independent bootstrap.
    /// This in-process port is never exposed as a renderer/model ownership claim.
    pub fn bind_native_browser_authority(
        &self,
        application_id: &str,
        instance_uuid: Uuid,
    ) -> Result<RuntimeNativeBrowserAuthority, RuntimeNativeBrowserError> {
        let browser = self
            .browser
            .as_ref()
            .ok_or(RuntimeNativeBrowserError::Unavailable)?;
        browser
            .identity
            .revalidate()
            .map_err(|_| RuntimeNativeBrowserError::Denied)?;
        if application_id.is_empty()
            || application_id.len() > 256
            || application_id.chars().any(char::is_control)
            || instance_uuid.is_nil()
        {
            return Err(RuntimeNativeBrowserError::Denied);
        }
        Ok(RuntimeNativeBrowserAuthority {
            runtime_id: browser.runtime_id.clone(),
            application_id: application_id.into(),
            instance: instance_uuid,
        })
    }

    /// Human admission requires accepted Embedded capability and a same-host native presenter.
    pub fn native_browser_available(&self, role: &RuntimeNativeBrowserAuthority) -> bool {
        self.browser.as_ref().is_some_and(|browser| {
            authority(browser, role).is_ok()
                && !browser.native.stopped.load(Ordering::Acquire)
                && browser.native.presenter.is_some()
                && {
                    let caps = browser.coordinator.capabilities();
                    caps.available
                        && caps.restrictive_egress
                        && caps.modes.contains(&BrowserMode::Embedded)
                }
        })
    }

    /// Admit one human page through the ordinary browser.open effect gateway.
    /// An owned worker retains allocation and cleanup even if this request future is dropped.
    pub async fn open_native_browser(
        &self,
        role: &RuntimeNativeBrowserAuthority,
        input: NativeBrowserOpenRequest,
    ) -> Result<RuntimeNativeBrowserSession, RuntimeNativeBrowserError> {
        let browser = self
            .browser
            .as_ref()
            .ok_or(RuntimeNativeBrowserError::Unavailable)?
            .clone();
        authority(&browser, role)?;
        if !self.native_browser_available(role) {
            return Err(RuntimeNativeBrowserError::Unavailable);
        }
        let placeholder = BrowserSessionId::parse(format!("bs_{}", Uuid::now_v7().simple()))
            .map_err(|_| RuntimeNativeBrowserError::LimitExceeded)?;
        let configure = Configure {
            session: placeholder,
            target: colossus_contracts::BrowserTarget {
                tab_id: colossus_contracts::BrowserTabId::parse(format!(
                    "bt_{}",
                    Uuid::now_v7().simple()
                ))
                .map_err(|_| RuntimeNativeBrowserError::LimitExceeded)?,
                document_id: colossus_contracts::BrowserDocumentId::parse(format!(
                    "bd_{}",
                    Uuid::now_v7().simple()
                ))
                .map_err(|_| RuntimeNativeBrowserError::LimitExceeded)?,
            },
            control_generation: 0,
            viewport_generation: input.viewport_generation,
            width: input.width,
            height: input.height,
            scale_milli: input.scale_milli,
            lease_ms: input.lease_ms,
        };
        configure.validate()?;
        let conversation_id = match input.conversation_id {
            Some(id) => {
                validate_conversation(self, &id, &role.application_id)?;
                id
            }
            None => {
                let id = Uuid::now_v7().to_string();
                self.create_application_session(
                    &id,
                    None,
                    Actor {
                        actor_type: ActorType::Application,
                        id: role.application_id.clone(),
                    },
                )
                .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
                validate_conversation(self, &id, &role.application_id)?;
                id
            }
        };

        let binding = BrowserSessionBinding {
            runtime_id: browser.runtime_id.clone(),
            workspace_id: browser.workspace_id.clone(),
            application_id: role.application_id.clone(),
            scope: BrowserScope::Conversation {
                id: conversation_id.clone(),
            },
        };
        let options = BrowserOpenOptions {
            profile: Default::default(),
            mode: BrowserMode::Embedded,
            allowed_origins: vec![input.url.origin()],
            initial_url: Some(input.url),
        };
        let entry = Arc::new(Entry {
            id: Uuid::now_v7(),
            instance: role.instance,
            binding,
            conversation_id,
            request: StdMutex::new(None),
            control: RunControl::default(),
            operation: TokioMutex::new(()),
            view: StdMutex::new(View {
                configure: Some(configure),
                ..View::default()
            }),
        });
        {
            let mut entries = browser
                .native
                .entries
                .lock()
                .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
            if browser.native.stopped.load(Ordering::Acquire) {
                return Err(RuntimeNativeBrowserError::Unavailable);
            }
            if entries.len() >= usize::from(browser.coordinator.capabilities().limits.max_sessions)
            {
                return Err(RuntimeNativeBrowserError::LimitExceeded);
            }
            entries.insert(entry.id, entry.clone());
        }
        let gateway = self.gateway.clone();
        let (mut sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut guard = lifecycle::WorkerGuard::new(browser.clone(), entry.clone());
            let mut pending = Box::pin(open_owned(&browser, &gateway, &entry, options));
            let result = tokio::select! {
                result=&mut pending=>result,
                ()=sender.closed()=>{lifecycle::fence_entry(&entry);pending.await}
            };
            match result {
                Ok(session) => {
                    if sender.send(Ok(session)).is_err() {
                        let _ = lifecycle::close_entry(&browser, &entry).await;
                    }
                }
                Err(error) => {
                    let cleanup = lifecycle::close_entry(&browser, &entry).await;
                    let _ = sender.send(Err(if cleanup.is_ok() {
                        error
                    } else {
                        RuntimeNativeBrowserError::OutcomeUnknown
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

fn validate_conversation(
    runtime: &Runtime,
    id: &str,
    app: &str,
) -> Result<(), RuntimeNativeBrowserError> {
    if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
        return Err(RuntimeNativeBrowserError::Denied);
    }
    let events = runtime
        .journal
        .read_stream_from(&format!("session:{id}"), 0, 1)
        .map_err(|_| RuntimeNativeBrowserError::Denied)?;
    let created = events.first().ok_or(RuntimeNativeBrowserError::Denied)?;
    if created.event_type != "session.created.v1"
        || created.actor.actor_type != ActorType::Application
        || created.actor.id != app
    {
        return Err(RuntimeNativeBrowserError::Denied);
    }
    if runtime
        .get_session(id)
        .map_err(|_| RuntimeNativeBrowserError::Denied)?
        .is_none()
    {
        return Err(RuntimeNativeBrowserError::Denied);
    }
    Ok(())
}

async fn open_owned(
    browser: &Arc<super::super::RuntimeBrowserTools>,
    gateway: &Arc<EffectGateway>,
    entry: &Arc<Entry>,
    options: BrowserOpenOptions,
) -> Result<RuntimeNativeBrowserSession, RuntimeNativeBrowserError> {
    let _operation = entry.operation.lock().await;
    let _opening = browser.native.opening.lock().await;
    if entry.control.is_cancelled() || browser.native.stopped.load(Ordering::Acquire) {
        return Err(RuntimeNativeBrowserError::Denied);
    }
    *browser
        .native
        .current
        .lock()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)? = Some(entry.clone());
    let _current = CurrentGuard {
        state: browser.native.clone(),
        entry: entry.clone(),
    };
    let mut request = effect_request(
        Actor {
            actor_type: ActorType::Application,
            id: entry.binding.application_id.clone(),
        },
        "browser.open",
        options
            .initial_url
            .as_ref()
            .map_or("browser:session", |url| url.as_str()),
        json!({"operation":"native_open","mode":"embedded","allowed_origins":options.allowed_origins,"initial_url":options.initial_url}),
    );
    request.capabilities = vec!["browser.open".into()];
    request.context.session_id = Some(entry.conversation_id.clone());
    let released = gateway
        .execute(
            request,
            &HumanOpenEffect {
                browser,
                entry,
                options,
            },
        )
        .await;
    released.map_err(|error| {
        if entry
            .request
            .lock()
            .map_or(true, |request| request.is_some())
            || matches!(error, GatewayError::OutcomeUnknown(_))
        {
            RuntimeNativeBrowserError::OutcomeUnknown
        } else {
            RuntimeNativeBrowserError::Denied
        }
    })?;
    if entry.control.is_cancelled() || browser.native.stopped.load(Ordering::Acquire) {
        return Err(RuntimeNativeBrowserError::Denied);
    }
    let original = entry
        .request
        .lock()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
        .clone()
        .ok_or(RuntimeNativeBrowserError::OutcomeUnknown)?;
    let provider = browser
        .native
        .presenter
        .as_ref()
        .ok_or(RuntimeNativeBrowserError::Unavailable)?;
    let (client, digest) = provider.acquire(&original).await?;
    if entry.control.is_cancelled() || browser.native.stopped.load(Ordering::Acquire) {
        client.disconnect();
        return Err(RuntimeNativeBrowserError::Denied);
    }
    let mut view = entry
        .view
        .lock()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
    if view.closed {
        client.disconnect();
        return Err(RuntimeNativeBrowserError::Stale);
    }
    let configure = view
        .configure
        .as_mut()
        .ok_or(RuntimeNativeBrowserError::OutcomeUnknown)?;
    configure.session = original.session_id.clone();
    configure.target = target(&original);
    view.client = Some(client.clone());
    view.digest = Some(digest);
    Ok(RuntimeNativeBrowserSession {
        entry: entry.clone(),
        client,
        digest,
        session_id: original.session_id,
    })
}

struct CurrentGuard {
    state: Arc<NativeState>,
    entry: Arc<Entry>,
}
impl Drop for CurrentGuard {
    fn drop(&mut self) {
        let mut slot = self
            .state
            .current
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &self.entry))
        {
            *slot = None;
        }
    }
}

struct HumanOpenEffect<'a> {
    browser: &'a super::super::RuntimeBrowserTools,
    entry: &'a Entry,
    options: BrowserOpenOptions,
}
#[async_trait]
impl EffectExecutor for HumanOpenEffect<'_> {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if request.action != "browser.open"
            || request.actor.actor_type != ActorType::Application
            || request.actor.id != self.entry.binding.application_id
            || request.context.session_id.as_deref() != Some(&self.entry.conversation_id)
            || request.content["operation"] != "native_open"
            || request.capabilities != ["browser.open"]
            || !permit.obligations().require_post_effect
        {
            return Err(ExecutionError::Failed(
                "native browser admission denied".into(),
            ));
        }
        for origin in &self.options.allowed_origins {
            if colossus_policy::http_transport_authority_match(
                permit.obligations(),
                origin.as_str(),
            )
            .map_err(|_| ExecutionError::Failed("native browser destination denied".into()))?
            .is_none()
            {
                return Err(ExecutionError::Failed(
                    "native browser destination denied".into(),
                ));
            }
        }
        let summary = self
            .browser
            .coordinator
            .open(
                &self.entry.binding,
                self.options.clone(),
                &self.entry.control,
            )
            .await
            .map_err(super::super::service::execution_error)?;
        let bytes =
            serde_json::to_vec(&json!({"session_id":summary.session_id})).map_err(|_| {
                ExecutionError::OutcomeUnknown("native admission encoding unknown".into())
            })?;
        if bytes.len() as u64 > permit.obligations().max_output_bytes {
            return Err(ExecutionError::OutcomeUnknown(
                "native admission output bound".into(),
            ));
        }
        Ok(QuarantinedEffectResult {
            bytes,
            media_type: "application/json".into(),
            effect_succeeded: true,
        })
    }
}
