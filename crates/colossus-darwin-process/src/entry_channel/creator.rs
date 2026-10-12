//! Separate creator entry: four sealed Core streams plus one genuine keeper.
use super::{admit, invalid, take_five_fixed_streams};
use crate::{DarwinAuditPeer, DarwinProcessIdentity};
use core_foundation::{
    base::{CFType, TCFType as _},
    data::CFData,
    dictionary::CFDictionary,
    string::CFString,
};
use security_framework::os::macos::code_signing::{Flags, SecCode};
use std::{
    ffi::c_void,
    io,
    os::{fd::AsRawFd as _, unix::net::UnixStream},
};

/// Four exact-parent Core transports and an independently admitted native keeper.
/// The separate fixed7 proof never broadens the all-five-parent browser entry.
pub struct DarwinCreatorEntry {
    /// Compiler-sealed exact Core bootstrap stream at descriptor three.
    pub bootstrap: UnixStream,
    /// Compiler-sealed exact Core data/lifetime stream at descriptor four.
    pub data: UnixStream,
    /// Compiler-sealed exact Core control stream at descriptor five.
    pub control: UnixStream,
    /// Compiler-sealed exact Core presentation stream at descriptor six.
    pub presentation: UnixStream,
    /// Direct independently verified native keeper stream at descriptor seven.
    pub keeper: UnixStream,
    /// Exact signed Core parent identity admitted on all four Core transports.
    pub parent: DarwinAuditPeer,
    /// Exact live same-Core keeper incarnation, with identical native image seal.
    pub keeper_peer: DarwinAuditPeer,
}

/// Consume the independent creator's fixed3..7 channels once, without raw selectors.
/// Core on3..6 is checked against its compiler-bound hash. Only after that proof,
/// the current creator's genuine signed OS code seal authorizes an identical
/// keeper image whose genuine stream peer is an exact live child of that Core.
/// UID, audit session, process/exec incarnation, Core/self/keeper code and native
/// parent relationship are revalidated before any endpoint is yielded.
pub fn take_connected_creator_channels(expected_core: [u8; 20]) -> io::Result<DarwinCreatorEntry> {
    if expected_core == [0; 20] {
        return Err(invalid("native creator requires an independent Core seal"));
    }
    let [bootstrap, data, control, presentation, keeper] = take_five_fixed_streams()?;
    let parent = admit(&bootstrap, expected_core)?;
    for stream in [&data, &control, &presentation] {
        let peer = admit(stream, expected_core)?;
        if peer.identity() != parent.identity() {
            return Err(invalid(
                "native creator Core channels changed parent incarnation",
            ));
        }
    }
    parent.verify()?;
    let own_identity = DarwinProcessIdentity::bind(std::process::id())?;
    let own_hash = current_code_hash()?;
    let own_peer = DarwinAuditPeer::enroll(&own_identity, own_hash)?;
    let identity = keeper_identity(&keeper)?;
    validate_relationship(&own_identity, &identity, parent.identity())?;
    require_parent(&identity, parent.identity())?;
    let keeper_peer = DarwinAuditPeer::enroll(&identity, own_hash)?;
    parent.verify()?;
    own_peer.verify()?;
    keeper_peer.verify()?;
    require_parent(&identity, parent.identity())?;
    validate_relationship(&own_identity, &identity, parent.identity())?;
    parent.verify()?;
    own_peer.verify()?;
    keeper_peer.verify()?;
    Ok(DarwinCreatorEntry {
        bootstrap,
        data,
        control,
        presentation,
        keeper,
        parent,
        keeper_peer,
    })
}

