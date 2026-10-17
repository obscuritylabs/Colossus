use crate::{
    CreationFence, DeathFence, Frame, HelperRole, KeeperClient, Message, OwnerLeases,
    SpawnArguments, arguments::invalid,
};
use colossus_darwin_process::{
    DarwinAuditPeer, DarwinProcessIdentity, SpawnedParentChannel, spawn_suspended_parent_channel,
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    os::unix::{net::UnixStream, process::ExitStatusExt},
    path::PathBuf,
};

/// One fixed signed front from an independently verified allocation manifest.
#[derive(Clone)]
pub struct HelperImage {
    /// Role selects the immutable policy and fixed native front.
    pub role: HelperRole,
    /// Trusted canonical executable; never supplied on the browser channel.
    pub executable: PathBuf,
    /// Exact front cdhash from the verified manifest.
    pub cdhash: [u8; 20],
    /// Fixed sibling Body.dylib SHA256 from independently verified native inventory.
    pub body_sha256: [u8; 32],
}

/// Native allocation metadata that a browser cannot nominate or update.
pub struct Allocation {
    /// Random, nonzero native allocation identity.
    pub nonce: [u8; 16],
    /// Exact sealed policy resource from independently verified native package.
    pub policy_sha256: [u8; 32],
    /// Exact already enrolled logical browser process.
    pub browser: DarwinAuditPeer,
    /// Fixed signed role images.
    pub images: Vec<HelperImage>,
    /// Fixed native environment; the browser protocol has no environment field.
    pub environment: Vec<OsString>,
}

struct Member {
    child: SpawnedParentChannel,
    resumed: bool,
    status: Option<i32>,
    enrollment: KeeperEnrollment,
    role: u8,
    body_sha256: [u8; 32],
}

enum KeeperEnrollment {
    NotAttempted,
    Pending,
    Acknowledged,
    Reaped,
}

impl KeeperEnrollment {
    fn acknowledge_exit(&mut self, keeper: &mut KeeperClient, slot: u64) -> io::Result<()> {
        match self {
            Self::Acknowledged => {
                keeper.reaped(slot)?;
                *self = Self::Reaped;
                Ok(())
            }
            Self::Pending => Err(invalid("keeper enrollment acknowledgement is uncertain")),
            // Identity failures before any Enroll create no keeper slot.
            // Sending Reaped here would poison unrelated live obligations.
            Self::NotAttempted | Self::Reaped => Ok(()),
        }
    }
}

/// Bounded direct-child owner and two-phase creation server.
///
/// Every successful spawn is inserted before identity validation or keeper I/O.
/// Child Drop owns kill/reap on all returned errors. This owner cannot issue
/// whole-browser cleanup receipts: the frontend/CEF integration, immutable
/// execution policy and partial-creation crash boundary remain unaccepted.
pub struct ProcessBroker {
    allocation: Allocation,
    keeper: KeeperClient,
    members: BTreeMap<u64, Member>,
    next_slot: u64,
    sequence: u64,
    closed: bool,
    fence: Option<CreationFence>,
    unknown_creations: u16,
    keeper_empty: bool,
    main_owned: bool,
    output_drains: Vec<crate::relay::OutputDrains>,
    output_failed: bool,
}

impl ProcessBroker {
    /// Construct from trusted native composition, never from browser requests.
    pub fn new(allocation: Allocation, keeper: KeeperClient) -> io::Result<Self> {
        Self::validate_images(
            allocation.nonce,
            &allocation.images,
            &allocation.environment,
        )?;
        allocation.browser.verify()?;
        if allocation.policy_sha256 == [0; 32] {
            return Err(invalid("missing immutable custody policy binding"));
        }
        Ok(Self {
            allocation,
            keeper,
            members: BTreeMap::new(),
            next_slot: 1,
            sequence: 0,
            closed: false,
            fence: None,
            unknown_creations: 0,
            keeper_empty: false,
            main_owned: false,
            output_drains: Vec::new(),
            output_failed: false,
        })
    }

