//! Shared stable and libFuzzer exercises for security-critical parsers.

use colossus_contracts::{
    AuditEvidence, EffectRequest, EventEnvelope, PolicyDecision, SignedCheckpoint,
};
use colossus_workflow::{Condition, validate_definition};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use std::fmt::Debug;

fn strict_json_round_trip<T>(data: &[u8])
where
    T: Serialize + DeserializeOwned + PartialEq + Debug,
{
    let Ok(value) = serde_json::from_slice::<T>(data) else {
        return;
    };
    let encoded = serde_json::to_vec(&value).expect("accepted contract must serialize");
    let decoded = serde_json::from_slice::<T>(&encoded)
        .expect("serialized accepted contract must deserialize");
    assert_eq!(decoded, value);
}

/// Exercise every strict journal, audit, effect, and policy JSON boundary.
pub fn exercise_contracts_json(data: &[u8]) {
    strict_json_round_trip::<EventEnvelope>(data);
    strict_json_round_trip::<AuditEvidence>(data);
    strict_json_round_trip::<SignedCheckpoint>(data);
    strict_json_round_trip::<EffectRequest>(data);
    strict_json_round_trip::<PolicyDecision>(data);
}

/// Exercise strict workflow YAML parsing, schema compilation, and trust hashing.
pub fn exercise_workflow_yaml(data: &[u8]) {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let first = validate_definition(source);
    let second = validate_definition(source);
    match (first, second) {
        (Ok(first), Ok(second)) => assert_eq!(first, second),
        (Err(_), Err(_)) => {}
        _ => panic!("workflow validation must be deterministic"),
    }
}

/// Exercise the non-executable condition grammar and bounded evaluator.
pub fn exercise_workflow_condition(data: &[u8]) {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };
    let first = Condition::parse(source);
    let second = Condition::parse(source);
    match (first, second) {
        (Ok(first), Ok(second)) => {
            assert_eq!(first, second);
            for context in [
                json!({}),
                json!({"inputs": {"name": "alpha", "count": 2, "enabled": true}}),
                json!({"steps": {"previous": {"ok": false, "value": null}}}),
            ] {
                assert_eq!(first.evaluate(&context), second.evaluate(&context));
            }
        }
        (Err(_), Err(_)) => {}
        _ => panic!("condition parsing must be deterministic"),
    }
}

#[cfg(test)]
mod tests;
