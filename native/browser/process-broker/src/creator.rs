//! Independent native creator owns Main and every delegated helper directly.

use crate::{
    Allocation, DeathFence, HelperImage, KeeperClient, Message, OwnerDeath, OwnerLeases,
    ProcessBroker, arguments::invalid,
};
use colossus_darwin_process::{
    DarwinAuditPeer, DarwinChild, DarwinProcessIdentity, SpawnedOwnedBrowserChannels,
    spawn_suspended_owned_browser_channels,
};
use std::{
    ffi::OsString,
    fs::File,
    io,
    os::unix::{net::UnixStream, process::ExitStatusExt},
    path::PathBuf,
    time::Duration,
};

const MAIN_SLOT: u64 = 1;

/// Fixed Main front and body hashes from trusted native package verification.
pub struct MainImage {
    /// Canonical fixed no-exec Main front, never nominated by the browser.
    pub executable: PathBuf,
    /// Exact signed front code hash from the immutable native inventory.
    pub cdhash: [u8; 20],
    /// SHA256 of the fixed sibling Main body library.
    pub body_sha256: [u8; 32],
}

/// Creator-only metadata; it deliberately has no caller-created Main PID.
pub struct CreatorAllocation {
    /// Nonzero private allocation identity.
    pub nonce: [u8; 16],
    /// Exact immutable role-policy resource from verified native composition.
    pub policy_sha256: [u8; 32],
    /// Fixed Main front and dylib inventory bindings.
    pub main: MainImage,
    /// Reviewed helper role images, outside browser protocol authority.
    pub helpers: Vec<HelperImage>,
    /// Complete fixed environment shared by Main and helpers.
    pub environment: Vec<OsString>,
}

/// Main's operation endpoints, with process ownership retained by the creator.
///
/// Trusted native composition forwards these endpoints to the authorized Core
/// and presentation owners. FD seven, the keeper channel, and lifetime leases
/// remain creator-only. Output readers require bounded native drains.
pub struct MainChannels {
    /// Admission channel connected to Main's fixed descriptor three.
    pub bootstrap: UnixStream,
    /// Operation channel connected to fixed descriptor four.
    pub data: UnixStream,
    /// Cancellation channel connected to fixed descriptor five.
    pub control: UnixStream,
    /// Native presentation channel connected to fixed descriptor six.
    pub presentation: UnixStream,
    /// Separate Main stdout reader for a bounded native drain.
    pub stdout: File,
    /// Separate Main stderr reader for a bounded native drain.
    pub stderr: File,
}

/// Retained exact-child evidence, never a whole-browser cleanup receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreatorCleanup {
    /// Actual parent's retained Main wait status, if reaped.
    pub main_status: Option<i32>,
    /// Known delegated helpers with actual-parent terminal wait status.
    pub helpers_reaped: u16,
    /// Creation attempts without a positively retained returned child.
    pub unknown_creations: u16,
    /// Independently observed known roster positively acknowledged empty.
    pub keeper_empty: bool,
    /// Every retained helper's independent stdout/stderr drains observed EOF.
    pub helper_output_drained: bool,
    /// First native lifetime lease loss, preserved after orderly retirement.
    pub owner_death: Option<OwnerDeath>,
}

/// Independent owner of Main and all its delegated helper creation.
///
/// Construct and run this inside a separately supervised native creator process;
/// hosting it inside Core/Desktop does not establish parent-crash acceptance.
/// Lifetime watchers start before Main creation. Successful Main creation is
/// retained before signed-code validation, keeper enrollment, bootstrap writes,
/// or resume. There is no PID adoption or process-group membership inference.
/// Creator death between kernel spawn and retention/enrollment remains unresolved.
pub struct BrowserCreator {
    allocation: CreatorAllocation,
    watchers: DeathFence,
    keeper: Option<KeeperClient>,
    broker: Option<ProcessBroker>,
    main: Option<DarwinChild>,
    creator_stream: Option<UnixStream>,
    channels: Option<MainChannels>,
    spawn_attempted: bool,
    main_enrolled: bool,
    main_keeper_reaped: bool,
    main_status: Option<i32>,
    unknown_creations: u16,
    keeper_empty: bool,
    closed: bool,
    markers: Option<crate::custody_markers::Markers>,
}

