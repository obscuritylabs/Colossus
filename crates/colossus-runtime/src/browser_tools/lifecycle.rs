use super::service::{BrowserRun, RuntimeBrowserTools, binding, denied};
use crate::prelude::*;
use colossus_contracts::BrowserActor;
use colossus_ports::AgentRunLifecycle;
use std::sync::atomic::{AtomicBool, Ordering};

impl RuntimeBrowserTools {
    pub(crate) fn has_run_registration(&self, id: &str) -> bool {
        self.runs.lock().map_or(true, |runs| runs.contains_key(id))
    }

    /// Registration rollback is only valid before model/tool dispatch has begun.
    pub(crate) fn rollback_run_registration(&self, context: &ExecutionContext) -> bool {
        let Some(id) = context.run_id.as_deref() else {
            return false;
        };
        let Ok(mut runs) = self.runs.lock() else {
            return false;
        };
        if runs
            .get(id)
            .is_some_and(|run| run.context == *context && run.sessions.is_empty())
        {
            runs.remove(id);
            return true;
        }
        false
    }
}

#[async_trait]
impl AgentRunLifecycle for RuntimeBrowserTools {
    async fn complete_run(&self, run_id: &str) {
        let run = self
            .runs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(run_id)
            .cloned();
        let Some(run) = run else {
            return;
        };
        if run.control.is_cancelled() {
            self.finish_run(run_id).await;
            return;
        }
        let guard = run.cleanup.lock().await;
        if run.control.is_cancelled() {
            drop(guard);
            self.finish_run(run_id).await;
            return;
        }
        if self.coordinator.complete_run(&run.actor).await.is_ok() && !run.control.is_cancelled() {
            for session_id in run.sessions.keys() {
                if let Ok(summary) = self.coordinator.get(&run.actor.binding, session_id)
                    && summary.control == colossus_contracts::BrowserControlState::Paused
                {
                    tracing::info!(target: "colossus.browser", event = "session_suspended", run_id, session_id = %session_id, control_generation = summary.control_generation, "Browser session suspended after completed run");
                }
            }
            super::native::lifecycle::suspend_run(&self.native, &run);
            let removed = {
                let mut runs = self.runs.lock().unwrap_or_else(|error| error.into_inner());
                if run.control.is_cancelled() {
                    false
                } else {
                    runs.remove(run_id);
                    true
                }
            };
            if removed {
                return;
            }
        }
        let category = if run.control.is_cancelled() {
            "cancelled"
        } else {
            "cleanup_required"
        };
        tracing::warn!(target: "colossus.browser", event = "session_suspend_failed", run_id, category, "Browser completed-run retention stopped; closing owned sessions");
        drop(guard);
        self.finish_run(run_id).await;
    }
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
            // Failed, cancelled and unsupported runs retain terminal cleanup.
            // Successful supported conversations use complete_run's suspension fence.
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
