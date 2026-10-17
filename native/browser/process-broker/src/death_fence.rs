//! Native-only EOF leases that wake an idle creator and fence further execution.

use std::{
    io::{self, Read},
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    thread::{self, JoinHandle},
};

const RETIRED: u8 = 255;

use crate::OwnerDeath;

/// Read endpoints of native-composition lifetime socket pairs.
///
/// Each owner retains its unique opposite endpoint, writes no bytes, and never
/// passes a duplicate to browser/helper processes. EOF, I/O failure and unexpected
/// bytes all revoke ownership. This structure grants no reconnect or renewal.
pub struct OwnerLeases {
    /// Required Core/sidecar parent lease.
    pub core: UnixStream,
    /// Optional Desktop lease for Embedded placement.
    pub desktop: Option<UnixStream>,
    /// Required independent keeper-peer lease.
    pub keeper: UnixStream,
}

/// Shared creation fence; clones contain no descriptors or authority.
#[derive(Clone)]
pub struct CreationFence(Arc<Admission>);

struct Admission {
    cause: AtomicU8,
    action: Mutex<()>,
}

impl CreationFence {
    /// Reject execution after owner loss or orderly admission retirement.
    pub fn check(&self) -> io::Result<()> {
        if self.0.cause.load(Ordering::Acquire) == 0 {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "native owner lease is closed",
            ))
        }
    }

    /// Serialize one finite native creation/resume with owner-loss publication.
    ///
    /// Once loss is published, no admitted action can begin. An action already
    /// holding admission finishes before publication; callers must retain its
    /// returned exact child immediately and recheck ownership before proceeding.
    /// Do not hold this guard across browser/keeper I/O or an unbounded wait.
    pub fn admit<T>(&self, action: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        let _admission = self
            .0
            .action
            .lock()
            .map_err(|_| io::Error::other("native creation admission failed"))?;
        self.check()?;
        action()
    }
    /// First owner-loss cause, preserved across orderly teardown.
    pub fn owner_death(&self) -> Option<OwnerDeath> {
        match self.0.cause.load(Ordering::Acquire) {
            1 => Some(OwnerDeath::Core),
            2 => Some(OwnerDeath::Desktop),
            3 => Some(OwnerDeath::Keeper),
            _ => None,
        }
    }
}

/// Bounded native lease readers and an idle-browser wakeup handle.
///
/// A death atomically fences creation before shutting down the underlying browser
/// socket. This wakes even a server blocked halfway through a frame. Orderly
/// retirement uses a separate state and wakes only lease readers, preserving the
/// browser channel long enough to return its cleanup acknowledgement.
pub struct DeathFence {
    state: CreationFence,
    readers: Vec<(UnixStream, JoinHandle<()>)>,
    wakeups: Arc<Mutex<Vec<UnixStream>>>,
}

impl DeathFence {
    /// Start at most three native reader threads; the browser gets no lease FDs.
    pub fn start(leases: OwnerLeases, browser: &UnixStream) -> io::Result<Self> {
        let mut owner = Self::start_before_creation(leases)?;
        owner.attach_wakeup(browser)?;
        Ok(owner)
    }

    /// Install lifetime readers before an independently owned Main is created.
    ///
    /// The stopped child and its creator stream do not yet exist. Admission is
    /// already fenced by owner loss; attach the returned stream before resume.
    pub fn start_before_creation(leases: OwnerLeases) -> io::Result<Self> {
        let state = CreationFence(Arc::new(Admission {
            cause: AtomicU8::new(0),
            action: Mutex::new(()),
        }));
        let mut owner = Self {
            state,
            readers: Vec::with_capacity(3),
            wakeups: Arc::new(Mutex::new(Vec::with_capacity(2))),
        };
        let mut endpoints = vec![
            (OwnerDeath::Core, leases.core),
            (OwnerDeath::Keeper, leases.keeper),
        ];
        if let Some(desktop) = leases.desktop {
            endpoints.push((OwnerDeath::Desktop, desktop));
        }
        for (cause, mut endpoint) in endpoints {
            let reader_control = endpoint.try_clone()?;
            let wakeups = owner.wakeups.clone();
            let state = owner.state.clone();
            let reader = thread::Builder::new()
                .name("browser-owner-lease".into())
                .spawn(move || {
                    let mut byte = [0];
                    loop {
                        match endpoint.read(&mut byte) {
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                            _ => break,
                        }
                    }
                    // A panicking admitted action must not prevent owner EOF
                    // from fencing and waking the creator's retained channel.
                    let _admission = state
                        .0
                        .action
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if state
                        .0
                        .cause
                        .compare_exchange(0, cause as u8, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                        && let Ok(streams) = wakeups.lock()
                    {
                        for stream in streams.iter() {
                            let _ = stream.shutdown(Shutdown::Both);
                        }
                    }
                })?;
            owner.readers.push((reader_control, reader));
        }
        Ok(owner)
    }

    /// Wake this native-only operation stream on owner loss, before resume.
    ///
    /// Registration and the death reader share one lock, so loss racing stream
    /// attachment cannot miss its shutdown. Late attachment is rejected and the
    /// stream is closed. At most two wakeup streams may be retained.
    pub fn attach_wakeup(&mut self, stream: &UnixStream) -> io::Result<()> {
        let mut wakeups = self
            .wakeups
            .lock()
            .map_err(|_| io::Error::other("native wakeup registry failed"))?;
        if let Err(error) = self.state.check() {
            let _ = stream.shutdown(Shutdown::Both);
            return Err(error);
        }
        if wakeups.len() >= 2 {
            return Err(io::Error::other("native wakeup registry is exhausted"));
        }
        wakeups.push(stream.try_clone()?);
        Ok(())
    }

    /// Clone the admission fence before attempting any native creation/resume.
    pub fn creation_fence(&self) -> CreationFence {
        self.state.clone()
    }

    /// Retire all readers without hiding an already observed owner loss.
    pub fn retire(&mut self) -> io::Result<()> {
        {
            let _admission = self
                .state
                .0
                .action
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = self.state.0.cause.compare_exchange(
                0,
                RETIRED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        let mut failure = None;
        for (stream, _) in &self.readers {
            if let Err(error) = stream.shutdown(Shutdown::Read)
                && error.kind() != io::ErrorKind::NotConnected
            {
                failure.get_or_insert(error);
            }
        }
        for (_, reader) in self.readers.drain(..) {
            if reader.join().is_err() {
                failure.get_or_insert_with(|| io::Error::other("native lease reader failed"));
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl Drop for DeathFence {
    fn drop(&mut self) {
        let _ = self.retire();
    }
}
