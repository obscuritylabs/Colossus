use super::*;

const OWNER_LOST: &str = "runtime ownership was lost after execution began";

impl EffectGateway {
    /// Bind the trusted shared execution lease. Native runtimes keep their
    /// existing local ownership boundary when no distributed fence is supplied.
    #[must_use]
    pub fn with_execution_fence(
        mut self,
        fence: Arc<dyn colossus_ports::RuntimeExecutionFence>,
    ) -> Self {
        self.execution_fence = Some(fence);
        self
    }

    pub(super) fn verify_execution_fence(&self) -> Result<(), GatewayError> {
        if let Some(fence) = &self.execution_fence {
            fence.verify()?;
        }
        Ok(())
    }

    pub(super) fn verify_started_owner(&self) -> Result<(), GatewayError> {
        self.verify_execution_fence()
            .map_err(|_| GatewayError::OutcomeUnknown(OWNER_LOST.into()))
    }

    pub(super) async fn execute_owned(
        &self,
        request: &EffectRequest,
        executor: &dyn EffectExecutor,
        permit: ExecutionPermit,
        timeout_ms: u64,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        self.verify_started_owner()
            .map_err(|_| ExecutionError::OutcomeUnknown(OWNER_LOST.into()))?;
        let execution = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            executor.execute(request, permit),
        );
        tokio::pin!(execution);
        let outcome = if self.execution_fence.is_some() {
            let mut ownership = tokio::time::interval(Duration::from_millis(100));
            ownership.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    biased;
                    // Poll the adapter even when a slow lease verification has
                    // delayed the timer. The post-result fence still guards all
                    // output; a perpetually-ready timer must not starve I/O.
                    result = &mut execution => break result,
                    _ = ownership.tick() => {
                        if self.verify_execution_fence().is_err() {
                            // Drop the adapter future. Its durable start remains
                            // unresolved if the fenced journal refuses a terminal
                            // append; takeover must reconcile it, never replay it.
                            return Err(ExecutionError::OutcomeUnknown(OWNER_LOST.into()));
                        }
                    }
                }
            }
        } else {
            execution.await
        };
        self.verify_started_owner()
            .map_err(|_| ExecutionError::OutcomeUnknown(OWNER_LOST.into()))?;
        outcome.unwrap_or_else(|_| {
            Err(ExecutionError::OutcomeUnknown(
                "adapter timed out after execution began".into(),
            ))
        })
    }
}

#[cfg(test)]
mod tests;
