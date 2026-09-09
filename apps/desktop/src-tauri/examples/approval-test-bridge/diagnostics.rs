//! Acceptance-only categorical diagnostics, including runs with no next model turn.

use colossus_sdk::{OutcomeCertainty, RunStatus, RunTerminal};
use serde_json::{Value, json};

pub(super) fn terminal(status: RunStatus, terminal: Option<&RunTerminal>) -> Value {
    let (reason, outcome) = match terminal {
        Some(RunTerminal::Failure(failure)) => {
            let reason = match failure.reason.as_str() {
                "runtime.outcome_unknown"
                | "runtime.failed"
                | "effect.outcome_unknown"
                | "effect.denied"
                | "tool.denied"
                | "agent.max_turns"
                | "provider.invalid_tool_arguments"
                | "provider.empty_turn" => failure.reason.as_str(),
                _ => "unclassified",
            };
            (reason, Some(failure.outcome_certainty))
        }
        Some(RunTerminal::Result(_)) => ("completed", Some(OutcomeCertainty::Known)),
        Some(RunTerminal::Cancellation(_)) => ("cancelled", Some(OutcomeCertainty::Known)),
        None => ("missing", None),
    };
    json!({
        "status": format!("{status:?}"),
        "reason": reason,
        "outcome": outcome.map(|certainty| format!("{certainty:?}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_sdk::RunFailure;

    fn failure(reason: &str, certainty: OutcomeCertainty) -> RunTerminal {
        RunTerminal::Failure(RunFailure {
            reason: reason.into(),
            message: "private-credential private-command private-binding".into(),
            outcome_certainty: certainty,
            recoverable: false,
            http_status: None,
            retry_after_ms: None,
        })
    }

    #[test]
    fn unknown_terminal_outcome_does_not_require_another_provider_request() {
        assert_eq!(
            terminal(
                RunStatus::OutcomeUnknown,
                Some(&failure(
                    "runtime.outcome_unknown",
                    OutcomeCertainty::Unknown
                )),
            ),
            json!({"status": "OutcomeUnknown", "reason": "runtime.outcome_unknown", "outcome": "Unknown"}),
        );
    }

    #[test]
    fn unrecognized_reason_and_private_failure_text_are_not_disclosed() {
        let output = terminal(
            RunStatus::Failed,
            Some(&failure("private-reason", OutcomeCertainty::Known)),
        );
        assert_eq!(
            output,
            json!({"status": "Failed", "reason": "unclassified", "outcome": "Known"})
        );
        assert!(!output.to_string().contains("private-"));
    }

    #[test]
    fn missing_terminal_evidence_never_invents_outcome_certainty() {
        assert_eq!(
            terminal(RunStatus::Running, None),
            json!({"status": "Running", "reason": "missing", "outcome": null}),
        );
    }
}
