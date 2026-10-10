use std::{
    fs::{File, OpenOptions},
    io, mem,
    os::{
        fd::{AsRawFd as _, RawFd},
        unix::fs::{MetadataExt as _, OpenOptionsExt as _},
    },
    time::Instant,
};

use super::{MAX_RECORD_BYTES, invalid};

// Darwin _IOR('S', command, payload), matching the shipped audit_ioctl.h ABI.
const GET_MAXDATA: libc::c_ulong = 0x4004_5307;
const GET_DROPS: libc::c_ulong = 0x4008_53ca;

pub(crate) fn open_current() -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open("/dev/auditsessions")?;
    let mode = file.metadata()?.mode();
    if mode & u32::from(libc::S_IFMT) != u32::from(libc::S_IFCHR) {
        return Err(invalid("audit device is not an OS character device"));
    }
    let fd = file.as_raw_fd();
    let mut maximum = 0_u32;
    // SAFETY: this readonly character descriptor was opened at the fixed OS path.
    // GET_MAXDATA writes exactly one initialized u32 and retains no pointer.
    if unsafe { libc::ioctl(fd, GET_MAXDATA, &mut maximum) } == -1 {
        return Err(io::Error::last_os_error());
    }
    if maximum == 0 || maximum as usize > MAX_RECORD_BYTES {
        return Err(invalid("audit device declares unsupported record bounds"));
    }
    let mut nonblocking = 1_i32;
    // SAFETY: FIONBIO changes only this descriptor's read behavior. The initialized
    // integer remains live and no global audit policy or all-session flag is set.
    if unsafe { libc::ioctl(fd, libc::FIONBIO, &mut nonblocking) } == -1 {
        return Err(io::Error::last_os_error());
    }
    require_no_drops(fd)?;
    Ok(file)
}

pub(crate) fn require_no_drops(fd: RawFd) -> io::Result<()> {
    // The SDK encodes u64 but current XNU writes u32. Initialize the entire buffer
    // so neither form leaves uninitialized upper bytes or hides a nonzero count.
    let mut drops = 0_u64;
    // SAFETY: GET_DROPS writes at most the SDK's eight-byte initialized output;
    // the owned live descriptor and scalar remain valid throughout this call.
    if unsafe { libc::ioctl(fd, GET_DROPS, &mut drops) } == -1 {
        return Err(io::Error::last_os_error());
    }
    if drops != 0 {
        return Err(invalid("audit device dropped scoped session records"));
    }
    Ok(())
}

pub(crate) fn select_readable(fd: RawFd, deadline: Instant) -> io::Result<()> {
    if fd < 0 || fd as usize >= libc::FD_SETSIZE {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "audit descriptor exceeds bounded select capacity",
        ));
    }
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "kernel session END was not observed within its deadline",
            ));
        }
        // SAFETY: zero bits initialize the fixed SDK fd_set backing integer array.
        let mut reading: libc::fd_set = unsafe { mem::zeroed() };
        // SAFETY: fd was checked against the fd_set capacity; reading is initialized
        // writable storage and these macros do not retain its address.
        unsafe {
            libc::FD_ZERO(&mut reading);
            libc::FD_SET(fd, &mut reading);
        }
        let mut timeout = libc::timeval {
            tv_sec: remaining.as_secs() as libc::time_t,
            tv_usec: remaining.subsec_micros() as libc::suseconds_t,
        };
        // SAFETY: one fixed-capacity read set and bounded writable timeval remain
        // live for the call. fd+1 is within the capacity; other sets are absent.
        let result = unsafe {
            libc::select(
                fd + 1,
                &mut reading,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut timeout,
            )
        };
        if result > 0 {
            return Ok(());
        }
        if result == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "kernel session END was not observed within its deadline",
            ));
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}
