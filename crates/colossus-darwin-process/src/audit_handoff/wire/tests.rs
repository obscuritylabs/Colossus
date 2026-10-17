//! Synthetic bytes verify framing only. These tests invoke no native APIs and
//! cannot establish that a trailer was delivered by the kernel.

use super::*;

fn received(kind: Kind) -> [u8; BUFFER_SIZE] {
    let mut bytes = [0; BUFFER_SIZE];
    let challenge = if kind == Kind::Hello {
        [0; 16]
    } else {
        [2; 16]
    };
    initialize(&mut bytes, kind, 10, 0, [1; 16], challenge);
    put_word(
        &mut bytes,
        0,
        if matches!(kind, Kind::Hello | Kind::Transfer) {
            MOVE_SEND_ONCE
        } else {
            0
        },
    );
    put_word(
        &mut bytes,
        8,
        if matches!(kind, Kind::Hello | Kind::Transfer) {
            9
        } else {
            0
        },
    );
    put_word(&mut bytes, 12, 10);
    if kind == Kind::Transfer {
        add_transfer(
            &mut bytes,
            11,
            12,
            77,
            DeviceMetadata {
                dev: 1,
                ino: 2,
                rdev: 3,
                mode: 4,
                flags: 5,
            },
        );
        // Kernel receive converts both transmitted dispositions to PORT_SEND.
        bytes[38] = MOVE_SEND;
        bytes[50] = MOVE_SEND;
    }
    put_word(&mut bytes, MESSAGE_SIZE + 4, 52);
    for index in 0..8 {
        put_word(&mut bytes, MESSAGE_SIZE + 20 + index * 4, index as u32 + 1);
    }
    bytes
}

#[test]
fn framing_accepts_only_exact_atomic_two_right_transfer() {
    let bytes = received(Kind::Transfer);
    let frame = parse(&bytes, Kind::Transfer).unwrap();
    assert_eq!(frame.asid, 77);
    assert_eq!(frame.nonce, [1; 16]);
    assert_eq!(frame.challenge, [2; 16]);
    assert_eq!(audit_trailer(&bytes).unwrap(), [1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(matches_challenge(&frame, [1; 16], [2; 16]).is_ok());
    assert!(matches_challenge(&frame, [3; 16], [2; 16]).is_err());
    assert!(matches_challenge(&frame, [1; 16], [3; 16]).is_err());
}

#[test]
fn framing_rejects_partial_foreign_kinds_aliases_and_descriptor_dispositions() {
    let valid = received(Kind::Transfer);
    for (offset, value) in [
        (4, 127),
        (20, 0),
        (24, 1),
        (24, 3),
        (52, 2),
        (56, 1),
        (16, 1),
        (28, 0),
        (40, u32::MAX),
        (40, 11),
        (32, 1),
    ] {
        let mut bytes = valid;
        put_word(&mut bytes, offset, value);
        assert!(
            parse(&bytes, Kind::Transfer).is_err(),
            "offset {offset}, value {value}"
        );
    }
    for (offset, value) in [
        (38, COPY_SEND),
        (50, 18),
        (39, 1),
        (51, 4),
        (36, 1),
        (48, 1),
    ] {
        let mut bytes = valid;
        bytes[offset] = value;
        assert!(
            parse(&bytes, Kind::Transfer).is_err(),
            "offset {offset}, value {value}"
        );
    }
    let mut bytes = valid;
    put_word(&mut bytes, 0, MOVE_SEND_ONCE);
    assert!(parse(&bytes, Kind::Transfer).is_err());
}

#[test]
fn handshake_rejects_embedded_rights_unexpected_metadata_and_absent_challenges() {
    for kind in [Kind::Hello, Kind::Challenge, Kind::Accepted] {
        let valid = received(kind);
        assert!(parse(&valid, kind).is_ok());
        for offset in [24, 28, 40, 92, 96, 104, 112, 120, 124] {
            let mut bytes = valid;
            bytes[offset] = 1;
            assert!(
                parse(&bytes, kind).is_err(),
                "kind {kind:?}, offset {offset}"
            );
        }
        let mut bytes = valid;
        bytes[60..76].fill(0);
        assert!(parse(&bytes, kind).is_err());
        let mut bytes = valid;
        if kind == Kind::Hello {
            bytes[76] = 1;
        } else {
            bytes[76..92].fill(0);
        }
        assert!(parse(&bytes, kind).is_err());
    }
}

#[test]
fn trailer_requires_exact_format_size_and_complete_all_eight_words() {
    let valid = received(Kind::Transfer);
    for size in [0, 23, 461, u32::MAX] {
        let mut bytes = valid;
        put_word(&mut bytes, 4, size);
        assert!(audit_trailer(&bytes).is_err());
    }
    for (offset, value) in [(128, 1), (132, 0), (132, 51), (132, 53)] {
        let mut bytes = valid;
        put_word(&mut bytes, offset, value);
        assert!(audit_trailer(&bytes).is_err());
    }
    for size in [0, 23, 127, 128, 179] {
        assert!(audit_trailer(&valid[..size]).is_err());
        assert!(parse(&valid[..size.min(127)], Kind::Transfer).is_err());
    }
}

#[test]
fn send_framing_has_only_copy_session_and_move_device() {
    let mut bytes = [0; BUFFER_SIZE];
    initialize(&mut bytes, Kind::Transfer, 10, 20, [1; 16], [2; 16]);
    add_transfer(
        &mut bytes,
        11,
        12,
        77,
        DeviceMetadata {
            dev: 1,
            ino: 2,
            rdev: 3,
            mode: 4,
            flags: 5,
        },
    );
    assert_eq!(word(&bytes, 24).unwrap(), 2);
    assert_eq!(bytes[38], COPY_SEND);
    assert_eq!(bytes[50], MOVE_SEND);
    assert_eq!(
        word(&bytes, 0).unwrap(),
        COMPLEX | u32::from(COPY_SEND) | (MAKE_SEND_ONCE << 8)
    );
}
