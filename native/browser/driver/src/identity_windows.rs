//! One expiring native request; only the separate Desktop owner may choose a key.
//! Public certificate metadata only; Chromium performs all Windows private-key use.
#[cfg(test)]
mod tests;

use colossus_browser_presentation::{
    ClientIdentityCandidate, ClientIdentityChallenge, ClientIdentityKey, ClientIdentityPolicy,
    PresentationError,
};
use colossus_contracts::BrowserOrigin;
use colossus_native_browser_pki::{fingerprint, identity_label, identity_validity};
use colossus_ports::BrowserDriverControl;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone)]
pub(super) struct Candidate {
    index: i32,
    fingerprint: String,
    label: String,
    not_before: i64,
    not_after: i64,
}
impl Candidate {
    fn current(&self, now: i64) -> bool {
        self.not_before <= now && now < self.not_after
    }
}

fn now() -> Option<i64> {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()
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
    pub fn key(&self) -> ClientIdentityKey {
        ClientIdentityKey {
            tab: self.tab,
            generation: self.generation,
            request_id: self.request_id,
            control_generation: self.control_generation,
        }
    }
    fn selected(&self, fingerprint: Option<&str>) -> i32 {
        let Some(now) = now().filter(|_| !self.cancelled()) else {
            return -1;
        };
        self.candidates
            .iter()
            .find(|candidate| {
                Some(candidate.fingerprint.as_str()) == fingerprint && candidate.current(now)
            })
            .map_or(-1, |candidate| candidate.index)
    }
    fn challenge(&self) -> Option<ClientIdentityChallenge> {
        let now = now().filter(|_| !self.cancelled())?;
        let remaining_ms = u32::try_from(
            self.deadline
                .saturating_duration_since(Instant::now())
                .as_millis(),
        )
        .ok()?
        .max(1);
        Some(ClientIdentityChallenge {
            key: self.key(),
            origin: self.origin.clone(),
            candidates: self
                .candidates
                .iter()
                .filter(|candidate| candidate.current(now))
                .map(|candidate| ClientIdentityCandidate {
                    fingerprint_sha256: candidate.fingerprint.clone(),
                    label: candidate.label.clone(),
                })
                .collect(),
            remaining_ms,
        })
    }
}

enum Decision {
    Pending,
    Select(String),
    Decline,
}
struct Pending {
    request: Request,
    decision: Decision,
}

#[derive(Default)]
pub(crate) struct Review {
    enabled: bool,
    operation: Option<(u64, BrowserDriverControl)>,
    pending: Option<Pending>,
    attempts: u8,
}
impl Review {
    pub fn enable(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.cancel(None);
        }
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
    pub fn request(&self) -> Option<Request> {
        self.pending.as_ref().map(|pending| pending.request.clone())
    }
    pub fn start(
        &mut self,
        tab: u64,
        generation: u64,
        request_id: u64,
        origin: BrowserOrigin,
        certificates: &[&[u8]],
    ) -> bool {
        if !self.enabled
            || self.pending.is_some()
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
                let Ok(label) = identity_label(der) else {
                    return false;
                };
                candidates.push(Candidate {
                    index: index as i32,
                    fingerprint,
                    label,
                    not_before,
                    not_after,
                });
            }
        }
        if candidates.is_empty() {
            return false;
        }
        self.attempts += 1;
        self.pending = Some(Pending {
            request: Request {
                tab,
                generation,
                request_id,
                control_generation,
                origin,
                candidates,
                deadline: Instant::now() + TIMEOUT,
                cancel: Arc::new(AtomicBool::new(false)),
                control,
            },
            decision: Decision::Pending,
        });
        true
    }
    /// Explicit policy is applied only to this pending request, never future requests.
    pub fn pending(
        &mut self,
        policy: ClientIdentityPolicy,
    ) -> Result<Option<ClientIdentityChallenge>, PresentationError> {
        let Some(pending) = self.pending.as_mut() else {
            return Ok(None);
        };
        if !matches!(pending.decision, Decision::Pending) {
            return Ok(None);
        }
        let Some(challenge) = pending.request.challenge() else {
            return Ok(None);
        };
        if challenge.candidates.is_empty() {
            pending.decision = Decision::Decline;
            return Ok(None);
        }
        challenge.validate()?;
        if policy == ClientIdentityPolicy::AutomaticIfUnique && challenge.candidates.len() == 1 {
            pending.decision = Decision::Select(challenge.candidates[0].fingerprint_sha256.clone());
            return Ok(None);
        }
        Ok(Some(challenge))
    }
    /// Queue one native Desktop decision. Consumption still belongs to the CEF thread.
    pub fn select(
        &mut self,
        key: ClientIdentityKey,
        fingerprint: Option<String>,
    ) -> Result<(), PresentationError> {
        key.validate()?;
        let pending = self
            .pending
            .as_mut()
            .filter(|pending| {
                pending.request.key() == key
                    && !pending.request.cancelled()
                    && matches!(pending.decision, Decision::Pending)
            })
            .ok_or(PresentationError::Stale)?;
        if let Some(fingerprint) = fingerprint {
            if pending.request.selected(Some(&fingerprint)) < 0 {
                return Err(PresentationError::Invalid);
            }
            pending.decision = Decision::Select(fingerprint);
        } else {
            pending.decision = Decision::Decline;
        }
        Ok(())
    }
    /// Move a decision once. The owning CEF thread must repeat native authority checks.
    pub fn ready(&mut self) -> Option<(Request, i32)> {
        let pending = self.pending.as_ref()?;
        let fingerprint = if pending.request.cancelled() {
            None
        } else {
            match &pending.decision {
                Decision::Pending => return None,
                Decision::Select(fingerprint) => Some(fingerprint.as_str()),
                Decision::Decline => None,
            }
        };
        let index = pending.request.selected(fingerprint);
        let pending = self.pending.take()?;
        Some((pending.request, index))
    }
    pub fn finish(&mut self) {
        self.cancel(None);
        self.pending = None;
        self.operation = None;
    }
}
impl Drop for Review {
    fn drop(&mut self) {
        self.finish();
    }
}