impl BrowserCreator {
    /// Validate trusted metadata and install native EOF readers before spawning.
    ///
    /// The supplied keeper endpoint and leases must already be authenticated by
    /// native composition, with no browser/renderer copies or reconnect path.
    pub fn new(
        allocation: CreatorAllocation,
        keeper: KeeperClient,
        leases: OwnerLeases,
    ) -> io::Result<Self> {
        ProcessBroker::validate_images(
            allocation.nonce,
            &allocation.helpers,
            &allocation.environment,
        )?;
        if !allocation.main.executable.is_absolute()
            || allocation.main.cdhash == [0; 20]
            || allocation.main.body_sha256 == [0; 32]
        {
            return Err(invalid("invalid trusted Main image"));
        }
        Ok(Self {
            allocation,
            watchers: DeathFence::start_before_creation(leases)?,
            keeper: Some(keeper),
            broker: None,
            main: None,
            creator_stream: None,
            channels: None,
            spawn_attempted: false,
            main_enrolled: false,
            main_keeper_reaped: false,
            main_status: None,
            unknown_creations: 0,
            keeper_empty: false,
            closed: false,
            markers: None,
        })
    }

    /// Create, retain and independently enroll stopped Main exactly once.
    ///
    /// This method does not resume it. Call `serve` on the independent creator
    /// after transferring the fixed operation endpoints. Every returned error
    /// leaves cleanup state on this owner; callers must close and inspect it.
    pub fn create_main(&mut self) -> io::Result<MainChannels> {
        self.require_owner()?;
        if self.spawn_attempted {
            return Err(invalid("creator Main cannot be recreated"));
        }
        self.spawn_attempted = true;
        self.markers = Some(crate::custody_markers::Markers::prepare(
            &self.allocation.main.executable,
            self.allocation.policy_sha256,
        )?);
        let fence = self.watchers.creation_fence();
        let spawned = fence.admit(|| {
            self.unknown_creations = 1;
            spawn_suspended_owned_browser_channels(
                &self.allocation.main.executable,
                &["3".into(), "4".into(), "5".into(), "6".into()],
                &self.allocation.environment,
            )
        })?;
        let SpawnedOwnedBrowserChannels {
            child,
            bootstrap,
            data,
            control,
            presentation,
            creator,
            stdout,
            stderr,
        } = spawned;
        // Retain first. Everything following this assignment can fail without
        // losing the exact actual-parent reap obligation.
        self.main = Some(child);
        self.unknown_creations = 0;
        self.creator_stream = Some(creator);
        self.channels = Some(MainChannels {
            bootstrap,
            data,
            control,
            presentation,
            stdout,
            stderr,
        });
        self.require_owner()?;
        let child = self
            .main
            .as_ref()
            .ok_or_else(|| invalid("retained Main disappeared"))?;
        let pid = child.pid();
        let identity = DarwinProcessIdentity::bind(pid)?;
        let browser = DarwinAuditPeer::enroll(&identity, self.allocation.main.cdhash)?;
        let cdhash = self.allocation.main.cdhash;
        self.keeper_mut()?.enroll(MAIN_SLOT, pid, cdhash)?;
        self.main_enrolled = true;
        self.require_owner()?;
        let creator = self
            .creator_stream
            .as_mut()
            .ok_or_else(|| invalid("retained creator stream disappeared"))?;
        self.watchers.attach_wakeup(creator)?;
        creator.set_write_timeout(Some(Duration::from_secs(2)))?;
        crate::custody_bootstrap::write(
            creator,
            self.allocation.nonce,
            pid,
            0,
            self.allocation.main.body_sha256,
            self.allocation.policy_sha256,
        )?;
        self.require_owner()?;
        let keeper = self
            .keeper
            .take()
            .ok_or_else(|| invalid("creator keeper disappeared"))?;
        self.broker = Some(ProcessBroker::for_owned_main(
            Allocation {
                nonce: self.allocation.nonce,
                policy_sha256: self.allocation.policy_sha256,
                browser,
                images: self.allocation.helpers.clone(),
                environment: self.allocation.environment.clone(),
            },
            keeper,
            self.watchers.creation_fence(),
        )?);
        self.channels
            .take()
            .ok_or_else(|| invalid("Main channels were already transferred"))
    }

