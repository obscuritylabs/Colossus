//! Fixed local rendezvous frames with retained, cancellable overlapped I/O.
use super::super::{OwnedHandle, WindowsProcessError};
use std::{
    cell::{Cell, UnsafeCell},
    mem::size_of,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED,
        GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
        WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
    },
    System::{
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_TYPE_MESSAGE, SetNamedPipeHandleState,
        },
        SystemServices::SECURITY_DESCRIPTOR_REVISION,
        Threading::{CreateEventW, INFINITE, WaitForMultipleObjects, WaitForSingleObject},
    },
};

const HEADER: usize = 32;
const MAX_FRAME: usize = 256;
const MAGIC: [u8; 4] = *b"CBC1";
const VERSION: u16 = 1;
const CANCEL_WAIT: Duration = Duration::from_millis(100);

pub(super) struct Frame {
    pub op: u16,
    pub payload: Vec<u8>,
}

pub(super) struct Pipe {
    handle: Option<OwnedHandle>,
    nonce: [u8; 16],
    sent: u32,
    received: u32,
    server: bool,
    connected: bool,
    finished: bool,
    pending: Option<Box<Pending>>,
    read_completed: Option<Completion>,
}

// The whole operation is boxed before submission. Neither its buffer nor its
// OVERLAPPED/event may move or be freed until completion is positively observed.
struct Pending {
    overlapped: UnsafeCell<OVERLAPPED>,
    event: OwnedHandle,
    bytes: UnsafeCell<[u8; MAX_FRAME]>,
    kind: Operation,
    cancelled: Cell<bool>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Operation {
    Connect,
    Read,
    Write,
}

struct Completion {
    pending: Box<Pending>,
    transferred: usize,
    error: u32,
}

impl Pending {
    fn new(kind: Operation) -> Result<Box<Self>, WindowsProcessError> {
        // SAFETY: unnamed manual-reset event, initially nonsignaled, not inherited.
        let event = OwnedHandle::new(
            unsafe { CreateEventW(null(), 1, 0, null()) },
            "create browser rendezvous event",
        )?;
        let overlapped = OVERLAPPED {
            hEvent: event.raw(),
            ..OVERLAPPED::default()
        };
        Ok(Box::new(Self {
            overlapped: UnsafeCell::new(overlapped),
            event,
            bytes: UnsafeCell::new([0; MAX_FRAME]),
            kind,
            cancelled: Cell::new(false),
        }))
    }
}

struct Descriptor(*mut core::ffi::c_void);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: conversion allocated this exact descriptor with LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}

pub(super) fn server(
    name: &str,
    nonce: [u8; 16],
    owner_sid: &str,
) -> Result<Pipe, WindowsProcessError> {
    let name = pipe_name(name, nonce)?;
    // Permit only a numeric SID inside the fixed descriptor, never SDDL syntax.
    if !numeric_sid(owner_sid) {
        return Err(invalid("invalid browser rendezvous owner"));
    }
    let sddl = wide(&format!(
        "O:{owner_sid}D:P(A;;GA;;;{owner_sid})(A;;GA;;;SY)"
    ));
    let mut descriptor = null_mut();
    // SAFETY: fixed descriptor syntax with a bounded numeric SID, terminated input.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SECURITY_DESCRIPTOR_REVISION,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(last_error("build browser rendezvous security"));
    }
    let descriptor = Descriptor(descriptor);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // SAFETY: local nonce-derived name and retained protected owner/SYSTEM ACL.
    // One first-instance message pipe accepts no remote client or inherited handle.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            MAX_FRAME as u32,
            MAX_FRAME as u32,
            0,
            &attributes,
        )
    };
    pipe(handle, nonce, true)
}

pub(super) fn client(name: &str, nonce: [u8; 16]) -> Result<Pipe, WindowsProcessError> {
    let name = pipe_name(name, nonce)?;
    // SAFETY: open once in the exact local namespace. Null attributes forbid
    // inheritance; identification-only SQOS grants no server impersonation.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            null_mut(),
        )
    };
    let pipe = pipe(handle, nonce, false)?;
    let mode = PIPE_READMODE_MESSAGE;
    // SAFETY: retained duplex message pipe, only this endpoint's read mode changes.
    if unsafe { SetNamedPipeHandleState(pipe.raw(), &mode, null(), null()) } == 0 {
        return Err(last_error("set browser rendezvous message mode"));
    }
    Ok(pipe)
}

