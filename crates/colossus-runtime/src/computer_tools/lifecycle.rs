use super::service::{ComputerRun, RuntimeComputerTools, binding, denied};
use crate::prelude::*;
use colossus_ports::AgentRunLifecycle;
use std::sync::atomic::{AtomicBool, Ordering};

#[async_trait]
impl AgentRunLifecycle for RuntimeComputerTools {
    fn tools_offered(&self, context: &ExecutionContext) -> Result<(), ToolError> {
        self.identity.revalidate().map_err(|_| denied())?;
        let id = context.run_id.as_ref().ok_or_else(denied)?;
        let mut runs = self.runs.lock().map_err(|_| denied())?;
        let run = runs.get_mut(id).ok_or_else(denied)?;
        let mut expected = run.context.clone();
        expected.offered_tools.clone_from(&context.offered_tools);
        if expected != *context {
            return Err(denied());
        }
        if !run.control.is_cancelled() {
            run.context.offered_tools.clone_from(&context.offered_tools);
        }
        Ok(())
    }

    fn begin_run(
        &self,
        context: &ExecutionContext,
        initiator: &Actor,
        control: RunControl,
    ) -> Result<(), ToolError> {
        self.identity.revalidate().map_err(|_| denied())?;
        let binding = binding(self, context, initiator)?;
        let mut runs = self.runs.lock().map_err(|_| denied())?;
        if runs.len() >= 4096 || runs.contains_key(&binding.run_id) {
            return Err(denied());
        }
        runs.insert(
            binding.run_id.clone(),
            ComputerRun {
                binding,
                context: context.clone(),
                control,
                state: Arc::new(StdMutex::new(Default::default())),
                dispatch: Arc::new(TokioMutex::new(())),
                cleanup: Arc::new(TokioMutex::new(())),
                cleanup_scheduled: Arc::new(AtomicBool::new(false)),
            },
        );
        Ok(())
    }

    fn cancel_run(&self, run_id: &str) {
        let run = self
            .runs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(run_id)
            .cloned();
        if let Some(run) = run {
            run.control.cancel();
            self.driver.revoke_run(&run.binding);
            if let Ok(handle) = tokio::runtime::Handle::try_current()
                && !run.cleanup_scheduled.swap(true, Ordering::AcqRel)
            {
                let driver = Arc::clone(&self.driver);
                let runs = Arc::clone(&self.runs);
                handle.spawn(async move {
                    let _guard = run.cleanup.lock().await;
                    if runs
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .get(&run.binding.run_id)
                        .is_none_or(|current| current.binding != run.binding)
                    {
                        return;
                    }
                    if tokio::time::timeout(Duration::from_secs(12), driver.close_run(&run.binding))
                        .await
                        .is_ok_and(|result| result.is_ok())
                    {
                        runs.lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .remove(&run.binding.run_id);
                    }
                });
            }
        }
    }

    async fn finish_run(&self, run_id: &str) {
        self.cancel_run(run_id);
        let run = self
            .runs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(run_id)
            .cloned();
        if let Some(run) = run {
            let cleanup = async {
                let _guard = run.cleanup.lock().await;
                if self
                    .runs
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .get(run_id)
                    .is_none_or(|current| current.binding != run.binding)
                {
                    return;
                }
                if self.driver.close_run(&run.binding).await.is_ok() {
                    self.runs
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .remove(run_id);
                }
            };
            let _ = tokio::time::timeout(Duration::from_secs(12), cleanup).await;
        }
    }
}
