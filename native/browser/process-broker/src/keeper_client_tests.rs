//! Exact keeper acknowledgements and sticky retry denial, using only sockets.

use super::KeeperClient;
use crate::{Frame, Message};
use std::{io::Read, os::unix::net::UnixStream, thread};

#[test]
fn uncertain_applied_operation_permanently_fences_retries_and_empty_queries() {
    let (client, mut server) = UnixStream::pair().expect("keeper pair");
    let mut keeper = KeeperClient::new(client, [1; 16]).expect("keeper");
    let peer = thread::spawn(move || {
        let frame = Frame::read(&mut server).expect("one reap operation");
        assert_eq!(frame.message, Message::Reaped(7));
        Frame {
            sequence: frame.sequence,
            nonce: [2; 16], // Applied operation, but its reply is inadmissible.
            message: Message::Reply {
                slot: 7,
                pid: 0,
                status: None,
            },
        }
        .write(&mut server)
        .expect("uncertain reply");
        assert_eq!(server.read(&mut [0]).expect("fenced channel EOF"), 0);
    });
    assert!(keeper.reaped(7).is_err());
    assert!(keeper.reaped(7).is_err());
    assert!(keeper.confirm_empty().is_err());
    peer.join().expect("only one irreversible request");
}

#[test]
fn enrollment_requires_the_exact_stopped_pid_and_absent_wait_status() {
    for reply in [
        Message::Reply {
            slot: 7,
            pid: 999,
            status: None,
        },
        Message::Reply {
            slot: 7,
            pid: 123,
            status: Some(0),
        },
    ] {
        let (client, mut server) = UnixStream::pair().expect("keeper pair");
        let mut keeper = KeeperClient::new(client, [1; 16]).expect("keeper");
        let peer = thread::spawn(move || {
            let frame = Frame::read(&mut server).expect("enroll request");
            assert_eq!(
                frame.message,
                Message::Enroll {
                    slot: 7,
                    pid: 123,
                    cdhash: [3; 20]
                }
            );
            Frame {
                message: reply,
                ..frame
            }
            .write(&mut server)
            .expect("changed reply");
            assert_eq!(server.read(&mut [0]).expect("fenced channel EOF"), 0);
        });
        assert!(keeper.enroll(7, 123, [3; 20]).is_err());
        assert!(keeper.confirm_empty().is_err());
        peer.join().expect("keeper peer");
    }
}