    pub(crate) fn validate_images(
        nonce: [u8; 16],
        images: &[HelperImage],
        environment: &[OsString],
    ) -> io::Result<()> {
        if nonce == [0; 16] || images.is_empty() || images.len() > 4 || environment.len() > 16 {
            return Err(invalid("invalid trusted browser allocation"));
        }
        let mut environment_names = std::collections::BTreeSet::new();
        for entry in environment {
            let value = entry
                .to_str()
                .ok_or_else(|| invalid("non-UTF-8 helper environment"))?;
            let (name, value) = value
                .split_once('=')
                .ok_or_else(|| invalid("invalid helper environment entry"))?;
            if !environment_names.insert(name) || value.len() > 4096 || value.contains('\0') {
                return Err(invalid("duplicate or oversized helper environment"));
            }
            let valid = match name {
                "HOME" | "XDG_CONFIG_HOME" | "XDG_DATA_HOME" | "TMPDIR" | "MAC_CHROMIUM_TMPDIR" => {
                    value.starts_with('/') && !value.contains('\n')
                }
                "PATH" => value == "/usr/bin:/bin",
                "LANG" => value == "en_US.UTF-8",
                "MallocNanoZone" => value == "0",
                _ => false,
            };
            if !valid {
                return Err(invalid("unreviewed helper environment"));
            }
        }
        if environment_names.len() != 8 {
            return Err(invalid("incomplete fixed helper environment"));
        }
        let mut roles = std::collections::BTreeSet::new();
        for image in images {
            if !image.executable.is_absolute()
                || image.cdhash == [0; 20]
                || image.body_sha256 == [0; 32]
                || !roles.insert(image.role as u8)
            {
                return Err(invalid("invalid trusted helper image"));
            }
        }
        Ok(())
    }

    pub(crate) fn for_owned_main(
        allocation: Allocation,
        keeper: KeeperClient,
        fence: CreationFence,
    ) -> io::Result<Self> {
        let mut broker = Self::new(allocation, keeper)?;
        broker.main_owned = true;
        broker.next_slot = 2; // Keeper slot one belongs to creator-owned Main.
        broker.fence = Some(fence);
        Ok(broker)
    }

    pub(crate) fn keeper(&mut self) -> &mut KeeperClient {
        &mut self.keeper
    }

    pub(crate) fn serve_owned_main(&mut self, stream: UnixStream) -> io::Result<()> {
        if !self.main_owned || self.sequence != 0 {
            return Err(invalid("creator Main cannot reconnect"));
        }
        self.serve_stream(stream)
    }

    /// Serve the inherited browser-only byte stream, with no ancillary receive.
    ///
    /// Core/Desktop parent-death composition must close this channel or invoke
    /// `close`. Browser EOF and all protocol errors fence creation before cleanup.
    /// There is no reconnect, ambient service discovery or launch fallback.
    pub fn serve(&mut self, stream: UnixStream, leases: OwnerLeases) -> io::Result<()> {
        if self.closed || self.fence.is_some() {
            return Err(invalid("browser creator cannot reconnect"));
        }
        let mut watchers = DeathFence::start(leases, &stream)?;
        self.fence = Some(watchers.creation_fence());
        let result = self.serve_stream(stream);
        let retired = watchers.retire();
        result.and(retired)
    }

    fn serve_stream(&mut self, mut stream: UnixStream) -> io::Result<()> {
        let result = (|| loop {
            let frame = Frame::read(&mut stream)?;
            if frame.nonce != self.allocation.nonce
                || self.sequence.checked_add(1) != Some(frame.sequence)
            {
                return Err(invalid("browser broker replay or allocation mismatch"));
            }
            self.sequence = frame.sequence;
            self.allocation.browser.verify()?;
            let message = self.apply(frame.message)?;
            Frame {
                sequence: self.sequence,
                nonce: self.allocation.nonce,
                message,
            }
            .write(&mut stream)?;
            if self.closed {
                return Ok(());
            }
        })();
        let cleanup = self.close().map(|_| ());
        result.and(cleanup)
    }

