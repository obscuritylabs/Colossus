//! Finite native transport workers. Frames are opaque to this relay.

#[cfg(any(target_os = "macos", test))]
use std::{
    io,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

#[cfg(any(feature = "macos-process-entry-development", test))]
use std::{net::Shutdown, os::unix::net::UnixStream, sync::mpsc::Sender};

#[cfg(any(feature = "macos-process-entry-development", test))]
pub(crate) struct Relays {
    streams: Vec<UnixStream>,
    completion: Receiver<io::Result<()>>,
    sender: Sender<io::Result<()>>,
    count: usize,
    failed: bool,
    failure_lease: Option<UnixStream>,
}

#[cfg(any(feature = "macos-process-entry-development", test))]
impl Relays {
    pub(crate) fn new() -> Self {
        let (sender, completion) = mpsc::channel();
        Self {
            streams: Vec::new(),
            completion,
            sender,
            count: 0,
            failed: false,
            failure_lease: None,
        }
    }

    #[cfg(any(feature = "macos-process-entry-development", test))]
    pub(crate) fn fence_failures(&mut self, lease: &UnixStream) -> io::Result<()> {
        self.failure_lease = Some(lease.try_clone()?);
        Ok(())
    }

    pub(crate) fn connect(&mut self, left: UnixStream, right: UnixStream) -> io::Result<()> {
        self.streams.push(left.try_clone()?);
        self.streams.push(right.try_clone()?);
        self.start(left.try_clone()?, right.try_clone()?)?;
        self.start(right, left)
    }

    fn start(&mut self, mut source: UnixStream, mut destination: UnixStream) -> io::Result<()> {
        let sender = self.sender.clone();
        let failure_lease = self
            .failure_lease
            .as_ref()
            .map(UnixStream::try_clone)
            .transpose()?;
        thread::Builder::new()
            .name("browser-opaque-relay".into())
            .spawn(move || {
                let result = io::copy(&mut source, &mut destination)
                    .and_then(|_| destination.shutdown(Shutdown::Write));
                if result.is_err()
                    && let Some(lease) = failure_lease
                {
                    let _ = lease.shutdown(Shutdown::Both);
                }
                let _ = sender.send(result);
            })?;
        self.count += 1;
        Ok(())
    }

    pub(crate) fn close_and_settle(&mut self, bound: Duration) -> io::Result<()> {
        for stream in &self.streams {
            let _ = stream.shutdown(Shutdown::Both);
        }
        self.settle(bound)
    }

    pub(crate) fn settle(&mut self, bound: Duration) -> io::Result<()> {
        let deadline = Instant::now() + bound;
        while self.count != 0 {
            let result = self
                .completion
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "native relays did not settle")
                })?;
            self.count -= 1;
            // Shutdown revocation normally interrupts an in-flight copy. That
            // is terminal transport closure, not a graceful browser receipt.
            if let Err(error) = result
                && !matches!(
                    error.kind(),
                    io::ErrorKind::BrokenPipe
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::NotConnected
                        | io::ErrorKind::UnexpectedEof
                )
            {
                self.failed = true;
            }
        }
        if self.failed {
            Err(io::Error::other("native relay failed"))
        } else {
            Ok(())
        }
    }
}

#[cfg(any(feature = "macos-process-entry-development", test))]
impl Drop for Relays {
    fn drop(&mut self) {
        for stream in &self.streams {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) struct OutputDrains {
    completion: Receiver<io::Result<()>>,
    count: usize,
    failed: bool,
    #[cfg(any(
        feature = "macos-process-observation-fixture",
        feature = "macos-process-output-diagnostic"
    ))]
    capture: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
}

