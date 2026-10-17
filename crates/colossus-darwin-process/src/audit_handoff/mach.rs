//! Owned generic Mach transport rights; no received kobject interpretation.

use std::{ffi::CString, io, mem, time::Instant};

use super::{
    invalid,
    wire::{self, BUFFER_SIZE, Kind, MESSAGE_SIZE},
};

unsafe extern "C" {
    static mach_task_self_: u32;
    static bootstrap_port: u32;
    fn bootstrap_check_in(bootstrap: u32, name: *const libc::c_char, port: *mut u32) -> i32;
    fn bootstrap_look_up(bootstrap: u32, name: *const libc::c_char, port: *mut u32) -> i32;
    fn mach_port_allocate(task: u32, right: i32, name: *mut u32) -> i32;
    fn mach_port_set_attributes(
        task: u32,
        name: u32,
        flavor: i32,
        info: *mut u32,
        count: u32,
    ) -> i32;
    fn mach_port_destroy(task: u32, name: u32) -> i32;
    fn mach_port_deallocate(task: u32, name: u32) -> i32;
    fn mach_msg(
        message: *mut libc::c_void,
        options: u32,
        send_size: u32,
        receive_size: u32,
        receive_name: u32,
        timeout: u32,
        notify: u32,
    ) -> i32;
    fn mach_msg_destroy(message: *mut libc::c_void);
    fn fileport_makeport(fd: i32, port: *mut u32) -> i32;
    fn fileport_makefd(port: u32) -> i32;
}

pub(super) struct ReceivePort(u32);

impl ReceivePort {
    pub(super) fn check_in(service: &str) -> io::Result<Self> {
        let service = service_name(service)?;
        let mut name = 0;
        // SAFETY: this is the fixed launchd bootstrap interface. The bounded,
        // NUL-terminated name and writable output remain valid during the call.
        let status = unsafe { bootstrap_check_in(bootstrap_port, service.as_ptr(), &mut name) };
        let port = Self(name);
        if status != 0 {
            return Err(status_error("launchd audit service check-in", status));
        }
        port.bound_queue()?;
        Ok(port)
    }

    pub(super) fn reply() -> io::Result<Self> {
        let mut name = 0;
        // SAFETY: the SDK receive-right selector is one. name is one writable
        // port-sized output and the task-self name is this process's native port.
        let status = unsafe { mach_port_allocate(self_task(), 1, &mut name) };
        let port = Self(name);
        if status != 0 {
            return Err(status_error("audit reply allocation", status));
        }
        port.bound_queue()?;
        Ok(port)
    }

    fn bound_queue(&self) -> io::Result<()> {
        if matches!(self.0, 0 | u32::MAX) {
            return Err(invalid("audit receive right is absent"));
        }
        let mut limit = 1;
        // SAFETY: this owned receive right permits the fixed LIMITS_INFO flavor.
        // Its SDK record is exactly one writable natural_t queue-limit scalar.
        let status = unsafe { mach_port_set_attributes(self_task(), self.0, 1, &mut limit, 1) };
        if status != 0 {
            return Err(status_error("audit queue capacity", status));
        }
        Ok(())
    }

    pub(super) fn name(&self) -> u32 {
        self.0
    }
}

impl Drop for ReceivePort {
    fn drop(&mut self) {
        if !matches!(self.0, 0 | u32::MAX) {
            // SAFETY: this value exclusively owns its local receive right;
            // destroying it also generically releases all queued message rights.
            let _ = unsafe { mach_port_destroy(self_task(), self.0) };
        }
    }
}

pub(super) struct SendRight(u32);

impl SendRight {
    pub(super) fn lookup(service: &str) -> io::Result<Self> {
        let service = service_name(service)?;
        let mut name = 0;
        // SAFETY: fixed bootstrap interface with a bounded NUL-terminated name
        // and one writable port-sized output; no nominated raw port is accepted.
        let status = unsafe { bootstrap_look_up(bootstrap_port, service.as_ptr(), &mut name) };
        let right = Self(name);
        if status != 0 {
            return Err(status_error("launchd audit service lookup", status));
        }
        if matches!(name, 0 | u32::MAX) {
            return Err(invalid("audit service has no send right"));
        }
        Ok(right)
    }

    pub(super) fn fileport(fd: i32) -> io::Result<Self> {
        let mut name = 0;
        // SAFETY: the live descriptor is the closed current-only audit device.
        // The SDK writes one owned fileport send right to initialized storage.
        if unsafe { fileport_makeport(fd, &mut name) } == -1 {
            return Err(io::Error::last_os_error());
        }
        if matches!(name, 0 | u32::MAX) {
            return Err(invalid("audit fileport is absent"));
        }
        Ok(Self(name))
    }

