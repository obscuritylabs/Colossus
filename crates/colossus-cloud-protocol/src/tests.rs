use super::*;

#[test]
fn unknown_operations_and_fields_are_rejected() {
    assert!(decode::<Command>(br#"{"operation":"shell","program":"sh"}"#).is_err());
    assert!(
        decode::<Command>(br#"{"operation":"watch","run_id":"run-1","actor":"admin"}"#).is_err()
    );
}

#[test]
fn empty_and_oversized_payloads_are_rejected_without_echoing_input() {
    assert!(decode::<Command>(&[]).is_err());
    let oversized = vec![b'x'; MAX_PAYLOAD_BYTES + 1];
    assert_eq!(
        decode::<Command>(&oversized).unwrap_err().to_string(),
        "cloud payload is invalid or exceeds its bound"
    );
}
