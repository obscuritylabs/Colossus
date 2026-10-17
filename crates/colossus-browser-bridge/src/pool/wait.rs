use super::*;
use std::time::Duration;
use tokio::time::{Instant, interval};

struct CleanupOnDrop {
    inner: Arc<Inner>,
    entry: Arc<Entry>,
    armed: bool,
}

impl Drop for CleanupOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.entry.admission_fenced.store(true, Ordering::Release);
            let _ = cleanup::start(Arc::clone(&self.inner), Arc::clone(&self.entry));
        }
    }
}

pub(super) async fn response<T>(
    inner: Arc<Inner>,
    entry: Arc<Entry>,
    mut receiver: oneshot::Receiver<Result<T, BrowserDriverError>>,
    control: &BrowserDriverControl,
    timeout_ms: u32,
) -> Result<T, BrowserDriverError> {
    let mut guard = CleanupOnDrop {
        inner,
        entry,
        armed: true,
    };
    let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
    let mut poll = interval(Duration::from_millis(10));
    loop {
        tokio::select! {
            result = &mut receiver => {
                let result = result.map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                if control.is_cancelled() || guard.entry.authority.is_cancelled() && result.is_ok() {
                    return Err(BrowserDriverError::OutcomeUnknown);
                }
                guard.armed = matches!(result, Err(BrowserDriverError::OutcomeUnknown));
                return result;
            }
            _ = poll.tick() => {
                if control.is_cancelled() || Instant::now() >= deadline { return Err(BrowserDriverError::OutcomeUnknown); }
            }
        }
    }
}
