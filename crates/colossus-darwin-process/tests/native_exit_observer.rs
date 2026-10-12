//! Exact process exit evidence; no audit-session ports or descendant inference.
#![cfg(target_os = "macos")]

use colossus_darwin_process::{
    DarwinProcessExitObserver, DarwinProcessIdentity, spawn_suspended_pipes,
};
use std::{ffi::OsString, os::unix::process::ExitStatusExt, path::Path, time::Duration};

#[test]
#[ignore = "explicit native process acceptance; never includes session-port probes"]
fn retained_kernel_observer_reports_exit_before_reap_and_preserves_its_receipt() {
    let mut spawned = spawn_suspended_pipes(Path::new("/usr/bin/true"), &[], &[]).unwrap();
    let identity = DarwinProcessIdentity::bind(spawned.child.pid()).unwrap();
    let mut observer = DarwinProcessExitObserver::register(&identity).unwrap();
    assert!(observer.wait(Duration::ZERO).unwrap().is_none());
    assert_eq!(
        observer.wait(Duration::from_secs(61)).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    spawned.child.resume().unwrap();
    let exit = observer.wait(Duration::from_secs(3)).unwrap().unwrap();
    assert_eq!(exit.identity(), &identity);
    assert_eq!(exit.status().code(), Some(0));
    let reaped = spawned.child.wait().unwrap();
    assert_eq!(exit.status(), reaped);
    let retained = observer.wait(Duration::ZERO).unwrap().unwrap();
    assert_eq!(retained.identity(), &identity);
    assert_eq!(retained.status(), reaped);
    println!(
        "PASS kernel NOTE_EXIT before direct-parent reap, stable enrolled receipt, bounded timeout"
    );
}

#[test]
#[ignore = "explicit native process acceptance; never includes session-port probes"]
fn retained_kernel_observer_reports_forced_exit_after_exact_direct_child_cleanup() {
    let mut spawned =
        spawn_suspended_pipes(Path::new("/bin/sleep"), &[OsString::from("30")], &[]).unwrap();
    let identity = DarwinProcessIdentity::bind(spawned.child.pid()).unwrap();
    let mut observer = DarwinProcessExitObserver::register(&identity).unwrap();
    spawned.child.resume().unwrap();
    assert!(observer.wait(Duration::from_millis(10)).unwrap().is_none());
    let reaped = spawned.child.kill_and_reap().unwrap();
    assert_eq!(reaped.signal(), Some(libc::SIGKILL));
    let exit = observer.wait(Duration::from_secs(3)).unwrap().unwrap();
    assert_eq!(exit.identity(), &identity);
    assert_eq!(exit.status(), reaped);
    assert_eq!(exit.status().signal(), Some(libc::SIGKILL));
    println!("PASS retained kernel NOTE_EXIT matches exact direct-child SIGKILL reap");
}