    /// Resume enrolled Main and own delegated helper service until shutdown.
    ///
    /// Core/Desktop/keeper death wakes an idle or partial-frame service. Every
    /// exit fences creation, reaps retained helpers and Main, then asks the keeper
    /// for its independently observed known-roster state. Errors remain visible
    /// through `retained_cleanup`; EOF is not a graceful Chromium acknowledgement.
    pub fn serve(&mut self) -> io::Result<()> {
        let served = (|| {
            self.require_owner()?;
            self.markers
                .as_ref()
                .ok_or_else(|| invalid("custody markers were not prepared"))?
                .revalidate()?;
            if !self.main_enrolled || self.broker.is_none() {
                return Err(invalid("creator Main has not been admitted"));
            }
            let mut stream = self
                .creator_stream
                .take()
                .ok_or_else(|| invalid("creator Main cannot reconnect"))?;
            self.watchers.creation_fence().admit(|| {
                self.main
                    .as_mut()
                    .ok_or_else(|| invalid("retained Main disappeared"))?
                    .resume()
            })?;
            self.require_owner()?;
            crate::custody_bootstrap::admit(
                &mut stream,
                self.allocation.nonce,
                0,
                self.allocation.main.body_sha256,
                self.allocation.policy_sha256,
            )?;
            self.require_owner()?;
            self.broker
                .as_mut()
                .ok_or_else(|| invalid("retained helper owner disappeared"))?
                .serve_owned_main(stream)
        })();
        let cleanup = self.close();
        served.and(cleanup)
    }

    /// Fence creation and attempt exact actual-parent cleanup for Main/helpers.
    pub fn close(&mut self) -> io::Result<()> {
        self.closed = true;
        self.creator_stream.take();
        self.channels.take();
        let mut failure = None;
        if let Some(broker) = &mut self.broker
            && let Err(error) = broker.close()
        {
            failure.get_or_insert(error);
        }
        if self.main_status.is_none()
            && let Some(main) = &mut self.main
        {
            match main.kill_and_reap() {
                Ok(status) => self.main_status = Some(status.into_raw()),
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        if self.main_enrolled && self.main_status.is_some() && !self.main_keeper_reaped {
            match self
                .keeper_mut()
                .and_then(|keeper| keeper.reaped(MAIN_SLOT))
            {
                Ok(()) => self.main_keeper_reaped = true,
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        if !self.keeper_empty {
            match self.keeper_mut().and_then(KeeperClient::confirm_empty) {
                Ok(empty) => self.keeper_empty = empty,
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        if let Err(error) = self.watchers.retire() {
            failure.get_or_insert(error);
        }
        if self.keeper_empty
            && self.unknown_creations == 0
            && self.main_status.is_some()
            && let Some(markers) = &mut self.markers
            && let Err(error) = markers.finish()
        {
            failure.get_or_insert(error);
        }
        failure.map_or(Ok(()), Err)
    }

    /// Inspect retained cleanup facts after any preparation/service/close error.
    pub fn retained_cleanup(&self) -> CreatorCleanup {
        let (helpers_reaped, helper_unknown) = match self.broker.as_ref() {
            Some(broker) => match broker.retained_cleanup() {
                Message::Shutdown {
                    reaped, unknown, ..
                } => (reaped, unknown),
                _ => (0, 0),
            },
            None => (0, 0),
        };
        CreatorCleanup {
            main_status: self.main_status,
            helpers_reaped,
            unknown_creations: self.unknown_creations.saturating_add(helper_unknown),
            keeper_empty: self.keeper_empty,
            helper_output_drained: self
                .broker
                .as_ref()
                .is_none_or(ProcessBroker::output_drained),
            owner_death: self.watchers.creation_fence().owner_death(),
        }
    }

    fn keeper_mut(&mut self) -> io::Result<&mut KeeperClient> {
        match &mut self.broker {
            Some(broker) => Ok(broker.keeper()),
            None => self
                .keeper
                .as_mut()
                .ok_or_else(|| invalid("creator keeper acknowledgement is unavailable")),
        }
    }

    fn require_owner(&self) -> io::Result<()> {
        if self.closed {
            return Err(invalid("independent creator is closed"));
        }
        self.watchers.creation_fence().check()
    }
}

impl Drop for BrowserCreator {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