fn pipe(handle: HANDLE, nonce: [u8; 16], server: bool) -> Result<Pipe, WindowsProcessError> {
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(last_error("open private browser rendezvous"));
    }
    Ok(Pipe {
        handle: Some(OwnedHandle::new(handle, "own browser rendezvous")?),
        nonce,
        sent: 0,
        received: 0,
        server,
        connected: !server,
        finished: false,
        pending: None,
        read_completed: None,
    })
}

impl Pipe {
    pub(super) fn raw(&self) -> HANDLE {
        self.handle.as_ref().map_or(null_mut(), OwnedHandle::raw)
    }

    pub(super) fn connect(
        &mut self,
        peer: &OwnedHandle,
        deadline: Instant,
    ) -> Result<(), WindowsProcessError> {
        if !self.server || self.connected {
            return Err(invalid("browser rendezvous connect phase"));
        }
        self.begin(Operation::Connect, Some(deadline))?;
        let handle = self.raw();
        let pending = self.pending.as_ref().expect("begin owns operation");
        // SAFETY: the operation is already boxed and owned by this Pipe. Its event
        // and OVERLAPPED remain retained until completion or explicit cancellation.
        let connected = unsafe { ConnectNamedPipe(handle, pending.overlapped.get()) };
        let error = if connected == 0 {
            // SAFETY: capture immediately after the failed native call.
            unsafe { GetLastError() }
        } else {
            0
        };
        if connected != 0 || error == ERROR_PIPE_CONNECTED {
            // No asynchronous operation remains in either immediate completion case.
            self.pending.take();
        } else if error == ERROR_IO_PENDING {
            let completion = self.wait(peer, Some(deadline))?;
            self.accept(completion)?;
        } else {
            self.pending.take();
            self.finished = true;
            return Err(code_error("connect browser rendezvous", error));
        }
        self.connected = true;
        Ok(())
    }

    pub(super) fn send(
        &mut self,
        op: u16,
        payload: &[u8],
        peer: &OwnedHandle,
        deadline: Instant,
    ) -> Result<(), WindowsProcessError> {
        self.check_connected()?;
        let sequence = self
            .sent
            .checked_add(1)
            .ok_or_else(|| invalid("browser rendezvous sequence exhausted"))?;
        let bytes = encode(op, payload, self.nonce, sequence)?;
        self.begin(Operation::Write, Some(deadline))?;
        let handle = self.raw();
        let pending = self.pending.as_ref().expect("begin owns operation");
        // SAFETY: no I/O has been submitted; this Pipe exclusively owns the buffer.
        unsafe { (&mut *pending.bytes.get())[..bytes.len()].copy_from_slice(&bytes) };
        // SAFETY: boxed owned buffer and OVERLAPPED are retained for the full I/O.
        let written = unsafe {
            WriteFile(
                handle,
                pending.bytes.get().cast(),
                bytes.len() as u32,
                null_mut(),
                pending.overlapped.get(),
            )
        };
        self.submitted(written, "write browser rendezvous")?;
        let completion = self.wait(peer, Some(deadline))?;
        let (_, transferred) = self.accept(completion)?;
        if transferred != bytes.len() {
            self.finished = true;
            return Err(invalid("short browser rendezvous write"));
        }
        self.sent = sequence;
        Ok(())
    }

    pub(super) fn receive(
        &mut self,
        peer: &OwnedHandle,
        deadline: Option<Instant>,
    ) -> Result<Frame, WindowsProcessError> {
        if !self.connected || self.finished || self.handle.is_none() {
            return Err(invalid("browser rendezvous is not ready"));
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.kind != Operation::Read)
        {
            return Err(invalid("browser rendezvous has unfinished non-read I/O"));
        }
        let sequence = self
            .received
            .checked_add(1)
            .ok_or_else(|| invalid("browser rendezvous sequence exhausted"))?;
        loop {
            let completion = if let Some(completed) = self.read_completed.take() {
                completed
            } else {
                if self.pending.is_none() {
                    self.begin(Operation::Read, deadline)?;
                    let handle = self.raw();
                    let pending = self.pending.as_ref().expect("begin owns operation");
                    // SAFETY: boxed owned buffer and OVERLAPPED are retained for the full I/O.
                    let read = unsafe {
                        ReadFile(
                            handle,
                            pending.bytes.get().cast(),
                            MAX_FRAME as u32,
                            null_mut(),
                            pending.overlapped.get(),
                        )
                    };
                    // An oversized message fails with MORE_DATA; never accept its first chunk.
                    self.submitted(read, "read browser rendezvous")?;
                }
                self.wait(peer, deadline)?
            };
            if completion.error == ERROR_OPERATION_ABORTED
                && completion.pending.cancelled.get()
                && completion.transferred == 0
            {
                // A prior deadline cancelled without consuming a message. A new
                // read is safe, with the same expected sequence and new deadline.
                continue;
            }
            let (pending, transferred) = self.accept(completion)?;
            // SAFETY: completion was positively observed and its byte count bounded;
            // the kernel no longer writes this buffer while the frame is inspected.
            let bytes = unsafe { &*pending.bytes.get() };
            let frame = decode(&bytes[..transferred], self.nonce, sequence).inspect_err(|_| {
                self.finished = true;
            })?;
            self.received = sequence;
            return Ok(frame);
        }
    }

