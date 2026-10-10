#![cfg(target_os = "macos")]
//! Native, nonbrowser launchd and atomic session/fileport transfer proofs.
//!
//! Requires a logged-in GUI launchd domain, native SDK compiler and permission to
//! create exact temporary user jobs. Received session rights are retained and
//! released only; no conveyed right is used for spawning or joining a session.
//! Execution is suspended after two observed kernel panics on macOS 26.5.2.
//! The deliberate wrong-port cases were removed. Do not run this target until
//! the remaining incoming-right provenance design has been reviewed independently.
//! This fixture proves primitives; it does not accept a production receiver or browser containment.

mod session_ports;

use colossus_darwin_process::{DarwinProcessIdentity, DarwinProcessSignal};
use session_ports::{Fixture, Job};
use std::{fs, thread, time::Duration};

#[test]
#[ignore = "execution suspended after macOS audit-session port kernel panics"]
fn launchd_wait_for_debugger_binds_a_fresh_session_before_first_instruction() {
    let fixture = Fixture::compile();
    let marker = fixture.directory().join("first-userspace-marker");
    let mut job = Job::start(&fixture, "marker", &[marker.to_str().unwrap()], true, true);
    let pid = job.wait_for_pid();
    let parent =
        DarwinProcessIdentity::bind(std::process::id()).expect("genuine observer identity");
    session_ports::wait_stopped(pid);
    let identity = DarwinProcessIdentity::bind(pid).expect("genuine stopped launchd job identity");
    assert_eq!(identity.real_uid(), parent.real_uid());
    assert_eq!(identity.effective_uid(), parent.effective_uid());
    assert_ne!(identity.audit_session_id(), parent.audit_session_id());
    for _ in 0..5 {
        assert!(!marker.exists(), "no userspace marker before exact resume");
        assert_eq!(fs::metadata(job.stdout()).unwrap().len(), 0);
        assert_eq!(fs::metadata(job.stderr()).unwrap().len(), 0);
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        DarwinProcessIdentity::bind(pid).unwrap(),
        identity,
        "same genuine incarnation remains stopped"
    );
    identity
        .signal(DarwinProcessSignal::Continue)
        .expect("exact audited Continue");
    job.wait_for_output("MARKER_REACHED ");
    assert_eq!(fs::read(&marker).unwrap(), b"M");
    let output = fs::read_to_string(job.stdout()).unwrap();
    assert!(output.contains(&format!("asid={}", identity.audit_session_id())));
    assert_eq!(fs::metadata(job.stderr()).unwrap().len(), 0);
    job.wait_for_exit_zero();
    job.remove_and_require_absent();
    assert_eq!(
        identity
            .signal(DarwinProcessSignal::Kill)
            .unwrap_err()
            .raw_os_error(),
        Some(libc::ESRCH),
        "removed exact job cannot be addressed by stale identity"
    );
    println!(
        "launchd_first_instruction_suspended=true fresh_asid={} observer_asid={} genuine_token_bound_before_resume=true no_userspace_marker_or_output=true exact_continue=true owned_job_absent=true",
        identity.audit_session_id(),
        parent.audit_session_id()
    );
}

#[test]
#[ignore = "execution suspended after macOS audit-session port kernel panics"]
fn atomic_mach_fileport_handoff_retains_current_session_until_kernel_end() {
    let fixture = Fixture::compile();
    let mut receiver = Job::start(&fixture, "receiver", &[], false, false);
    receiver.wait_for_output("RECEIVER_READY\n");
    let service = receiver.label().to_owned();
    let mut producer = Job::start(&fixture, "producer", &[&service], true, false);
    producer.wait_for_output("atomic_session_fileport_transfer=true ");
    receiver.wait_for_output("kernel_session_end=true ");
    let output = fs::read_to_string(receiver.stdout()).unwrap();
    println!("{output}");
    assert!(output.contains("fileport_makefd=true exact_readonly_char_device=true cloexec=true zero_reads_and_drops=true genuine_mach_peer=true"));
    assert!(output.contains("keeper_never_joined=true drops=0 retained_through_end=true"));
    producer.wait_for_exit_zero();
    receiver.wait_for_exit_zero();
    assert_eq!(fs::metadata(producer.stderr()).unwrap().len(), 0);
    assert_eq!(fs::metadata(receiver.stderr()).unwrap().len(), 0);
    producer.remove_and_require_absent();
    receiver.remove_and_require_absent();
    println!(
        "producer_owned_job_absent=true receiver_owned_job_absent=true browser_launched=false audit_policy_changed=false"
    );
}
