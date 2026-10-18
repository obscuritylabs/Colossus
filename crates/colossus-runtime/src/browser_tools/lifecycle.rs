use super::service::{BrowserRun, RuntimeBrowserTools, binding, denied};
use crate::prelude::*;
use colossus_contracts::BrowserActor;
use colossus_ports::AgentRunLifecycle;
use std::sync::atomic::{AtomicBool, Ordering};

#[async_trait]
impl AgentRunLifecycle for RuntimeBrowserTools {
    fn tools_offered(&self, context: &ExecutionContext) -> Result<(), ToolError> {
        self.identity.revalidate().map_err(|_| denied())?;
        let id = context.run_id.as_deref().ok_or_else(denied)?;
        let mut runs = self.runs.lock().map_err(|_| denied())?;
        let run = runs.get_mut(id).ok_or_else(denied)?;
        let mut expected = run.context.clone();
        expected.offered_tools.clone_from(&context.offered_tools);
        if expected != *context {
            return Err(denied());
        }
        // Cancellation may race trusted turn preparation. The engine must still
        // finish its cancellation path; a revoked run never regains a tool ceiling.
        if run.control.is_cancelled() {
            return Ok(());
        }
        run.context.offered_tools.clone_from(&context.offered_tools);
        Ok(())
    }
    fn begin_run(
        &self,
        context: &ExecutionContext,
        initiator: &Actor,
        control: RunControl,
    ) -> Result<(), ToolError> {
        self.identity.revalidate().map_err(|_| denied())?;
        let run_id = context.run_id.clone().ok_or_else(denied)?;
        let actor = BrowserActor {
            binding: binding(self, context, initiator)?,
            run_id: run_id.clone(),
        };
        let mut runs = self.runs.lock().map_err(|_| denied())?;
        if runs.len() >= 4096 || runs.contains_key(&run_id) {
            return Err(denied());
        }
        runs.insert(
            run_id,
            BrowserRun {
                actor,
                context: context.clone(),
                control,
                sessions: BTreeMap::new(),
                cleanup_scheduled: Arc::new(AtomicBool::new(false)),
                cleanup: Arc::new(TokioMutex::new(())),
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
            let _ = self.coordinator.revoke_run(&run.actor);
            // Dropping the agent future still schedules bounded native cleanup.
            // The synchronous revocation above already prevents new dispatch.
            if let Ok(handle) = tokio::runtime::Handle::try_current()
                && !run.cleanup_scheduled.swap(true, Ordering::AcqRel)
            {
                let coordinator = Arc::clone(&self.coordinator);
                let cleanup = Arc::clone(&run.cleanup);
                let runs = Arc::clone(&self.runs);
                let native = Arc::clone(&self.native);
                handle.spawn(async move {
                    let _guard = cleanup.lock().await;
                    if coordinator.finish_run(&run.actor).await.is_ok() {
                        super::native::lifecycle::retire_run(&native, &run);
                        let mut runs = runs.lock().unwrap_or_else(|error| error.into_inner());
                        if runs
                            .get(&run.actor.run_id)
                            .is_some_and(|current| current.actor == run.actor)
                        {
                            runs.remove(&run.actor.run_id);
                        }
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
            // Ephemeral run-owned contexts have no implicit background grant. Close
            // after normal completion as well as cancellation; driver cleanup is bounded.
            let _guard = run.cleanup.lock().await;
            if self.coordinator.finish_run(&run.actor).await.is_ok() {
                super::native::lifecycle::retire_run(&self.native, &run);
                self.runs
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(run_id);
            }
        }
    }
}