    pub(super) fn duplicate_file(&self) -> io::Result<std::fs::File> {
        use std::os::fd::FromRawFd as _;
        // SAFETY: this received right is selected only from the authenticated
        // closed sender's second fixed port descriptor. Failure does not expose
        // or interpret a session/task port; the SDK returns -1 for non-fileports.
        let fd = unsafe { fileport_makefd(self.0) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fileport_makefd returned exactly one new owned descriptor.
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        // SAFETY: this owned descriptor remains live; F_SETFD only sets inheritance.
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(file)
    }

    pub(super) fn name(&self) -> u32 {
        self.0
    }
    pub(super) fn into_message(self) -> u32 {
        let name = self.0;
        mem::forget(self);
        name
    }
}

impl Drop for SendRight {
    fn drop(&mut self) {
        if !matches!(self.0, 0 | u32::MAX) {
            // SAFETY: one generic owned send/send-once reference. No kobject
            // classification, session join or raw-right effect occurs here.
            let _ = unsafe { mach_port_deallocate(self_task(), self.0) };
        }
    }
}

#[repr(C, align(8))]
struct Buffer([u8; BUFFER_SIZE]);

pub(super) struct Message {
    buffer: Buffer,
    owned: bool,
}

pub(super) struct ReceiveFailure {
    pub(super) error: io::Error,
    pub(super) partial: Option<Box<Message>>,
}

impl From<io::Error> for ReceiveFailure {
    fn from(error: io::Error) -> Self {
        Self {
            error,
            partial: None,
        }
    }
}

impl From<ReceiveFailure> for io::Error {
    fn from(failure: ReceiveFailure) -> Self {
        // Any partially installed unaccepted rights are destroyed by Message's
        // generic destructor unless the exact enrolled receiver retains them.
        failure.error
    }
}

impl Message {
    pub(super) fn new(
        kind: Kind,
        remote: u32,
        reply: u32,
        nonce: [u8; 16],
        challenge: [u8; 16],
    ) -> Self {
        let mut result = Self {
            buffer: Buffer([0; BUFFER_SIZE]),
            owned: true,
        };
        wire::initialize(&mut result.buffer.0, kind, remote, reply, nonce, challenge);
        result
    }

    pub(super) fn bytes(&self) -> &[u8; BUFFER_SIZE] {
        &self.buffer.0
    }
    pub(super) fn bytes_mut(&mut self) -> &mut [u8; BUFFER_SIZE] {
        &mut self.buffer.0
    }

    pub(super) fn send(mut self, deadline: Instant) -> io::Result<()> {
        let timeout = milliseconds(deadline)?;
        // SAFETY: the aligned initialized buffer contains the exact SDK header
        // and fixed inline payload/descriptors built only by this module. The
        // bounded send uses no nominated notify right or receive operation.
        let status = unsafe {
            mach_msg(
                self.buffer.0.as_mut_ptr().cast(),
                1 | 0x10 | 0x40,
                MESSAGE_SIZE as u32,
                0,
                0,
                timeout,
                0,
            )
        };
        if status != 0 {
            self.release_unsent_local_right();
            return Err(status_error("audit Mach send", status));
        }
        self.owned = false;
        Ok(())
    }

    pub(super) fn receive(port: &ReceivePort, deadline: Instant) -> Result<Self, ReceiveFailure> {
        let mut result = Self {
            buffer: Buffer([0; BUFFER_SIZE]),
            owned: false,
        };
        let timeout = milliseconds(deadline)?;
        // SAFETY: the aligned writable buffer has the advertised 512-byte
        // capacity, including the requested FORMAT_0/AUDIT trailer. The receive
        // name is exclusively owned and the queue/deadline are both bounded.
        let status = unsafe {
            mach_msg(
                result.buffer.0.as_mut_ptr().cast(),
                2 | 0x100 | 0x400 | (3 << 24),
                0,
                BUFFER_SIZE as u32,
                port.0,
                timeout,
                0,
            )
        };
        if status != 0 {
            return Err(receive_failure(result, "audit Mach receive", status));
        }
        result.owned = true;
        Ok(result)
    }

    pub(super) fn try_receive(port: &ReceivePort) -> Result<Option<Self>, ReceiveFailure> {
        let mut result = Self {
            buffer: Buffer([0; BUFFER_SIZE]),
            owned: false,
        };
        // SAFETY: the same owned receive endpoint and aligned 512-byte capacity
        // as receive(). RCV_TIMEOUT with timeout zero is an immediate queue check
        // with the same genuine AUDIT trailer; it cannot block.
        let status = unsafe {
            mach_msg(
                result.buffer.0.as_mut_ptr().cast(),
                2 | 0x100 | 0x400 | (3 << 24),
                0,
                BUFFER_SIZE as u32,
                port.0,
                0,
                0,
            )
        };
        if status as u32 == 0x1000_4003 {
            return Ok(None);
        }
        if status != 0 {
            return Err(receive_failure(result, "audit replay queue check", status));
        }
        result.owned = true;
        Ok(Some(result))
    }

