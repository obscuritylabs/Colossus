use colossus_ports::BrowserDriver;
use std::{fmt, sync::Arc};

/// Native-composition browser driver binding. It carries no model-facing authority.
///
/// Construct this only after verifying the component, private transport, engine
/// sandbox, and egress containment. Drivers remain unavailable unless they report
/// those capabilities. No executable path or certificate material enters runtime YAML.
#[derive(Clone)]
pub struct RuntimeBrowserHost {
    pub(super) driver: Arc<dyn BrowserDriver>,
}

impl RuntimeBrowserHost {
    /// Bind a driver selected and verified by the trusted runtime host.
    #[must_use]
    pub fn new(driver: Arc<dyn BrowserDriver>) -> Self {
        Self { driver }
    }
}

impl fmt::Debug for RuntimeBrowserHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeBrowserHost")
            .finish_non_exhaustive()
    }
}
