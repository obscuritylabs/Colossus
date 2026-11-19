//! Transcript scans have independent bounded capacity from metadata listings.
use super::{AdmissionLimitReached, MAX_TRACKED_APPLICATIONS, TokenBucket, lock};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_ACTIVE_GLOBAL: usize = 2;
const MAX_WAITING_GLOBAL: usize = 4;
const WAIT_LIMIT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(25);

pub(crate) struct TranscriptAdmission {
    state: Mutex<State>,
}

struct State {
    active: usize,
    waiting: usize,
    bucket: TokenBucket,
    applications: BTreeMap<String, Application>,
}

struct Application {
    active: bool,
    waiting: bool,
    bucket: TokenBucket,
}

impl TranscriptAdmission {
    pub(crate) fn new() -> Arc<Self> {
        Self::new_at(Instant::now())
    }

    fn new_at(now: Instant) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                active: 0,
                waiting: 0,
                bucket: TokenBucket::new(8, 16, now),
                applications: BTreeMap::new(),
            }),
        })
    }

    pub(crate) async fn acquire(
        self: &Arc<Self>,
        application_id: &str,
    ) -> Result<Permit, AdmissionLimitReached> {
        self.acquire_until(application_id, tokio::time::Instant::now() + WAIT_LIMIT)
            .await
    }

    async fn acquire_until(
        self: &Arc<Self>,
        application_id: &str,
        deadline: tokio::time::Instant,
    ) -> Result<Permit, AdmissionLimitReached> {
        if let Ok(permit) = self.try_acquire_at(application_id, Instant::now()) {
            return Ok(permit);
        }
        // Both guards release synchronously when this future is cancelled. A
        // caller cannot accumulate unbounded pending work behind a busy reader.
        let _waiting = self.enqueue(application_id)?;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(AdmissionLimitReached);
            }
            if let Ok(permit) = self.try_acquire_at(application_id, Instant::now()) {
                return Ok(permit);
            }
            tokio::time::sleep_until((tokio::time::Instant::now() + POLL_INTERVAL).min(deadline))
                .await;
        }
    }

    fn try_acquire_at(
        self: &Arc<Self>,
        application_id: &str,
        now: Instant,
    ) -> Result<Permit, AdmissionLimitReached> {
        let mut state = lock(&self.state);
        // Contenders may capture timestamps before taking this mutex in the
        // opposite order. Never replay an already credited refill interval.
        let now = now.max(state.bucket.last_refill);
        state.bucket.refill(now);
        if !state.applications.contains_key(application_id) {
            if state.applications.len() >= MAX_TRACKED_APPLICATIONS {
                state.applications.retain(|_, application| {
                    application.bucket.refill(now);
                    application.active || application.waiting || !application.bucket.is_full()
                });
            }
            if state.applications.len() >= MAX_TRACKED_APPLICATIONS {
                return Err(AdmissionLimitReached);
            }
            state.applications.insert(
                application_id.into(),
                Application {
                    active: false,
                    waiting: false,
                    bucket: TokenBucket::new(4, 8, now),
                },
            );
        }
        let available = {
            let application = state
                .applications
                .get_mut(application_id)
                .expect("transcript application is present");
            application.bucket.refill(now);
            !application.active && application.bucket.has_token()
        };
        if state.active >= MAX_ACTIVE_GLOBAL || !state.bucket.has_token() || !available {
            return Err(AdmissionLimitReached);
        }
        state.bucket.consume();
        state.active += 1;
        let application = state
            .applications
            .get_mut(application_id)
            .expect("transcript application is present");
        application.active = true;
        application.bucket.consume();
        Ok(Permit {
            admission: self.clone(),
            application_id: application_id.into(),
        })
    }

    fn enqueue(self: &Arc<Self>, application_id: &str) -> Result<Waiting, AdmissionLimitReached> {
        let mut state = lock(&self.state);
        if state.waiting >= MAX_WAITING_GLOBAL
            || state
                .applications
                .get(application_id)
                .is_none_or(|application| application.waiting)
        {
            return Err(AdmissionLimitReached);
        }
        state.waiting += 1;
        state
            .applications
            .get_mut(application_id)
            .expect("transcript application is present")
            .waiting = true;
        Ok(Waiting {
            admission: self.clone(),
            application_id: application_id.into(),
        })
    }
}

pub(crate) struct Permit {
    admission: Arc<TranscriptAdmission>,
    application_id: String,
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = lock(&self.admission.state);
        state.active = state.active.saturating_sub(1);
        if let Some(application) = state.applications.get_mut(&self.application_id) {
            application.active = false;
        }
    }
}

struct Waiting {
    admission: Arc<TranscriptAdmission>,
    application_id: String,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut state = lock(&self.admission.state);
        state.waiting = state.waiting.saturating_sub(1);
        if let Some(application) = state.applications.get_mut(&self.application_id) {
            application.waiting = false;
        }
    }
}

#[cfg(test)]
mod tests;