    pub(super) fn take_reply(&mut self) -> io::Result<SendRight> {
        let bits = wire::word(self.bytes(), 0)?;
        let name = wire::word(self.bytes(), 8)?;
        if bits & 0xff != wire::MOVE_SEND_ONCE || matches!(name, 0 | u32::MAX) {
            return Err(invalid("audit handshake lacks its one-time reply right"));
        }
        wire::put_word(self.bytes_mut(), 8, 0);
        Ok(SendRight(name))
    }

    fn release_unsent_local_right(&mut self) {
        let bits = wire::word(self.bytes(), 0).unwrap_or(0);
        let disposition = (bits >> 8) & 0xff;
        if matches!(disposition, 17 | 18) {
            // A failed send's pseudo-receive keeps header fields unreversed:
            // MAKE_SEND_ONCE may have become a newly owned local SEND_ONCE
            // name. Generic mach_msg_destroy deliberately ignores local_port.
            // Release that reference separately, exactly as libSystem's unsent
            // message cleanup does. Never apply this to a received message,
            // whose local_port is its borrowed receive destination.
            let name = wire::word(self.bytes(), 12).unwrap_or(0);
            wire::put_word(self.bytes_mut(), 12, 0);
            wire::put_word(self.bytes_mut(), 0, bits & !0xff00);
            drop(SendRight(name));
        }
    }

    /// Preserve only bounded generic send rights from an exact enrolled sender.
    /// This is also used on malformed/partial handoffs before permanent fencing.
    pub(super) fn take_ports(&mut self) -> Vec<SendRight> {
        let mut rights = Vec::with_capacity(2);
        if wire::word(self.bytes(), 0).is_ok_and(|bits| bits & wire::COMPLEX != 0) {
            let count = wire::word(self.bytes(), 24).unwrap_or(0).min(2);
            for index in 0..count as usize {
                let offset = 28 + index * 12;
                let bytes = self.bytes();
                if bytes[offset + 11] != 0 || !matches!(bytes[offset + 10], 17 | 18) {
                    break;
                }
                let name = wire::word(bytes, offset).unwrap_or(0);
                if !matches!(name, 0 | u32::MAX) {
                    wire::put_word(self.bytes_mut(), offset, 0);
                    rights.push(SendRight(name));
                }
            }
        }
        rights
    }
}

fn receive_failure(mut message: Message, operation: &str, status: i32) -> ReceiveFailure {
    // XNU BODY_ERROR can install some received rights before a later copyout
    // fails. The SDK's special IPC/VM shortage bits decorate that base status.
    // Such a genuine partial message still owns its installed names and must
    // reach either exact-owner retention or generic Mach destruction.
    let partial = if status as u32 & !0x0000_3e00 == 0x1000_400c {
        message.owned = true;
        Some(Box::new(message))
    } else {
        None
    };
    ReceiveFailure {
        error: status_error(operation, status),
        partial,
    }
}

impl Drop for Message {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: this is either a genuine successful kernel receive or a
            // send buffer returned by mach_msg's failure contract. libSystem's
            // generic destroy honors descriptor dispositions and releases all
            // remaining rights/OOL storage without interpreting their kobjects.
            unsafe { mach_msg_destroy(self.buffer.0.as_mut_ptr().cast()) };
        }
    }
}

pub(super) fn random_challenge() -> [u8; 16] {
    let mut bytes = [0; 16];
    // SAFETY: arc4random_buf initializes exactly the writable bounded buffer;
    // Darwin supplies the fixed cryptographic OS random source.
    unsafe { libc::arc4random_buf(bytes.as_mut_ptr().cast(), bytes.len()) };
    bytes
}

fn service_name(service: &str) -> io::Result<CString> {
    if service.is_empty()
        || service.len() > 127
        || !service.starts_with("com.colossus.")
        || !service
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err(invalid(
            "audit handoff requires a bounded native Colossus MachService name",
        ));
    }
    CString::new(service).map_err(|_| invalid("invalid audit MachService name"))
}

fn milliseconds(deadline: Instant) -> io::Result<u32> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "audit handoff deadline elapsed",
        ));
    }
    u32::try_from(remaining.as_millis().max(1))
        .map_err(|_| invalid("audit timeout exceeds the native bound"))
}

fn self_task() -> u32 {
    // SAFETY: libSystem initializes this process's immutable task-self name.
    unsafe { mach_task_self_ }
}

fn status_error(operation: &str, status: i32) -> io::Error {
    let kind = if matches!(status as u32, 0x1000_0004 | 0x1000_4003) {
        io::ErrorKind::TimedOut
    } else {
        io::ErrorKind::Other
    };
    io::Error::new(
        kind,
        format!("Darwin {operation} failed (Mach status {status})"),
    )
}