    fn check_connected(&self) -> Result<(), WindowsProcessError> {
        if !self.connected
            || self.finished
            || self.handle.is_none()
            || self.pending.is_some()
            || self.read_completed.is_some()
        {
            return Err(invalid("browser rendezvous is not ready"));
        }
        Ok(())
    }

    fn begin(
        &mut self,
        kind: Operation,
        deadline: Option<Instant>,
    ) -> Result<(), WindowsProcessError> {
        if self.finished
            || self.handle.is_none()
            || self.pending.is_some()
            || self.read_completed.is_some()
        {
            return Err(invalid("browser rendezvous has unfinished I/O"));
        }
        remaining(deadline)?;
        self.pending = Some(Pending::new(kind)?);
        Ok(())
    }

    fn submitted(
        &mut self,
        result: i32,
        operation: &'static str,
    ) -> Result<(), WindowsProcessError> {
        if result == 0 {
            // SAFETY: caller invokes this immediately after ReadFile/WriteFile.
            let error = unsafe { GetLastError() };
            if error != ERROR_IO_PENDING {
                // A failed submission did not leave an outstanding operation.
                self.pending.take();
                self.finished = true;
                return Err(code_error(operation, error));
            }
        }
        Ok(())
    }

    fn completed(&mut self) -> Option<Completion> {
        let handle = self.raw();
        let pending = self.pending.as_ref()?;
        let mut transferred = 0;
        // SAFETY: exact live pipe and boxed OVERLAPPED for its sole outstanding I/O.
        // A non-waiting result never outlives or releases unfinished native storage.
        let done =
            unsafe { GetOverlappedResult(handle, pending.overlapped.get(), &mut transferred, 0) };
        let error = if done == 0 {
            // SAFETY: capture immediately after GetOverlappedResult.
            let error = unsafe { GetLastError() };
            if error == ERROR_IO_INCOMPLETE || error == ERROR_IO_PENDING {
                return None;
            }
            error
        } else {
            0
        };
        // With this retained valid handle/OVERLAPPED, other errors report the
        // completed operation's result, including cancellation and pipe EOF.
        let pending = self.pending.take().expect("observed owned operation");
        Some(Completion {
            pending,
            transferred: transferred as usize,
            error,
        })
    }

    fn accept(
        &mut self,
        completion: Completion,
    ) -> Result<(Box<Pending>, usize), WindowsProcessError> {
        if completion.error != 0 {
            self.finished = true;
            return Err(code_error(
                "complete browser rendezvous I/O",
                completion.error,
            ));
        }
        if completion.transferred > MAX_FRAME {
            self.finished = true;
            return Err(invalid("browser rendezvous completion size"));
        }
        Ok((completion.pending, completion.transferred))
    }

