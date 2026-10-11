//! Real networkless AppContainer/Job conformance; no WFP or loopback exemption.
use super::*;
use colossus_windows_native::{ExclusiveAppContainerProfile, system_windows_directory};
use std::io::Read as _;
use std::io::Write as _;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt as _, BufReader};

const PARENT_CRASH_SID: &str = "COLOSSUS_WINDOWS_JOB_FIXTURE_SID";
const PARENT_CRASH_HOME: &str = "COLOSSUS_WINDOWS_JOB_FIXTURE_HOME";

fn networkless_request(sid: String, home: PathBuf, payload: &str) -> SpawnRequest {
    let system = system_windows_directory().unwrap();
    let home = home.to_string_lossy().into_owned();
    SpawnRequest {
        executable: system.join("System32/cmd.exe"),
        arguments: vec!["/D".into(), "/S".into(), "/C".into(), payload.into()],
        cwd: system.join("System32"),
        environment: BTreeMap::from([
            ("SystemRoot".into(), system.to_string_lossy().into_owned()),
            (
                "PATH".into(),
                system.join("System32").to_string_lossy().into_owned(),
            ),
            ("USERPROFILE".into(), home.clone()),
            ("LOCALAPPDATA".into(), home.clone()),
            ("APPDATA".into(), home.clone()),
            ("TEMP".into(), home.clone()),
            ("TMP".into(), home),
        ]),
        appcontainer_sid: sid,
        max_processes: 8,
        max_memory_bytes: 256 * 1024 * 1024,
        proxy_port: None,
        network_filter_id: None,
    }
}

#[test]
#[ignore = "fixture child, invoked only by the bounded native parent-crash test"]
fn parent_crash_fixture_process() {
    let sid = std::env::var(PARENT_CRASH_SID).expect("fixture-only package");
    let home = PathBuf::from(std::env::var_os(PARENT_CRASH_HOME).expect("fixture-only home"));
    let child = spawn(&networkless_request(
        sid,
        home,
        "for /L %I in (1,1,2147483647) do @ver >NUL",
    ))
    .unwrap();
    println!("OWNED-JOB-CHILD={}", child.pid);
    std::io::stdout().flush().unwrap();
    // EOF also bounds cleanup if the observing test is cancelled or panics.
    let mut byte = [0];
    let _ = std::io::stdin().read(&mut byte);
    drop(child);
}

#[tokio::test(flavor = "current_thread")]
async fn kernel_job_kills_the_retained_appcontainer_child_after_parent_process_crash() {
    use std::os::windows::io::{AsHandle as _, AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use std::process::{Command, Stdio};
    use windows_sys::Win32::{
        Foundation::WAIT_OBJECT_0,
        System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject},
    };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut profile =
        ExclusiveAppContainerProfile::create(&format!("colossus.browser.{nonce:032x}")).unwrap();
    let temporary = tempfile::tempdir_in(std::env::var_os("USERPROFILE").unwrap()).unwrap();
    let mut parent = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "tests::native::parent_crash_fixture_process",
            "--nocapture",
        ])
        .env(PARENT_CRASH_SID, profile.sid_string().unwrap())
        .env(PARENT_CRASH_HOME, temporary.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout: OwnedHandle = parent.stdout.take().unwrap().into();
    let (reader, lease) = crate::PrivateReader::from_file(std::fs::File::from(stdout)).unwrap();
    let mut reader = BufReader::new(reader);
    let ready = tokio::time::timeout(Duration::from_secs(5), async {
        for _ in 0..8 {
            let mut line = String::new();
            if reader.read_line(&mut line).await? == 0 {
                break;
            }
            if let Some(pid) = line.trim().strip_prefix("OWNED-JOB-CHILD=") {
                let pid = pid.parse::<u32>()?;
                // SAFETY: observation only; this never terminates a PID lookup.
                // The returned handle is checked against the exact exclusive package
                // while its parent/Job owner still lives, then retained across crash.
                let raw =
                    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | 0x0010_0000, 0, pid) };
                if raw.is_null() {
                    return Err(std::io::Error::last_os_error().into());
                }
                // SAFETY: successful OpenProcess transferred exactly one live handle.
                let process = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
                profile.verify_process(process.as_handle())?;
                return Ok::<_, Box<dyn std::error::Error>>(process);
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "fixture child not reported",
        )
        .into())
    })
    .await;
    parent
        .kill()
        .expect("crash only the exact retained fixture parent");
    parent.wait().unwrap();
    parent.stdin.take();
    drop(reader);
    lease.cancel();
    assert!(lease.drain(Duration::from_secs(2)));
    let process = ready
        .expect("bounded parent readiness")
        .expect("live exact-package child");
    // SAFETY: the positively attested process object stays retained; this is a
    // bounded kernel wait, independent of PID reuse or process-name matching.
    assert_eq!(
        unsafe { WaitForSingleObject(process.as_raw_handle().cast(), 5000) },
        WAIT_OBJECT_0,
        "the non-inherited kill-on-close Job must retire its child after owner crash"
    );
    drop(process);
    profile
        .close()
        .expect("exact package cleanup after observed process exit");
}

