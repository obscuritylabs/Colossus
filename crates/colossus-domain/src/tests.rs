use super::{ActorType, DecisionOutcome, WorkflowStatus};
use std::str::FromStr;

#[test]
fn wire_values_round_trip_without_dependencies() {
    assert_eq!(DecisionOutcome::Allow.to_string(), "allow");
    assert_eq!(
        WorkflowStatus::from_str("interrupted"),
        Ok(WorkflowStatus::Interrupted)
    );
    assert_eq!(ActorType::Application.to_string(), "application");
    assert_eq!(
        ActorType::from_str("application"),
        Ok(ActorType::Application)
    );
    assert!(WorkflowStatus::from_str("paused").is_err());
}