    fn apply(&mut self, message: Message) -> io::Result<Message> {
        match message {
            Message::Spawn(arguments) => self.spawn(arguments),
            Message::Resume(slot) => {
                self.require_creation_owner()?;
                if self.closed {
                    return Err(invalid("broker creation is fenced"));
                }
                let fence = self
                    .fence
                    .clone()
                    .ok_or_else(|| invalid("native death watchers are not installed"))?;
                let member = self
                    .members
                    .get_mut(&slot)
                    .ok_or_else(|| invalid("unknown helper slot"))?;
                if member.resumed || member.status.is_some() {
                    return Err(invalid("helper cannot be resumed twice"));
                }
                fence.admit(|| member.child.child.resume())?;
                member.resumed = true;
                fence.check()?;
                crate::custody_bootstrap::admit(
                    &mut member.child.channel,
                    self.allocation.nonce,
                    member.role,
                    member.body_sha256,
                    self.allocation.policy_sha256,
                )?;
                fence.check()?;
                Ok(Message::Reply {
                    slot,
                    pid: member.child.child.pid(),
                    status: None,
                })
            }
            Message::Status(slot) => self.status(slot, false),
            Message::Terminate(slot) => self.status(slot, true),
            Message::Close => self.close(),
            _ => Err(invalid("native-only message on browser broker channel")),
        }
    }

    fn spawn(&mut self, arguments: SpawnArguments) -> io::Result<Message> {
        self.require_creation_owner()?;
        if self.closed || self.members.len() >= if self.main_owned { 63 } else { 64 } {
            return Err(invalid("broker creation is fenced or exhausted"));
        }
        let image = self
            .allocation
            .images
            .iter()
            .find(|image| image.role == arguments.role())
            .ok_or_else(|| invalid("helper role is not admitted"))?
            .clone();
        let slot = self.next_slot;
        self.next_slot = self
            .next_slot
            .checked_add(1)
            .ok_or_else(|| invalid("helper slots exhausted"))?;
        let arguments = arguments
            .values()
            .iter()
            .map(OsString::from)
            .collect::<Vec<_>>();
        let fence = self
            .fence
            .clone()
            .ok_or_else(|| invalid("native death watchers are not installed"))?;
        let child = fence.admit(|| {
            self.unknown_creations = self
                .unknown_creations
                .checked_add(1)
                .ok_or_else(|| invalid("unknown creation bound exceeded"))?;
            spawn_suspended_parent_channel(
                &image.executable,
                &arguments,
                &self.allocation.environment,
            )
        })?;
        let pid = child.child.pid();
        self.members.insert(
            slot,
            Member {
                child,
                resumed: false,
                status: None,
                enrollment: KeeperEnrollment::NotAttempted,
                role: image.role as u8,
                body_sha256: image.body_sha256,
            },
        );
        // Clear unknown only after owning the exact returned child. A launch
        // error cannot prove that the launcher created no process internally.
        self.unknown_creations -= 1;
        // Retain before fallible duplication/thread setup. Constant-buffer
        // native drains service every helper's output while it is running.
        let member = self
            .members
            .get(&slot)
            .ok_or_else(|| invalid("retained helper disappeared"))?;
        let output = (|| {
            crate::relay::OutputDrains::start(vec![
                member.child.stdout.try_clone()?,
                member.child.stderr.try_clone()?,
            ])
        })();
        match output {
            Ok(output) => self.output_drains.push(output),
            Err(error) => {
                self.output_failed = true;
                return Err(error);
            }
        }
        self.require_creation_owner()?;
        // Successful creation is retained before either independent validation.
        let identity = DarwinProcessIdentity::bind(pid)?;
        DarwinAuditPeer::enroll(&identity, image.cdhash)?;
        self.members
            .get_mut(&slot)
            .ok_or_else(|| invalid("retained helper disappeared"))?
            .enrollment = KeeperEnrollment::Pending;
        self.keeper.enroll(slot, pid, image.cdhash)?;
        self.members
            .get_mut(&slot)
            .ok_or_else(|| invalid("retained helper disappeared"))?
            .enrollment = KeeperEnrollment::Acknowledged;
        self.require_creation_owner()?;
        // Fixed broker-created fd3 socket, not a browser descriptor or argument. The
        // trusted frontend consumes it before applying the final policy/dlopen.
        let member = self
            .members
            .get_mut(&slot)
            .ok_or_else(|| invalid("retained helper disappeared"))?;
        crate::custody_bootstrap::write(
            &mut member.child.channel,
            self.allocation.nonce,
            self.allocation.browser.identity().pid(),
            image.role as u8,
            image.body_sha256,
            self.allocation.policy_sha256,
        )?;
        Ok(Message::Reply {
            slot,
            pid,
            status: None,
        })
    }

