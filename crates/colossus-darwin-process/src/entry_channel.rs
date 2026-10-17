//! One-use connected native child entry, independent of broker authority.

use std::{
    io,
    os::{
        fd::{AsRawFd as _, FromRawFd as _, OwnedFd},
        unix::net::UnixStream,
    },
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{DarwinAuditPeer, DarwinProcessIdentity};

const ENTRY_FD: libc::c_int = 3;
static TAKEN: AtomicBool = AtomicBool::new(false);

mod creator;
pub use creator::{DarwinCreatorEntry, take_connected_creator_channels};

/// The five fixed channels inherited by the independent browser creator.
///
/// Every stream is admitted against the same genuine, signed direct parent
/// before any caller receives it. These channels grant no browser operation,
/// helper creation, network access, or whole-process cleanup authority.
pub struct DarwinOwnedBrowserEntry {
    /// Parent bootstrap transport inherited at descriptor three.
    pub bootstrap: UnixStream,
    /// Parent operation transport inherited at descriptor four.
    pub data: UnixStream,
    /// Parent cancellation transport inherited at descriptor five.
    pub control: UnixStream,
    /// Parent presentation transport inherited at descriptor six.
    pub presentation: UnixStream,
    /// Parent keeper transport inherited at descriptor seven.
    pub creator: UnixStream,
    /// Independently authenticated live parent incarnation.
    pub parent: DarwinAuditPeer,
}

/// Consume the creator's fixed connected descriptors three through seven once.
///
/// The dedicated creator entry supplies its compiler-sealed parent code hash,
/// independently of all channel bytes. Each Unix stream must have the genuine
/// unprivileged direct parent's kernel credentials and the same exact enrolled
/// audit identity. All original handles are closed, including on failure; the
/// returned handles are above seven and close-on-exec. This entry and the
/// single-channel entry are mutually exclusive within a process.
///
/// # Errors
/// Rejects repeated entry, missing or invalid streams, privileged or foreign
/// peers, changed parent incarnations, or code that differs from the seal.
pub fn take_connected_owned_browser_channels(
    expected_parent_cdhash: [u8; 20],
) -> io::Result<DarwinOwnedBrowserEntry> {
    if expected_parent_cdhash == [0; 20] {
        return Err(invalid(
            "native entry requires an independent parent code seal",
        ));
    }
    let [bootstrap, data, control, presentation, creator] = take_five_fixed_streams()?;
    let parent = admit(&bootstrap, expected_parent_cdhash)?;
    for stream in [&data, &control, &presentation, &creator] {
        let peer = admit(stream, expected_parent_cdhash)?;
        if peer.identity() != parent.identity() {
            return Err(invalid(
                "native entry channels have different parent incarnations",
            ));
        }
    }
    parent.verify()?;
    Ok(DarwinOwnedBrowserEntry {
        bootstrap,
        data,
        control,
        presentation,
        creator,
        parent,
    })
}

fn take_five_fixed_streams() -> io::Result<[UnixStream; 5]> {
    if TAKEN.swap(true, Ordering::AcqRel) {
        return Err(invalid("native entry was already consumed"));
    }
    let fixed = [3, 4, 5, 6, 7];
    let mut duplicates = Vec::with_capacity(fixed.len());
    let mut failure = None;
    for descriptor in fixed {
        // SAFETY: this dedicated entry owns only the fixed inherited handles.
        // Duplicating above seven cannot replace another inherited channel.
        let duplicate = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 8) };
        if duplicate < 0 {
            failure.get_or_insert_with(io::Error::last_os_error);
        } else {
            // SAFETY: successful fcntl returned a new uniquely owned handle.
            duplicates.push(unsafe { OwnedFd::from_raw_fd(duplicate) });
        }
    }
    for descriptor in fixed {
        // SAFETY: entry consumed each fixed handle. Never retry close.
        if unsafe { libc::close(descriptor) } != 0 {
            failure.get_or_insert_with(io::Error::last_os_error);
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    let streams: Vec<UnixStream> = duplicates.into_iter().map(UnixStream::from).collect();
    streams
        .try_into()
        .map_err(|_| invalid("native entry channel inventory changed"))
}

/// An inherited connected stream admitted against its exact signed native parent.
///
/// This proves transport provenance and live parent code, not origin authority,
/// a publisher receipt, frame authentication, or complete process containment.
pub struct DarwinParentChannel {
    stream: UnixStream,
    parent: DarwinAuditPeer,
}

impl DarwinParentChannel {
    /// Keep the exact parent enrollment with the stream for later revalidation.
    #[must_use]
    pub fn into_parts(self) -> (UnixStream, DarwinAuditPeer) {
        (self.stream, self.parent)
    }
}

/// Consume the fixed descriptor-three child entry once, before reading key bytes.
///
/// The independently verified package must supply the expected parent cdhash;
/// a value reported by this stream cannot authorize itself. The stream must be
/// connected to the genuine direct parent and have this process's non-root UID.
/// Descriptor three is consumed; the returned duplicate is close-on-exec.
/// Invoke only in the dedicated child entry, never in a general application.
///
/// # Errors
/// Rejects repeated entry, missing/invalid streams, foreign credentials, changed
/// process incarnations, or live code differing from the independent requirement.
pub fn take_connected_parent_channel(
    expected_parent_cdhash: [u8; 20],
) -> io::Result<DarwinParentChannel> {
    if expected_parent_cdhash == [0; 20] {
        return Err(invalid(
            "native entry requires an independent parent code seal",
        ));
    }
    if TAKEN.swap(true, Ordering::AcqRel) {
        return Err(invalid("native entry was already consumed"));
    }
    // SAFETY: this dedicated entry takes ownership of fixed inherited descriptor
    // three. fcntl creates a new descriptor above the fixed entry without reading
    // caller memory. No caller-supplied descriptor is accepted.
    let duplicate = unsafe { libc::fcntl(ENTRY_FD, libc::F_DUPFD_CLOEXEC, ENTRY_FD + 1) };
    let duplicate = if duplicate < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: successful fcntl returned a new uniquely owned descriptor.
        Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
    };
    // SAFETY: descriptor three was transferred to this one-use entry. Always
    // attempt its close, including after duplication failure; never retry close.
    let closed = unsafe { libc::close(ENTRY_FD) };
    let stream = UnixStream::from(duplicate?);
    if closed != 0 {
        return Err(io::Error::last_os_error());
    }
    let parent = admit(&stream, expected_parent_cdhash)?;
    Ok(DarwinParentChannel { stream, parent })
}

