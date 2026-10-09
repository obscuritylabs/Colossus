use colossus_api::{OutcomeCertainty, RunFailure, RunStatus, RunUpdateKind};
use colossus_ports::{ContextError, ModelProviderError, StoreError};
use colossus_runtime::RuntimeError;

pub(super) fn runtime_failure(error: &RuntimeError) -> RunUpdateKind {
    let failure = released_runtime_failure(error);
    let status = if failure.outcome == OutcomeCertainty::Unknown {
        RunStatus::OutcomeUnknown
    } else {
        RunStatus::Failed
    };
    RunUpdateKind::Failure { status, failure }
}

pub(super) fn released_runtime_failure(error: &RuntimeError) -> RunFailure {
    match error {
        RuntimeError::Agent(colossus_agent::AgentError::Context(error))
        | RuntimeError::Context(error) => released_context_failure(error),
        RuntimeError::Agent(colossus_agent::AgentError::Provider(error)) => {
            released_provider_failure(error)
        }
        RuntimeError::Agent(colossus_agent::AgentError::Tool(
            colossus_ports::ToolError::OutcomeUnknown(_),
        ))
        | RuntimeError::Store(StoreError::OutcomeUnknown(_))
        | RuntimeError::SearchPort(colossus_ports::SearchError::OutcomeUnknown(_)) => {
            generic_failure(
                "runtime.outcome_unknown",
                "an external effect has no trustworthy terminal outcome",
                OutcomeCertainty::Unknown,
            )
        }
        RuntimeError::Agent(colossus_agent::AgentError::Tool(
            colossus_ports::ToolError::Denied(_),
        )) => generic_failure(
            "tool.denied",
            "the requested tool was denied before execution; review policy, tool access, and approval settings",
            OutcomeCertainty::Known,
        ),
        RuntimeError::Gateway(error) => released_gateway_failure(error),
        RuntimeError::Agent(colossus_agent::AgentError::MaxTurns { .. }) => generic_failure(
            "agent.max_turns",
            "the model reached the configured turn limit before producing a final response",
            OutcomeCertainty::Known,
        ),
        RuntimeError::Agent(colossus_agent::AgentError::EmptyTurn) => generic_failure(
            "provider.empty_turn",
            "the provider returned no visible response or tool call",
            OutcomeCertainty::Known,
        ),
        RuntimeError::Agent(colossus_agent::AgentError::ToolArgumentRecoveryExhausted {
            ..
        }) => generic_failure(
            "provider.invalid_tool_arguments",
            "the provider repeatedly returned invalid tool arguments",
            OutcomeCertainty::Known,
        ),
        _ if error.outcome_unknown() => generic_failure(
            "runtime.outcome_unknown",
            "an external effect has no trustworthy terminal outcome",
            OutcomeCertainty::Unknown,
        ),
        _ => generic_failure(
            "runtime.failed",
            "the run could not complete; review the runtime and model settings",
            OutcomeCertainty::Known,
        ),
    }
}

fn released_provider_failure(error: &ModelProviderError) -> RunFailure {
    match error {
        ModelProviderError::Rejected(failure)
        | ModelProviderError::ResponseDiagnostic {
            failure: Some(failure),
            ..
        } => released_provider_rejection(failure),
        ModelProviderError::Recoverable {
            code,
            http_status,
            retry_after_ms,
            ..
        } => RunFailure {
            code: code.clone(),
            message: released_recoverable_provider_message(code, *http_status).into(),
            outcome: OutcomeCertainty::Known,
            recoverable: true,
            http_status: *http_status,
            retry_after_ms: *retry_after_ms,
        },
        ModelProviderError::HttpStatus { status, .. } => RunFailure {
            code: "provider.http_status".into(),
            message: format!("provider endpoint returned HTTP {status}"),
            outcome: OutcomeCertainty::Known,
            recoverable: false,
            http_status: Some(*status),
            retry_after_ms: None,
        },
        ModelProviderError::ResponseDiagnostic { diagnostic, .. } => RunFailure {
            code: "provider.http_status".into(),
            message: format!("provider endpoint returned HTTP {}", diagnostic.status),
            outcome: OutcomeCertainty::Known,
            recoverable: false,
            http_status: Some(diagnostic.status),
            retry_after_ms: None,
        },
        ModelProviderError::Configuration(_) => generic_failure(
            "provider.configuration",
            "the configured provider request is invalid",
            OutcomeCertainty::Known,
        ),
        ModelProviderError::Failed(_) => generic_failure(
            "provider.failed",
            "the provider could not complete the request; check the provider and model settings",
            OutcomeCertainty::Known,
        ),
        ModelProviderError::OutcomeUnknown(_) => generic_failure(
            "provider.outcome_unknown",
            "provider transport failed after execution began; the outcome is unknown",
            OutcomeCertainty::Unknown,
        ),
    }
}

