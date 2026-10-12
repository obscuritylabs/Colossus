//! Retained nonblocking receipt bytes with one finite native read deadline.

use std::{
    fs::File,
    io::{self, Read as _},
    time::{Duration, Instant},
};

pub(crate) struct ReceiptOutput {
    file: File,
    bytes: Vec<u8>,
    complete: bool,
    failed: bool,
}

impl ReceiptOutput {
    pub(crate) fn new(file: File) -> Self {
        Self {
            file,
            bytes: Vec::new(),
            complete: false,
            failed: false,
        }
    }

    #[cfg(feature = "macos-process-observation-fixture")]
    pub(crate) fn untouched_reader(&self) -> io::Result<File> {
        if self.complete || self.failed || !self.bytes.is_empty() {
            return Err(io::Error::other(
                "fixture receipt has already been consumed",
            ));
        }
        self.file.try_clone()
    }

    pub(crate) fn drain(&mut self, bound: Duration) -> io::Result<&[u8]> {
        if self.failed {
            return Err(io::Error::other("creator receipt output is fenced"));
        }
        let flags = rustix::fs::fcntl_getfl(&self.file)?;
        rustix::fs::fcntl_setfl(&self.file, flags | rustix::fs::OFlags::NONBLOCK)?;
        let deadline = Instant::now() + bound;
        while !self.complete {
            let mut buffer = [0; 4096];
            match self.file.read(&mut buffer) {
                Ok(0) => self.complete = true,
                Ok(count) => {
                    if self.bytes.len() + count > 2060 {
                        self.failed = true;
                        return Err(io::Error::other("creator emitted oversized receipt"));
                    }
                    self.bytes.extend_from_slice(&buffer[..count]);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "creator receipt EOF is not verified",
                        ));
                    }
                    let mut wait = [rustix::event::PollFd::new(
                        &self.file,
                        rustix::event::PollFlags::IN,
                    )];
                    let timeout =
                        rustix::event::Timespec::try_from(remaining).map_err(io::Error::other)?;
                    match rustix::event::poll(&mut wait, Some(&timeout)) {
                        Ok(_) => {}
                        Err(rustix::io::Errno::INTR) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => {
                    self.failed = true;
                    return Err(error);
                }
            }
        }
        Ok(&self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write as _, os::unix::net::UnixStream};
    #[test]
    fn missing_eof_has_a_finite_deadline_and_retains_partial_receipt_for_retry() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let mut reader = ReceiptOutput::new(File::from(std::os::fd::OwnedFd::from(reader)));
        writer.write_all(b"receipt").unwrap();
        assert_eq!(
            reader.drain(Duration::from_millis(10)).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        writer.write_all(b"-tail").unwrap();
        drop(writer);
        assert_eq!(
            reader.drain(Duration::from_secs(1)).unwrap(),
            b"receipt-tail"
        );
        assert_eq!(reader.drain(Duration::ZERO).unwrap(), b"receipt-tail");
    }
    #[test]
    fn excessive_receipt_stays_fenced_after_output_closes() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let mut reader = ReceiptOutput::new(File::from(std::os::fd::OwnedFd::from(reader)));
        writer.write_all(&[0; 2061]).unwrap();
        drop(writer);
        assert!(reader.drain(Duration::from_secs(1)).is_err());
        assert!(reader.drain(Duration::ZERO).is_err());
    }
}
