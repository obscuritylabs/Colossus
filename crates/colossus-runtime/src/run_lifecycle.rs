//! One lifecycle owns process, browser, and computer cleanup for every host combination.

use crate::{
    ProcessSessions, browser_tools::RuntimeBrowserTools, computer_tools::RuntimeComputerTools,
    prelude::*,
};
use colossus_ports::AgentRunLifecycle;

#[cfg(test)]
mod tests;

pub(crate) struct RuntimeRunLifecycle {
    processes: Arc<ProcessSessions>,
    browser: Option<Arc<RuntimeBrowserTools>>,
    computer: Option<Arc<RuntimeComputerTools>>,
    sessions: Option<Arc<crate::native_session_lifecycle::NativeSessionLifecycle>>,
}

impl RuntimeRunLifecycle {
    pub(crate) fn new(
        processes: Arc<ProcessSessions>,
        browser: Option<Arc<RuntimeBrowserTools>>,
        computer: Option<Arc<RuntimeComputerTools>>,
    ) -> Self {
        Self {
            processes,
            browser,
            computer,
            sessions: None,
        }
    }

    pub(crate) fn with_session_lifecycle(
        mut self,
        sessions: Arc<crate::native_session_lifecycle::NativeSessionLifecycle>,
    ) -> Self {
        self.sessions = Some(sessions);
        self
    }

    fn release_session_after_cleanup(&self, run_id: &str) {
        if self
            .browser
            .as_ref()
            .is_some_and(|browser| browser.has_run_registration(run_id))
            || self
                .computer
                .as_ref()
                .is_some_and(|computer| computer.has_run_registration(run_id))
        {
            return;
        }
        if let Some(sessions) = &self.sessions {
            sessions.finish_run(run_id);
        }
    }
}

#[async_trait]
impl AgentRunLifecycle for RuntimeRunLifecycle {
    fn tools_offered(&self, context: &ExecutionContext) -> Result<(), ToolError> {
        if let Some(browser) = &self.browser {
            browser.tools_offered(context)?;
        }
        if let Some(computer) = &self.computer {
            computer.tools_offered(context)?;
        }
        Ok(())
    }

    fn begin_run(
        &self,
        context: &ExecutionContext,
        initiator: &Actor,
        control: RunControl,
    ) -> Result<(), ToolError> {
        if let Some(sessions) = &self.sessions {
            sessions.begin_run(context)?;
        }
        self.processes
            .begin_run(context, initiator, control.clone())
            .inspect_err(|_| {
                if let Some(sessions) = &self.sessions
                    && let Some(id) = context.run_id.as_deref()
                {
                    sessions.finish_run(id);
                }
            })?;
        let mut browser_registered = false;
        let result = (|| {
            if let Some(browser) = &self.browser {
                browser.begin_run(context, initiator, control.clone())?;
                browser_registered = true;
            }
            if let Some(computer) = &self.computer {
                computer.begin_run(context, initiator, control)?;
            }
            Ok(())
        })();
        if result.is_err()
            && let Some(id) = context.run_id.as_deref()
        {
            // These synchronous begin methods only register metadata. Roll back
            // exactly our successful registrations; a rejected duplicate adapter
            // may still belong to another owner and must never be cancelled here.
            let browser_removed = !browser_registered
                || self
                    .browser
                    .as_ref()
                    .is_some_and(|browser| browser.rollback_run_registration(context));
            let process_removed = self.processes.rollback_run_registration(context, initiator);
            if browser_removed
                && process_removed
                && let Some(sessions) = &self.sessions
            {
                sessions.finish_run(id);
            }
        }
        result
    }

    fn cancel_run(&self, run_id: &str) {
        // Synchronous authority fences precede every asynchronous cleanup operation.
        if let Some(computer) = &self.computer {
            computer.cancel_run(run_id);
        }
        if let Some(browser) = &self.browser {
            browser.cancel_run(run_id);
        }
        self.processes.cancel_run(run_id);
    }

    async fn finish_run(&self, run_id: &str) {
        self.cancel_run(run_id);
        if let Some(computer) = &self.computer {
            computer.finish_run(run_id).await;
        }
        if let Some(browser) = &self.browser {
            browser.finish_run(run_id).await;
        }
        self.processes.finish_run(run_id).await;
        self.release_session_after_cleanup(run_id);
    }

    async fn complete_run(&self, run_id: &str) {
        // Browser suspension must precede cleanup owners that cancel the shared
        // run signal; cancelled work must always take the terminal browser path.
        if let Some(browser) = &self.browser {
            browser.complete_run(run_id).await;
        }
        if let Some(computer) = &self.computer {
            computer.finish_run(run_id).await;
        }
        self.processes.finish_run(run_id).await;
        self.release_session_after_cleanup(run_id);
    }
}
