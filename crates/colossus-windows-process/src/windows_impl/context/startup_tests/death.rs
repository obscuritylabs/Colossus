//! Owner death at two closed known sets; never an outstanding CPWL guarantee.
use super::*;
use std::{
    os::windows::io::AsRawHandle as _,
    process::{Child, Command, Stdio},
};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};
use windows_sys::Win32::System::Threading::OpenProcess;

const SID: &str = "COLOSSUS_WINDOWS_JOB_FIXTURE_SID";
const HOME: &str = "COLOSSUS_WINDOWS_JOB_FIXTURE_HOME";

struct DeathCase {
    fixture: Fixture,
    core: Option<Child>,
    core_observer: Option<OwnedHandle>,
    read_lease: Option<crate::PrivateIoLease>,
    closed_set: bool,
}

impl DeathCase {
    fn rollback_core(&mut self, timeout: Duration) -> bool {
        let Some(core) = &mut self.core else {
            return true;
        };
        // Before exact readiness, do not turn a failed test into an accidental
        // CPWL-pending death experiment. EOF asks the fixed fixture to roll back.
        core.stdin.take();
        if let Some(observer) = &self.core_observer
            && !matches!(wait_timeout(observer, timeout), Ok(Some(_)))
        {
            return false;
        }
        if !matches!(core.try_wait(), Ok(Some(_))) {
            return false;
        }
        self.core.take();
        true
    }

    fn stop_core(&mut self, timeout: Duration) -> bool {
        let Some(core) = &mut self.core else {
            return true;
        };
        if !matches!(core.try_wait(), Ok(Some(_))) {
            // Child::kill uses its exact retained process object, never a PID lookup.
            let _ = core.kill();
            if let Some(observer) = &self.core_observer
                && !matches!(wait_timeout(observer, timeout), Ok(Some(_)))
            {
                return false;
            }
        }
        if !matches!(core.try_wait(), Ok(Some(_))) {
            return false;
        }
        self.core.take();
        true
    }

    fn join_reader(&mut self, timeout: Duration) -> bool {
        let Some(lease) = &self.read_lease else {
            return true;
        };
        lease.cancel();
        if !lease.drain(timeout) {
            return false;
        }
        self.read_lease.take();
        true
    }
}

