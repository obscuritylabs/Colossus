//! Trusted Core-side owner of the two independent native supervisor processes.

use crate::{DeploymentSetup, ProcessReceipt, receipt_output::ReceiptOutput, relay::OutputDrains};
use colossus_darwin_process::{
    DarwinAuditPeer, DarwinChild, DarwinProcessIdentity,
    spawn_suspended_owned_browser_channels_with_keeper, spawn_suspended_parent_channel,
};
use std::{
    ffi::OsString,
    fs::File,
    io,
    net::Shutdown,
    os::unix::net::UnixStream,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

/// Independently verified native package bindings; this is no browser request.
pub struct DeploymentLaunch {
    /// Fixed signed creator/keeper executable from native package composition.
    pub executable: PathBuf,
    /// Independently verified exact native executable CDHash.
    pub cdhash: [u8; 20],
    /// Fixed Main/helper package metadata and complete native environment.
    pub setup: DeploymentSetup,
}

/// Opaque existing authenticated Core/browser endpoints, with no Main PID.
pub struct DeploymentChannels {
    /// Main admission bytes relayed through its independently signed creator.
    pub bootstrap: UnixStream,
    /// Authenticated operation frames, unmodified by the creator.
    pub data: UnixStream,
    /// Authenticated cancellation frames, unmodified by the creator.
    pub control: UnixStream,
    /// Native compositor frames and input, unmodified by the creator.
    pub presentation: UnixStream,
    /// Private diagnostics only; receipt stdout stays with the retained owner.
    pub output: Vec<File>,
}

/// Operator-only observation for a sacrificial Core process; absent by default.
/// Only creator/keeper hints are exposed. They authorize no adoption or signal;
/// the coordinator must validate live code, actual parent relation and kernel
/// incarnation, then attach existing exit observers before allowing Core exit.
#[cfg(feature = "macos-process-observation-fixture")]
pub struct CoreFixtureObservation {
    /// Direct retained creator identifier, never Main or a helper identifier.
    pub creator_hint: u32,
    /// Direct retained keeper identifier, never Main or a helper identifier.
    pub keeper_hint: u32,
    /// Read-only plain receipt pipe, with no native control or lifetime rights.
    /// The coordinator must be its sole active reader after abrupt Core exit.
    pub receipt_reader: File,
    /// Exact keeper stdout/stderr and creator stderr read-only FIFOs. Core has
    /// never read them; the coordinator starts finite drains before crash ACK.
    pub diagnostic_readers: [File; 3],
}

/// Core retains both direct children before any validation or admission awaits.
///
/// Main and helpers remain actual children of the independent creator. No PID
/// sent by that process authorizes signaling/adoption. Complete browser-domain
/// acceptance remains absent because creator spawn-to-enrollment crashes and
/// simultaneous owner losses are not solved by this known-child composition.
pub struct CoreProcessOwner {
    deployment: DeploymentLaunch,
    attempted: bool,
    creator: Option<DarwinChild>,
    keeper: Option<DarwinChild>,
    core_fence: Option<UnixStream>,
    receipt_output: Option<ReceiptOutput>,
    output_drains: Vec<OutputDrains>,
    #[cfg(feature = "macos-process-observation-fixture")]
    fixture_output: Vec<File>,
    output_failed: bool,
    receipt: Option<ProcessReceipt>,
    creator_status: Option<i32>,
    keeper_status: Option<i32>,
    uncertain_creation: bool,
    closed: bool,
}

impl CoreProcessOwner {
    /// Validate trusted package metadata without creating any process.
    pub fn new(deployment: DeploymentLaunch) -> io::Result<Self> {
        deployment.setup.validate()?;
        if !deployment.executable.is_absolute() || deployment.cdhash == [0; 20] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid native supervisor package",
            ));
        }
        Ok(Self {
            deployment,
            attempted: false,
            creator: None,
            keeper: None,
            core_fence: None,
            receipt_output: None,
            output_drains: Vec::new(),
            #[cfg(feature = "macos-process-observation-fixture")]
            fixture_output: Vec::new(),
            output_failed: false,
            receipt: None,
            creator_status: None,
            keeper_status: None,
            uncertain_creation: false,
            closed: false,
        })
    }

    /// Allocate the independently signed keeper and creator exactly once.
    ///
    /// Retain this object before calling. Every error preserves exact direct
    /// children and uncertainty here for explicit cleanup; no returned error
    /// means that nothing was allocated.
    pub fn start(&mut self) -> io::Result<DeploymentChannels> {
        self.start_with_admission(|| Ok(()))
    }

    /// Check trusted local cancellation immediately before each native startup
    /// admission. This is cooperative cancellation, not atomic exclusion of
    /// cancellation against a kernel spawn or resume already in progress.
    /// Every partial allocation stays retained by this same owner on error.
    pub fn start_with_admission(
        &mut self,
        mut check: impl FnMut() -> io::Result<()>,
    ) -> io::Result<DeploymentChannels> {
        if self.attempted || self.closed {
            return Err(io::Error::other("native supervisor cannot reconnect"));
        }
        check()?;
        let environment = self
            .deployment
            .setup
            .environment
            .iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        check()?;
        self.attempted = true;
        self.uncertain_creation = true;
        let keeper = spawn_suspended_parent_channel(
            &self.deployment.executable,
            &["keeper".into()],
            &environment,
        )?;
        self.keeper = Some(keeper.child);
        self.uncertain_creation = false;
        self.retain_output(vec![keeper.stdout, keeper.stderr])?;
        admit(
            self.keeper.as_ref().expect("retained keeper"),
            self.deployment.cdhash,
        )?;
        check()?;
        self.keeper.as_mut().expect("retained keeper").resume()?;
        let mut keeper_channel = keeper.channel;
        keeper_channel.set_write_timeout(Some(Duration::from_secs(5)))?;
        check()?;
        self.deployment.setup.write(&mut keeper_channel)?;
        keeper_channel.set_write_timeout(None)?;
        check()?;
        self.uncertain_creation = true;
        let creator = spawn_suspended_owned_browser_channels_with_keeper(
            &self.deployment.executable,
            &["creator".into()],
            &environment,
            keeper_channel,
        )?;
        self.creator = Some(creator.child);
        self.uncertain_creation = false;
        self.receipt_output = Some(ReceiptOutput::new(creator.stdout));
        self.retain_output(vec![creator.stderr])?;
        self.core_fence = Some(creator.data.try_clone()?);
        admit(
            self.creator.as_ref().expect("retained creator"),
            self.deployment.cdhash,
        )?;
        check()?;
        self.creator.as_mut().expect("retained creator").resume()?;
        let mut bootstrap = creator.bootstrap;
        bootstrap.set_write_timeout(Some(Duration::from_secs(5)))?;
        check()?;
        self.deployment.setup.write(&mut bootstrap)?;
        bootstrap.set_write_timeout(None)?;
        Ok(DeploymentChannels {
            bootstrap,
            data: creator.data,
            control: creator.control,
            presentation: creator.presentation,
            output: Vec::new(),
        })
    }

    /// Revoke native channels, retain exact reaps and validate the creator receipt.
    ///
    /// Call only after the authenticated CEF shutdown attempt. Creator exit and
    /// EOF are not a CEF acknowledgement or complete helper-domain proof.
    pub fn close(&mut self) -> io::Result<()> {
        use std::os::unix::process::ExitStatusExt as _;
        if self.closed {
            return Ok(());
        }
        #[cfg(feature = "macos-process-observation-fixture")]
        if !self.fixture_output.is_empty() {
            self.output_drains.push(OutputDrains::start(std::mem::take(
                &mut self.fixture_output,
            ))?);
        }
        if let Some(stream) = self.core_fence.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        let mut failure = None;
        if let Some(creator) = self.creator.as_mut() {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match creator.try_wait() {
                    Ok(Some(status)) => {
                        self.creator_status = Some(status.into_raw());
                        break;
                    }
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Ok(None) => {
                        failure.get_or_insert_with(|| {
                            io::Error::new(
                                io::ErrorKind::TimedOut,
                                "independent creator did not finish",
                            )
                        });
                        break;
                    }
                    Err(error) => {
                        failure.get_or_insert(error);
                        break;
                    }
                }
            }
            if self.creator_status.is_none() {
                match creator.terminate_and_reap(Duration::from_secs(2)) {
                    Ok(evidence) => self.creator_status = Some(evidence.status().into_raw()),
                    Err(error) => {
                        failure.get_or_insert(error);
                    }
                }
            }
            if self.creator_status.is_some() {
                self.creator.take();
            }
        }
        if let Some(keeper) = self.keeper.as_mut() {
            let deadline = Instant::now() + Duration::from_secs(6);
            loop {
                match keeper.try_wait() {
                    Ok(Some(status)) => {
                        self.keeper_status = Some(status.into_raw());
                        break;
                    }
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Ok(None) => break,
                    Err(error) => {
                        failure.get_or_insert(error);
                        break;
                    }
                }
            }
            match keeper.terminate_and_reap(Duration::from_secs(5)) {
                Ok(evidence) => {
                    self.keeper_status = Some(evidence.status().into_raw());
                    self.keeper.take();
                }
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        if self.receipt.is_none()
            && self.creator_status.is_some()
            && let Some(output) = self.receipt_output.as_mut()
        {
            let result = output
                .drain(Duration::from_secs(2))
                .and_then(|mut bytes| ProcessReceipt::read(&mut bytes));
            match result {
                Ok(receipt) => self.receipt = Some(receipt),
                Err(error) => {
                    failure.get_or_insert(error);
                }
            }
        }
        for output in &mut self.output_drains {
            if let Err(error) = output.settle(Duration::from_secs(2)) {
                self.output_failed |= error.kind() != io::ErrorKind::TimedOut;
                failure.get_or_insert(error);
            }
        }
        if self.attempted
            && (self.uncertain_creation
                || self.creator_status.is_none()
                || self.keeper_status.is_none()
                || self.output_failed
                || self.receipt.is_none_or(|receipt| {
                    !receipt.known_children_closed(self.deployment.setup.nonce)
                }))
        {
            failure
                .get_or_insert_with(|| io::Error::other("native process cleanup remains unknown"));
        }
        if let Some(error) = failure {
            return Err(error);
        }
        self.closed = true;
        Ok(())
    }

    /// Exact independent known-child evidence only, retained after failure.
    #[must_use]
    pub fn receipt(&self) -> Option<ProcessReceipt> {
        self.receipt
    }

    /// Duplicate an untouched plain receipt reader for the finite crash fixture.
    /// This must not be used concurrently with `close`: the sacrificial process
    /// waits for coordinator observer admission and exits without running Drop.
    #[cfg(feature = "macos-process-observation-fixture")]
    pub fn crash_fixture_observation(&mut self) -> io::Result<CoreFixtureObservation> {
        if self.closed || self.uncertain_creation || self.receipt.is_some() {
            return Err(io::Error::other("fixture process ownership is not live"));
        }
        let creator = self
            .creator
            .as_mut()
            .ok_or_else(|| io::Error::other("missing creator"))?;
        let keeper = self
            .keeper
            .as_mut()
            .ok_or_else(|| io::Error::other("missing keeper"))?;
        if creator.try_wait()?.is_some() || keeper.try_wait()?.is_some() {
            return Err(io::Error::other("fixture supervisor has exited"));
        }
        admit(creator, self.deployment.cdhash)?;
        admit(keeper, self.deployment.cdhash)?;
        if self.fixture_output.len() != 3 || !self.output_drains.is_empty() {
            return Err(io::Error::other("fixture diagnostics already consumed"));
        }
        let receipt_reader = self
            .receipt_output
            .as_ref()
            .ok_or_else(|| io::Error::other("missing creator receipt"))?
            .untouched_reader()?;
        let diagnostic_readers = std::mem::take(&mut self.fixture_output)
            .try_into()
            .map_err(|_| io::Error::other("fixture diagnostics incomplete"))?;
        Ok(CoreFixtureObservation {
            creator_hint: creator.pid(),
            keeper_hint: keeper.pid(),
            receipt_reader,
            diagnostic_readers,
        })
    }

    /// Retained actual-parent raw statuses, separate from non-child observations.
    #[cfg(feature = "macos-process-observation-fixture")]
    pub fn fixture_supervisor_statuses(&self) -> [Option<i32>; 2] {
        [self.creator_status, self.keeper_status]
    }

    /// Bounded private native diagnostics for the isolated operator fixture.
    /// This returns no operation/control endpoint and is absent by default.
    #[cfg(feature = "macos-process-observation-fixture")]
    pub fn fixture_supervisor_output(&self) -> io::Result<Vec<Vec<u8>>> {
        self.output_drains
            .iter()
            .map(OutputDrains::captured)
            .collect::<io::Result<Vec<_>>>()
            .map(|outputs| outputs.into_iter().flatten().collect())
    }

    fn retain_output(&mut self, files: Vec<File>) -> io::Result<()> {
        #[cfg(feature = "macos-process-observation-fixture")]
        self.fixture_output.extend(files);
        #[cfg(not(feature = "macos-process-observation-fixture"))]
        self.output_drains.push(OutputDrains::start(files)?);
        Ok(())
    }
}