    fn status(&mut self, slot: u64, terminate: bool) -> io::Result<Message> {
        let member = self
            .members
            .get_mut(&slot)
            .ok_or_else(|| invalid("unknown helper slot"))?;
        if member.status.is_none() {
            let status = if terminate {
                Some(member.child.child.kill_and_reap()?)
            } else {
                member.child.child.try_wait()?
            };
            if let Some(status) = status {
                member.status = Some(status.into_raw());
                #[cfg(feature = "macos-process-entry-development")]
                {
                    use std::io::Write as _;
                    let _ = writeln!(
                        std::io::stderr().lock(),
                        "EVIDENCE browser-helper-exit role={} raw_status={}",
                        member.role,
                        member.status.expect("retained wait status")
                    );
                }
            }
        }
        if member.status.is_some() {
            member.enrollment.acknowledge_exit(&mut self.keeper, slot)?;
        }
        Ok(Message::Reply {
            slot,
            pid: member.child.child.pid(),
            status: member.status,
        })
    }

    /// Fence admission before killing and reaping every known direct child.
    ///
    /// Success is a direct-child ledger result only. It cannot erase an unknown
    /// partial-creation obligation or acknowledge the missing kernel domain.
    pub fn close(&mut self) -> io::Result<Message> {
        self.closed = true;
        let slots = self.members.keys().copied().collect::<Vec<_>>();
        let mut failure = None;
        for slot in slots {
            if let Err(error) = self.status(slot, true) {
                failure.get_or_insert(error);
            }
        }
        for output in &mut self.output_drains {
            if let Err(error) = output.settle(std::time::Duration::from_secs(2)) {
                self.output_failed |= error.kind() != io::ErrorKind::TimedOut;
                failure.get_or_insert(error);
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        if !self.keeper_empty && !self.main_owned {
            self.keeper_empty = self.keeper.confirm_empty()?;
        }
        Ok(Message::Shutdown {
            reaped: self.members.len() as u16,
            unknown: self.unknown_creations,
            keeper_empty: self.keeper_empty,
        })
    }
    /// Inspect retained cleanup state after any serve/close error.
    ///
    /// This remains available when a keeper acknowledgement fails, so native
    /// composition can persist uncertain allocation ownership instead of
    /// discarding it. Counts refer only to known direct children, never a kernel
    /// process domain. Nonzero unknown or an absent keeper acknowledgement fences
    /// allocation reuse independently of the known-child count.
    pub fn retained_cleanup(&self) -> Message {
        Message::Shutdown {
            reaped: self
                .members
                .values()
                .filter(|member| member.status.is_some())
                .count() as u16,
            unknown: self.unknown_creations,
            keeper_empty: self.keeper_empty,
        }
    }

    pub(crate) fn output_drained(&self) -> bool {
        !self.output_failed
            && self
                .output_drains
                .iter()
                .all(crate::relay::OutputDrains::is_settled)
    }
    fn require_creation_owner(&self) -> io::Result<()> {
        if self.closed {
            return Err(invalid("broker creation is fenced"));
        }
        self.fence
            .as_ref()
            .ok_or_else(|| invalid("native death watchers are not installed"))?
            .check()
    }
}
impl Drop for ProcessBroker {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
#[path = "owner_tests.rs"]
mod tests;
