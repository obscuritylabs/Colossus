use colossus_api::{ApiError, ApiErrorCode, OutcomeCertainty};
use colossus_api_proto::{
    google_rpc::Status as RichStatus,
    v1alpha1::{ColossusErrorDetail, FieldViolation, OutcomeCertainty as ProtoOutcomeCertainty},
};
use prost::Message as _;
use prost_types::Any;
use tonic::{Code, Status};

const ERROR_DETAIL_TYPE_URL: &str = "type.googleapis.com/colossus.api.v1alpha1.ColossusErrorDetail";

/// Translate a bounded public API error to gRPC status and typed binary details.
pub fn api_status(error: ApiError) -> Status {
    let code = match error.code {
        ApiErrorCode::InvalidArgument => Code::InvalidArgument,
        ApiErrorCode::Unauthenticated => Code::Unauthenticated,
        ApiErrorCode::PermissionDenied => Code::PermissionDenied,
        ApiErrorCode::NotFound => Code::NotFound,
        ApiErrorCode::AlreadyExists => Code::AlreadyExists,
        ApiErrorCode::Conflict => Code::Aborted,
        ApiErrorCode::FailedPrecondition => Code::FailedPrecondition,
        ApiErrorCode::ResourceExhausted => Code::ResourceExhausted,
        ApiErrorCode::Cancelled => Code::Cancelled,
        ApiErrorCode::Unavailable => Code::Unavailable,
        ApiErrorCode::Internal => Code::Internal,
        ApiErrorCode::OutcomeUnknown => Code::Unknown,
    };
    let reason = serde_json::to_value(error.reason)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "internal_invariant".into());
    let detail = ColossusErrorDetail {
        reason,
        request_id: error
            .correlation_id
            .as_ref()
            .map_or_else(String::new, |request_id| request_id.as_str().to_owned()),
        retryable: error.retryable,
        retry_after: None,
        outcome_certainty: match error.outcome {
            OutcomeCertainty::Known => ProtoOutcomeCertainty::Known as i32,
            OutcomeCertainty::Unknown => ProtoOutcomeCertainty::Unknown as i32,
        },
        violations: error
            .violations
            .into_iter()
            .map(|violation| FieldViolation {
                field: violation.field,
                description: violation.description,
            })
            .collect(),
    };
    let message = error.message;
    let rich_status = RichStatus {
        code: code as i32,
        message: message.clone(),
        details: vec![Any {
            type_url: ERROR_DETAIL_TYPE_URL.into(),
            value: detail.encode_to_vec(),
        }],
    };
    Status::with_details(code, message, rich_status.encode_to_vec().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_api::{ApiErrorReason, RequestId};

    #[test]
    fn typed_details_preserve_safe_retry_semantics() {
        let error = ApiError::invalid(ApiErrorReason::InvalidArgument, "role", "role is invalid")
            .with_correlation_id(RequestId::new("request-1").expect("request id"));
        let status = api_status(error);
        assert_eq!(status.code(), Code::InvalidArgument);
        let rich_status = RichStatus::decode(status.details()).expect("rich status");
        assert_eq!(rich_status.code, Code::InvalidArgument as i32);
        assert_eq!(
            rich_status.details[0].type_url,
            "type.googleapis.com/colossus.api.v1alpha1.ColossusErrorDetail"
        );
        let detail =
            ColossusErrorDetail::decode(rich_status.details[0].value.as_slice()).expect("detail");
        assert_eq!(detail.reason, "invalid_argument");
        assert_eq!(detail.request_id, "request-1");
        assert_eq!(detail.violations[0].field, "role");
        assert!(!detail.retryable);
    }
}
