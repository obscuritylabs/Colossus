#[cfg(any(windows, target_os = "macos"))]
use std::sync::{Arc, atomic::AtomicBool};

#[cfg(any(windows, target_os = "macos"))]
use crate::ColorScheme;
use crate::DialogAppearance;
use colossus_contracts::HostSecret;

#[cfg(any(windows, target_os = "macos"))]
use crate::lifecycle::{CancelOnDrop, Completion};

/// Categorical native errors; never contain credential data or platform errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptError {
    Cancelled,
    Busy,
    Unavailable,
    Unsupported,
}

impl std::fmt::Display for PromptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "Credential entry was cancelled.",
            Self::Busy => "A credential entry window is already open.",
            Self::Unavailable => "The native credential entry window could not open.",
            Self::Unsupported => "Native credential entry is unavailable on this platform.",
        })
    }
}

impl std::error::Error for PromptError {}

/// Open one native credential dialog owned by `parent` and await its result.
///
/// Creation and native callbacks run on Tauri's UI thread without a nested or
/// blocking event loop. Dropping this future cancels the native dialog. Its
/// lifetime retains the global single-dialog lease until cleanup has completed.
///
/// # Errors
/// Returns a categorical error if native entry is unsupported, already active,
/// cancelled, or could not open. Errors never contain the entered value.
pub async fn prompt(
    parent: tauri::WebviewWindow,
    appearance: DialogAppearance,
) -> Result<HostSecret, PromptError> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (parent, appearance);
        return Err(PromptError::Unsupported);
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        let mut appearance = appearance;
        if appearance.color_scheme == ColorScheme::System {
            appearance.color_scheme = if parent.theme().ok() == Some(tauri::Theme::Dark) {
                ColorScheme::Dark
            } else {
                ColorScheme::Light
            };
        }
        let (completion, receiver) = Completion::acquire()?;
        let cancellation = CancelOnDrop(Arc::new(AtomicBool::new(false)));
        let cancelled = cancellation.0.clone();
        let native_parent = parent.clone();
        parent
            .run_on_main_thread(move || {
                #[cfg(windows)]
                crate::windows::open(&native_parent, cancelled, completion, appearance);
                #[cfg(target_os = "macos")]
                crate::macos::open(&native_parent, cancelled, completion, appearance);
            })
            .map_err(|_| PromptError::Unavailable)?;
        receiver.await.map_err(|_| PromptError::Cancelled)?
    }
}