impl Drop for DeathCase {
    fn drop(&mut self) {
        let stopped = if self.closed_set {
            self.stop_core(Duration::from_secs(1))
        } else {
            self.rollback_core(Duration::from_secs(1))
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        let known_exited =
            self.closed_set
                && self.fixture.processes.iter().all(|process| {
                    matches!(wait_timeout(process, remaining(deadline)), Ok(Some(_)))
                });
        let joined = self.join_reader(Duration::from_secs(1));
        if stopped && known_exited && joined {
            self.fixture.external_pending = false;
        }
        // Unknown keeps exact profile/state/stage authority through Fixture::Drop.
        // Unjoined native I/O storage remains owned by the lease's existing policy.
        if !joined {
            preserve(&mut self.read_lease);
        }
        if !stopped {
            preserve(&mut self.core);
            preserve(&mut self.core_observer);
        }
    }
}

pub(super) async fn run(point: Point) -> TestResult {
    let fixture = Fixture::new()?;
    let mut case = DeathCase {
        fixture,
        core: None,
        core_observer: None,
        read_lease: None,
        closed_set: false,
    };
    let name = match point {
        Point::ReturnedContextBound => {
            "windows_impl::context::startup_tests::death::returned_context_core_fixture"
        }
        Point::StationReady => {
            "windows_impl::context::startup_tests::death::station_ready_core_fixture"
        }
        _ => return Err("unsupported fixed death phase".into()),
    };
    let child = Command::new(std::env::current_exe()?)
        .args(["--ignored", "--exact", name, "--nocapture"])
        .env(SID, &case.fixture.request.appcontainer_sid)
        .env(
            HOME,
            case.fixture
                .request
                .environment
                .get("HOME")
                .ok_or("fixture home absent")?,
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    case.core = Some(child);
    case.fixture.external_pending = true;
    let core = held(&case.core)?;
    let core_pid = core.id();
    // SAFETY: duplicate only the exact Child's retained process object into this
    // spawning watcher; helpers receive no Core DUP_HANDLE or mutation authority.
    let reference = identity::duplicate(
        core.as_raw_handle().cast(),
        unsafe { GetCurrentProcess() },
        PROCESS_QUERY_LIMITED_INFORMATION | SYNC,
        false,
    )?;
    case.core_observer = Some(OwnedHandle::new(
        usize::try_from(reference)? as windows_sys::Win32::Foundation::HANDLE,
        "retain exact death-test Core",
    )?);
    let core = held(&case.core_observer)?;
    // SAFETY: observe only the current watcher's image with its pseudo handle.
    let image = identity::image(unsafe { GetCurrentProcess() })?;
    identity::verify_process(
        core,
        core_pid,
        identity::birth(core.raw())?,
        image.identity(),
    )?;
    let core_token = identity::token(core.raw())?;
    // SAFETY: observes current watcher primary token, never an input identity.
    identity::ordinary(
        &core_token,
        &identity::token(unsafe { GetCurrentProcess() })?,
        false,
    )?;
    let stdout: std::os::windows::io::OwnedHandle = held_mut(&mut case.core)?
        .stdout
        .take()
        .ok_or("fixture stdout absent")?
        .into();
    let (reader, lease) = crate::PrivateReader::from_file(File::from(stdout))?;
    case.read_lease = Some(lease);
    let mut reader = BufReader::new(reader.take(2048));
    let ready = tokio::time::timeout(Duration::from_secs(12), async {
        for _ in 0..12 {
            let mut line = String::new();
            if reader.read_line(&mut line).await? == 0 {
                break;
            }
            if line.starts_with("CBCTX-READY ") {
                return Ok::<_, std::io::Error>(line);
            }
        }
        Err(std::io::Error::other("bounded Core readiness absent"))
    })
    .await??;
    require(ready.len() <= 193, "Core readiness length rejected")?;
    let words: Vec<_> = ready.split_whitespace().collect();
    require(
        words.len() == 4 && words[0] == "CBCTX-READY" && words[1].parse::<u8>()? == point as u8,
        "Core readiness phase rejected",
    )?;
    let roles = if point == Point::ReturnedContextBound {
        [1, 2]
    } else {
        [2, 3]
    };
    let mut context_token = None;
    let mut pids = Vec::new();
    for (word, expected_role) in words[2..].iter().zip(roles) {
        let fields: Vec<_> = word.split(':').collect();
        require(
            fields.len() == 3 && fields[0].parse::<u8>()? == expected_role,
            "Core readiness role rejected",
        )?;
        let pid: u32 = fields[1].parse()?;
        let birth: u64 = fields[2].parse()?;
        require(
            pid != 0 && birth != 0 && !pids.contains(&pid),
            "Core readiness identity rejected",
        )?;
        pids.push(pid);
        // SAFETY: query/sync only. Bind the returned object to actual reported
        // birth/image/token while Core still holds its original ownership.
        let observed = OwnedHandle::new(
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNC, 0, pid) },
            "observe known death-test process",
        )?;
        case.fixture.processes.push(observed); // Own before any fallible attestation.
        let observed = case
            .fixture
            .processes
            .last()
            .ok_or("known observer absent")?;
        identity::verify_process(observed, pid, birth, case.fixture.bindings[0].identity())?;
        require(
            wait_timeout(observed, Duration::ZERO)?.is_none(),
            "reported process already exited before Core death",
        )?;
        let token = identity::token(observed.raw())?;
        match expected_role {
            1 => identity::ordinary(&token, &core_token, false)?,
            2 => {
                identity::ordinary(&token, &core_token, true)?;
                context_token = Some(token);
            }
            3 => {
                held(&case.fixture.profile)?.verify_process(observed.borrow())?;
                let context = held(&context_token)?;
                require(
                    token.authentication == context.authentication
                        && token.session == context.session
                        && !token.elevated,
                    "browser differs from exact fresh death-test context",
                )?;
            }
            _ => return Err("unexpected known process role".into()),
        }
    }
    case.fixture.validate_leases()?;
    require(
        case.fixture.jobs.is_empty() && case.fixture.processes.len() == 2,
        "death watcher must not retain Job authority",
    )?;
    case.closed_set = true;
    require(
        held_mut(&mut case.core)?.try_wait()?.is_none(),
        "Core exited before the requested crash",
    )?;
    // No graceful cleanup, no helper termination and NO Job handle in this watcher.
    require(
        case.stop_core(Duration::from_secs(5)),
        "exact Core crash did not complete",
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    for process in &case.fixture.processes {
        require(
            wait_timeout(process, remaining(deadline))?.is_some(),
            "known process survived Core death",
        )?;
    }
    drop(reader);
    require(
        case.join_reader(Duration::from_secs(2)),
        "death watcher I/O remains unjoined",
    )?;
    case.fixture.external_pending = false;
    require(
        case.fixture.settle(Duration::from_secs(1)),
        "death-case profile/state cleanup failed",
    )?;
    Ok(())
}

fn fixture(point: Point) -> TestResult {
    let component = PathBuf::from(
        std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT")
            .ok_or("pinned component required")?,
    );
    let executable =
        BoundPath::open_immutable_file(&component.join("colossus-native-browser-host.exe"))?;
    let cwd = executable
        .canonical_path()
        .parent()
        .ok_or("component parent absent")?
        .to_owned();
    let _client = BoundPath::open_immutable_file(&cwd.join("colossus-native-browser-host.dll"))?;
    let request = request(
        std::env::var(SID)?,
        PathBuf::from(std::env::var_os(HOME).ok_or("fixture home required")?),
        executable.canonical_path().to_owned(),
        cwd,
    )?;
    match spawn_at(&request, point, true) {
        Err(WindowsProcessError::BrowserSetup {
            source: _,
            mut cleanup,
        }) => {
            let mut reported = false;
            loop {
                if matches!(cleanup.cleanup_timeout(Duration::from_secs(5)), Ok(true)) {
                    // Only positive cleanup permits this Core to exit. The same
                    // boxed owner and both image leases stayed alive on retries.
                    return Ok(());
                }
                if !reported {
                    eprintln!("fixed Core fixture retaining unknown startup cleanup custody");
                    reported = true;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        Ok(mut supervised) => {
            let mut reported = false;
            loop {
                let deadline = Instant::now() + Duration::from_secs(5);
                let child = &mut supervised.child;
                if child.terminate(1).is_ok()
                    && matches!(child.wait_tree_timeout(remaining(deadline)), Ok(true))
                    && child.close_browser_context(remaining(deadline)).is_ok()
                    && child.retire_network().is_ok()
                {
                    // Never drop an unexpected live browser and exit its Core.
                    drop(supervised);
                    return Err(
                        "fixed death barrier did not fire; actual child cleanup confirmed".into(),
                    );
                }
                if !reported {
                    eprintln!("fixed Core fixture retaining unexpected live browser custody");
                    reported = true;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        Err(_) => Err("fixed death fixture failed before native custody".into()),
    }
}

#[test]
#[ignore = "fixture-only child selected by the bounded returned-context owner-death test"]
fn returned_context_core_fixture() -> TestResult {
    fixture(Point::ReturnedContextBound)
}
#[test]
#[ignore = "fixture-only child selected by the bounded station-ready owner-death test"]
fn station_ready_core_fixture() -> TestResult {
    fixture(Point::StationReady)
}
