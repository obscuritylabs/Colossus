//! Actual fixed-host startup failures. The helper itself has no test selector.
use super::*;
use colossus_windows_native::{
    ExclusiveAppContainerProfile, PrivateDirectoryCreation, system_windows_directory,
};
use std::{
    collections::BTreeMap,
    io::{Read as _, Write as _},
    path::PathBuf,
};
use windows_sys::Win32::System::Threading::{GetProcessId, PROCESS_QUERY_LIMITED_INFORMATION};

mod death;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum Point {
    RequestQueued = 1,
    ReturnedContextBound,
    EndpointsRegistered,
    BrowserCommitQueued,
    BrowserPulled,
    StationReady,
}

pub(super) struct StartupControl {
    requested: Option<Point>,
    reached: Option<Point>,
    pause: bool,
}

pub(super) fn reach(
    control: &mut Option<StartupControl>,
    point: Point,
    processes: &[(u8, &OwnedHandle)],
) -> Result<(), WindowsProcessError> {
    let Some(control) = control
        .as_mut()
        .filter(|value| value.requested == Some(point))
    else {
        return Ok(());
    };
    control.requested = None;
    control.reached = Some(point);
    if control.pause {
        require(
            matches!(point, Point::ReturnedContextBound | Point::StationReady)
                && processes.len() == 2,
            "test death barrier requires the exact closed process set",
        )?;
        let mut line = format!("CBCTX-READY {}", point as u8);
        for (role, process) in processes {
            // SAFETY: observes an already retained, Core-attested actual object.
            let pid = unsafe { GetProcessId(process.raw()) };
            line.push_str(&format!(
                " {role}:{pid}:{}",
                identity::birth(process.raw())?
            ));
        }
        require(line.len() <= 192, "test readiness bound")?;
        println!("\n{line}");
        std::io::stdout()
            .flush()
            .map_err(|_| WindowsProcessError::Invalid("test readiness write failed".into()))?;
        // The death parent kills only this Core. EOF permits ordinary owned
        // rollback if that parent is cancelled; this never resumes a browser.
        let _ = std::io::stdin().read(&mut [0_u8; 1]);
    }
    Err(WindowsProcessError::Invalid(
        "test-only startup boundary failure".into(),
    ))
}

fn spawn_at(
    request: &SpawnRequest,
    point: Point,
    pause: bool,
) -> Result<SupervisedChild, WindowsProcessError> {
    crate::api::validate_request(request)?;
    require(
        request.arguments.is_empty(),
        "fixed startup test has empty caller arguments",
    )?;
    let mut setup = Box::new(BrowserSetupOwner::empty());
    setup.test_control = Some(StartupControl {
        requested: Some(point),
        reached: None,
        pause,
    });
    // Exactly the production catch_unwind/error payload boundary and rollback owner.
    run_setup(setup, request, 4)
}

struct Fixture {
    profile: Option<ExclusiveAppContainerProfile>,
    state: Option<PrivateDirectoryCreation>,
    state_binding: Option<BoundPath>,
    bindings: Vec<BoundPath>,
    request: SpawnRequest,
    pending: Option<PendingBrowserCleanup>,
    unexpected: Option<SupervisedChild>,
    processes: Vec<OwnedHandle>,
    jobs: Vec<OwnedHandle>,
    external_pending: bool,
}

impl Fixture {
    fn new() -> TestResult<Self> {
        let component = PathBuf::from(
            std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT")
                .ok_or("pinned acceptance component required")?,
        );
        let parent = PathBuf::from(
            std::env::var_os("COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT")
                .ok_or("private acceptance parent required")?,
        );
        let mut nonce = [0_u8; 16];
        // SAFETY: fixed writable nonce buffer and system-selected RNG.
        require(
            unsafe {
                BCryptGenRandom(
                    null_mut(),
                    nonce.as_mut_ptr(),
                    16,
                    BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                )
            } >= 0,
            "test nonce unavailable",
        )?;
        let nonce = format!("{:032x}", u128::from_le_bytes(nonce));
        let state =
            PrivateDirectoryCreation::create(&parent.join(format!("context-startup-{nonce}")))?;
        let state_binding = state.bind()?;
        let profile = ExclusiveAppContainerProfile::create(&format!("colossus.browser.{nonce}"))?;
        // The maintained runner has verified the whole installed inventory. These
        // exact leases plus their immutable ancestors remain through every failure.
        let executable =
            BoundPath::open_immutable_file(&component.join("colossus-native-browser-host.exe"))?;
        let cwd = executable
            .canonical_path()
            .parent()
            .ok_or("fixed component parent absent")?
            .to_owned();
        let client = BoundPath::open_immutable_file(&cwd.join("colossus-native-browser-host.dll"))?;
        let request = request(
            profile.sid_string()?,
            state.path().join("profile"),
            executable.canonical_path().to_owned(),
            cwd,
        )?;
        Ok(Self {
            profile: Some(profile),
            state: Some(state),
            state_binding: Some(state_binding),
            bindings: vec![executable, client],
            request,
            pending: None,
            unexpected: None,
            processes: Vec::new(),
            jobs: Vec::new(),
            external_pending: false,
        })
    }

