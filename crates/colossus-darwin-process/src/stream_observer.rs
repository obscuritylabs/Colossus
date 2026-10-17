//! Passive kernel EOF observations that do not consume protocol bytes.

use std::{
    io, mem,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::net::UnixStream,
    },
    time::{Duration, Instant},
};

/// A private EOF observer for one retained connected Unix stream.
///
/// This retains a close-on-exec duplicate of the exact socket and observes its
/// kernel read-side EOF without reading, peeking, or receiving ancillary rights.
/// It proves no peer identity or application shutdown. Callers must independently
/// authenticate the stream and fence authority when observation fails.
pub struct DarwinStreamDisconnectObserver {
    queue: OwnedFd,
    stream: OwnedFd,
    disconnected: bool,
}

impl DarwinStreamDisconnectObserver {
    /// Attach to this stream before admitting work on its authenticated protocol.
    ///
    /// # Errors
    /// Rejects disconnected sockets, descriptor allocation failures, or absent
    /// kernel registration receipts. No caller-selected raw handle is accepted.
    pub fn register(stream: &UnixStream) -> io::Result<Self> {
        stream.peer_addr()?;
        // SAFETY: duplicate a live borrowed socket into a new owned descriptor.
        let raw = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 8) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful fcntl returned this uniquely owned descriptor.
        let stream = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: kqueue has no caller pointers and returns a new descriptor.
        let raw = unsafe { libc::kqueue() };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful kqueue returned this uniquely owned descriptor.
        let queue = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: change only inheritance flags of this live owned queue.
        if unsafe { libc::fcntl(queue.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let change = libc::kevent {
            ident: stream.as_raw_fd() as libc::uintptr_t,
            filter: libc::EVFILT_READ,
            flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_CLEAR | libc::EV_RECEIPT,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let mut receipt = empty_event();
        let zero = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: fixed event and timeout records remain valid for this call.
        let count = unsafe { libc::kevent(queue.as_raw_fd(), &change, 1, &mut receipt, 1, &zero) };
        if count < 0 {
            return Err(io::Error::last_os_error());
        }
        if count != 1 || receipt.flags & libc::EV_ERROR == 0 {
            return Err(invalid("absent stream registration receipt"));
        }
        if receipt.data != 0 {
            let error = i32::try_from(receipt.data)
                .ok()
                .filter(|code| *code > 0)
                .ok_or_else(|| invalid("invalid stream registration error"))?;
            return Err(io::Error::from_raw_os_error(error));
        }
        Ok(Self {
            queue,
            stream,
            disconnected: false,
        })
    }

    /// Wait at most sixty seconds for read-side EOF on the retained stream.
    ///
    /// Incoming bytes remain unread for the protocol owner. Interrupted waits
    /// preserve the absolute deadline, and an observed EOF is sticky.
    ///
    /// # Errors
    /// Rejects excessive timeouts, failed kernel waits, or mismatched events.
    pub fn wait(&mut self, timeout: Duration) -> io::Result<bool> {
        if timeout > Duration::from_secs(60) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stream observation timeout exceeds bound",
            ));
        }
        if self.disconnected {
            return Ok(true);
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = libc::timespec {
                tv_sec: remaining.as_secs() as libc::time_t,
                tv_nsec: remaining.subsec_nanos() as libc::c_long,
            };
            let mut event = empty_event();
            // SAFETY: queue and bounded writable event/timeout records are live.
            let count = unsafe {
                libc::kevent(
                    self.queue.as_raw_fd(),
                    std::ptr::null(),
                    0,
                    &mut event,
                    1,
                    &timeout,
                )
            };
            if count < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted && Instant::now() < deadline {
                    continue;
                }
                return Err(error);
            }
            if count == 0 {
                return Ok(false);
            }
            if count != 1
                || event.ident != self.stream.as_raw_fd() as libc::uintptr_t
                || event.filter != libc::EVFILT_READ
                || event.flags & libc::EV_ERROR != 0
            {
                return Err(invalid("invalid stream disconnect event"));
            }
            if event.flags & libc::EV_EOF != 0 {
                self.disconnected = true;
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
        }
    }
}

fn empty_event() -> libc::kevent {
    // SAFETY: the SDK event record contains integers and a nullable pointer.
    unsafe { mem::zeroed() }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::Shutdown,
    };

    #[test]
    fn disconnect_does_not_consume_buffered_protocol_bytes() {
        let (mut stream, mut peer) = UnixStream::pair().expect("private pair");
        let mut observer =
            DarwinStreamDisconnectObserver::register(&stream).expect("kernel registration");
        peer.write_all(b"framed-protocol").expect("protocol bytes");
        assert!(!observer.wait(Duration::ZERO).expect("readable is not EOF"));
        peer.shutdown(Shutdown::Write).expect("peer EOF");
        assert!(
            observer
                .wait(Duration::from_secs(1))
                .expect("EOF observation")
        );
        assert!(observer.wait(Duration::ZERO).expect("sticky EOF"));
        let mut bytes = Vec::new();
        stream
            .read_to_end(&mut bytes)
            .expect("protocol owner reads");
        assert_eq!(bytes, b"framed-protocol");
    }

    #[test]
    fn live_idle_stream_times_out_without_peer_authority() {
        let (stream, _peer) = UnixStream::pair().expect("private pair");
        let mut observer =
            DarwinStreamDisconnectObserver::register(&stream).expect("kernel registration");
        assert!(
            !observer
                .wait(Duration::from_millis(1))
                .expect("idle timeout")
        );
        assert!(observer.wait(Duration::from_secs(61)).is_err());
    }
}
