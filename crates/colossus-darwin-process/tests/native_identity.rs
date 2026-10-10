//! Real macOS primitive evidence; these tests do not accept browser containment.
#![cfg(target_os = "macos")]

mod support;

use colossus_darwin_process::{
    DarwinProcessIdentity, DarwinProcessSignal, DarwinProcessSnapshot,
    audit_token_signals_supported,
};
use std::{
    fs::OpenOptions,
    io,
    os::{
        fd::AsRawFd as _,
        unix::{fs::OpenOptionsExt as _, process::ExitStatusExt as _},
    },
    sync::mpsc::RecvTimeoutError,
    time::Duration,
};
use support::{Program, number};

#[test]
fn genuine_tokens_stop_continue_and_kill_only_the_retained_child() {
    assert!(
        audit_token_signals_supported(),
        "native libproc ABI required"
    );
    let program = Program::compile();
    let mut probe = program.spawn("control");
    let ready = probe.line();
    assert!(ready.starts_with("READY "));
    let identity = DarwinProcessIdentity::bind(probe.child.pid()).expect("genuine child identity");
    assert_eq!(i64::from(identity.pid()), number(&ready, "pid"));
    assert_eq!(i64::from(identity.real_uid()), number(&ready, "uid"));
    assert_eq!(identity.real_uid(), identity.effective_uid());
    assert_eq!(
        i64::from(identity.audit_session_id()),
        number(&ready, "asid")
    );
    identity.signal(DarwinProcessSignal::Stop).unwrap();
    support::wait_state(identity.pid(), 'T');
    probe.command(b'P');
    assert_eq!(
        probe.receiver.recv_timeout(Duration::from_millis(100)),
        Err(RecvTimeoutError::Timeout),
        "stopped child must not consume the ping"
    );
    identity.signal(DarwinProcessSignal::Continue).unwrap();
    assert_eq!(probe.line(), "PONG");
    identity.signal(DarwinProcessSignal::Kill).unwrap();
    assert_eq!(probe.wait().signal(), Some(libc::SIGKILL));
    assert_eq!(
        identity
            .signal(DarwinProcessSignal::Kill)
            .unwrap_err()
            .raw_os_error(),
        Some(libc::ESRCH),
        "reaped identity cannot signal a reused PID"
    );
}

#[test]
fn self_exec_makes_the_old_genuine_token_stale_without_exposing_raw_tokens() {
    let program = Program::compile();
    let mut probe = program.spawn("control");
    assert!(probe.line().starts_with("READY "));
    let original = DarwinProcessIdentity::bind(probe.child.pid()).unwrap();
    probe.command(b'E');
    assert!(probe.line().starts_with("EXECED "));
    assert_eq!(
        original
            .signal(DarwinProcessSignal::Terminate)
            .unwrap_err()
            .raw_os_error(),
        Some(libc::ESRCH)
    );
    probe.command(b'P');
    assert_eq!(probe.line(), "PONG", "new exec must survive stale signal");
    let current = DarwinProcessIdentity::bind(probe.child.pid()).unwrap();
    assert_ne!(current, original);
    assert_eq!(current.pid(), original.pid());
    assert_eq!(current.audit_session_id(), original.audit_session_id());
    current.signal(DarwinProcessSignal::Terminate).unwrap();
    assert_eq!(probe.wait().signal(), Some(libc::SIGTERM));
}

#[test]
fn direct_child_cleanup_reports_orderly_and_forced_reaping_without_tree_claims() {
    let program = Program::compile();

    let mut orderly = program.spawn("control");
    assert!(orderly.line().starts_with("READY "));
    let cleanup = orderly
        .child
        .terminate_and_reap(Duration::from_secs(1))
        .expect("terminate and reap direct child");
    assert!(!cleanup.forced());
    assert_eq!(cleanup.status().signal(), Some(libc::SIGTERM));

    let mut forced = program.spawn("ignore-term");
    assert!(forced.line().starts_with("READY "));
    let cleanup = forced
        .child
        .terminate_and_reap(Duration::from_millis(20))
        .expect("force and reap direct child after deadline");
    assert!(cleanup.forced());
    assert_eq!(cleanup.status().signal(), Some(libc::SIGKILL));
}