#[cfg(target_os = "macos")]
impl OutputDrains {
    pub(crate) fn is_settled(&self) -> bool {
        self.count == 0 && !self.failed
    }
    pub(crate) fn start(files: Vec<std::fs::File>) -> io::Result<Self> {
        use std::io::Read as _;
        let (sender, completion) = mpsc::channel();
        let mut owner = Self {
            completion,
            count: 0,
            failed: false,
            #[cfg(any(
                feature = "macos-process-observation-fixture",
                feature = "macos-process-output-diagnostic"
            ))]
            capture: std::sync::Arc::new(std::sync::Mutex::new(vec![Vec::new(); files.len()])),
        };
        for (index, mut file) in files.into_iter().enumerate() {
            let sender = sender.clone();
            #[cfg(any(
                feature = "macos-process-observation-fixture",
                feature = "macos-process-output-diagnostic"
            ))]
            let capture = std::sync::Arc::clone(&owner.capture);
            #[cfg(not(any(
                feature = "macos-process-observation-fixture",
                feature = "macos-process-output-diagnostic"
            )))]
            let _ = index;
            thread::Builder::new()
                .name("browser-bounded-output".into())
                .spawn(move || {
                    // Output is deliberately never interpreted as native authority.
                    // A constant-size buffer keeps browser logging from blocking
                    // its stdout pipes or allocating memory proportional to output.
                    let result = (|| {
                        let mut buffer = [0; 8192];
                        loop {
                            match file.read(&mut buffer) {
                                Ok(0) => return Ok(()),
                                Ok(count) => {
                                    #[cfg(any(
                                        feature = "macos-process-observation-fixture",
                                        feature = "macos-process-output-diagnostic"
                                    ))]
                                    if let Ok(mut capture) = capture.lock() {
                                        let bytes = &mut capture[index];
                                        let retained = count
                                            .min((64 * 1024_usize).saturating_sub(bytes.len()));
                                        bytes.extend_from_slice(&buffer[..retained]);
                                    }
                                    #[cfg(not(any(
                                        feature = "macos-process-observation-fixture",
                                        feature = "macos-process-output-diagnostic"
                                    )))]
                                    let _ = count;
                                }
                                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                                Err(error) => return Err(error),
                            }
                        }
                    })();
                    let _ = sender.send(result);
                })?;
            owner.count += 1;
        }
        Ok(owner)
    }

    #[cfg(any(
        feature = "macos-process-observation-fixture",
        feature = "macos-process-output-diagnostic"
    ))]
    pub(crate) fn captured(&self) -> io::Result<Vec<Vec<u8>>> {
        self.capture
            .lock()
            .map(|bytes| bytes.clone())
            .map_err(|_| io::Error::other("fixture native output capture is poisoned"))
    }

    pub(crate) fn settle(&mut self, bound: Duration) -> io::Result<()> {
        let deadline = Instant::now() + bound;
        let mut failure = None;
        while self.count != 0 {
            let result = self
                .completion
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "native outputs did not settle")
                })?;
            self.count -= 1;
            if let Err(error) = result {
                self.failed = true;
                failure.get_or_insert(error);
            }
        }
        if self.failed {
            Err(failure
                .unwrap_or_else(|| io::Error::other("native output drain previously failed")))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    #[test]
    fn relay_preserves_half_close_without_reading_protocol_authority() {
        let (mut core, a) = UnixStream::pair().unwrap();
        let (b, mut browser) = UnixStream::pair().unwrap();
        let mut relays = Relays::new();
        relays.connect(a, b).unwrap();
        core.write_all(b"private-opaque-frame").unwrap();
        core.shutdown(Shutdown::Write).unwrap();
        let mut received = Vec::new();
        browser.read_to_end(&mut received).unwrap();
        assert_eq!(received, b"private-opaque-frame");
        browser.write_all(b"admission-phases").unwrap();
        browser.shutdown(Shutdown::Write).unwrap();
        let mut received = Vec::new();
        core.read_to_end(&mut received).unwrap();
        assert_eq!(received, b"admission-phases");
        relays.settle(Duration::from_secs(1)).unwrap();
    }
    #[test]
    fn revocation_settles_idle_bidirectional_workers() {
        let (_core, a) = UnixStream::pair().unwrap();
        let (b, _browser) = UnixStream::pair().unwrap();
        let mut relays = Relays::new();
        relays.connect(a, b).unwrap();
        relays.close_and_settle(Duration::from_secs(1)).unwrap();
    }
    #[test]
    fn write_failure_explicitly_revokes_the_independent_owner_lease() {
        let (mut core, a) = UnixStream::pair().unwrap();
        let (b, browser) = UnixStream::pair().unwrap();
        let (lease, mut observer) = UnixStream::pair().unwrap();
        observer
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut relays = Relays::new();
        relays.fence_failures(&lease).unwrap();
        relays.connect(a, b).unwrap();
        drop(browser);
        core.write_all(b"frame-after-browser-channel-loss").unwrap();
        let mut byte = [0];
        assert_eq!(observer.read(&mut byte).unwrap(), 0);
        relays.close_and_settle(Duration::from_secs(1)).unwrap();
    }
}

#[cfg(all(test, feature = "macos-process-observation-fixture"))]
mod fixture_capture_tests {
    use super::*;
    use std::{fs::File, io::Write as _, os::fd::OwnedFd};
    #[test]
    fn fixture_native_output_is_bounded_and_does_not_block_draining() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let mut drain = OutputDrains::start(vec![File::from(OwnedFd::from(reader))]).unwrap();
        writer.write_all(&vec![b'x'; 65537]).unwrap();
        drop(writer);
        drain.settle(Duration::from_secs(1)).unwrap();
        assert_eq!(drain.captured().unwrap()[0].len(), 65536);
        assert!(drain.is_settled());
    }
    #[test]
    fn captured_startup_diagnostic_does_not_replace_output_eof() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let mut drain = OutputDrains::start(vec![File::from(OwnedFd::from(reader))]).unwrap();
        writer.write_all(b"finite diagnostic").unwrap();
        assert_eq!(
            drain.settle(Duration::from_millis(10)).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(!drain.is_settled());
        drop(writer);
        drain.settle(Duration::from_secs(1)).unwrap();
        assert_eq!(drain.captured().unwrap()[0], b"finite diagnostic");
    }
}
