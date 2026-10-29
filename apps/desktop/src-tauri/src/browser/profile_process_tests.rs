//! The lock API reports contention as Ok(false), not an I/O failure.
use super::*;
use colossus_windows_native::{BoundPath, KillOnCloseJob, configure_suspended_process};
use std::{
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[test]
fn competing_process_cannot_write_preferences_or_acquire_a_live_profile() {
    for kind in ["preferences", "profile"] {
        let directory = tempfile::tempdir().unwrap();
        let image = BoundPath::open_immutable_file(&std::env::current_exe().unwrap()).unwrap();
        let mut command = Command::new(image.canonical_path());
        command
            .args([
                "--ignored",
                "--exact",
                "browser::profile::process_tests::retained_lock_child",
                "--nocapture",
            ])
            .env_clear()
            .env("COLOSSUS_BROWSER_LOCK_TEST_ROOT", directory.path())
            .env("COLOSSUS_BROWSER_LOCK_TEST_KIND", kind)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for name in [
            "SystemRoot",
            "windir",
            "USERPROFILE",
            "LOCALAPPDATA",
            "APPDATA",
            "TEMP",
            "TMP",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        configure_suspended_process(&mut command);
        let mut child = command.spawn().unwrap();
        let job =
            match KillOnCloseJob::assign_verify_and_resume(&child, child.id(), image.identity()) {
                Ok(job) => job,
                Err(failure) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("lock-test child admission failed: {failure}");
                }
            };
        let stdout = child.stdout.take().unwrap();
        let (ready_send, ready_receive) = mpsc::sync_channel(1);
        let (done_send, done_receive) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || read_lock_child(stdout, ready_send, &done_send));
        ready_receive
            .recv_timeout(Duration::from_secs(10))
            .expect("child never acquired fixture lock");
        let store = ProfileStore::at(directory.path(), "process-contention").unwrap();
        if kind == "profile" {
            assert!(
                store.lease().is_err(),
                "a live process's profile lease was admitted"
            );
        } else {
            assert!(
                store.write(&HumanBrowserPreferences::default()).is_err(),
                "a live process's preferences writer was admitted"
            );
        }
        child.stdin.take().unwrap().write_all(&[1]).unwrap();
        settle_owned_lock_child(&mut child, &job);
        done_receive.recv_timeout(Duration::from_secs(2)).unwrap();
        let reader_deadline = Instant::now() + Duration::from_secs(2);
        while !reader.is_finished() {
            assert!(
                Instant::now() < reader_deadline,
                "lock output reader join deadline"
            );
            thread::sleep(Duration::from_millis(5));
        }
        reader.join().unwrap();
        if kind == "profile" {
            assert!(store.lease().is_ok());
        } else {
            store.write(&HumanBrowserPreferences::default()).unwrap();
        }
    }
}

#[test]
#[ignore = "invoked only by the retained-Job process contention test"]
fn retained_lock_child() {
    let directory =
        std::env::var_os("COLOSSUS_BROWSER_LOCK_TEST_ROOT").expect("owned lock fixture root");
    let kind = std::env::var("COLOSSUS_BROWSER_LOCK_TEST_KIND").unwrap();
    let store = ProfileStore::at(Path::new(&directory), "process-contention").unwrap();
    let lease = (kind == "profile").then(|| store.lease().unwrap());
    let lock = if kind == "preferences" {
        let lock = store.root.open_file(Path::new("preferences.lock")).unwrap();
        assert!(FileExt::try_lock_exclusive(lock.file()).unwrap());
        Some(lock)
    } else {
        None
    };
    println!("PROFILE_LOCK_READY");
    std::io::stdout().flush().unwrap();
    std::io::stdin().read_exact(&mut [0u8; 1]).unwrap();
    drop(lock);
    drop(lease);
}

fn settle_owned_lock_child(child: &mut std::process::Child, job: &KillOnCloseJob) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "lock-test child failed to exit normally"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    let deadline = Instant::now() + Duration::from_secs(5);
    while job.active_processes().unwrap() != 0 {
        assert!(
            Instant::now() < deadline,
            "lock-test Job did not reach zero"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn read_lock_child(
    stdout: std::process::ChildStdout,
    ready: mpsc::SyncSender<()>,
    done: &mpsc::SyncSender<()>,
) {
    let mut input = stdout.take(32_769);
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 256];
    let mut ready = Some(ready);
    loop {
        let count = input.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        assert!(
            bytes.len() <= 32_768,
            "lock child output exceeded its bound"
        );
        if bytes
            .windows(b"PROFILE_LOCK_READY".len())
            .any(|window| window == b"PROFILE_LOCK_READY")
            && let Some(ready) = ready.take()
        {
            ready.send(()).unwrap();
        }
    }
    done.send(()).unwrap();
}