    fn validate_leases(&self) -> TestResult {
        held(&self.profile)?.sid_string()?;
        held(&self.state_binding)?.revalidate()?;
        for binding in &self.bindings {
            binding.revalidate()?;
        }
        Ok(())
    }

    fn settle(&mut self, timeout: Duration) -> bool {
        if self.external_pending {
            return false;
        }
        let deadline = Instant::now() + timeout;
        if let Some(pending) = &mut self.pending
            && !matches!(pending.cleanup_timeout(remaining(deadline)), Ok(true))
        {
            return false;
        }
        if let Some(unexpected) = &mut self.unexpected {
            let child = &mut unexpected.child;
            if child.terminate(1).is_err()
                || !matches!(child.wait_tree_timeout(remaining(deadline)), Ok(true))
                || child.close_browser_context(remaining(deadline)).is_err()
                || child.retire_network().is_err()
            {
                return false;
            }
        }
        self.pending.take();
        self.unexpected.take();
        self.processes.clear();
        self.jobs.clear();
        if let Some(profile) = &mut self.profile
            && profile.close().is_err()
        {
            return false;
        }
        self.profile.take();
        self.state_binding.take();
        if let Some(state) = &mut self.state
            && state.remove_empty().is_err()
        {
            return false;
        }
        self.state.take();
        self.bindings.clear();
        true
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.settle(Duration::from_secs(1)) {
            // Installation/profile/state authority cannot be deleted beneath an
            // unacknowledged startup/death obligation, including assertion panic.
            preserve(&mut self.pending);
            preserve(&mut self.unexpected);
            preserve(&mut self.profile);
            preserve(&mut self.state);
            preserve(&mut self.state_binding);
            std::mem::forget(std::mem::take(&mut self.bindings));
            std::mem::forget(std::mem::take(&mut self.processes));
            std::mem::forget(std::mem::take(&mut self.jobs));
        }
    }
}

fn request(
    sid: String,
    home: PathBuf,
    executable: PathBuf,
    cwd: PathBuf,
) -> TestResult<SpawnRequest> {
    let system = system_windows_directory()?;
    let home = home.to_string_lossy().into_owned();
    Ok(SpawnRequest {
        executable,
        cwd,
        arguments: Vec::new(),
        environment: BTreeMap::from([
            ("SystemRoot".into(), system.to_string_lossy().into_owned()),
            ("windir".into(), system.to_string_lossy().into_owned()),
            (
                "PATH".into(),
                system.join("System32").to_string_lossy().into_owned(),
            ),
            ("HOME".into(), home.clone()),
            ("USERPROFILE".into(), home.clone()),
            ("LOCALAPPDATA".into(), home.clone()),
            ("APPDATA".into(), home.clone()),
            ("TEMP".into(), home.clone()),
            ("TMP".into(), home),
        ]),
        appcontainer_sid: sid,
        max_processes: 64,
        max_memory_bytes: 1024 * 1024 * 1024,
        proxy_port: None,
        network_filter_id: None,
    })
}

fn observe(source: &OwnedHandle, rights: u32) -> TestResult<OwnedHandle> {
    // SAFETY: duplicate target is this actual test process, never a helper.
    let remote = identity::duplicate(source.raw(), unsafe { GetCurrentProcess() }, rights, false)?;
    let raw = usize::try_from(remote)? as windows_sys::Win32::Foundation::HANDLE;
    Ok(OwnedHandle::new(
        raw,
        "retain actual startup test observer",
    )?)
}