fn admit(stream: &UnixStream, expected: [u8; 20]) -> io::Result<DarwinAuditPeer> {
    stream.peer_addr()?;
    let mut kind: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&kind) as libc::socklen_t;
    // SAFETY: a live owned descriptor and bounded writable scalar storage.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            std::ptr::from_mut(&mut kind).cast(),
            &mut length,
        )
    } != 0
        || length as usize != std::mem::size_of_val(&kind)
        || kind != libc::SOCK_STREAM
    {
        return Err(invalid("native entry is not a connected Unix stream"));
    }
    let mut pid: libc::pid_t = 0;
    length = std::mem::size_of_val(&pid) as libc::socklen_t;
    // SAFETY: getsockopt writes only the kernel peer PID to bounded stack storage.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEEREPID,
            std::ptr::from_mut(&mut pid).cast(),
            &mut length,
        )
    } != 0
        || length as usize != std::mem::size_of_val(&pid)
    {
        return Err(invalid("native entry has no kernel peer identity"));
    }
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    // SAFETY: getpeereid writes two fixed-size credentials to valid pointers.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(invalid("native entry has no kernel peer credentials"));
    }
    // SAFETY: these process identity getters have no caller memory or side effects.
    let (parent_pid, real_uid, effective_uid) =
        unsafe { (libc::getppid(), libc::getuid(), libc::geteuid()) };
    validate_peer(pid, uid, parent_pid, real_uid, effective_uid)?;
    let identity = DarwinProcessIdentity::bind(pid as u32)?;
    if identity.real_uid() != real_uid || identity.effective_uid() != effective_uid {
        return Err(invalid("native parent changed credentials"));
    }
    DarwinAuditPeer::enroll(&identity, expected)
}

fn validate_peer(
    pid: libc::pid_t,
    uid: libc::uid_t,
    parent_pid: libc::pid_t,
    real_uid: libc::uid_t,
    effective_uid: libc::uid_t,
) -> io::Result<()> {
    if parent_pid <= 1
        || pid != parent_pid
        || real_uid == 0
        || real_uid != effective_uid
        || uid != real_uid
    {
        return Err(invalid(
            "native entry peer is not the exact unprivileged parent",
        ));
    }
    Ok(())
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_and_privileged_peer_shapes_never_authorize_entry() {
        assert!(validate_peer(42, 501, 42, 501, 501).is_ok());
        for shape in [
            (1, 501, 1, 501, 501),
            (43, 501, 42, 501, 501),
            (42, 502, 42, 501, 501),
            (42, 0, 42, 0, 0),
            (42, 501, 42, 501, 0),
        ] {
            assert!(validate_peer(shape.0, shape.1, shape.2, shape.3, shape.4).is_err());
        }
    }

    #[test]
    fn socket_from_same_process_is_not_a_parent_capability() {
        let (stream, _peer) = UnixStream::pair().expect("ordinary socket pair");
        assert!(admit(&stream, [1; 20]).is_err());
    }
}