fn admit(child: &DarwinChild, expected: [u8; 20]) -> io::Result<()> {
    let identity = DarwinProcessIdentity::bind(child.pid())?;
    DarwinAuditPeer::enroll(&identity, expected)?.verify()
}

impl Drop for CoreProcessOwner {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeploymentImage;

    #[test]
    fn rejected_startup_checkpoints_never_attempt_native_creation() {
        for accepted_checks in 0..=1 {
            let mut owner = CoreProcessOwner::new(DeploymentLaunch {
                executable: "/private/tmp/unallocated-browser-test/broker".into(),
                cdhash: [1; 20],
                setup: DeploymentSetup {
                    nonce: [2; 16],
                    policy_sha256: [3; 32],
                    images: (0..=4)
                        .map(|role| DeploymentImage {
                            role,
                            executable: "/private/tmp/unallocated-browser-test/front".into(),
                            cdhash: [4; 20],
                            body_sha256: [5; 32],
                        })
                        .collect(),
                    environment: [
                        "HOME=/private/tmp/unallocated-browser-test/home",
                        "XDG_CONFIG_HOME=/private/tmp/unallocated-browser-test/config",
                        "XDG_DATA_HOME=/private/tmp/unallocated-browser-test/data",
                        "TMPDIR=/private/tmp/unallocated-browser-test/tmp",
                        "MAC_CHROMIUM_TMPDIR=/private/tmp/unallocated-browser-test/tmp",
                        "PATH=/usr/bin:/bin",
                        "LANG=en_US.UTF-8",
                        "MallocNanoZone=0",
                    ]
                    .map(String::from)
                    .into(),
                },
            })
            .unwrap();
            let mut checks = 0;
            let error = owner
                .start_with_admission(|| {
                    checks += 1;
                    if checks > accepted_checks {
                        Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
                    } else {
                        Ok(())
                    }
                })
                .err()
                .unwrap();
            assert_eq!(error.kind(), io::ErrorKind::Interrupted);
            assert!(!owner.attempted);
            assert!(owner.creator.is_none());
            assert!(owner.keeper.is_none());
            assert!(!owner.uncertain_creation);
            assert!(owner.receipt().is_none());
        }
    }
}
