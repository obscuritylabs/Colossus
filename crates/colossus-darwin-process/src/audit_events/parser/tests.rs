use super::*;

// Actual 125-byte kernel END captured from a fresh owned macOS 26.5.2 session.
// ASID101051; SHA256 8f1196fdae4c098455929149697d9d649411af6af5d0dc7a4e59489351be1d64.
// The source fixture authenticated the producer/child and observed drops=0.
const NATIVE_ASID: u32 = 101051;

fn native_end() -> Vec<u8> {
    include_str!("fixtures/native-session-end.hex")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).unwrap())
        .collect()
}

#[test]
fn genuine_native_end_requires_exact_scope_and_complete_record() {
    let record = native_end();
    assert_eq!(record_size(&record).unwrap(), Some(125));
    assert!(session_record(&record, NATIVE_ASID).unwrap());
    assert!(session_record(&record, NATIVE_ASID + 1).is_err());
    for length in 0..record.len() {
        assert!(session_record(&record[..length], NATIVE_ASID).is_err());
    }
}

#[test]
fn well_formed_other_session_events_are_never_end_receipts() {
    for event in [44901_u16, 44902, 44904] {
        let mut record = native_end();
        record[6..8].copy_from_slice(&event.to_be_bytes());
        assert!(!session_record(&record, NATIVE_ASID).unwrap());
    }
}

#[test]
fn unsupported_framing_and_tampered_trailers_are_rejected() {
    let original = native_end();
    for (offset, replacement) in [(0, 0xff), (5, 0xff), (18, 0xff), (119, 0), (124, 0)] {
        let mut record = original.clone();
        record[offset] = replacement;
        assert!(session_record(&record, NATIVE_ASID).is_err());
    }
    let mut trailing = original;
    trailing.push(0);
    assert!(session_record(&trailing, NATIVE_ASID).is_err());
    for size in [0_u32, 67, MAX_RECORD_BYTES as u32 + 1, u32::MAX] {
        let mut header = native_end();
        header[1..5].copy_from_slice(&size.to_be_bytes());
        assert!(record_size(&header).is_err());
    }
}

#[test]
fn framing_waits_for_a_complete_prefix_without_guessing_size() {
    let record = native_end();
    for length in 0..5 {
        assert_eq!(record_size(&record[..length]).unwrap(), None);
    }
    assert_eq!(record_size(&record[..5]).unwrap(), Some(125));
}
