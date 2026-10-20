//! Native page lifecycle receipts used only behind the supervisor's egress fence.
use super::*;

impl Host {
    pub fn suspend(&mut self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        if self.session.as_ref() != Some(session) || self.cancelled.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Stale);
        }
        if self.suspended {
            return Ok(());
        }
        self.revoke_human_presentation();
        self.expected_revision = None;
        let native: Vec<_> = self.tabs.values().map(|tab| tab.native).collect();
        let control = BrowserDriverControl::new(Default::default(), Default::default());
        for tab in native {
            // A fixed internal lifecycle command, never model-selectable CDP.
            // The external proxy has already drained every active connection.
            self.method(
                tab,
                "Page.setWebLifecycleState",
                json!({"state":"frozen"}),
                &control,
            )?;
        }
        for tab in self.tabs.values_mut() {
            tab.snapshot = None;
            tab.elements.clear();
        }
        self.screenshot = None;
        self.suspended = true;
        Ok(())
    }

    pub fn resume(
        &mut self,
        session: &BrowserSessionId,
        generation: u64,
    ) -> Result<(), BrowserDriverError> {
        if self.session.as_ref() != Some(session)
            || !self.suspended
            || self.cancelled.load(Ordering::Acquire)
            || generation <= self.presentation.control_generation
        {
            return Err(BrowserDriverError::Stale);
        }
        let native: Vec<_> = self.tabs.values().map(|tab| tab.native).collect();
        let control = BrowserDriverControl::new(Default::default(), Default::default());
        for tab in native {
            self.method(
                tab,
                "Page.setWebLifecycleState",
                json!({"state":"active"}),
                &control,
            )?;
        }
        self.begin_agent_action(generation);
        self.suspended = false;
        Ok(())
    }
}