fn released_recoverable_provider_message(code: &str, http_status: Option<u16>) -> &'static str {
    match code {
        "provider.temporarily_unavailable" => {
            "provider endpoint returned HTTP 503; retry after the endpoint reports ready"
        }
        "provider.invalid_tool_arguments" => "the provider returned invalid tool arguments",
        _ if http_status.is_some() => "the provider returned a recoverable HTTP response",
        _ => "the provider request failed with a recoverable error",
    }
}

fn released_gateway_failure(error: &colossus_policy::GatewayError) -> RunFailure {
    match error {
        colossus_policy::GatewayError::ProviderRejected(failure) => {
            released_provider_rejection(failure)
        }
        colossus_policy::GatewayError::RecoverableExecution {
            code,
            message,
            http_status,
            retry_after_ms,
        } => RunFailure {
            code: code.clone(),
            message: message.clone(),
            outcome: OutcomeCertainty::Known,
            recoverable: true,
            http_status: *http_status,
            retry_after_ms: *retry_after_ms,
        },
        colossus_policy::GatewayError::HttpStatus { status, message } => RunFailure {
            code: "effect.http_status".into(),
            message: message.clone(),
            outcome: OutcomeCertainty::Known,
            recoverable: false,
            http_status: Some(*status),
            retry_after_ms: None,
        },
        colossus_policy::GatewayError::OutcomeUnknown(_) => generic_failure(
            "effect.outcome_unknown",
            "an external effect has no trustworthy terminal outcome",
            OutcomeCertainty::Unknown,
        ),
        colossus_policy::GatewayError::Denied(_) => generic_failure(
            "effect.denied",
            "policy denied the requested effect",
            OutcomeCertainty::Known,
        ),
        colossus_policy::GatewayError::Approval(_) => generic_failure(
            "effect.approval_required",
            "the requested effect was not approved",
            OutcomeCertainty::Known,
        ),
        _ => generic_failure(
            "runtime.failed",
            "the run could not complete; review the runtime and model settings",
            OutcomeCertainty::Known,
        ),
    }
}

fn released_provider_rejection(failure: &colossus_contracts::ProviderFailure) -> RunFailure {
    RunFailure {
        code: failure.reason.code().into(),
        message: failure.reason.message().into(),
        outcome: OutcomeCertainty::Known,
        recoverable: false,
        http_status: failure.http_status,
        retry_after_ms: None,
    }
}

fn released_context_failure(error: &ContextError) -> RunFailure {
    match error {
        ContextError::BudgetExceeded(budget) => {
            generic_failure(budget.code(), &budget.to_string(), OutcomeCertainty::Known)
        }
        ContextError::Provider(error) => released_provider_failure(error),
        ContextError::Store(StoreError::OutcomeUnknown(_)) => generic_failure(
            "runtime.outcome_unknown",
            "context storage has no trustworthy terminal outcome",
            OutcomeCertainty::Unknown,
        ),
        ContextError::Configuration(_) => generic_failure(
            "context.configuration",
            "the context configuration is invalid; review the model and compaction settings",
            OutcomeCertainty::Known,
        ),
        ContextError::Store(_) => generic_failure(
            "context.unavailable",
            "session context could not be loaded or saved; check runtime storage availability",
            OutcomeCertainty::Known,
        ),
    }
}

pub(super) fn generic_failure(code: &str, message: &str, outcome: OutcomeCertainty) -> RunFailure {
    RunFailure {
        code: code.into(),
        message: message.into(),
        outcome,
        recoverable: false,
        http_status: None,
        retry_after_ms: None,
    }
}

#[cfg(test)]
mod tests;