    fn wait(
        &mut self,
        peer: &OwnedHandle,
        deadline: Option<Instant>,
    ) -> Result<Completion, WindowsProcessError> {
        if let Some(completed) = self.completed() {
            return Ok(completed);
        }
        let result = (|| {
            let milliseconds = remaining(deadline)?;
            let pending = self.pending.as_ref().expect("wait owns operation");
            // Prefer completed I/O when a final bound frame and peer exit race.
            let handles = [pending.event.raw(), peer.raw()];
            // SAFETY: both handles remain retained throughout the wait. The second
            // is the caller's exact authenticated peer, with SYNCHRONIZE authority.
            let outcome = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, milliseconds) };
            match outcome {
                WAIT_OBJECT_0 => self
                    .completed()
                    .ok_or_else(|| invalid("browser rendezvous incomplete signal")),
                value if value == WAIT_OBJECT_0 + 1 => {
                    Err(invalid("browser rendezvous peer exited"))
                }
                WAIT_TIMEOUT => Err(invalid("browser rendezvous deadline")),
                WAIT_FAILED => Err(last_error("wait browser rendezvous")),
                _ => Err(invalid("browser rendezvous wait result")),
            }
        })();
        if result.is_err() {
            let reading = self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.kind == Operation::Read);
            self.finished = !reading;
            // Cancellation alone is not completion. A still-pending operation
            // stays owned by Pipe for finish/retry or conservative Drop leakage.
            self.cancel();
            if let Some(completed) = self.completed()
                && reading
            {
                // Cancellation may race a valid CPWL terminal response. Keep
                // that entire completed frame for the next receive attempt.
                self.read_completed = Some(completed);
            }
        }
        result
    }

    fn cancel(&mut self) {
        let handle = self.raw();
        if let Some(pending) = &self.pending {
            pending.cancelled.set(true);
            // SAFETY: cancel only this exact owned operation; never another pipe's
            // I/O. Its storage remains retained whether cancellation succeeds or not.
            unsafe { CancelIoEx(handle, pending.overlapped.get()) };
        }
    }

    /// Irreversibly stop this endpoint. False retains its unresolved native I/O.
    pub(super) fn finish(&mut self) -> bool {
        self.finished = true;
        if self.pending.is_some() {
            self.cancel();
            if self.completed().is_none() {
                let event = self
                    .pending
                    .as_ref()
                    .expect("retained incomplete I/O")
                    .event
                    .raw();
                // SAFETY: retained exact event; fixed bounded cancellation join.
                unsafe { WaitForSingleObject(event, CANCEL_WAIT.as_millis() as u32) };
                if self.completed().is_none() {
                    return false;
                }
            }
        }
        self.read_completed.take();
        self.handle.take();
        true
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        if !self.finish() {
            // Windows may still reference BOTH the OVERLAPPED and its byte buffer.
            // Keep the event and pipe handle too; Drop cannot certify cancellation.
            if let Some(pending) = self.pending.take() {
                std::mem::forget(pending);
            }
            if let Some(handle) = self.handle.take() {
                std::mem::forget(handle);
            }
        }
    }
}

fn pipe_name(name: &str, nonce: [u8; 16]) -> Result<Vec<u16>, WindowsProcessError> {
    if nonce == [0; 16] {
        return Err(invalid("empty browser rendezvous nonce"));
    }
    let hex: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let prefix = format!(r"\\.\pipe\ColossusBrowserContext-{hex}");
    if name != format!("{prefix}-launcher") && name != format!("{prefix}-context") {
        return Err(invalid("invalid browser rendezvous namespace"));
    }
    Ok(wide(name))
}

fn numeric_sid(sid: &str) -> bool {
    if sid.len() > 184 {
        return false;
    }
    let Some(body) = sid.strip_prefix("S-1-") else {
        return false;
    };
    let mut parts = body.split('-');
    if !parts.next().is_some_and(|authority| {
        !authority.is_empty()
            && authority.bytes().all(|byte| byte.is_ascii_digit())
            && authority
                .parse::<u64>()
                .is_ok_and(|value| value <= 0xffff_ffff_ffff)
    }) {
        return false;
    }
    let subauthorities: Vec<_> = parts.collect();
    (1..=15).contains(&subauthorities.len())
        && subauthorities.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && part.parse::<u32>().is_ok()
        })
}

fn remaining(deadline: Option<Instant>) -> Result<u32, WindowsProcessError> {
    let Some(deadline) = deadline else {
        return Ok(INFINITE);
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(invalid("browser rendezvous deadline"));
    }
    Ok(remaining
        .as_millis()
        .saturating_add(1)
        .min(u128::from(INFINITE - 1)) as u32)
}

fn known_op(op: u16) -> bool {
    matches!(op, 1 | 10..=13 | 20..=25 | 26..=30)
}

