//! Retained custody for a bounded, disposable UIA/WGC subprocess.
use super::Error;
use std::time::{Duration, Instant};

pub(super) struct ReadWorker {
    pub child: std::process::Child,
    pub job: Option<colossus_windows_native::KillOnCloseJob>,
    pub reader: Option<std::thread::JoinHandle<()>>,
}
impl ReadWorker {
    pub fn cleanup(&mut self) -> Result<(), Error> {
        if self
            .child
            .try_wait()
            .map_err(|_| Error::OutcomeUnknown)?
            .is_none()
        {
            if let Some(job) = &self.job {
                job.terminate().map_err(|_| Error::OutcomeUnknown)?;
            } else {
                self.child.kill().map_err(|_| Error::OutcomeUnknown)?;
            }
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if self
                .child
                .try_wait()
                .map_err(|_| Error::OutcomeUnknown)?
                .is_some()
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err(Error::OutcomeUnknown);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Process exit closes its pipe. Never join an unbounded read thread.
        while self.reader.as_ref().is_some_and(|r| !r.is_finished()) {
            if Instant::now() >= deadline {
                return Err(Error::OutcomeUnknown);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some(reader) = self.reader.take() {
            reader.join().map_err(|_| Error::OutcomeUnknown)?;
        }
        Ok(())
    }
}
impl Drop for ReadWorker {
    fn drop(&mut self) {
        // Only fallback, never an acknowledged cleanup result. The host retains
        // its supervising broker job if cleanup returns OutcomeUnknown.
        if let Some(job) = &self.job {
            let _ = job.terminate();
        } else {
            let _ = self.child.kill();
        }
    }
}
