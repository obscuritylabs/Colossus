use super::{
    arguments::ComputerInvocation,
    effect::{ComputerEffect, StagedComputerReply},
    service::RuntimeComputerTools,
};
use crate::prelude::*;
use colossus_contracts::{ComputerControlState, ComputerStopReason};
use colossus_ports::AgentRunLifecycle;

pub(crate) struct ComputerToolExecutor {
    pub(crate) gateway: Arc<EffectGateway>,
    pub(crate) registry: Arc<dyn ToolRegistry>,
    pub(crate) computer: Arc<RuntimeComputerTools>,
    pub(crate) journal: Arc<dyn EventJournal>,
    pub(crate) inner: Arc<dyn ToolExecutor>,
}

#[async_trait]
impl ToolExecutor for ComputerToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        if !call.name.starts_with("computer.") {
            return self.inner.execute(call, context).await;
        }
        self.registry.validate(&call)?;
        if !context.offered_tools.contains(&call.name) {
            return Err(super::service::denied());
        }
        let run = self.computer.run(&context)?;
        let _dispatch = run.dispatch.lock().await;
        self.computer.run(&context)?;
        let invocation = ComputerInvocation::from_call(&call, &context, &self.computer, &run)?;
        if invocation.action() != call.name {
            return Err(super::service::denied());
        }
        if call.name == "computer.screenshot" {
            super::capture::admit(&self.computer, &context)?;
        }
        let mut request = effect_request(
            crate::model_actor(&call, &context),
            invocation.action(),
            invocation.resource(),
            serde_json::to_value(&invocation).map_err(|_| super::service::denied())?,
        );
        request.capabilities = vec![invocation.action().into()];
        request.context = context.clone();
        let staged = StdMutex::new(None);
        let result = self
            .gateway
            .execute(
                request,
                &ComputerEffect {
                    computer: &self.computer,
                    staged: &staged,
                },
            )
            .await;
        let receipt = staged.into_inner().map_err(|_| unknown())?;
        let released = match result {
            Ok(released) => released,
            Err(error) => {
                if receipt.is_some() {
                    self.computer.cancel_run(&run.binding.run_id);
                }
                return Err(crate::tool_gateway_error(error));
            }
        };
        self.computer.run(&context).map_err(|_| unknown())?;
        let receipt = receipt.ok_or_else(unknown)?;
        if let StagedComputerReply::Screenshot(descriptor, observed_at) = receipt {
            let result = super::capture::publish(
                self,
                call,
                context,
                &run,
                descriptor,
                released.bytes,
                observed_at,
            )
            .await;
            if result.is_err() {
                self.computer.cancel_run(&run.binding.run_id);
            }
            return result;
        }
        let user_stopped = call.name == "computer.status"
            && matches!(
                &receipt,
                StagedComputerReply::Session(session)
                    if session.state == ComputerControlState::Stopped
                        && session.reason == Some(ComputerStopReason::UserStopped)
            );
        super::effect::commit(&run, receipt).map_err(|_| unknown())?;
        if user_stopped {
            self.computer.cancel_run(&run.binding.run_id);
        }
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output: String::from_utf8(released.bytes).map_err(|_| unknown())?,
            exit_code: 0,
            images: Vec::new(),
        })
    }
}

pub(super) fn unknown() -> ToolError {
    ToolError::OutcomeUnknown(
        "computer evidence publication did not positively complete; automatic retry is prohibited"
            .into(),
    )
}
