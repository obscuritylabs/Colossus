#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use std::sync::{Arc, atomic::AtomicBool};

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
use crate::ColorScheme;
use crate::{DialogAppearance, NativePassword, purpose::Purpose};
use colossus_contracts::HostSecret;

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
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
    parent: tauri::Window,
    appearance: DialogAppearance,
) -> Result<HostSecret, PromptError> {
    prompt_for(parent, appearance, Purpose::Token)
        .await?
        .into_token()
}

/// Enter a PKCS#12 passphrase in a native secure control without saving it.
///
/// Spaces, Unicode and an empty passphrase are preserved. The UTF-8 value is
/// bounded to 65,536 bytes; NUL, line breaks and control characters are rejected.
/// Native custody, cancellation and the exclusive dialog lease match [`prompt`].
///
/// # Errors
/// Returns only categorical native entry failures, never the entered value.
pub async fn prompt_password(
    parent: tauri::Window,
    appearance: DialogAppearance,
) -> Result<NativePassword, PromptError> {
    prompt_for(parent, appearance, Purpose::Pkcs12Password).await
}

async fn prompt_for(
    parent: tauri::Window,
    appearance: DialogAppearance,
    purpose: Purpose,
) -> Result<NativePassword, PromptError> {
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (parent, appearance, purpose);
        return Err(PromptError::Unsupported);
    }
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
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
                crate::windows::open(&native_parent, cancelled, completion, appearance, purpose);
                #[cfg(target_os = "macos")]
                crate::macos::open(&native_parent, cancelled, completion, appearance, purpose);
                #[cfg(target_os = "linux")]
                crate::linux::open(&native_parent, cancelled, completion, appearance, purpose);
            })
            .map_err(|_| PromptError::Unavailable)?;
        receiver.await.map_err(|_| PromptError::Cancelled)?
    }
}
