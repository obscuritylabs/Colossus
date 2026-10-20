use super::{
    arguments::BrowserInvocation,
    service::{BrowserSessionAuthority, RuntimeBrowserTools, execution_error},
};
use crate::prelude::*;
use colossus_contracts::{BrowserAction, BrowserTarget};

pub(super) struct BrowserEffect<'a> {
    pub(super) browser: &'a RuntimeBrowserTools,
    pub(super) screenshot: &'a StdMutex<Option<super::screenshot::Receipt>>,
    pub(super) upload: &'a StdMutex<Option<colossus_ports::BrowserUploadArtifact>>,
    pub(super) download: &'a StdMutex<Option<super::download::Receipt>>,
}

#[async_trait]
impl EffectExecutor for BrowserEffect<'_> {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if !permit.obligations().require_post_effect {
            return Err(ExecutionError::Failed(
                "browser observations require post-effect policy".into(),
            ));
        }
        let invocation = super::transfer::invocation(request)?;
        if invocation.action() != request.action
            || invocation.resource() != request.resource
            || request.capabilities != [invocation.action()]
            || !request
                .context
                .offered_tools
                .iter()
                .any(|name| name == invocation.action())
        {
            return Err(ExecutionError::Failed(
                "browser request does not match its authorized operation".into(),
            ));
        }
        let run = self
            .browser
            .run(&request.context)
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let authority = self
            .browser
            .authority(&run, &invocation)
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let obligations = permit.obligations();
        if authority
            .as_ref()
            .is_some_and(|authority| authority.public_web)
            && !super::service::permits_public_web(obligations)
        {
            return Err(ExecutionError::Failed(
                "browser public network grant is no longer authorized".into(),
            ));
        }
        if obligations.max_output_bytes
            < u64::from(
                self.browser
                    .coordinator
                    .capabilities()
                    .limits
                    .max_observation_bytes,
            )
        {
            return Err(ExecutionError::Failed(
                "browser policy output bound is below the installed observation contract".into(),
            ));
        }
        let session_origins = run
            .sessions
            .values()
            .flat_map(|session| session.origins.iter())
            .cloned()
            .collect::<Vec<_>>();
        let origins = match &invocation {
            BrowserInvocation::Open { options } => &options.allowed_origins,
            BrowserInvocation::Sessions {} => &session_origins,
            _ => {
                &authority
                    .as_ref()
                    .ok_or_else(|| {
                        ExecutionError::Failed("browser session authority absent".into())
                    })?
                    .origins
            }
        };
        // Exact destinations always require policy. Public browsing is separately
        // narrowed to the permit's network grant before native allocation.
        for origin in origins {
            let allowed =
                colossus_policy::http_transport_authority_match(obligations, origin.as_str())
                    .map_err(|_| {
                        ExecutionError::Failed("invalid browser egress obligations".into())
                    })?
                    .is_some();
            if !allowed {
                return Err(ExecutionError::Failed(
                    "browser origin envelope exceeds authorized destinations".into(),
                ));
            }
        }
        let value = match invocation {
            BrowserInvocation::Sessions {} => {
                let mut sessions = Vec::new();
                for owned in run.sessions.values() {
                    let lease = &owned.lease;
                    self.browser
                        .coordinator
                        .lease(&run.actor, &lease.session_id, lease.control_generation)
                        .map_err(execution_error)?;
                    let summary = self
                        .browser
                        .coordinator
                        .get(&run.actor.binding, &lease.session_id)
                        .map_err(execution_error)?;
                    sessions.push(json!({"session":summary,"control_generation":lease.control_generation,"expires_at_ms":lease.expires_at_ms}));
                }
                json!({"sessions":sessions})
            }
            BrowserInvocation::Open { mut options } => {
                options.public_web &= super::service::permits_public_web(obligations);
                let resumed = self
                    .browser
                    .coordinator
                    .resume_for_run(&run.actor, &options, &run.control)
                    .await
                    .map_err(execution_error)?;
                let reused = resumed.is_some();
                let summary = if let Some((summary, _)) = &resumed {
                    summary.clone()
                } else {
                    self.browser
                        .coordinator
                        .open_for_run(&run.actor, options.clone(), &run.control)
                        .await
                        .map_err(execution_error)?
                };
                let lease_ms = self.browser.coordinator.capabilities().limits.max_lease_ms;
                let lease = if let Some((_, lease)) = resumed {
                    lease
                } else {
                    match self.browser.coordinator.grant_control(
                        &run.actor,
                        &summary.session_id,
                        lease_ms,
                    ) {
                        Ok(lease) => lease,
                        Err(error) => {
                            let _ = self
                                .browser
                                .coordinator
                                .close(&run.actor.binding, &summary.session_id)
                                .await;
                            return Err(execution_error(error));
                        }
                    }
                };
                if self
                    .browser
                    .remember_session(
                        &run.actor,
                        BrowserSessionAuthority {
                            lease: lease.clone(),
                            origins: options.allowed_origins,
                            public_web: options.public_web,
                        },
                    )
                    .is_err()
                {
                    let _ = self
                        .browser
                        .coordinator
                        .close(&run.actor.binding, &summary.session_id)
                        .await;
                    return Err(ExecutionError::Failed(
                        "browser run ended during allocation".into(),
                    ));
                }
                if reused {
                    tracing::info!(target: "colossus.browser", event = "session_resumed", run_id = %run.actor.run_id, session_id = %summary.session_id, control_generation = lease.control_generation, "Browser session resumed under a fresh run lease");
                }
                json!({"session": self.browser.coordinator.get(&run.actor.binding, &summary.session_id).map_err(execution_error)?, "control_generation": lease.control_generation, "expires_at_ms": lease.expires_at_ms})
            }
            BrowserInvocation::Status {
                session_id,
                tabs_only,
            } => {
                let summary = self
                    .browser
                    .coordinator
                    .get(&run.actor.binding, &session_id)
                    .map_err(execution_error)?;
                if tabs_only {
                    json!({"session_id": session_id, "tabs": summary.tabs, "selected_tab_id": summary.selected_tab_id, "control_generation": summary.control_generation})
                } else {
                    serde_json::to_value(summary).map_err(encoding_error)?
                }
            }
            BrowserInvocation::Close {
                session_id,
                control_generation,
            } => {
                self.browser
                    .coordinator
                    .close_for_run(&run.actor, &session_id, control_generation)
                    .await
                    .map_err(execution_error)?;
                self.browser.forget_session(&run.actor.run_id, &session_id);
                json!({"session_id": session_id, "closed": true})
            }
            BrowserInvocation::Action {
                session_id,
                control_generation,
                target,
                action,
            } => {
                let lease = self
                    .browser
                    .coordinator
                    .lease(&run.actor, &session_id, control_generation)
                    .map_err(execution_error)?;
                let target = match target {
                    Some(target) => target,
                    None => {
                        let summary = self
                            .browser
                            .coordinator
                            .get(&run.actor.binding, &session_id)
                            .map_err(execution_error)?;
                        let wanted = match &action {
                            BrowserAction::TabSelect { tab_id }
                            | BrowserAction::TabClose { tab_id } => Some(tab_id),
                            _ => summary.selected_tab_id.as_ref(),
                        };
                        let tab = summary
                            .tabs
                            .iter()
                            .find(|tab| Some(&tab.tab_id) == wanted)
                            .ok_or_else(|| {
                                ExecutionError::Failed("owned browser target absent".into())
                            })?;
                        BrowserTarget {
                            tab_id: tab.tab_id.clone(),
                            document_id: tab.document_id.clone(),
                        }
                    }
                };
                if matches!(
                    action,
                    BrowserAction::Screenshot { .. } | BrowserAction::AnnotatedScreenshot { .. }
                ) {
                    return super::screenshot::capture(
                        self.browser,
                        &run,
                        &lease,
                        &target,
                        &action,
                        obligations.max_output_bytes,
                        self.screenshot,
                    )
                    .await;
                }
                if let BrowserAction::Upload {
                    element,
                    artifact_id,
                    ..
                } = action
                {
                    return super::transfer::upload(
                        self.browser,
                        &run,
                        &lease,
                        &target,
                        element,
                        &artifact_id,
                        request,
                        self.upload,
                    )
                    .await;
                }
                if let BrowserAction::Download { element, .. } = action {
                    return super::download::capture(
                        self.browser,
                        &run,
                        &lease,
                        &target,
                        element,
                        obligations.max_output_bytes,
                        self.download,
                    )
                    .await;
                }
                let observation = self
                    .browser
                    .coordinator
                    .execute(&run.actor, &lease, &target, action, &run.control)
                    .await
                    .map_err(execution_error)?;
                super::native::selection::record(self.browser, &run, &lease)?;
                serde_json::to_value(observation).map_err(encoding_error)?
            }
        };
        let bytes = serde_json::to_vec(&value).map_err(encoding_error)?;
        if bytes.len() as u64 > obligations.max_output_bytes {
            return Err(ExecutionError::OutcomeUnknown("browser completed without releasable bounded evidence; automatic retry is prohibited".into()));
        }
        Ok(QuarantinedEffectResult {
            bytes,
            media_type: "application/json".into(),
            effect_succeeded: true,
        })
    }
}

fn encoding_error(_: serde_json::Error) -> ExecutionError {
    ExecutionError::Failed("browser result encoding failed".into())
}
