//! Readonly scoped-device provenance checks before an authenticated handoff.

use std::{
    fs::File,
    io,
    os::{fd::AsRawFd as _, unix::fs::MetadataExt as _},
};

use super::{invalid, wire::DeviceMetadata};

const GET_MAXDATA: libc::c_ulong = 0x4004_5307;
const GET_ALLSESSIONS: libc::c_ulong = 0x4004_5364;
const GET_READS: libc::c_ulong = 0x4008_53c9;

pub(super) fn metadata(file: &File) -> io::Result<DeviceMetadata> {
    let value = file.metadata()?;
    // SAFETY: this owned descriptor is live. F_GETFL returns only its flags.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(DeviceMetadata {
        dev: value.dev(),
        ino: value.ino(),
        rdev: value.rdev(),
        mode: value.mode(),
        flags: flags as u32,
    })
}

pub(super) fn validate_unread(file: &File, expected: DeviceMetadata) -> io::Result<()> {
    validate_continuing(file, expected)?;
    let mut reads = 0_u64;
    // SAFETY: the received descriptor was authenticated as the closed sender's
    // readonly audit device. GET_READS writes at most the SDK's initialized u64.
    if unsafe { libc::ioctl(file.as_raw_fd(), GET_READS, &mut reads) } == -1 {
        return Err(io::Error::last_os_error());
    }
    if reads != 0 {
        return Err(invalid("audit handoff device was already read"));
    }
    Ok(())
}

pub(super) fn validate_continuing(file: &File, expected: DeviceMetadata) -> io::Result<()> {
    let actual = metadata(file)?;
    let native = std::fs::symlink_metadata("/dev/auditsessions")?;
    let native = DeviceMetadata {
        dev: native.dev(),
        ino: native.ino(),
        rdev: native.rdev(),
        mode: native.mode(),
        flags: 0,
    };
    if !matches_device(actual, expected, native) {
        return Err(invalid(
            "audit handoff device metadata or readonly flags changed",
        ));
    }
    let mut maximum = 0_u32;
    // SAFETY: fixed audit-device metadata ioctl writes one initialized u32.
    if unsafe { libc::ioctl(file.as_raw_fd(), GET_MAXDATA, &mut maximum) } == -1 {
        return Err(io::Error::last_os_error());
    }
    if maximum == 0 || maximum > 0x7fff {
        return Err(invalid(
            "audit handoff device has unsupported record bounds",
        ));
    }
    let mut all_sessions = 1_u32;
    // SAFETY: this readonly query writes one initialized u32 and never enables
    // all-session observation or changes global audit policy.
    let queried = unsafe { libc::ioctl(file.as_raw_fd(), GET_ALLSESSIONS, &mut all_sessions) };
    if queried == -1 {
        // XNU gates this query itself on audit privilege. EPERM is permissible
        // only because this descriptor arrived atomically from authenticated
        // closed current-only source code, which has no all-sessions setter.
        if io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
            return Err(io::Error::last_os_error());
        }
    } else if all_sessions != 0 {
        return Err(invalid("audit handoff device exposes foreign sessions"));
    }
    crate::audit_events::device::require_no_drops(file.as_raw_fd())
}

fn matches_device(
    actual: DeviceMetadata,
    expected: DeviceMetadata,
    native: DeviceMetadata,
) -> bool {
    let flags = actual.flags as i32;
    actual == expected && actual.dev == native.dev && actual.ino == native.ino
        // auditsessions is a devfs clone device. Path lookup can allocate a
        // different minor while preserving the original devnode's dev/inode.
        // Compare its OS device class here; full cloned rdev stays bound to the
        // authenticated sender's exact descriptor metadata above.
        && libc::major(actual.rdev as libc::dev_t) == libc::major(native.rdev as libc::dev_t)
        && actual.mode & u32::from(libc::S_IFMT) == u32::from(libc::S_IFCHR)
        && native.mode & u32::from(libc::S_IFMT) == u32::from(libc::S_IFCHR)
        && flags & libc::O_ACCMODE == libc::O_RDONLY && flags & libc::O_NONBLOCK != 0
        && flags & (libc::O_APPEND | libc::O_ASYNC | libc::O_TRUNC | libc::O_CREAT | libc::O_EXLOCK | libc::O_SHLOCK) == 0
}

#[cfg(test)]
mod tests;
