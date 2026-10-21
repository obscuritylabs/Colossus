//! Ensure early native startup errors are logged before the subscriber drains.

#[derive(Default)]
pub(super) struct StartupLog {
    ready: bool,
}

impl StartupLog {
    pub(super) fn ready(&mut self) {
        self.ready = true;
        tracing::info!(target: "colossus.lifecycle", event_name = "sidecar.ready");
    }
}

impl Drop for StartupLog {
    fn drop(&mut self) {
        if !self.ready {
            tracing::error!(target: "colossus.lifecycle", event_name = "sidecar.startup_failed", error_type = "startup_incomplete");
        }
    }
}
