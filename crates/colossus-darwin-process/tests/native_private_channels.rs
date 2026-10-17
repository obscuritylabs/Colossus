//! Native fixed-channel bootstrap evidence; direct-child ownership only.

#![cfg(target_os = "macos")]

use colossus_darwin_process::{
    DarwinProcessIdentity, DarwinProcessSignal, SpawnedPrivateChannels,
    spawn_suspended_private_channels,
};
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{self, Read as _, Write as _},
    os::{fd::AsRawFd as _, unix::fs::OpenOptionsExt as _},
    path::Path,
    process::Command,
    sync::Mutex,
    time::Duration,
};

static OWNED_DESCRIPTOR_TESTS: Mutex<()> = Mutex::new(());

fn compile(directory: &Path) -> std::path::PathBuf {
    let executable = directory.join("private-channels-probe");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/private_channels_probe.c");
    let result = Command::new("/usr/bin/xcrun")
        .args(["clang", "-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg(fixture)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("fixed native SDK fixture compile");
    assert!(
        result.status.success(),
        "native fixture compilation failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    executable
}

fn flags(descriptor: i32) -> i32 {
    // SAFETY: fcntl only inspects the integer descriptor. Tests retain each owner
    // except the explicit post-Drop EBADF observation; no pointer or close occurs.
    unsafe { libc::fcntl(descriptor, libc::F_GETFD) }
}

fn pipe_has_no_userspace_output(file: &mut File) {
    let descriptor = file.as_raw_fd();
    // SAFETY: this uniquely borrowed File keeps its descriptor valid throughout
    // the fixed flag reads/changes. Neither operation closes or retains it.
    let original = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    assert!(original >= 0);
    // SAFETY: the same owned live pipe descriptor; only local read behavior changes.
    let changed = unsafe { libc::fcntl(descriptor, libc::F_SETFL, original | libc::O_NONBLOCK) };
    assert_eq!(changed, 0);
    let mut byte = [0];
    assert_eq!(
        file.read(&mut byte).unwrap_err().kind(),
        io::ErrorKind::WouldBlock,
        "START_SUSPENDED must prevent all fixture userspace output"
    );
    // SAFETY: the borrowed pipe descriptor is still valid; restore its original flags.
    let restored = unsafe { libc::fcntl(descriptor, libc::F_SETFL, original) };
    assert_eq!(restored, 0);
}

fn parent_descriptors(spawned: &SpawnedPrivateChannels) -> [i32; 6] {
    [
        spawned.bootstrap.as_raw_fd(),
        spawned.data.as_raw_fd(),
        spawned.control.as_raw_fd(),
        spawned.presentation.as_raw_fd(),
        spawned.stdout.as_raw_fd(),
        spawned.stderr.as_raw_fd(),
    ]
}

#[test]
fn fixed_private_channels_start_suspended_and_close_before_helper_exec() {
    let _guard = OWNED_DESCRIPTOR_TESTS.lock().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let executable = compile(directory.path());
    let ambient = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.path().join("undeclared-ambient"))
        .unwrap();
    let ambient_fd = ambient.as_raw_fd();
    // SAFETY: this owned synthetic file stays open; only inheritance flags change
    // to prove CLOEXEC_DEFAULT suppresses undeclared descriptors in the child.
    let exposed = unsafe {
        libc::fcntl(
            ambient_fd,
            libc::F_SETFD,
            flags(ambient_fd) & !libc::FD_CLOEXEC,
        )
    };
    assert_eq!(exposed, 0);
    exercise_channels(&executable);
}

fn exercise_channels(executable: &Path) {
    let mut spawned =
        spawn_suspended_private_channels(executable, &[OsString::from("channels")], &[])
            .expect("exact stopped native bootstrap");
    let owned = parent_descriptors(&spawned);
    assert!(owned.iter().all(|fd| *fd >= 7));
    assert!(owned.iter().all(|fd| flags(*fd) & libc::FD_CLOEXEC != 0));
    let mut unique = owned.to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 6, "each parent endpoint has a distinct owner");
    pipe_has_no_userspace_output(&mut spawned.stdout);
    pipe_has_no_userspace_output(&mut spawned.stderr);
    for stream in [
        &mut spawned.bootstrap,
        &mut spawned.data,
        &mut spawned.control,
        &mut spawned.presentation,
    ] {
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut byte = [0];
        assert_eq!(
            stream.read(&mut byte).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
    spawned.child.resume().unwrap();
    for (index, stream) in [
        &mut spawned.bootstrap,
        &mut spawned.data,
        &mut spawned.control,
        &mut spawned.presentation,
    ]
    .into_iter()
    .enumerate()
    {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut identity = [0];
        stream.read_exact(&mut identity).unwrap();
        assert_eq!(identity[0], u8::try_from(index + 3).unwrap());
        stream.write_all(&[identity[0] + 64]).unwrap();
    }
    assert!(spawned.child.wait().unwrap().success());
    let mut stdout = String::new();
    let mut stderr = String::new();
    spawned.stdout.read_to_string(&mut stdout).unwrap();
    spawned.stderr.read_to_string(&mut stderr).unwrap();
    assert!(stdout.contains("STDOUT_ONLY\n"));
    assert!(stdout.contains("BOOTSTRAP fd_order=3,4,5,6 socket_count=4 fd_count=7\n"));
    assert!(stdout.contains("HELPER_EXEC channels_closed=4 fd_count=3\n"));
    assert_eq!(stderr, "STDERR_ONLY\n");
    assert!(!stdout.contains("STDERR_ONLY"));
}

#[test]
fn fixed_private_channels_work_with_closed_parent_stdio() {
    let _guard = OWNED_DESCRIPTOR_TESTS.lock().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let executable = compile(directory.path());
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fixture_closed_stdio_entrypoint", "--ignored"])
        .env("COLOSSUS_NATIVE_CLOSED_STDIO_PROBE", executable)
        .status()
        .expect("exact nested fixture process");
    assert!(
        result.success(),
        "closed-stdio bootstrap must preserve every channel"
    );
}

#[test]
#[ignore = "internal subprocess fixture invoked by the closed-stdio native test"]
fn fixture_closed_stdio_entrypoint() {
    let Some(executable) = std::env::var_os("COLOSSUS_NATIVE_CLOSED_STDIO_PROBE") else {
        return;
    };
    for descriptor in 0..3 {
        // SAFETY: this dedicated nested process deliberately transfers its own
        // standard descriptors to the fixture. No other test runs in this process.
        let closed = unsafe { libc::close(descriptor) };
        assert_eq!(closed, 0);
    }
    let result = std::panic::catch_unwind(|| exercise_channels(Path::new(&executable)));
    // SAFETY: a fixed terminated path opens a new process-owned standard handle.
    // All stdio slots are closed and the native child was already reaped.
    let standard = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDWR) };
    assert_eq!(standard, 0);
    for descriptor in 1..3 {
        // SAFETY: descriptor0 is our new valid null handle. Each target is a closed
        // standard slot intentionally restored for this isolated libtest process.
        let restored = unsafe { libc::dup2(standard, descriptor) };
        assert_eq!(restored, descriptor);
    }
    if let Err(failure) = result {
        std::panic::resume_unwind(failure);
    }
}

#[test]
fn cancelling_private_channels_drops_every_owner_and_reaps_only_the_exact_child() {
    let _guard = OWNED_DESCRIPTOR_TESTS.lock().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let executable = compile(directory.path());
    let mut spawned =
        spawn_suspended_private_channels(&executable, &[OsString::from("hold")], &[]).unwrap();
    let descriptors = parent_descriptors(&spawned);
    spawned.child.resume().unwrap();
    spawned
        .bootstrap
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut ready = [0];
    spawned.bootstrap.read_exact(&mut ready).unwrap();
    assert_eq!(ready, [3]);
    let identity = DarwinProcessIdentity::bind(spawned.child.pid()).unwrap();
    drop(spawned);
    for descriptor in descriptors {
        assert_eq!(flags(descriptor), -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EBADF));
    }
    assert_eq!(
        identity
            .signal(DarwinProcessSignal::Kill)
            .unwrap_err()
            .raw_os_error(),
        Some(libc::ESRCH)
    );
    let mut status = 0;
    // SAFETY: the retained fixture PID is used only for a nonblocking wait query;
    // status points to initialized writable storage and no signal is sent.
    let result = unsafe { libc::waitpid(identity.pid() as i32, &mut status, libc::WNOHANG) };
    assert_eq!(result, -1);
    assert_eq!(
        io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}
