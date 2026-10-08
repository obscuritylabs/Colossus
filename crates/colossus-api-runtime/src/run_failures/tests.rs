use super::*;
use colossus_contracts::{ProviderFailure, ProviderFailureReason};
use colossus_ports::{ContextBudgetExceeded, ContextBudgetScope};

#[test]
fn context_token_and_byte_limits_survive_both_runtime_wrappers() {
    for budget in [
        ContextBudgetExceeded::tokens(5_048, 3_072, ContextBudgetScope::NewestTurn),
        ContextBudgetExceeded::request_bytes(917_643, 917_504, ContextBudgetScope::NewestTurn),
    ] {
        for error in [
            RuntimeError::Context(ContextError::BudgetExceeded(budget)),
            RuntimeError::Agent(colossus_agent::AgentError::Context(
                ContextError::BudgetExceeded(budget),
            )),
        ] {
            let RunUpdateKind::Failure { status, failure } = runtime_failure(&error) else {
                panic!("failure expected")
            };
            assert_eq!(status, RunStatus::Failed);
            assert_eq!(failure.code, budget.code());
            assert!(failure.message.contains(&budget.required.to_string()));
            assert!(failure.message.contains(&budget.limit.to_string()));
            assert!(failure.message.contains("shorten the message"));
            assert!(!failure.message.contains("known outcome"));
            assert_eq!(failure.outcome, OutcomeCertainty::Known);
            assert!(!failure.recoverable);
            assert_eq!(failure.http_status, None);
        }
    }
}

#[test]
fn categorized_provider_failures_survive_provider_context_and_gateway_wrappers() {
    for http_status in [None, Some(400)] {
        let evidence = ProviderFailure {
            reason: ProviderFailureReason::ContextLimitExceeded,
            http_status,
        };
        for error in [
            RuntimeError::Agent(colossus_agent::AgentError::Provider(
                ModelProviderError::Rejected(evidence),
            )),
            RuntimeError::Context(ContextError::Provider(ModelProviderError::Rejected(
                evidence,
            ))),
            RuntimeError::Agent(colossus_agent::AgentError::Context(ContextError::Provider(
                ModelProviderError::Rejected(evidence),
            ))),
            RuntimeError::Gateway(colossus_policy::GatewayError::ProviderRejected(evidence)),
        ] {
            let failure = released_runtime_failure(&error);
            assert_eq!(failure.code, evidence.reason.code());
            assert_eq!(failure.message, evidence.reason.message());
            assert_eq!(failure.http_status, http_status);
            assert_eq!(failure.outcome, OutcomeCertainty::Known);
            assert!(!failure.recoverable);
        }
    }
}

#[test]
fn nested_context_provider_unknown_outcome_keeps_its_certainty() {
    let failure = released_runtime_failure(&RuntimeError::Agent(
        colossus_agent::AgentError::Context(ContextError::Provider(
            ModelProviderError::OutcomeUnknown("PRIVATE_TRANSPORT".into()),
        )),
    ));
    assert_eq!(failure.code, "provider.outcome_unknown");
    assert_eq!(failure.outcome, OutcomeCertainty::Unknown);
    assert!(!failure.message.contains("PRIVATE"));
    assert!(!failure.recoverable);
}

#[test]
fn explicit_diagnostic_evidence_never_releases_its_body_in_public_failure() {
    let failure = released_runtime_failure(&RuntimeError::Agent(
        colossus_agent::AgentError::Provider(ModelProviderError::ResponseDiagnostic {
            diagnostic: Box::new(colossus_contracts::ProviderResponseDiagnostic {
                request_method: "POST".into(),
                request_url: "https://PRIVATE_ENDPOINT/responses".into(),
                request_body: Some(serde_json::json!({"input": "PRIVATE_INPUT"})),
                status: 400,
                content_type: Some("application/json".into()),
                body: "PRIVATE_RESPONSE".into(),
                body_encoding: "utf8".into(),
                body_truncated: false,
            }),
            failure: Some(ProviderFailure {
                reason: ProviderFailureReason::ContextLimitExceeded,
                http_status: Some(400),
            }),
        }),
    ));
    assert_eq!(failure.code, "provider.context_limit_exceeded");
    assert!(failure.message.contains("context window"));
    assert!(!serde_json::to_string(&failure).unwrap().contains("PRIVATE"));
}
