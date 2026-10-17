//! Portable hostile-wire and authority-nomination regressions; no native probes.

use colossus_mac_browser_process_broker::{
    Frame, HelperRole, MAX_FRAME_BYTES, Message, SpawnArguments,
};
use std::io::Cursor;

fn frame(message: Message) -> Frame {
    Frame {
        sequence: 1,
        nonce: [7; 16],
        message,
    }
}
fn encode(message: Message) -> Vec<u8> {
    let mut output = Vec::new();
    frame(message).write(&mut output).expect("encode");
    output
}

#[test]
fn typed_helper_request_round_trips_without_fd_path_or_environment_fields() {
    let arguments = SpawnArguments::new(
        HelperRole::Renderer,
        vec![
            "--type=renderer".into(),
            "--renderer-client-id=7".into(),
            "--shared-files=".into(),
        ],
    )
    .expect("arguments");
    let input = frame(Message::Spawn(arguments));
    let mut output = Vec::new();
    input.write(&mut output).expect("write");
    assert_eq!(Frame::read(&mut Cursor::new(output)).expect("read"), input);
}

#[test]
fn rejects_exec_environment_fd_and_sandbox_nominations() {
    for argument in [
        "--browser-subprocess-path=/bin/sh",
        "--seatbelt-client=7",
        "--shared-files=data:100",
        "--no-sandbox",
        "--user-data-dir=/Users/shared",
        "--load-plugin=/tmp/plugin",
        "--remote-debugging-port=9222",
        "HOME=/tmp",
        "--type=renderer\n",
    ] {
        assert!(
            SpawnArguments::new(
                HelperRole::Renderer,
                vec!["--type=renderer".into(), argument.into()]
            )
            .is_err(),
            "{argument}"
        );
    }
}

#[test]
fn role_cannot_be_rebound_by_process_arguments() {
    assert!(SpawnArguments::new(HelperRole::Network, vec!["--type=renderer".into()]).is_err());
    assert!(
        SpawnArguments::new(
            HelperRole::Utility,
            vec![
                "--type=utility".into(),
                "--service-sandbox-type=none".into()
            ]
        )
        .is_err()
    );
    assert!(
        SpawnArguments::new(
            HelperRole::Renderer,
            vec!["--type=renderer".into(), "--type=utility".into()]
        )
        .is_err()
    );
}

#[test]
fn header_bound_precedes_payload_allocation_or_read() {
    let mut encoded = encode(Message::Close);
    encoded[32..36].copy_from_slice(&(MAX_FRAME_BYTES as u32).to_be_bytes());
    encoded.truncate(36);
    assert_eq!(
        Frame::read(&mut Cursor::new(encoded))
            .expect_err("oversized")
            .kind(),
        std::io::ErrorKind::InvalidData
    );
}

#[test]
fn unknown_operation_and_trailing_payload_are_not_ignored() {
    let mut encoded = encode(Message::Close);
    encoded[36] = 255;
    assert!(Frame::read(&mut Cursor::new(encoded)).is_err());
    let mut encoded = encode(Message::Close);
    encoded[32..36].copy_from_slice(&2u32.to_be_bytes());
    encoded.push(0);
    assert!(Frame::read(&mut Cursor::new(encoded)).is_err());
}

#[test]
fn every_truncated_frame_is_rejected() {
    let encoded = encode(Message::Reply {
        slot: 17,
        pid: 901,
        status: Some(9),
    });
    for length in 0..encoded.len() {
        assert!(
            Frame::read(&mut Cursor::new(&encoded[..length])).is_err(),
            "{length}"
        );
    }
}

#[test]
fn native_only_keeper_operation_is_distinct_from_browser_spawn() {
    let message = Message::Enroll {
        slot: 17,
        pid: 901,
        cdhash: [3; 20],
    };
    assert_eq!(
        Frame::read(&mut Cursor::new(encode(message.clone())))
            .expect("read")
            .message,
        message
    );
    assert_eq!(
        Frame::read(&mut Cursor::new(encode(Message::Reaped(17))))
            .expect("read")
            .message,
        Message::Reaped(17)
    );
}

#[test]
fn zero_allocation_sequence_and_slot_fail_closed() {
    let mut output = Vec::new();
    assert!(
        Frame {
            sequence: 0,
            nonce: [7; 16],
            message: Message::Close
        }
        .write(&mut output)
        .is_err()
    );
    assert!(
        Frame {
            sequence: 1,
            nonce: [0; 16],
            message: Message::Close
        }
        .write(&mut output)
        .is_err()
    );
    assert!(Frame::read(&mut Cursor::new(encode(Message::Resume(0)))).is_err());
}

#[test]
fn unknown_creation_survives_empty_keeper_roster_acknowledgement() {
    let record = Message::Shutdown {
        reaped: 0,
        unknown: 1,
        keeper_empty: true,
    };
    assert_eq!(
        Frame::read(&mut Cursor::new(encode(record.clone())))
            .expect("read")
            .message,
        record
    );
    assert!(
        frame(Message::Shutdown {
            reaped: 65,
            unknown: 0,
            keeper_empty: true
        })
        .write(&mut Vec::new())
        .is_err()
    );
}
