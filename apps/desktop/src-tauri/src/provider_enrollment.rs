//! Native credential entry; secret values never enter `WebView` memory or IPC.

use crate::dto::CommandErrorDto;
use colossus_contracts::HostSecret;
use colossus_native_credential_ui::{ColorScheme, DialogAppearance, PromptError, TextSize};
use serde::Deserialize;

/// Only bounded, non-secret appearance preferences cross the renderer boundary.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DialogAppearanceInput {
    color_scheme: ColorSchemeInput,
    text_size: TextSizeInput,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ColorSchemeInput {
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum TextSizeInput {
    Compact,
    Comfortable,
    Large,
}

impl From<DialogAppearanceInput> for DialogAppearance {
    fn from(value: DialogAppearanceInput) -> Self {
        Self {
            color_scheme: match value.color_scheme {
                ColorSchemeInput::System => ColorScheme::System,
                ColorSchemeInput::Dark => ColorScheme::Dark,
                ColorSchemeInput::Light => ColorScheme::Light,
            },
            text_size: match value.text_size {
                TextSizeInput::Compact => TextSize::Compact,
                TextSizeInput::Comfortable => TextSize::Comfortable,
                TextSizeInput::Large => TextSize::Large,
            },
        }
    }
}

pub(crate) async fn request_provider_secret(
    parent: tauri::Window,
    appearance: DialogAppearance,
) -> Result<HostSecret, CommandErrorDto> {
    request_managed_credential_secret(parent, appearance).await
}

pub(crate) async fn request_managed_credential_secret(
    parent: tauri::Window,
    appearance: DialogAppearance,
) -> Result<HostSecret, CommandErrorDto> {
    colossus_native_credential_ui::prompt(parent, appearance)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dialog_appearance_accepts_only_bounded_non_secret_preferences() {
        for color in ["system", "dark", "light"] {
            for size in ["compact", "comfortable", "large"] {
                let input: DialogAppearanceInput = serde_json::from_value(json!({
                    "colorScheme": color,
                    "textSize": size,
                }))
                .unwrap();
                let _: DialogAppearance = input.into();
            }
        }
        for input in [
            json!({"colorScheme": "custom-css", "textSize": "large"}),
            json!({"colorScheme": "dark", "textSize": 1000}),
            json!({"colorScheme": "dark", "textSize": "comfortable", "secret": "rejected"}),
            json!({"colorScheme": "dark"}),
        ] {
            assert!(serde_json::from_value::<DialogAppearanceInput>(input).is_err());
        }
    }
}
