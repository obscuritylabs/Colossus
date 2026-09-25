//! Native credential entry; secret values never enter `WebView` memory or IPC.

use crate::dto::CommandErrorDto;
use colossus_contracts::HostSecret;
use colossus_native_credential_ui::PromptError;

pub(crate) async fn request_provider_secret(
    parent: tauri::WebviewWindow,
) -> Result<HostSecret, CommandErrorDto> {
    request_managed_credential_secret(parent).await
}

pub(crate) async fn request_managed_credential_secret(
    parent: tauri::WebviewWindow,
) -> Result<HostSecret, CommandErrorDto> {
    colossus_native_credential_ui::prompt(parent)
        .await
        .map_err(|error| {
            let (code, message, retryable) = match error {
                PromptError::Cancelled => (
                    "credential_entry_cancelled",
                    "Credential entry was cancelled.",
                    false,
                ),
                PromptError::Busy => (
                    "credential_entry_busy",
                    "A credential entry window is already open.",
                    true,
                ),
                PromptError::Unavailable => (
                    "credential_entry_unavailable",
                    "The native credential entry window could not open.",
                    true,
                ),
                PromptError::Unsupported => (
                    "provider_enrollment_unsupported",
                    "Native credential enrollment is unavailable on this platform.",
                    false,
                ),
            };
            CommandErrorDto::local_sanitized(code, message, retryable)
        })
}