fn encode(
    op: u16,
    payload: &[u8],
    nonce: [u8; 16],
    sequence: u32,
) -> Result<Vec<u8>, WindowsProcessError> {
    if !known_op(op) || sequence == 0 || payload.len() > MAX_FRAME - HEADER {
        return Err(invalid("invalid browser rendezvous frame"));
    }
    let mut bytes = Vec::with_capacity(HEADER + payload.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&op.to_le_bytes());
    bytes.extend_from_slice(&sequence.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&nonce);
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

fn decode(bytes: &[u8], nonce: [u8; 16], sequence: u32) -> Result<Frame, WindowsProcessError> {
    if !(HEADER..=MAX_FRAME).contains(&bytes.len())
        || bytes[..4] != MAGIC
        || u16::from_le_bytes([bytes[4], bytes[5]]) != VERSION
        || bytes[16..HEADER] != nonce
        || u32::from_le_bytes(bytes[8..12].try_into().expect("fixed header slice")) != sequence
        || sequence == 0
        || u32::from_le_bytes(bytes[12..16].try_into().expect("fixed header slice")) as usize
            != bytes.len() - HEADER
    {
        return Err(invalid("invalid browser rendezvous frame"));
    }
    let op = u16::from_le_bytes([bytes[6], bytes[7]]);
    if !known_op(op) {
        return Err(invalid("invalid browser rendezvous operation"));
    }
    Ok(Frame {
        op,
        payload: bytes[HEADER..].to_vec(),
    })
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn invalid(message: &'static str) -> WindowsProcessError {
    WindowsProcessError::Invalid(message.into())
}

fn last_error(operation: &'static str) -> WindowsProcessError {
    // SAFETY: caller captures the failure immediately after the native operation.
    code_error(operation, unsafe { GetLastError() })
}

fn code_error(operation: &'static str, code: u32) -> WindowsProcessError {
    WindowsProcessError::Win32 {
        operation,
        source: std::io::Error::from_raw_os_error(code as i32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::{
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Threading::{
            GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        },
    };

    // Owned by the existing Windows Process library suite in mandatory Windows CI.
    // Every endpoint is a real local kernel pipe; RAII cancels/joins even on panic.
    #[test]
    fn windows_cancelled_receive_preserves_late_terminal_response_and_sequence() {
        // SAFETY: open a retained query/synchronize handle to this exact process.
        let peer = OwnedHandle::new(
            unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    GetCurrentProcessId(),
                )
            },
            "open rendezvous test peer",
        )
        .unwrap();
        let owner = current_test_owner();
        let mut nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .to_le_bytes();
        // SAFETY: current PID is an observation, never used to mutate a peer.
        nonce[..4].copy_from_slice(&unsafe { GetCurrentProcessId() }.to_le_bytes());
        let hex: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let name = format!(r"\\.\pipe\ColossusBrowserContext-{hex}-launcher");
        let mut server = server(&name, nonce, &owner).unwrap();
        let mut client = client(&name, nonce).unwrap();
        server
            .connect(&peer, Instant::now() + Duration::from_secs(2))
            .unwrap();
        for (op, payload) in [(11, vec![1, 2, 3]), (12, vec![4, 5])] {
            // No writer yet: the real pending read must time out and request exact
            // cancellation. Its cancellation may complete immediately or on retry.
            assert!(
                server
                    .receive(&peer, Some(Instant::now() + Duration::from_millis(10)))
                    .is_err()
            );
            assert!(
                server.pending.as_ref().is_some_and(|pending| {
                    pending.kind == Operation::Read && pending.cancelled.get()
                }) || server.read_completed.as_ref().is_some_and(|completed| {
                    completed.pending.kind == Operation::Read
                        && completed.pending.cancelled.get()
                        && completed.error == ERROR_OPERATION_ABORTED
                        && completed.transferred == 0
                }),
                "the actual read was not submitted and cancelled before the late response"
            );
            client
                .send(op, &payload, &peer, Instant::now() + Duration::from_secs(2))
                .unwrap();
            let frame = server
                .receive(&peer, Some(Instant::now() + Duration::from_secs(2)))
                .unwrap();
            assert_eq!(frame.op, op);
            assert_eq!(frame.payload, payload);
        }
        server
            .send(13, &[], &peer, Instant::now() + Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            client
                .receive(&peer, Some(Instant::now() + Duration::from_secs(2)))
                .unwrap()
                .op,
            13
        );
        assert!(server.finish());
        assert!(client.finish());
        assert!(server.finish());
        assert!(client.finish());
    }

    fn current_test_owner() -> String {
        let mut raw = null_mut();
        // SAFETY: query only this process's token into one owned output handle.
        assert_ne!(
            unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) },
            0
        );
        let token = OwnedHandle::new(raw, "open rendezvous test token").unwrap();
        let mut size = 0;
        // SAFETY: documented sizing query with a retained token and writable length.
        unsafe { GetTokenInformation(token.raw(), TokenUser, null_mut(), 0, &mut size) };
        assert!((size_of::<TOKEN_USER>() as u32..=64 * 1024).contains(&size));
        let mut bytes = vec![0_u8; size as usize];
        // SAFETY: exact bounded writable allocation, retained token and output length.
        assert_ne!(
            unsafe {
                GetTokenInformation(
                    token.raw(),
                    TokenUser,
                    bytes.as_mut_ptr().cast(),
                    size,
                    &mut size,
                )
            },
            0
        );
        // SAFETY: successful TOKEN_USER query includes this complete fixed header.
        let user = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<TOKEN_USER>()) };
        let mut sid = null_mut();
        // SAFETY: the current kernel TOKEN_USER SID stays retained through conversion.
        assert_ne!(
            unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) },
            0
        );
        let allocation = Descriptor(sid.cast());
        let mut length = 0;
        // SAFETY: converted SID is a terminated Windows allocation, bounded by
        // the maximum supported textual SID length before constructing a slice.
        unsafe {
            while length < 184 && *sid.add(length) != 0 {
                length += 1;
            }
        }
        assert!(length < 184);
        // SAFETY: inspected UTF-16 SID prefix is within the retained allocation.
        let value = String::from_utf16(unsafe { std::slice::from_raw_parts(sid, length) }).unwrap();
        drop(allocation);
        value
    }

    #[test]
    fn rejects_malformed_or_replayed_bounded_frames() {
        let nonce = [0x5a; 16];
        let valid = encode(20, &[0x33; MAX_FRAME - HEADER], nonce, 7).unwrap();
        assert_eq!(decode(&valid, nonce, 7).unwrap().payload.len(), 224);
        for offset in [0, 4, 6, 8, 12, 16] {
            let mut bytes = valid.clone();
            bytes[offset] ^= 0xff;
            assert!(decode(&bytes, nonce, 7).is_err(), "offset={offset}");
        }
        assert!(decode(&valid, nonce, 6).is_err());
        assert!(decode(&valid[..HEADER - 1], nonce, 7).is_err());
        assert!(decode(&valid[..valid.len() - 1], nonce, 7).is_err());
        let mut oversized = valid;
        oversized.push(0);
        assert!(decode(&oversized, nonce, 7).is_err());
        assert!(encode(1, &[0; 225], nonce, 1).is_err());
        assert!(encode(2, &[], nonce, 1).is_err());
        assert!(encode(1, &[], nonce, 0).is_err());
    }

    #[test]
    fn browser_creation_operations_keep_closed_frame_namespace() {
        let nonce = [0x5a; 16];
        for op in 26..=30 {
            let bytes = encode(op, &[1, 2, 3], nonce, 1).unwrap();
            assert_eq!(decode(&bytes, nonce, 1).unwrap().op, op);
        }
        for op in [0_u16, 2, 14, 19, 31, u16::MAX] {
            assert!(encode(op, &[], nonce, 1).is_err());
            let mut bytes = encode(26, &[], nonce, 1).unwrap();
            bytes[6..8].copy_from_slice(&op.to_le_bytes());
            assert!(decode(&bytes, nonce, 1).is_err());
        }
    }

    #[test]
    fn local_namespace_and_numeric_owner_are_closed() {
        let nonce = [0x5a; 16];
        let local = format!(
            r"\\.\pipe\ColossusBrowserContext-{}-launcher",
            "5a".repeat(16)
        );
        assert!(pipe_name(&local, nonce).is_ok());
        assert!(pipe_name(&local.replace(r"\\.\", r"\\remote\"), nonce).is_err());
        assert!(pipe_name(&format!("{local}\\other"), nonce).is_err());
        assert!(pipe_name(&local, [1; 16]).is_err());
        assert!(numeric_sid("S-1-5-21-123-456-789-1001"));
        for sid in ["SY", "S-1-5-", "S-1-5-1)(A;;GA;;;WD)", "S-1-5-4294967296"] {
            assert!(!numeric_sid(sid));
        }
    }
}
