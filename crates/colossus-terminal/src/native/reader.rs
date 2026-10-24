//! Cancellable Unix output without changing the PTY's shared blocking mode.
use filedescriptor::{FileDescriptor, POLLIN, poll, pollfd};
use std::{
    io::{self, Read},
    os::fd::{AsRawFd, RawFd},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const FINAL_DRAIN_BYTES: usize = 64 * 1024;
const FINAL_DRAIN_TIME: Duration = Duration::from_millis(100);

pub(super) struct Reader {
    descriptor: FileDescriptor,
    closed: Arc<AtomicBool>,
    drain: Option<FinalDrain>,
}
struct FinalDrain {
    started: Instant,
    remaining: usize,
}
impl Reader {
    pub(super) fn new(
        master: &dyn portable_pty::MasterPty,
        closed: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        // Keep the actual master borrowed through duplication: the raw number
        // never supplies ownership or outlives the object proving its identity.
        struct MasterDescriptor<'a> {
            _owner: &'a dyn portable_pty::MasterPty,
            descriptor: RawFd,
        }
        impl AsRawFd for MasterDescriptor<'_> {
            fn as_raw_fd(&self) -> RawFd {
                self.descriptor
            }
        }
        let descriptor = master
            .as_raw_fd()
            .ok_or_else(|| io::Error::other("native PTY descriptor is unavailable"))?;
        let descriptor = FileDescriptor::dup(&MasterDescriptor {
            _owner: master,
            descriptor,
        })
        .map_err(io_error)?;
        let reader = Self {
            descriptor,
            closed,
            drain: None,
        };
        // Validate the platform readiness adapter before a child exists.
        let _ = reader.ready(Duration::ZERO)?;
        Ok(reader)
    }

    fn ready(&self, wait: Duration) -> io::Result<bool> {
        let mut descriptors = [pollfd {
            fd: self.descriptor.as_raw_fd(),
            events: POLLIN,
            revents: 0,
        }];
        poll(&mut descriptors, Some(wait))
            .map(|count| count != 0)
            .map_err(io_error)
    }
}
impl Read for Reader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            let closing = self.closed.load(Ordering::Acquire);
            let limit = if closing {
                let drain = self.drain.get_or_insert_with(|| FinalDrain {
                    started: Instant::now(),
                    remaining: FINAL_DRAIN_BYTES,
                });
                if drain.remaining == 0 || drain.started.elapsed() >= FINAL_DRAIN_TIME {
                    return Ok(0);
                }
                buffer.len().min(drain.remaining)
            } else {
                buffer.len()
            };
            // Only this reader consumes output. A readable descriptor can be
            // read without setting O_NONBLOCK, which would also alter input.
            if !self.ready(if closing {
                Duration::ZERO
            } else {
                POLL_INTERVAL
            })? {
                if closing {
                    return Ok(0);
                }
                continue;
            }
            let count = self.descriptor.read(&mut buffer[..limit])?;
            if let Some(drain) = &mut self.drain {
                drain.remaining -= count;
            }
            return Ok(count);
        }
    }
}
fn io_error(error: filedescriptor::Error) -> io::Error {
    match error {
        filedescriptor::Error::Io(error) | filedescriptor::Error::Poll(error) => error,
        error => io::Error::other(error),
    }
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;
