//! Bounded, non-secret destination copy for trusted native token entry.

/// Closed API-provider labels displayed beside the destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialProvider {
    OpenAiCompatible,
    OpenAiResponses,
    AnthropicMessages,
}

impl CredentialProvider {
    fn label(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "OpenAI compatible",
            Self::OpenAiResponses => "OpenAI Responses",
            Self::AnthropicMessages => "Anthropic Messages",
        }
    }
}

/// Exact destination displayed in the same native prompt that accepts the token.
/// The native host validates network authority before creating this bounded copy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderCredentialContext {
    provider: CredentialProvider,
    endpoint: String,
}

impl ProviderCredentialContext {
    /// Prepare ASCII URL presentation after host-side provider URL validation.
    ///
    /// # Errors
    /// Rejects missing, oversized, whitespace-containing, or non-HTTP(S) copy.
    pub fn new(provider: CredentialProvider, endpoint: &str) -> Result<Self, crate::PromptError> {
        if endpoint.is_empty()
            // A validated 2 KiB UTF-8 URL can expand during ASCII percent encoding.
            || endpoint.len() > 3 * 2_048
            || !endpoint.is_ascii()
            || endpoint
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
            || !(endpoint.starts_with("https://") || endpoint.starts_with("http://"))
        {
            return Err(crate::PromptError::Unavailable);
        }
        Ok(Self {
            provider,
            endpoint: endpoint.into(),
        })
    }

    pub(crate) fn heading(&self) -> String {
        format!("Use this token for {} at:", self.provider.label())
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_context_has_fixed_kind_copy_and_exact_bounded_destination() {
        let endpoint = "https://models.example.test:8443/custom/v1";
        let context =
            ProviderCredentialContext::new(CredentialProvider::AnthropicMessages, endpoint)
                .unwrap();
        assert_eq!(context.endpoint(), endpoint);
        assert_eq!(
            context.heading(),
            "Use this token for Anthropic Messages at:"
        );
        for rejected in [
            "",
            "file:///private/path",
            "https://example.test/\nSave now",
            "https://example.test/\0",
        ] {
            assert!(
                ProviderCredentialContext::new(CredentialProvider::OpenAiCompatible, rejected)
                    .is_err()
            );
        }
        assert!(
            ProviderCredentialContext::new(
                CredentialProvider::OpenAiCompatible,
                &format!("https://example.test/{}", "x".repeat(3 * 2_048))
            )
            .is_err()
        );
    }
}
