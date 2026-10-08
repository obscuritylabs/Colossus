use std::fmt;

/// Safe provider rejection category, independent of untrusted response text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderFailureReason {
    /// The provider's model context window cannot fit the request.
    ContextLimitExceeded,
    /// The provider's request-size bound cannot fit the request.
    RequestTooLarge,
    /// Generation stopped at its output-token ceiling.
    OutputLimitExceeded,
    /// The provider refused the request because of a rate limit.
    RateLimited,
    /// The provider account has exhausted its quota.
    QuotaExceeded,
    /// The provider stopped generation because of its content filter.
    ContentFiltered,
    /// The provider reported a service failure.
    Unavailable,
    /// A terminal provider failure did not contain a recognized reason.
    Other,
}

impl ProviderFailureReason {
    /// Stable, bounded code suitable for public failures and audit evidence.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ContextLimitExceeded => "provider.context_limit_exceeded",
            Self::RequestTooLarge => "provider.request_too_large",
            Self::OutputLimitExceeded => "provider.output_limit_exceeded",
            Self::RateLimited => "provider.rate_limited",
            Self::QuotaExceeded => "provider.quota_exceeded",
            Self::ContentFiltered => "provider.content_filtered",
            Self::Unavailable => "provider.unavailable",
            Self::Other => "provider.failed",
        }
    }

    /// Operator guidance containing no provider-controlled strings.
    pub const fn message(self) -> &'static str {
        match self {
            Self::ContextLimitExceeded => {
                "the request exceeds the model's context window; shorten the message, reduce tool arguments or output, or start a new session"
            }
            Self::RequestTooLarge => {
                "the provider request exceeds its size limit; reduce the message, tool arguments, attachments, or instructions"
            }
            Self::OutputLimitExceeded => {
                "the provider reached the output token limit; increase the configured output limit or request a shorter response"
            }
            Self::RateLimited => "the provider rate limit was reached; wait before retrying",
            Self::QuotaExceeded => {
                "the provider quota is exhausted; check the account's quota and billing settings"
            }
            Self::ContentFiltered => {
                "the provider stopped the response because of a content filter; revise the request"
            }
            Self::Unavailable => {
                "the provider could not complete the request; check its availability before retrying"
            }
            Self::Other => {
                "the provider could not complete the request; check the provider and model settings"
            }
        }
    }
}

/// Known terminal provider rejection with only safe categorical evidence.
///
/// This does not authorize replaying a run or any tools already executed in it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderFailure {
    /// Recognized reason, never raw provider message text.
    pub reason: ProviderFailureReason,
    /// Response status when the rejection came from HTTP rather than an SSE event.
    pub http_status: Option<u16>,
}

impl fmt::Display for ProviderFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason.message())
    }
}

impl std::error::Error for ProviderFailure {}
