use super::*;
use colossus_darwin_process::spawn_suspended_pipes;
use std::{ffi::OsString, os::unix::process::ExitStatusExt, path::Path, process::Command, thread};

fn sleep_cdhash() -> [u8; 20] {
    let output = Command::new("/usr/bin/codesign")
        .args(["-d", "--verbose=4", "/bin/sleep"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.len() < 16 * 1024);
    let text = std::str::from_utf8(&output.stderr).unwrap();
    let hashes = text
        .lines()
        .filter_map(|line| line.strip_prefix("CDHash="))
        .collect::<Vec<_>>();
    assert_eq!(hashes.len(), 1);
    assert_eq!(hashes[0].len(), 40);
    let mut hash = [0; 20];
    for (index, byte) in hash.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hashes[0][index * 2..index * 2 + 2], 16).unwrap();
    }
    hash
}

fn enroll(stream: &mut UnixStream, pid: u32, nonce: [u8; 16]) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    Frame {
        sequence: 1,
        nonce,
        message: Message::Enroll {
            slot: 1,
            pid,
            cdhash: sleep_cdhash(),
        },
    }
    .write(stream)
    .unwrap();
    assert_eq!(
        Frame::read(stream).unwrap(),
        Frame {
            sequence: 1,
            nonce,
            message: Message::Reply {
                slot: 1,
                pid,
                status: None
            }
        }
    );
}

#[test]
#[ignore = "explicit native known-roster exit acceptance; no CEF or audit-session ports"]
fn eof_terminates_and_observes_the_enrolled_incarnation() {
    let mut child =
        spawn_suspended_pipes(Path::new("/bin/sleep"), &[OsString::from("30")], &[]).unwrap();
    let nonce = [7; 16];
    let (mut client, server) = UnixStream::pair().unwrap();
    let (lifetime, _owner_lifetime) = UnixStream::pair().unwrap();
    let keeper = thread::spawn(move || {
        let mut keeper = KeeperServer::new(nonce).unwrap();
        let result = keeper.serve(server, lifetime);
        (result, keeper.members.len())
    });
    enroll(&mut client, child.child.pid(), nonce);
    child.child.resume().unwrap();
    drop(client);
    let (result, retained) = keeper.join().unwrap();
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(retained, 0, "only kernel-observed exits remove obligations");
    assert_eq!(child.child.wait().unwrap().signal(), Some(libc::SIGKILL));
    println!(
        "PASS keeper EOF, exact-incarnation SIGKILL, kernel exit observation, actual-parent reap"
    );
}

#[test]
#[ignore = "explicit native known-roster exit acceptance; no CEF or audit-session ports"]
fn a_live_member_cannot_be_removed_by_a_reaped_message() {
    let mut child =
        spawn_suspended_pipes(Path::new("/bin/sleep"), &[OsString::from("30")], &[]).unwrap();
    let nonce = [8; 16];
    let (mut client, server) = UnixStream::pair().unwrap();
    let (lifetime, _owner_lifetime) = UnixStream::pair().unwrap();
    let keeper = thread::spawn(move || {
        let mut keeper = KeeperServer::new(nonce).unwrap();
        let result = keeper.serve(server, lifetime);
        (result, keeper.members.len())
    });
    enroll(&mut client, child.child.pid(), nonce);
    Frame {
        sequence: 2,
        nonce,
        message: Message::Reaped(1),
    }
    .write(&mut client)
    .unwrap();
    let (result, retained) = keeper.join().unwrap();
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        retained, 0,
        "protocol failure positively terminates the known member"
    );
    assert_eq!(child.child.wait().unwrap().signal(), Some(libc::SIGKILL));
    println!(
        "PASS false reap rejected before removal; retained known member terminated and observed"
    );
}
