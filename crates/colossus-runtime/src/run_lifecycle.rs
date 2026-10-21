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
        self.processes
            .begin_run(context, initiator, control.clone())?;
        let result = (|| {
            if let Some(browser) = &self.browser {
                browser.begin_run(context, initiator, control.clone())?;
            }
            if let Some(computer) = &self.computer {
                computer.begin_run(context, initiator, control)?;
            }
            Ok(())
        })();
        if result.is_err()
            && let Some(id) = context.run_id.as_deref()
        {
            self.cancel_run(id);
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
    }
}
