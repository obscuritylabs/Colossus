//! Native credential entry; secret values never enter `WebView` memory or IPC.

use crate::dto::CommandErrorDto;
use crate::{desktop_settings::ProviderKindSetting, managed_configuration::CredentialKindSetting};
use colossus_contracts::HostSecret;
use colossus_native_credential_ui::{
    ColorScheme, CredentialProvider, DialogAppearance, PromptError, ProviderCredentialContext,
    TextSize,
};
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
    destination: ProviderCredentialDestination,
) -> Result<AcceptedProviderSecret, CommandErrorDto> {
    let secret = colossus_native_credential_ui::prompt_provider(
        parent,
        appearance,
        destination.presentation.clone(),
    )
    .await
    .map_err(prompt_error)?;
    Ok(AcceptedProviderSecret {
        secret,
        destination: AcceptedProviderDestination(destination),
    })
}

/// Native-validated immutable context; this type contains no credential bytes.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProviderCredentialDestination {
    kind: ProviderKindSetting,
    base_url: String,
    presentation: ProviderCredentialContext,
}

impl ProviderCredentialDestination {
    pub(crate) fn new(kind: ProviderKindSetting, base_url: &str) -> Result<Self, CommandErrorDto> {
        let provider = match kind {
            ProviderKindSetting::Compatible => CredentialProvider::OpenAiCompatible,
            ProviderKindSetting::Responses => CredentialProvider::OpenAiResponses,
            ProviderKindSetting::Anthropic => CredentialProvider::AnthropicMessages,
            ProviderKindSetting::Codex => return Err(destination_error()),
        };
        // URL serialization exposes the exact ASCII destination (including IDNA
        // host and escaped path), preventing invisible characters in trusted copy.
        let canonical = canonical_provider_endpoint(base_url)?;
        let presentation = ProviderCredentialContext::new(provider, &canonical)
            .map_err(|_| destination_error())?;
        Ok(Self {
            kind,
            base_url: base_url.into(),
            presentation,
        })
    }
}

/// Display validated destinations in the same ASCII form used by the transport.
pub(crate) fn canonical_provider_endpoint(base_url: &str) -> Result<String, CommandErrorDto> {
    colossus_sdk::validate_managed_provider_base_url(base_url).map_err(|_| destination_error())?;
    url::Url::parse(base_url)
        .map(|url| url.to_string())
        .map_err(|_| destination_error())
}

/// Created only after the matching native prompt accepts the key.
pub(crate) struct AcceptedProviderDestination(ProviderCredentialDestination);

impl AcceptedProviderDestination {
    pub(crate) fn kind(&self) -> ProviderKindSetting {
        self.0.kind
    }
    pub(crate) fn base_url(&self) -> &str {
        &self.0.base_url
    }
    fn matches(&self, expected: &ProviderCredentialDestination) -> bool {
        self.0 == *expected
    }

    #[cfg(test)]
    pub(crate) fn synthetic(kind: ProviderKindSetting, base_url: &str) -> Self {
        Self(ProviderCredentialDestination::new(kind, base_url).unwrap())
    }
}

pub(crate) struct AcceptedProviderSecret {
    secret: HostSecret,
    destination: AcceptedProviderDestination,
}

impl AcceptedProviderSecret {
    pub(crate) fn into_secret_for(
        self,
        expected: &ProviderCredentialDestination,
    ) -> Result<HostSecret, CommandErrorDto> {
        if !self.destination.matches(expected) {
            return Err(destination_error());
        }
        Ok(self.secret)
    }
}

pub(crate) enum CredentialEnrollment {
    Unbound(CredentialKindSetting),
    Provider(ProviderCredentialDestination),
}

impl CredentialEnrollment {
    pub(crate) fn kind(&self) -> CredentialKindSetting {
        match self {
            Self::Unbound(kind) => *kind,
            Self::Provider(_) => CredentialKindSetting::ApiKey,
        }
    }

    pub(crate) async fn request(
        self,
        parent: tauri::Window,
        appearance: DialogAppearance,
    ) -> Result<(HostSecret, Option<AcceptedProviderDestination>), CommandErrorDto> {
        match self {
            Self::Unbound(_) => Ok((
                request_managed_credential_secret(parent, appearance).await?,
                None,
            )),
            Self::Provider(destination) => {
                let accepted = request_provider_secret(parent, appearance, destination).await?;
                Ok((accepted.secret, Some(accepted.destination)))
            }
        }
    }
}

pub(crate) struct EnrolledCredential {
    pub(crate) id: String,
    pub(crate) destination: Option<AcceptedProviderDestination>,
}

