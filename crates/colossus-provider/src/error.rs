use colossus_contracts::{ProviderFailure, ProviderFailureReason, ProviderResponseDiagnostic};
use colossus_policy::ExecutionError;
use colossus_ports::CredentialResolutionError;
use serde_json::Value;
use thiserror::Error;

/// Provider configuration, transport, credential, or normalization failure.
#[derive(Debug, Error)]
pub enum ProviderError {
    /// Policy release or progress delivery failed; preserve its execution semantics.
    #[error(transparent)]
    Progress(Box<ExecutionError>),
    /// Strict profile configuration failed.
    #[error("provider configuration error: {0}")]
    Configuration(String),
    /// Credential reference could not be resolved.
    #[error("provider credential unavailable: {0}")]
    Credential(String),
    /// Endpoint was unreachable or timed out.
    #[error("provider transport failure: {0}")]
    Transport(String),
    /// Known terminal rejection with only safe categorical evidence.
    #[error(transparent)]
    Rejected(ProviderFailure),
    /// Endpoint returned a non-success status without a recognized rejection code.
    #[error("provider endpoint returned HTTP {status}")]
    Status {
        /// HTTP status code only; response bodies are never included.
        status: u16,
        /// Bounded provider retry lower bound parsed from `Retry-After`.
        retry_after_ms: Option<u64>,
    },
    /// Provider response failed the normalized contract.
    #[error("malformed provider output: {0}")]
    Malformed(String),
}

impl From<CredentialResolutionError> for ProviderError {
    fn from(error: CredentialResolutionError) -> Self {
        Self::Credential(error.to_string())
    }
}

impl From<reqwest::Error> for ProviderError {
    fn from(error: reqwest::Error) -> Self {
        Self::Transport(error.to_string())
    }
}

impl From<url::ParseError> for ProviderError {
    fn from(error: url::ParseError) -> Self {
        Self::Configuration(error.to_string())
    }
}

/// Classify bounded HTTP evidence without releasing provider-controlled text.
///
/// Runtime composition also uses this after explicit diagnostic output has passed
/// post-effect policy. Unknown or malformed codes remain ordinary HTTP failures.
pub fn classify_response_diagnostic(
    diagnostic: &ProviderResponseDiagnostic,
) -> Option<ProviderFailure> {
    let reason = match diagnostic.status {
        413 => Some(ProviderFailureReason::RequestTooLarge),
        _ if !diagnostic.body_truncated
            && diagnostic.body_encoding == "utf8"
            && diagnostic.body.len() <= super::MAX_PROVIDER_DIAGNOSTIC_BODY_BYTES =>
        {
            serde_json::from_str::<Value>(&diagnostic.body)
                .ok()
                .as_ref()
                .and_then(provider_failure_reason)
        }
        _ => None,
    };
    reason.map(|reason| ProviderFailure {
        reason,
        http_status: Some(diagnostic.status),
    })
}

pub(super) fn terminal_provider_error(value: &Value) -> ProviderError {
    ProviderError::Rejected(ProviderFailure {
        reason: provider_failure_reason(value).unwrap_or(ProviderFailureReason::Other),
        http_status: None,
    })
}

pub(super) fn chat_finish_error(reason: &str) -> Option<ProviderError> {
    let reason = match reason {
        "length" => ProviderFailureReason::OutputLimitExceeded,
        "content_filter" => ProviderFailureReason::ContentFiltered,
        _ => return None,
    };
    Some(ProviderError::Rejected(ProviderFailure {
        reason,
        http_status: None,
    }))
}

fn provider_failure_reason(value: &Value) -> Option<ProviderFailureReason> {
    // Codes and incomplete reasons are categorical evidence. Message text is never
    // used for classification, even if it names one of these categories.
    [
        "/response/error/code",
        "/response/error/type",
        "/error/code",
        "/error/type",
        "/code",
        "/response/incomplete_details/reason",
        "/incomplete_details/reason",
    ]
    .iter()
    .filter_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
    .find_map(|code| match code {
        "context_length_exceeded"
        | "context_window_exceeded"
        | "input_too_long"
        | "prompt_too_long" => Some(ProviderFailureReason::ContextLimitExceeded),
        "request_too_large" | "payload_too_large" => Some(ProviderFailureReason::RequestTooLarge),
        "max_output_tokens" => Some(ProviderFailureReason::OutputLimitExceeded),
        "rate_limit_exceeded" | "rate_limit_error" => Some(ProviderFailureReason::RateLimited),
        "insufficient_quota" | "quota_exceeded" => Some(ProviderFailureReason::QuotaExceeded),
        "content_filter" | "content_policy_violation" => {
            Some(ProviderFailureReason::ContentFiltered)
        }
        "server_error" | "overloaded_error" => Some(ProviderFailureReason::Unavailable),
        _ => None,
    })
}

#[cfg(test)]
mod tests;