#[test]
fn detached_sets_id_changes_the_group_but_preserves_the_genuine_audit_session() {
    let program = Program::compile();
    let mut probe = program.spawn("detached");
    let ready = probe.line();
    assert!(ready.starts_with("READY "));
    let pid = u32::try_from(number(&ready, "pid")).unwrap();
    assert_ne!(pid, probe.child.pid(), "exact owned fork descendant");
    let before = DarwinProcessIdentity::bind(pid).unwrap();
    probe.retain_descendant(before.clone());
    probe.command(b'D');
    let changed = probe.line();
    assert_eq!(number(&changed, "result"), i64::from(pid));
    assert_eq!(number(&changed, "error"), 0);
    let detached = probe.line();
    assert!(detached.starts_with("DETACHED "));
    assert_eq!(number(&detached, "pgid"), i64::from(pid));
    assert_eq!(number(&detached, "sid"), i64::from(pid));
    assert_ne!(number(&ready, "pgid"), number(&detached, "pgid"));
    let after = DarwinProcessIdentity::bind(pid).unwrap();
    assert_eq!(before.audit_session_id(), after.audit_session_id());
    assert_eq!(
        i64::from(after.audit_session_id()),
        number(&detached, "asid")
    );
    probe.command(b'X');
    let reaped = probe.line();
    assert!(reaped.starts_with("DESCENDANT_REAPED "));
    assert_eq!(number(&reaped, "pid"), i64::from(pid));
    assert_eq!(number(&reaped, "signal"), 0);
    assert!(probe.wait().success());
    assert_eq!(
        after
            .signal(DarwinProcessSignal::Kill)
            .unwrap_err()
            .raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[test]
fn syscall_denials_do_not_make_a_process_group_immutable_while_spawn_is_allowed() {
    let program = Program::compile();
    let mut probe = program.spawn("syscalls");
    let mut lines = Vec::new();
    loop {
        let line = probe.line();
        let complete = line.starts_with("SYSCALL_SUITE_COMPLETE ");
        lines.push(line);
        if complete {
            break;
        }
    }
    assert!(
        probe.wait().success(),
        "each nested fixture positively reaped"
    );
    for kind in ["setsid", "setpgid"] {
        let baseline = lines
            .iter()
            .find(|line| line.starts_with(&format!("DIRECT kind={kind} deny=0 ")))
            .unwrap();
        assert!(number(baseline, "result") >= 0);
        let denied = lines
            .iter()
            .find(|line| line.starts_with(&format!("DIRECT kind={kind} deny=1 ")))
            .unwrap();
        assert_eq!(number(denied, "result"), -1);
        assert_eq!(number(denied, "error"), i64::from(libc::EPERM));
    }
    for kind in ["setsid-spawn", "setpgroup-spawn"] {
        let spawned = lines
            .iter()
            .find(|line| line.starts_with(&format!("SPAWN kind={kind} deny=1 ")))
            .unwrap();
        assert_eq!(number(spawned, "result"), 0);
        let pid = number(spawned, "spawned");
        let child = lines
            .iter()
            .find(|line| line.starts_with("SPAWNED ") && number(line, "pid") == pid)
            .unwrap();
        assert_eq!(number(child, "pgid"), pid);
        assert_ne!(number(child, "pgid"), number(spawned, "pgid"));
        if kind == "setsid-spawn" {
            assert_eq!(number(child, "sid"), pid);
            assert_ne!(number(child, "sid"), number(spawned, "sid"));
        }
    }
    let denied = lines
        .iter()
        .find(|line| line.starts_with("SPAWN kind=default-spawn deny=2 "))
        .unwrap();
    assert_eq!(number(denied, "result"), i64::from(libc::EPERM));
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("CASE_REAPED "))
            .count(),
        8
    );
    assert!(lines.last().unwrap().contains("whole_tree_containment=0"));
}

#[test]
fn suspended_spawn_closes_an_undeclared_inheritable_descriptor_and_reaps_on_drop() {
    let program = Program::compile();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(program.path().join("owned-descriptor"))
        .unwrap();
    let descriptor = file.as_raw_fd();
    // SAFETY: this File exclusively owns the live descriptor. These fixed fcntl
    // calls inspect/change only its flags; no descriptor is closed or pointer used.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    assert!(flags >= 0);
    // SAFETY: same uniquely owned live descriptor; intentionally expose this
    // synthetic file to exec so the library's CLOEXEC_DEFAULT boundary is tested.
    let exposed = unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags & !libc::FD_CLOEXEC) };
    assert_eq!(exposed, 0);
    let mut probe = program.spawn("descriptors");
    let receipt = probe.line();
    assert_eq!(number(&receipt, "count"), 3);
    assert_eq!(number(&receipt, "unexpected"), 0);
    assert!(probe.wait().success());
    drop(probe);
    let probe = program.spawn("control");
    assert!(probe.line().starts_with("READY "));
    let identity = DarwinProcessIdentity::bind(probe.child.pid()).unwrap();
    drop(probe);
    assert_eq!(
        identity
            .signal(DarwinProcessSignal::Kill)
            .unwrap_err()
            .raw_os_error(),
        Some(libc::ESRCH)
    );
    drop(file);
}

#[test]
fn truncated_or_unreaped_uid_census_cannot_acknowledge_empty_cleanup() {
    let own = DarwinProcessIdentity::bind(std::process::id()).unwrap();
    match DarwinProcessSnapshot::for_uid(own.real_uid(), 4096) {
        Ok(snapshot) => {
            assert!(
                snapshot
                    .identities()
                    .iter()
                    .any(|identity| identity == &own)
            );
            assert!(snapshot.empty_evidence().is_none());
            assert!(
                snapshot
                    .identities()
                    .iter()
                    .all(|identity| identity.real_uid() == own.real_uid()
                        || identity.effective_uid() == own.real_uid())
            );
            eprintln!(
                "fresh_login_uid_census=complete identities={} production_containment=false",
                snapshot.identities().len()
            );
        }
        Err(error) => {
            // This result is an actual unavailable census. Do not discard an
            // unreadable PID as unrelated or manufacture empty cleanup evidence.
            eprintln!(
                "fresh_login_uid_census=unavailable kind={:?} reason={error} production_containment=false",
                error.kind(),
            );
        }
    }
    let program = Program::compile();
    let mut probe = program.spawn("control");
    assert!(probe.line().starts_with("READY "));
    let identity = DarwinProcessIdentity::bind(probe.child.pid()).unwrap();
    assert!(DarwinProcessSnapshot::for_uid(identity.real_uid(), 1).is_err());
    identity.signal(DarwinProcessSignal::Kill).unwrap();
    support::wait_state(identity.pid(), 'Z');
    assert_eq!(
        DarwinProcessIdentity::bind(identity.pid())
            .unwrap_err()
            .kind(),
        io::ErrorKind::WouldBlock
    );
    assert!(
        DarwinProcessSnapshot::for_uid(identity.real_uid(), 4096).is_err(),
        "a held zombie must preserve the independent cleanup obligation"
    );
    assert_eq!(probe.wait().signal(), Some(libc::SIGKILL));
}