fn keeper_identity(stream: &UnixStream) -> io::Result<DarwinProcessIdentity> {
    stream
        .peer_addr()
        .map_err(|_| invalid("native keeper stream is disconnected"))?;
    let mut pid: libc::pid_t = 0;
    let mut size = std::mem::size_of_val(&pid) as libc::socklen_t;
    // SAFETY: the borrowed connected socket and fixed writable PID/length are live.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEEREPID,
            std::ptr::from_mut(&mut pid).cast(),
            &mut size,
        )
    } != 0
        || size as usize != std::mem::size_of_val(&pid)
        || pid <= 1
    {
        return Err(invalid("native keeper has no genuine stream identity"));
    }
    let mut uid = 0;
    let mut gid = 0;
    // SAFETY: getpeereid writes only these two live fixed credential outputs.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(invalid("native keeper lacks genuine stream credentials"));
    }
    let identity = DarwinProcessIdentity::bind(pid as u32)?;
    if identity.real_uid() != uid || identity.effective_uid() != uid || uid == 0 {
        return Err(invalid("native keeper credentials differ from stream"));
    }
    Ok(identity)
}
fn validate_relationship(
    own: &DarwinProcessIdentity,
    keeper: &DarwinProcessIdentity,
    parent: &DarwinProcessIdentity,
) -> io::Result<()> {
    validate_shape(
        own.pid(),
        keeper.pid(),
        parent.pid(),
        own.real_uid(),
        own.effective_uid(),
        keeper.real_uid(),
        keeper.effective_uid(),
        parent.real_uid(),
        parent.effective_uid(),
        own.audit_session_id(),
        keeper.audit_session_id(),
        parent.audit_session_id(),
    )
}
#[allow(clippy::too_many_arguments)]
fn validate_shape(
    own: u32,
    keeper: u32,
    parent: u32,
    own_uid: u32,
    own_euid: u32,
    keeper_uid: u32,
    keeper_euid: u32,
    parent_uid: u32,
    parent_euid: u32,
    own_session: u32,
    keeper_session: u32,
    parent_session: u32,
) -> io::Result<()> {
    if own <= 1
        || keeper <= 1
        || parent <= 1
        || own == keeper
        || own == parent
        || keeper == parent
        || own_uid == 0
        || own_uid != own_euid
        || own_uid != keeper_uid
        || own_uid != keeper_euid
        || own_uid != parent_uid
        || own_uid != parent_euid
        || own_session == 0
        || own_session == u32::MAX
        || own_session != keeper_session
        || own_session != parent_session
    {
        return Err(invalid(
            "native keeper is not an unprivileged sibling incarnation",
        ));
    }
    Ok(())
}
fn require_parent(
    identity: &DarwinProcessIdentity,
    parent: &DarwinProcessIdentity,
) -> io::Result<()> {
    // SAFETY: proc_pidinfo writes at most the exact initialized BSD record size.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    // SAFETY: both live process IDs are bounded and this fixed buffer is writable.
    let size = unsafe {
        libc::proc_pidinfo(
            identity.pid() as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            std::ptr::from_mut(&mut info).cast(),
            std::mem::size_of_val(&info) as i32,
        )
    };
    if size as usize != std::mem::size_of_val(&info)
        || info.pbi_pid != identity.pid()
        || info.pbi_ppid != parent.pid()
        || DarwinProcessIdentity::bind(identity.pid())? != *identity
    {
        return Err(invalid(
            "native keeper does not have the exact live Core parent",
        ));
    }
    Ok(())
}
unsafe extern "C" {
    fn SecCodeCopySigningInformation(
        code: *const c_void,
        flags: u32,
        result: *mut *const c_void,
    ) -> i32;
    static kSecCodeInfoUnique: *const c_void;
}
fn current_code_hash() -> io::Result<[u8; 20]> {
    let code = SecCode::for_self(Flags::NONE)
        .map_err(|_| invalid("native creator lacks own signed code"))?;
    let mut raw = std::ptr::null();
    // SAFETY: the borrowed signed code is live; Security writes one retained CF object.
    let status =
        unsafe { SecCodeCopySigningInformation(code.as_concrete_TypeRef().cast(), 0, &mut raw) };
    let info = if raw.is_null() {
        None
    } else {
        // SAFETY: the nonnull Copy result transfers one owned CoreFoundation reference.
        Some(unsafe { CFType::wrap_under_create_rule(raw) })
    };
    if status != 0 {
        return Err(invalid("native creator signed information is unavailable"));
    }
    let untyped = info
        .and_then(|value| value.downcast::<CFDictionary>())
        .ok_or_else(|| invalid("native creator signed information is not a dictionary"))?;
    // SAFETY: the CF object was checked as a dictionary above; Security's
    // information dictionary contains CFString keys and CoreFoundation values.
    let info: CFDictionary<CFString, CFType> =
        unsafe { CFDictionary::wrap_under_get_rule(untyped.as_concrete_TypeRef()) };
    // SAFETY: this fixed Security framework CFString constant is process-lifetime.
    let key = unsafe { CFString::wrap_under_get_rule(kSecCodeInfoUnique.cast()) };
    let value = info
        .find(&key)
        .and_then(|value| value.downcast::<CFData>())
        .ok_or_else(|| invalid("native creator signed information has no CDHash"))?;
    let hash: [u8; 20] = value
        .bytes()
        .try_into()
        .map_err(|_| invalid("native creator CDHash length is invalid"))?;
    if hash == [0; 20] {
        return Err(invalid("native creator has a zero code seal"));
    }
    Ok(hash)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::FromRawFd as _;
    #[test]
    fn sibling_shape_rejects_foreign_parent_uid_session_and_reused_incarnation_shape() {
        assert!(validate_shape(10, 11, 12, 501, 501, 501, 501, 501, 501, 100, 100, 100).is_ok());
        assert!(validate_shape(10, 10, 12, 501, 501, 501, 501, 501, 501, 100, 100, 100).is_err());
        assert!(validate_shape(10, 12, 12, 501, 501, 501, 501, 501, 501, 100, 100, 100).is_err());
        assert!(validate_shape(10, 11, 12, 501, 501, 502, 502, 501, 501, 100, 100, 100).is_err());
        assert!(validate_shape(10, 11, 12, 501, 501, 501, 501, 501, 501, 100, 101, 100).is_err());
    }
    #[test]
    fn disconnected_fixed_keeper_stream_rejects_before_any_identity_lookup() {
        // SAFETY: this socket call returns one fresh unconnected Unix descriptor.
        let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
        assert!(raw >= 0);
        // SAFETY: successful socket creation transfers its exclusive descriptor.
        let stream = unsafe { UnixStream::from_raw_fd(raw) };
        assert_eq!(
            keeper_identity(&stream).err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