impl EnrolledCredential {
    pub(crate) fn provider_destination(
        &self,
        expected: &ProviderCredentialDestination,
    ) -> Result<&AcceptedProviderDestination, CommandErrorDto> {
        self.destination
            .as_ref()
            .filter(|destination| destination.matches(expected))
            .ok_or_else(destination_error)
    }
}

fn destination_error() -> CommandErrorDto {
    CommandErrorDto::invalid(
        "providerOrigin",
        "The native credential destination is missing or changed. Review the provider endpoint and enter the key again.",
    )
}

pub(crate) async fn request_managed_credential_secret(
    parent: tauri::Window,
    appearance: DialogAppearance,
) -> Result<HostSecret, CommandErrorDto> {
    colossus_native_credential_ui::prompt(parent, appearance)
        .await
        .map_err(prompt_error)
}

fn prompt_error(error: PromptError) -> CommandErrorDto {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_secret_and_enrollment_require_the_same_accepted_destination() {
        let destination = ProviderCredentialDestination::new(
            ProviderKindSetting::Compatible,
            "https://models.example.test/v1",
        )
        .unwrap();
        let changed = [
            ProviderCredentialDestination::new(
                ProviderKindSetting::Responses,
                "https://models.example.test/v1",
            )
            .unwrap(),
            ProviderCredentialDestination::new(
                ProviderKindSetting::Compatible,
                "https://other.example.test/v1",
            )
            .unwrap(),
            ProviderCredentialDestination::new(
                ProviderKindSetting::Compatible,
                "https://models.example.test/other",
            )
            .unwrap(),
        ];
        let unbound = EnrolledCredential {
            id: "synthetic-unbound".into(),
            destination: None,
        };
        assert!(unbound.provider_destination(&destination).is_err());
        for expected in changed {
            let accepted = AcceptedProviderSecret {
                secret: HostSecret::new("PRIVATE_TOKEN").unwrap(),
                destination: AcceptedProviderDestination(destination.clone()),
            };
            let error = accepted
                .into_secret_for(&expected)
                .expect_err("changed destination");
            assert_eq!(error.code, "invalid_argument");
            assert!(!error.message.contains("PRIVATE_TOKEN"));
            let enrolled = EnrolledCredential {
                id: "synthetic-key".into(),
                destination: Some(AcceptedProviderDestination(destination.clone())),
            };
            assert!(enrolled.provider_destination(&expected).is_err());
        }
        let accepted = AcceptedProviderSecret {
            secret: HostSecret::new("PRIVATE_TOKEN").unwrap(),
            destination: AcceptedProviderDestination(destination.clone()),
        };
        assert_eq!(
            accepted.into_secret_for(&destination).unwrap().expose(),
            "PRIVATE_TOKEN"
        );
        let enrolled = EnrolledCredential {
            id: "synthetic-key".into(),
            destination: Some(AcceptedProviderDestination(destination.clone())),
        };
        let context = enrolled.provider_destination(&destination).unwrap();
        assert_eq!(context.kind(), ProviderKindSetting::Compatible);
        assert_eq!(context.base_url(), "https://models.example.test/v1");
    }

    #[test]
    fn destination_validation_excludes_credentials_and_uses_ascii_native_copy() {
        for endpoint in [
            "",
            "http://remote.test/v1",
            "https://user:password@example.test/v1",
            "https://example.test/v1?token=PRIVATE_TOKEN",
            "https://example.test/#PRIVATE_TOKEN",
            "https://example.test/\nSave",
        ] {
            assert!(
                ProviderCredentialDestination::new(ProviderKindSetting::Compatible, endpoint)
                    .is_err()
            );
        }
        assert!(
            ProviderCredentialDestination::new(
                ProviderKindSetting::Codex,
                "https://chatgpt.com/backend-api/codex"
            )
            .is_err()
        );
        let destination = ProviderCredentialDestination::new(
            ProviderKindSetting::Anthropic,
            "https://bücher.example/v1",
        )
        .unwrap();
        assert_eq!(
            destination.presentation,
            ProviderCredentialContext::new(
                CredentialProvider::AnthropicMessages,
                "https://xn--bcher-kva.example/v1"
            )
            .unwrap()
        );
        assert_eq!(
            canonical_provider_endpoint("https://bücher.example/v1").unwrap(),
            "https://xn--bcher-kva.example/v1"
        );
        assert!(canonical_provider_endpoint("https://user:PRIVATE_TOKEN@example.test/v1").is_err());
    }

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