fn run_fault(point: Point) -> TestResult {
    let mut fixture = Fixture::new()?;
    match spawn_at(&fixture.request, point, false) {
        Err(WindowsProcessError::BrowserSetup { source: _, cleanup }) => {
            fixture.pending = Some(cleanup)
        }
        Ok(child) => {
            fixture.unexpected = Some(child);
            return Err("startup fault did not fire".into());
        }
        Err(_) => return Err("startup rejected before the requested real boundary".into()),
    }
    let owner = held(&held(&fixture.pending)?.owner)?;
    let address = (&**owner) as *const BrowserSetupOwner as usize;
    let control = held(&owner.test_control)?;
    require(
        control.requested.is_none() && control.reached == Some(point),
        "wrong startup boundary reached",
    )?;
    let context = held(&owner.context)?;
    match point {
        Point::RequestQueued => require(
            context.cpwl_pending && context.launcher.is_some(),
            "queued CPWL bookkeeping absent",
        )?,
        Point::ReturnedContextBound => require(
            context.bound
                && context.returned.is_some()
                && !context.pulled_ack
                && context.thread.is_some(),
            "bound returned context custody absent",
        )?,
        Point::EndpointsRegistered => require(
            !context.browser_create_pending && owner.browser.is_none(),
            "endpoints phase crossed creation barrier",
        )?,
        Point::BrowserCommitQueued => require(
            context.browser_create_pending
                && context.browser_returned.is_none()
                && owner.browser.is_none(),
            "queued browser bookkeeping absent",
        )?,
        Point::BrowserPulled => require(
            context.browser_create_pending
                && context.browser_returned.is_some()
                && owner.browser.is_some()
                && !context.station_ready,
            "pulled suspended browser custody absent",
        )?,
        Point::StationReady => require(
            context.station_ready && !context.browser_create_pending && owner.browser.is_some(),
            "real station barrier absent",
        )?,
    }
    // Observer handles are private to this live Core test. They are deliberately
    // NOT used in last-Job-handle death tests, where any Job copy changes behavior.
    for process in [
        context.launcher.as_ref().map(|value| &value.process),
        context.process.as_ref(),
        owner.browser.as_ref().map(|value| &value.process),
        owner.partial_browser.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        // Revalidate live objects. An already exited launcher still has its
        // original attested Core handle; do not require live-image APIs after exit.
        if wait_timeout(process, Duration::ZERO)?.is_none() {
            // SAFETY: only observes the actual retained object, not PID reuse.
            identity::verify_process(
                process,
                unsafe { GetProcessId(process.raw()) },
                identity::birth(process.raw())?,
                fixture.bindings[0].identity(),
            )?;
        }
        fixture
            .processes
            .push(observe(process, PROCESS_QUERY_LIMITED_INFORMATION | SYNC)?);
    }
    for job in [
        owner.inner.as_ref().map(|value| &value.0),
        context.outer.as_ref().map(|value| &value.0),
    ]
    .into_iter()
    .flatten()
    {
        fixture.jobs.push(observe(job, JOB_OBJECT_QUERY)?);
    }
    fixture.validate_leases()?;
    let zero = held_mut(&mut fixture.pending)?.cleanup_timeout(Duration::ZERO);
    if !matches!(zero, Ok(true)) {
        let same = held(&held(&fixture.pending)?.owner)?;
        require(
            (&**same) as *const BrowserSetupOwner as usize == address,
            "cleanup replaced the pending owner",
        )?;
        for job in &fixture.jobs {
            wait_tree_timeout(job, Duration::ZERO)?;
        }
        fixture.validate_leases()?;
    }
    let mut clean = matches!(zero, Ok(true));
    for _ in 0..2 {
        if clean {
            break;
        }
        clean = matches!(
            held_mut(&mut fixture.pending)?.cleanup_timeout(Duration::from_secs(5)),
            Ok(true)
        );
    }
    require(
        clean && held(&fixture.pending)?.owner.is_none(),
        "startup cleanup remains unknown with custody retained",
    )?;
    for process in &fixture.processes {
        require(
            wait_timeout(process, Duration::ZERO)?.is_some(),
            "cleanup acknowledged a live retained process",
        )?;
    }
    for job in &fixture.jobs {
        require(
            wait_tree_timeout(job, Duration::ZERO)?,
            "cleanup acknowledged a nonempty retained Job",
        )?;
    }
    require(
        held_mut(&mut fixture.pending)?.cleanup_timeout(Duration::ZERO)?,
        "positive cleanup is not idempotent",
    )?;
    require(
        fixture.settle(Duration::from_secs(1)),
        "exact profile/state cleanup failed",
    )?;
    Ok(())
}

#[test]
#[ignore = "requires inventory-verified normal CEF stage/private state parent; real queued CPWL bookkeeping only"]
fn networkless_request_queued_failure_retains_real_cpwl_cleanup_custody() -> TestResult {
    run_fault(Point::RequestQueued)
}
#[test]
#[ignore = "requires inventory-verified normal CEF stage/private state parent; real returned context"]
fn networkless_returned_context_failure_retains_bound_process_custody() -> TestResult {
    run_fault(Point::ReturnedContextBound)
}
#[test]
#[ignore = "requires inventory-verified normal CEF stage/private state parent; eleven real endpoints"]
fn networkless_registered_endpoints_failure_drains_real_context_handles() -> TestResult {
    run_fault(Point::EndpointsRegistered)
}
#[test]
#[ignore = "requires inventory-verified normal CEF stage/private state parent; queued commit is not an outstanding-call proof"]
fn networkless_browser_commit_queued_failure_retires_creator_before_final_job_zero() -> TestResult {
    run_fault(Point::BrowserCommitQueued)
}
#[test]
#[ignore = "requires inventory-verified normal CEF stage/private state parent; real suspended browser PI before ACK"]
fn networkless_pulled_browser_failure_retains_both_suspended_process_owners() -> TestResult {
    run_fault(Point::BrowserPulled)
}
#[test]
#[ignore = "requires inventory-verified normal CEF stage/private state parent; actual station barrier before resume"]
fn networkless_station_ready_failure_retires_owned_station_and_context() -> TestResult {
    run_fault(Point::StationReady)
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires inventory-verified normal CEF stage/private state parent; exact returned known set, not CPWL pending"]
async fn networkless_core_death_after_returned_context_binding_retires_both_helpers() -> TestResult
{
    death::run(Point::ReturnedContextBound).await
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires inventory-verified normal CEF stage/private state parent; browser never resumes, no full CEF death claim"]
async fn networkless_core_death_after_station_ready_retires_context_and_appcontainer() -> TestResult
{
    death::run(Point::StationReady).await
}
