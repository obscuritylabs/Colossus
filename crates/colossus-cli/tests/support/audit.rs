use serde_json::Value;
use std::{collections::BTreeSet, process::Output};

const EVIDENCE_FIELDS: [&str; 16] = [
    "actor",
    "classification",
    "context",
    "event_id",
    "event_type",
    "event_version",
    "global_sequence",
    "occurred_at",
    "payload_algorithm",
    "payload_key_id",
    "payload_plaintext_hash",
    "previous_hash",
    "record_hash",
    "schema_version",
    "stream_id",
    "stream_version",
];

pub fn assert_audit_evidence_jsonl(
    output: &Output,
    expected_key_id: &str,
    expected_from: u64,
    expected_limit: usize,
) {
    assert!(
        output.status.success(),
        "audit export failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("audit evidence JSONL record"))
        .collect::<Vec<Value>>();
    assert_audit_evidence_records(&records, expected_key_id, expected_from, expected_limit);
}

pub fn assert_audit_evidence_array(
    output: &Output,
    expected_key_id: &str,
    expected_from: u64,
    expected_limit: usize,
) {
    assert!(
        output.status.success(),
        "audit show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let records: Vec<Value> =
        serde_json::from_slice(&output.stdout).expect("audit evidence JSON array");
    assert_audit_evidence_records(&records, expected_key_id, expected_from, expected_limit);
}

fn assert_audit_evidence_records(
    records: &[Value],
    expected_key_id: &str,
    expected_from: u64,
    expected_limit: usize,
) {
    assert_eq!(records.len(), expected_limit);
    let expected_fields = EVIDENCE_FIELDS
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    for (offset, record) in records.iter().enumerate() {
        let object = record.as_object().expect("audit evidence object");
        assert_eq!(
            object.keys().cloned().collect::<BTreeSet<_>>(),
            expected_fields
        );
        assert_eq!(record["schema_version"], 1);
        assert_eq!(record["global_sequence"], expected_from + offset as u64);
        assert_eq!(record["payload_key_id"], expected_key_id);
        assert!(record["payload_algorithm"].is_string());
        assert!(record["payload_plaintext_hash"].is_string());
        assert!(record["actor"].is_object());
        assert!(record["context"].is_object());
        assert_no_encrypted_payload_fields(record);
    }
}

fn assert_no_encrypted_payload_fields(value: &Value) {
    match value {
        Value::Object(object) => {
            for (field, value) in object {
                assert!(
                    !matches!(field.as_str(), "payload" | "nonce" | "ciphertext"),
                    "audit evidence leaked encrypted payload field {field}"
                );
                assert_no_encrypted_payload_fields(value);
            }
        }
        Value::Array(values) => {
            for value in values {
                assert_no_encrypted_payload_fields(value);
            }
        }
        _ => {}
    }
}