#[tokio::test(flavor = "current_thread")]
async fn appcontainer_job_retains_and_terminates_descendants_after_the_direct_child_exits() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut profile =
        ExclusiveAppContainerProfile::create(&format!("colossus.browser.{nonce:032x}"))
            .expect("exclusive fixture package");
    let user_profile = std::env::var_os("USERPROFILE").unwrap();
    let temporary = tempfile::tempdir_in(user_profile).unwrap();
    let home = temporary.path().to_string_lossy().into_owned();
    let admitted_path = temporary.path().join("owned-profile");
    let system = system_windows_directory().unwrap();
    let command = system.join("System32/cmd.exe");
    let mut child = spawn(&SpawnRequest {
        executable: command,
        arguments: vec!["/D".into(), "/S".into(), "/C".into(),
            "start \"\" /B cmd.exe /D /S /C \"for /L %I in (1,1,2147483647) do @ver >NUL\" & echo OWNED-JOB-READY & set /p \"OWNED_JOB_EXIT=\" & echo owned>\"%OWNED_JOB_PROFILE%\\fixture.txt\" & exit /B 0".into()],
        cwd: system.join("System32"),
        environment: BTreeMap::from([
            ("SystemRoot".into(), system.to_string_lossy().into_owned()),
            ("PATH".into(), system.join("System32").to_string_lossy().into_owned()),
            ("USERPROFILE".into(), home.clone()), ("LOCALAPPDATA".into(), home.clone()),
            ("APPDATA".into(), home.clone()), ("TEMP".into(), home.clone()), ("TMP".into(), home),
            ("OWNED_JOB_PROFILE".into(), admitted_path.to_string_lossy().into_owned()),
        ]),
        appcontainer_sid: profile.sid_string().unwrap(),
        max_processes: 8,
        max_memory_bytes: 256 * 1024 * 1024,
        proxy_port: None,
        network_filter_id: None,
    }).expect("atomic networkless AppContainer/Job launch");
    let mut input = child.stdin.take().unwrap();
    let (reader, lease) = crate::PrivateReader::from_file(child.stdout.take().unwrap()).unwrap();
    let mut reader = BufReader::new(reader);
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let mut line = String::new();
        reader.read_line(&mut line).await?;
        let principal = profile.verify_process(child.process_handle())?;
        let tree = child.observe_process_tree()?;
        if tree.len() != 2 || !tree.iter().any(|process| process.process_id() == child.pid) {
            return Err(std::io::Error::other("incomplete root and descendant observation").into());
        }
        for process in &tree {
            profile.verify_process(process.process_handle())?;
        }
        let directory = principal.create_directory(&admitted_path)?;
        input.write_all(b"complete\r\n")?;
        let exit = child.wait_timeout(Duration::from_secs(2))?;
        let tree_empty = child.wait_tree_timeout(Duration::ZERO)?;
        let descendants = child.observe_process_tree()?;
        if descendants.len() != 1
            || descendants
                .iter()
                .any(|process| process.process_id() == child.pid)
        {
            return Err(std::io::Error::other("direct-child exit lost retained descendant").into());
        }
        profile.verify_process(descendants[0].process_handle())?;
        Ok::<_, Box<dyn std::error::Error>>((line, exit, tree_empty, directory))
    })
    .await;
    child.terminate(1).expect("terminate the exact entire Job");
    assert!(
        child.wait_tree_timeout(Duration::from_secs(5)).unwrap(),
        "positive full Job zero"
    );
    assert!(child.observe_process_tree().unwrap().is_empty());
    drop(reader);
    lease.cancel();
    assert!(
        lease.drain(Duration::from_secs(2)),
        "owned pipe worker drained"
    );
    drop(child);
    profile.close().expect("exact package cleanup");
    let (line, exit, tree_empty, directory) = result
        .expect("bounded fixture readiness and direct-child exit")
        .expect("actual token and process observations");
    assert_eq!(line.trim(), "OWNED-JOB-READY");
    assert_eq!(exit, Some(0));
    assert_eq!(
        std::fs::read(admitted_path.join("fixture.txt")).unwrap(),
        b"owned \r\n",
        "the actual Low AppContainer token writes only after exact-package profile admission"
    );
    drop(directory);
    std::fs::remove_file(admitted_path.join("fixture.txt")).unwrap();
    std::fs::remove_dir(admitted_path).unwrap();
    assert!(
        !tree_empty,
        "the retained Job must own the descendant after root exit"
    );
}
