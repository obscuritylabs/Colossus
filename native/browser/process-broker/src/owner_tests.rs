//! Keeper cleanup protocol tests with socket peers, never native child creation.

use super::KeeperEnrollment;
use crate::{Frame, KeeperClient, Message};
use std::{io::Read, os::unix::net::UnixStream, thread};

fn no_message(server: &mut UnixStream) {
    server.set_nonblocking(true).expect("nonblocking peer");
    assert_eq!(
        server
            .read(&mut [0])
            .expect_err("no spurious keeper message")
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn identity_failure_before_enrollment_does_not_poison_keeper_with_unknown_slot() {
    let (client, mut server) = UnixStream::pair().expect("keeper pair");
    let mut keeper = KeeperClient::new(client, [1; 16]).expect("keeper");
    let mut state = KeeperEnrollment::NotAttempted;
    state
        .acknowledge_exit(&mut keeper, 7)
        .expect("no enrollment");
    no_message(&mut server);
    assert!(matches!(state, KeeperEnrollment::NotAttempted));
}

#[test]
fn lost_enrollment_reply_preserves_uncertainty_without_advancing_keeper_protocol() {
    let (client, mut server) = UnixStream::pair().expect("keeper pair");
    let mut keeper = KeeperClient::new(client, [1; 16]).expect("keeper");
    let mut state = KeeperEnrollment::Pending;
    assert!(state.acknowledge_exit(&mut keeper, 7).is_err());
    no_message(&mut server);
    assert!(matches!(state, KeeperEnrollment::Pending));
}

#[test]
fn positively_enrolled_member_sends_one_exact_reap_acknowledgement() {
    let (client, mut server) = UnixStream::pair().expect("keeper pair");
    let mut keeper = KeeperClient::new(client, [1; 16]).expect("keeper");
    let peer = thread::spawn(move || {
        let frame = Frame::read(&mut server).expect("reap frame");
        assert_eq!(frame.sequence, 1);
        assert_eq!(frame.nonce, [1; 16]);
        assert_eq!(frame.message, Message::Reaped(7));
        Frame {
            message: Message::Reply {
                slot: 7,
                pid: 0,
                status: None,
            },
            ..frame
        }
        .write(&mut server)
        .expect("reap response");
        server
    });
    let mut state = KeeperEnrollment::Acknowledged;
    state
        .acknowledge_exit(&mut keeper, 7)
        .expect("acknowledged exit");
    let mut server = peer.join().expect("keeper peer");
    state
        .acknowledge_exit(&mut keeper, 7)
        .expect("already acknowledged");
    no_message(&mut server);
    assert!(matches!(state, KeeperEnrollment::Reaped));
}
