use super::{
    arguments::BrowserInvocation,
    service::{BrowserSessionAuthority, RuntimeBrowserTools, execution_error},
};
use crate::prelude::*;
use colossus_contracts::{BrowserAction, BrowserTarget};

pub(crate) struct BrowserToolExecutor {
    pub(crate) gateway: Arc<EffectGateway>,
    pub(crate) registry: Arc<dyn ToolRegistry>,
    pub(crate) browser: Arc<RuntimeBrowserTools>,
    pub(crate) inner: Arc<dyn ToolExecutor>,
}

#[async_trait]
impl ToolExecutor for BrowserToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        if !call.name.starts_with("browser.") {
            return self.inner.execute(call, context).await;
        }
        self.registry.validate(&call)?;
        if !context.offered_tools.contains(&call.name) {
            return Err(ToolError::Denied(
                "browser operation exceeds the active tool ceiling".into(),
            ));
        }
        self.browser.run(&context)?;
        let invocation = BrowserInvocation::from_call(&call)?;
        let mut request = effect_request(
            crate::model_actor(&call, &context),
            invocation.action(),
            invocation.resource(),
            serde_json::to_value(&invocation)
                .map_err(|_| ToolError::Failed("browser request encoding failed".into()))?,
        );
        request.capabilities = vec![invocation.action().into()];
        request.context = context;
        let released = self
            .gateway
            .execute(
                request,
                &BrowserEffect {
                    browser: &self.browser,
                },
            )
            .await
            .map_err(crate::tool_gateway_error)?;
        let output = String::from_utf8(released.bytes)
            .map_err(|_| ToolError::Failed("browser returned invalid released output".into()))?;
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output,
            exit_code: 0,
        })
    }
}

struct BrowserEffect<'a> {
    browser: &'a RuntimeBrowserTools,
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
        let invocation: BrowserInvocation = serde_json::from_value(request.content.clone())
            .map_err(|_| ExecutionError::Failed("invalid prepared browser operation".into()))?;
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
        let origins = match &invocation {
            BrowserInvocation::Open { options } => &options.allowed_origins,
            _ => {
                &authority
                    .as_ref()
                    .ok_or_else(|| {
                        ExecutionError::Failed("browser session authority absent".into())
                    })?
                    .origins
            }
        };
        // Native egress remains this exact immutable envelope, even when the
        // surrounding runtime acknowledges ambient resources or a public wildcard.
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
            BrowserInvocation::Open { options } => {
                let summary = self
                    .browser
                    .coordinator
                    .open_for_run(&run.actor, options.clone(), &run.control)
                    .await
                    .map_err(execution_error)?;
                let lease_ms = self.browser.coordinator.capabilities().limits.max_lease_ms;
                let lease = match self.browser.coordinator.grant_control(
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
                };
                if self
                    .browser
                    .remember_session(
                        &run.actor,
                        BrowserSessionAuthority {
                            lease: lease.clone(),
                            origins: options.allowed_origins,
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
                serde_json::to_value(
                    self.browser
                        .coordinator
                        .execute(&run.actor, &lease, &target, action, &run.control)
                        .await
                        .map_err(execution_error)?,
                )
                .map_err(encoding_error)?
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
