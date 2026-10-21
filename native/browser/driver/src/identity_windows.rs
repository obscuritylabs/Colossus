//! One expiring native review of Chromium's exact pending client-key request.
//! Public certificate metadata only; Chromium performs all Windows private-key use.
mod dialog;
#[cfg(test)]
mod tests;

use colossus_contracts::BrowserOrigin;
use colossus_native_browser_pki::{fingerprint, identity_validity};
use colossus_ports::BrowserDriverControl;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone)]
pub(super) struct Candidate {
    index: i32,
    fingerprint: String,
    not_before: i64,
    not_after: i64,
}

#[derive(Clone)]
pub(super) struct Request {
    pub tab: u64,
    pub generation: u64,
    pub request_id: u64,
    pub control_generation: u64,
    pub origin: BrowserOrigin,
    candidates: Vec<Candidate>,
    deadline: Instant,
    cancel: Arc<AtomicBool>,
    control: BrowserDriverControl,
}
impl Request {
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
            || self.control.is_cancelled()
            || Instant::now() >= self.deadline
    }
    fn selected(&self, fingerprint: Option<&str>) -> i32 {
        if self.cancelled() {
            return -1;
        }
        let Some(now) = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|value| i64::try_from(value.as_secs()).ok())
        else {
            return -1;
        };
        self.candidates
            .iter()
            .find(|candidate| {
                Some(candidate.fingerprint.as_str()) == fingerprint
                    && candidate.not_before <= now
                    && now < candidate.not_after
            })
            .map_or(-1, |candidate| candidate.index)
    }
}

struct Pending {
    request: Request,
    result: mpsc::Receiver<Option<String>>,
}

#[derive(Default)]
pub(crate) struct Review {
    enabled: bool,
    operation: Option<(u64, BrowserDriverControl)>,
    pending: Option<Pending>,
    worker: Option<JoinHandle<()>>,
    attempts: u8,
}
impl Review {
    pub fn enable(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    pub fn operation(&mut self, generation: u64, control: BrowserDriverControl) {
        self.cancel(None);
        self.operation = Some((generation, control));
        self.attempts = 0;
    }
    pub fn generation_changed(&mut self, generation: u64) {
        if self
            .operation
            .as_ref()
            .is_some_and(|(current, _)| *current != generation)
        {
            self.cancel(None);
            self.operation = None;
        }
    }
    pub fn cancel(&mut self, request_id: Option<u64>) {
        if let Some(pending) = &self.pending
            && request_id.is_none_or(|expected| expected == pending.request.request_id)
        {
            pending.request.cancel.store(true, Ordering::Release);
        }
    }
    fn retire_worker(&mut self) {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
    }
    pub fn start(
        &mut self,
        tab: u64,
        generation: u64,
        request_id: u64,
        origin: BrowserOrigin,
        certificates: &[&[u8]],
    ) -> bool {
        self.retire_worker();
        if !self.enabled
            || self.pending.is_some()
            || self.worker.is_some()
            || self.attempts >= 8
            || tab == 0
            || generation == 0
            || request_id == 0
            || !origin.as_str().starts_with("https://")
            || origin.as_str().len() > 4096
            || certificates.is_empty()
            || certificates.len() > 64
        {
            return false;
        }
        let Some((control_generation, control)) = self.operation.clone() else {
            return false;
        };
        if control.is_cancelled() {
            return false;
        }
        let mut candidates = Vec::<Candidate>::new();
        for (index, der) in certificates.iter().enumerate() {
            if der.is_empty() || der.len() > 65_536 {
                return false;
            }
            let Ok(validity) = identity_validity(der) else {
                return false;
            };
            if let Some((not_before, not_after)) = validity {
                let fingerprint = fingerprint(der);
                if candidates
                    .iter()
                    .any(|candidate| candidate.fingerprint == fingerprint)
                {
                    return false;
                }
                candidates.push(Candidate {
                    index: index as i32,
                    fingerprint,
                    not_before,
                    not_after,
                });
            }
        }
        if candidates.is_empty() {
            return false;
        }
        let request = Request {
            tab,
            generation,
            request_id,
            control_generation,
            origin,
            candidates,
            deadline: Instant::now() + TIMEOUT,
            cancel: Arc::new(AtomicBool::new(false)),
            control,
        };
        let prompt = request.clone();
        let (send, result) = mpsc::sync_channel(1);
        let Ok(worker) = std::thread::Builder::new()
            .name("colossus-browser-identity-review".into())
            .spawn(move || {
                let _ = send.send(dialog::prompt(&prompt));
            })
        else {
            return false;
        };
        self.worker = Some(worker);
        self.attempts += 1;
        self.pending = Some(Pending { request, result });
        true
    }
    /// Move a decision once. The owning CEF thread must recheck native authority.
    pub fn ready(&mut self) -> Option<(Request, i32)> {
        self.retire_worker();
        let pending = self.pending.as_ref()?;
        let selection = if pending.request.cancelled() {
            None
        } else {
            match pending.result.try_recv() {
                Ok(selection) => selection,
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => None,
            }
        };
        let pending = self.pending.take()?;
        let index = pending.request.selected(selection.as_deref());
        Some((pending.request, index))
    }
    pub fn finish(&mut self) {
        self.cancel(None);
        let deadline = Instant::now() + Duration::from_secs(2);
        while self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            if Instant::now() >= deadline {
                std::process::abort();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.retire_worker();
        self.pending = None;
        self.operation = None;
    }
}
impl Drop for Review {
    fn drop(&mut self) {
        self.finish();
    }
}
