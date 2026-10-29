//! Human-triggered, read-only capture. This API grants no agent input capability.
use colossus_ports::ComputerDriverError;

/// A window identity frozen by native host code before Colossus takes focus.
/// Fields are private and cannot be constructed from a renderer-supplied handle.
pub struct ForegroundAppshot {
    #[cfg(windows)]
    pub(crate) window: crate::platform::identity::Window,
    #[cfg(windows)]
    pub(crate) context: crate::TrustedComputerContext,
    #[cfg(windows)]
    pub(crate) title: String,
}

/// Bounded pixels and available visible control labels from the same window.
pub struct HumanAppshot {
    /// PNG encoded by target-only Windows Graphics Capture.
    pub png: Vec<u8>,
    /// Bounded, password-suppressed labels; absent when UIA is unavailable.
    pub text: Option<String>,
    /// Display metadata, never an application identity or authority handle.
    pub window_title: String,
    /// Width in native pixels.
    pub width: u32,
    /// Height in native pixels.
    pub height: u32,
}

/// Freeze the foreground ordinary-user window for one explicit human capture.
pub fn prepare_foreground_appshot() -> Result<ForegroundAppshot, ComputerDriverError> {
    #[cfg(windows)]
    {
        crate::platform::human_appshot::prepare()
    }
    #[cfg(not(windows))]
    Err(ComputerDriverError::Unavailable)
}

impl ForegroundAppshot {
    /// Capture through disposable, image-verified, Job-supervised workers.
    /// Native window identity is revalidated before and after each observation.
    pub fn capture(self, include_text: bool) -> Result<HumanAppshot, ComputerDriverError> {
        #[cfg(windows)]
        {
            crate::platform::human_appshot::capture(self, include_text)
        }
        #[cfg(not(windows))]
        {
            let _ = include_text;
            Err(ComputerDriverError::Unavailable)
        }
    }

    /// Human-readable title for a native confirmation dialog.
    pub fn window_title(&self) -> &str {
        #[cfg(windows)]
        {
            &self.title
        }
        #[cfg(not(windows))]
        {
            ""
        }
    }
}

/// Dispatch only the fixed private read worker before normal Desktop startup.
/// It verifies its real parent and same executable image before any capture.
pub fn run_appshot_worker_if_requested() -> Option<std::process::ExitCode> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("__appshot-read-v1")) {
        return None;
    }
    if args.next().is_some() {
        return Some(std::process::ExitCode::FAILURE);
    }
    #[cfg(windows)]
    return Some(if crate::platform::human_appshot::entry().is_ok() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    });
    #[cfg(not(windows))]
    Some(std::process::ExitCode::FAILURE)
}
