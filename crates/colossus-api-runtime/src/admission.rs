use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

const DEFAULT_MAX_ACTIVE_GLOBAL: usize = 32;
const DEFAULT_MAX_ACTIVE_PER_APPLICATION: usize = 8;
const DEFAULT_GLOBAL_RATE_PER_SECOND: u32 = 4;
const DEFAULT_GLOBAL_BURST: u32 = 16;
const DEFAULT_PER_APPLICATION_RATE_PER_SECOND: u32 = 1;
const DEFAULT_PER_APPLICATION_BURST: u32 = 4;
const DEFAULT_MAX_WATCHES_GLOBAL: usize = 64;
const DEFAULT_MAX_WATCHES_PER_APPLICATION: usize = 8;
const DEFAULT_MAX_LISTS_GLOBAL: usize = 4;
const DEFAULT_MAX_LISTS_PER_APPLICATION: usize = 1;
const DEFAULT_LIST_GLOBAL_RATE_PER_SECOND: u32 = 8;
const DEFAULT_LIST_GLOBAL_BURST: u32 = 8;
const DEFAULT_LIST_PER_APPLICATION_RATE_PER_SECOND: u32 = 2;
const DEFAULT_LIST_PER_APPLICATION_BURST: u32 = 2;
const MAX_TRACKED_APPLICATIONS: usize = 4_096;

/// Validated security limits for public run and watch admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunAdmissionConfig {
    max_active_global: usize,
    max_active_per_application: usize,
    global_rate_per_second: u32,
    global_burst: u32,
    per_application_rate_per_second: u32,
    per_application_burst: u32,
    max_watches_global: usize,
    max_watches_per_application: usize,
    max_lists_global: usize,
    max_lists_per_application: usize,
    list_global_rate_per_second: u32,
    list_global_burst: u32,
    list_per_application_rate_per_second: u32,
    list_per_application_burst: u32,
}

impl RunAdmissionConfig {
    /// Construct explicit run-admission limits.
    pub fn new(
        max_active_global: usize,
        max_active_per_application: usize,
        global_rate_per_second: u32,
        global_burst: u32,
        per_application_rate_per_second: u32,
        per_application_burst: u32,
    ) -> Result<Self, RunAdmissionConfigError> {
        let config = Self {
            max_active_global,
            max_active_per_application,
            global_rate_per_second,
            global_burst,
            per_application_rate_per_second,
            per_application_burst,
            max_watches_global: DEFAULT_MAX_WATCHES_GLOBAL,
            max_watches_per_application: DEFAULT_MAX_WATCHES_PER_APPLICATION,
            max_lists_global: DEFAULT_MAX_LISTS_GLOBAL,
            max_lists_per_application: DEFAULT_MAX_LISTS_PER_APPLICATION,
            list_global_rate_per_second: DEFAULT_LIST_GLOBAL_RATE_PER_SECOND,
            list_global_burst: DEFAULT_LIST_GLOBAL_BURST,
            list_per_application_rate_per_second: DEFAULT_LIST_PER_APPLICATION_RATE_PER_SECOND,
            list_per_application_burst: DEFAULT_LIST_PER_APPLICATION_BURST,
        };
        config.validate()?;
        Ok(config)
    }

    /// Override concurrent watch-stream limits.
    pub fn with_watch_limits(
        mut self,
        max_global: usize,
        max_per_application: usize,
    ) -> Result<Self, RunAdmissionConfigError> {
        self.max_watches_global = max_global;
        self.max_watches_per_application = max_per_application;
        self.validate()?;
        Ok(self)
    }

    /// Override bounded owner-index listing concurrency and token-bucket limits.
    #[allow(clippy::too_many_arguments)]
    pub fn with_list_limits(
        mut self,
        max_global: usize,
        max_per_application: usize,
        global_rate_per_second: u32,
        global_burst: u32,
        per_application_rate_per_second: u32,
        per_application_burst: u32,
    ) -> Result<Self, RunAdmissionConfigError> {
        self.max_lists_global = max_global;
        self.max_lists_per_application = max_per_application;
        self.list_global_rate_per_second = global_rate_per_second;
        self.list_global_burst = global_burst;
        self.list_per_application_rate_per_second = per_application_rate_per_second;
        self.list_per_application_burst = per_application_burst;
        self.validate()?;
        Ok(self)
    }

    /// Maximum non-terminal public runs retained by this process.
    pub fn max_active_global(&self) -> usize {
        self.max_active_global
    }

    /// Maximum non-terminal public runs retained for one application.
    pub fn max_active_per_application(&self) -> usize {
        self.max_active_per_application
    }

    /// Global fresh-run token refill rate.
    pub fn global_rate_per_second(&self) -> u32 {
        self.global_rate_per_second
    }

    /// Global fresh-run token-bucket burst.
    pub fn global_burst(&self) -> u32 {
        self.global_burst
    }

    /// Per-application fresh-run token refill rate.
    pub fn per_application_rate_per_second(&self) -> u32 {
        self.per_application_rate_per_second
    }

    /// Per-application fresh-run token-bucket burst.
    pub fn per_application_burst(&self) -> u32 {
        self.per_application_burst
    }

    /// Maximum concurrent public watch streams.
    pub fn max_watches_global(&self) -> usize {
        self.max_watches_global
    }

    /// Maximum concurrent public watch streams for one application.
    pub fn max_watches_per_application(&self) -> usize {
        self.max_watches_per_application
    }

    /// Maximum concurrent public owner-index listings.
    pub fn max_lists_global(&self) -> usize {
        self.max_lists_global
    }

    /// Maximum concurrent public owner-index listings for one application.
    pub fn max_lists_per_application(&self) -> usize {
        self.max_lists_per_application
    }

    /// Global owner-index listing token refill rate.
    pub fn list_global_rate_per_second(&self) -> u32 {
        self.list_global_rate_per_second
    }

    /// Global owner-index listing burst.
    pub fn list_global_burst(&self) -> u32 {
        self.list_global_burst
    }

    /// Per-application owner-index listing token refill rate.
    pub fn list_per_application_rate_per_second(&self) -> u32 {
        self.list_per_application_rate_per_second
    }

    /// Per-application owner-index listing burst.
    pub fn list_per_application_burst(&self) -> u32 {
        self.list_per_application_burst
    }

    fn validate(&self) -> Result<(), RunAdmissionConfigError> {
        if self.max_active_global == 0
            || self.max_active_per_application == 0
            || self.max_active_per_application > self.max_active_global
            || self.global_rate_per_second == 0
            || self.global_burst == 0
            || self.per_application_rate_per_second == 0
            || self.per_application_burst == 0
            || self.max_watches_global == 0
            || self.max_watches_per_application == 0
            || self.max_watches_per_application > self.max_watches_global
            || self.max_lists_global == 0
            || self.max_lists_per_application == 0
            || self.max_lists_per_application > self.max_lists_global
            || self.list_global_rate_per_second == 0
            || self.list_global_burst == 0
            || self.list_per_application_rate_per_second == 0
            || self.list_per_application_burst == 0
        {
            return Err(RunAdmissionConfigError);
        }
        Ok(())
    }
}

impl Default for RunAdmissionConfig {
    fn default() -> Self {
        Self {
            max_active_global: DEFAULT_MAX_ACTIVE_GLOBAL,
            max_active_per_application: DEFAULT_MAX_ACTIVE_PER_APPLICATION,
            global_rate_per_second: DEFAULT_GLOBAL_RATE_PER_SECOND,
            global_burst: DEFAULT_GLOBAL_BURST,
            per_application_rate_per_second: DEFAULT_PER_APPLICATION_RATE_PER_SECOND,
            per_application_burst: DEFAULT_PER_APPLICATION_BURST,
            max_watches_global: DEFAULT_MAX_WATCHES_GLOBAL,
            max_watches_per_application: DEFAULT_MAX_WATCHES_PER_APPLICATION,
            max_lists_global: DEFAULT_MAX_LISTS_GLOBAL,
            max_lists_per_application: DEFAULT_MAX_LISTS_PER_APPLICATION,
            list_global_rate_per_second: DEFAULT_LIST_GLOBAL_RATE_PER_SECOND,
            list_global_burst: DEFAULT_LIST_GLOBAL_BURST,
            list_per_application_rate_per_second: DEFAULT_LIST_PER_APPLICATION_RATE_PER_SECOND,
            list_per_application_burst: DEFAULT_LIST_PER_APPLICATION_BURST,
        }
    }
}

/// Invalid public admission configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunAdmissionConfigError;

impl fmt::Display for RunAdmissionConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "admission limits must be non-zero and per-application limits must not exceed global limits",
        )
    }
}

impl Error for RunAdmissionConfigError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RunReservation {
    generation: u64,
}

impl RunReservation {
    pub(crate) fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReserveRun {
    Reserved(RunReservation),
    AlreadyReserved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AdmissionLimitReached;

struct TokenBucket {
    tokens: f64,
    burst: u32,
    rate_per_second: u32,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(burst: u32, rate_per_second: u32, now: Instant) -> Self {
        Self {
            tokens: f64::from(burst),
            burst,
            rate_per_second,
            last_refill: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now
            .checked_duration_since(self.last_refill)
            .unwrap_or_default()
            .as_secs_f64();
        self.tokens =
            (self.tokens + elapsed * f64::from(self.rate_per_second)).min(f64::from(self.burst));
        self.last_refill = now;
    }

    fn has_token(&self) -> bool {
        self.tokens >= 1.0
    }

    fn consume(&mut self) {
        self.tokens -= 1.0;
    }

    fn is_full(&self) -> bool {
        self.tokens >= f64::from(self.burst)
    }
}

struct ApplicationAdmission {
    active: usize,
    bucket: TokenBucket,
}

struct ActiveReservation {
    application_id: String,
    generation: u64,
}

pub(crate) struct RunAdmissionState {
    config: RunAdmissionConfig,
    global_bucket: TokenBucket,
    applications: BTreeMap<String, ApplicationAdmission>,
    reservations: BTreeMap<String, ActiveReservation>,
    next_generation: u64,
}

impl RunAdmissionState {
    pub(crate) fn new(config: RunAdmissionConfig, now: Instant) -> Self {
        let global_bucket =
            TokenBucket::new(config.global_burst, config.global_rate_per_second, now);
        Self {
            config,
            global_bucket,
            applications: BTreeMap::new(),
            reservations: BTreeMap::new(),
            next_generation: 1,
        }
    }

    pub(crate) fn check_new(
        &mut self,
        application_id: &str,
        now: Instant,
    ) -> Result<(), AdmissionLimitReached> {
        self.check_capacity(application_id, now)?;
        self.global_bucket.refill(now);
        let application = self
            .applications
            .get_mut(application_id)
            .expect("application admission was inserted");
        application.bucket.refill(now);
        if !self.global_bucket.has_token() || !application.bucket.has_token() {
            return Err(AdmissionLimitReached);
        }
        Ok(())
    }

    fn check_capacity(
        &mut self,
        application_id: &str,
        now: Instant,
    ) -> Result<(), AdmissionLimitReached> {
        if self.reservations.len() >= self.config.max_active_global {
            return Err(AdmissionLimitReached);
        }
        self.ensure_application(application_id, now)?;
        let application = self
            .applications
            .get_mut(application_id)
            .expect("application admission was inserted");
        if application.active >= self.config.max_active_per_application {
            return Err(AdmissionLimitReached);
        }
        Ok(())
    }

    pub(crate) fn reserve_checked(
        &mut self,
        application_id: &str,
        run_id: &str,
        now: Instant,
    ) -> Result<ReserveRun, AdmissionLimitReached> {
        if self.reservations.contains_key(run_id) {
            return Ok(ReserveRun::AlreadyReserved);
        }
        self.check_new(application_id, now)?;
        self.global_bucket.consume();
        let application = self
            .applications
            .get_mut(application_id)
            .expect("checked application admission is present");
        application.bucket.consume();
        application.active += 1;
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        self.reservations.insert(
            run_id.into(),
            ActiveReservation {
                application_id: application_id.into(),
                generation,
            },
        );
        Ok(ReserveRun::Reserved(RunReservation { generation }))
    }

    pub(crate) fn reserve_existing(
        &mut self,
        application_id: &str,
        run_id: &str,
        now: Instant,
    ) -> Result<ReserveRun, AdmissionLimitReached> {
        if self.reservations.contains_key(run_id) {
            return Ok(ReserveRun::AlreadyReserved);
        }
        self.check_capacity(application_id, now)?;
        let application = self
            .applications
            .get_mut(application_id)
            .expect("checked application admission is present");
        application.active += 1;
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        self.reservations.insert(
            run_id.into(),
            ActiveReservation {
                application_id: application_id.into(),
                generation,
            },
        );
        Ok(ReserveRun::Reserved(RunReservation { generation }))
    }

    pub(crate) fn release(&mut self, run_id: &str, generation: u64, now: Instant) -> bool {
        let Some(reservation) = self.reservations.get(run_id) else {
            return false;
        };
        if reservation.generation != generation {
            return false;
        }
        let reservation = self
            .reservations
            .remove(run_id)
            .expect("matching reservation is present");
        let remove_application =
            if let Some(application) = self.applications.get_mut(&reservation.application_id) {
                application.active = application.active.saturating_sub(1);
                application.bucket.refill(now);
                application.active == 0 && application.bucket.is_full()
            } else {
                false
            };
        if remove_application {
            self.applications.remove(&reservation.application_id);
        }
        true
    }

    fn ensure_application(
        &mut self,
        application_id: &str,
        now: Instant,
    ) -> Result<(), AdmissionLimitReached> {
        if self.applications.contains_key(application_id) {
            return Ok(());
        }
        if self.applications.len() >= MAX_TRACKED_APPLICATIONS {
            self.applications.retain(|_, application| {
                application.bucket.refill(now);
                application.active != 0 || !application.bucket.is_full()
            });
        }
        if self.applications.len() >= MAX_TRACKED_APPLICATIONS {
            return Err(AdmissionLimitReached);
        }
        self.applications.insert(
            application_id.into(),
            ApplicationAdmission {
                active: 0,
                bucket: TokenBucket::new(
                    self.config.per_application_burst,
                    self.config.per_application_rate_per_second,
                    now,
                ),
            },
        );
        Ok(())
    }
}

pub(crate) struct WatchAdmission {
    max_global: usize,
    max_per_application: usize,
    state: Mutex<WatchState>,
}

#[derive(Default)]
struct WatchState {
    global: usize,
    applications: BTreeMap<String, usize>,
}

impl WatchAdmission {
    pub(crate) fn new(config: &RunAdmissionConfig) -> Arc<Self> {
        Arc::new(Self {
            max_global: config.max_watches_global,
            max_per_application: config.max_watches_per_application,
            state: Mutex::new(WatchState::default()),
        })
    }

    pub(crate) fn acquire(
        self: &Arc<Self>,
        application_id: &str,
    ) -> Result<WatchAdmissionPermit, AdmissionLimitReached> {
        let mut state = lock(&self.state);
        let application_count = state
            .applications
            .get(application_id)
            .copied()
            .unwrap_or_default();
        if state.global >= self.max_global || application_count >= self.max_per_application {
            return Err(AdmissionLimitReached);
        }
        state.global += 1;
        *state.applications.entry(application_id.into()).or_default() += 1;
        Ok(WatchAdmissionPermit {
            admission: Arc::clone(self),
            application_id: application_id.into(),
        })
    }

    fn release(&self, application_id: &str) {
        let mut state = lock(&self.state);
        state.global = state.global.saturating_sub(1);
        if let Some(count) = state.applications.get_mut(application_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.applications.remove(application_id);
            }
        }
    }
}

pub(crate) struct WatchAdmissionPermit {
    admission: Arc<WatchAdmission>,
    application_id: String,
}

pub(crate) struct ListAdmission {
    config: RunAdmissionConfig,
    state: Mutex<ListState>,
}

struct ListState {
    global_active: usize,
    global_bucket: TokenBucket,
    applications: BTreeMap<String, ApplicationAdmission>,
}

impl ListAdmission {
    pub(crate) fn new(config: &RunAdmissionConfig) -> Arc<Self> {
        Arc::new(Self {
            config: config.clone(),
            state: Mutex::new(ListState {
                global_active: 0,
                global_bucket: TokenBucket::new(
                    config.list_global_burst,
                    config.list_global_rate_per_second,
                    Instant::now(),
                ),
                applications: BTreeMap::new(),
            }),
        })
    }

    pub(crate) fn acquire(
        self: &Arc<Self>,
        application_id: &str,
    ) -> Result<ListAdmissionPermit, AdmissionLimitReached> {
        let now = Instant::now();
        let mut state = lock(&self.state);
        state.global_bucket.refill(now);
        if state.global_active >= self.config.max_lists_global {
            return Err(AdmissionLimitReached);
        }
        if !state.applications.contains_key(application_id) {
            if state.applications.len() >= MAX_TRACKED_APPLICATIONS {
                state.applications.retain(|_, application| {
                    application.bucket.refill(now);
                    application.active != 0 || !application.bucket.is_full()
                });
            }
            if state.applications.len() >= MAX_TRACKED_APPLICATIONS {
                return Err(AdmissionLimitReached);
            }
            state.applications.insert(
                application_id.into(),
                ApplicationAdmission {
                    active: 0,
                    bucket: TokenBucket::new(
                        self.config.list_per_application_burst,
                        self.config.list_per_application_rate_per_second,
                        now,
                    ),
                },
            );
        }
        let application_available = {
            let application = state
                .applications
                .get_mut(application_id)
                .expect("list admission application is present");
            application.bucket.refill(now);
            application.active < self.config.max_lists_per_application
                && application.bucket.has_token()
        };
        if !state.global_bucket.has_token() || !application_available {
            return Err(AdmissionLimitReached);
        }
        state.global_bucket.consume();
        state.global_active += 1;
        let application = state
            .applications
            .get_mut(application_id)
            .expect("list admission application is present");
        application.bucket.consume();
        application.active += 1;
        Ok(ListAdmissionPermit {
            admission: Arc::clone(self),
            application_id: application_id.into(),
        })
    }

    fn release(&self, application_id: &str) {
        let mut state = lock(&self.state);
        state.global_active = state.global_active.saturating_sub(1);
        if let Some(application) = state.applications.get_mut(application_id) {
            application.active = application.active.saturating_sub(1);
        }
    }
}

pub(crate) struct ListAdmissionPermit {
    admission: Arc<ListAdmission>,
    application_id: String,
}

impl Drop for ListAdmissionPermit {
    fn drop(&mut self) {
        self.admission.release(&self.application_id);
    }
}

impl Drop for WatchAdmissionPermit {
    fn drop(&mut self) {
        self.admission.release(&self.application_id);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn defaults_match_the_public_security_envelope() {
        let config = RunAdmissionConfig::default();
        assert_eq!(config.max_active_global(), 32);
        assert_eq!(config.max_active_per_application(), 8);
        assert_eq!(config.global_rate_per_second(), 4);
        assert_eq!(config.global_burst(), 16);
        assert_eq!(config.per_application_rate_per_second(), 1);
        assert_eq!(config.per_application_burst(), 4);
        assert_eq!(config.max_watches_global(), 64);
        assert_eq!(config.max_watches_per_application(), 8);
        assert_eq!(config.max_lists_global(), 4);
        assert_eq!(config.max_lists_per_application(), 1);
        assert_eq!(config.list_global_rate_per_second(), 8);
        assert_eq!(config.list_global_burst(), 8);
        assert_eq!(config.list_per_application_rate_per_second(), 2);
        assert_eq!(config.list_per_application_burst(), 2);
    }

    #[test]
    fn per_application_and_global_active_limits_are_enforced() {
        let now = Instant::now();
        let config = RunAdmissionConfig::new(3, 2, 100, 100, 100, 100).expect("config");
        let mut state = RunAdmissionState::new(config, now);
        assert!(matches!(
            state.reserve_checked("app:a", "run-1", now),
            Ok(ReserveRun::Reserved(_))
        ));
        assert!(matches!(
            state.reserve_checked("app:a", "run-2", now),
            Ok(ReserveRun::Reserved(_))
        ));
        assert_eq!(
            state.reserve_checked("app:a", "run-3", now),
            Err(AdmissionLimitReached)
        );
        assert!(matches!(
            state.reserve_checked("app:b", "run-3", now),
            Ok(ReserveRun::Reserved(_))
        ));
        assert_eq!(
            state.reserve_checked("app:c", "run-4", now),
            Err(AdmissionLimitReached)
        );
    }

    #[test]
    fn global_and_per_application_token_buckets_refill_monotonically() {
        let start = Instant::now();
        let config = RunAdmissionConfig::new(32, 32, 2, 2, 1, 1).expect("config");
        let mut state = RunAdmissionState::new(config, start);
        let first = match state
            .reserve_checked("app:a", "run-1", start)
            .expect("first")
        {
            ReserveRun::Reserved(reservation) => reservation,
            ReserveRun::AlreadyReserved => panic!("new run"),
        };
        assert!(state.release("run-1", first.generation(), start));
        assert_eq!(
            state.reserve_checked("app:a", "run-2", start),
            Err(AdmissionLimitReached)
        );
        let one_second = start + Duration::from_secs(1);
        assert!(matches!(
            state.reserve_checked("app:a", "run-2", one_second),
            Ok(ReserveRun::Reserved(_))
        ));

        let other = match state
            .reserve_checked("app:b", "run-3", one_second)
            .expect("remaining global burst")
        {
            ReserveRun::Reserved(reservation) => reservation,
            ReserveRun::AlreadyReserved => panic!("new run"),
        };
        assert!(state.release("run-3", other.generation(), one_second));
        assert_eq!(
            state.reserve_checked("app:c", "run-4", one_second),
            Err(AdmissionLimitReached)
        );
        assert!(matches!(
            state.reserve_checked("app:c", "run-4", one_second + Duration::from_millis(500)),
            Ok(ReserveRun::Reserved(_))
        ));
    }

    #[test]
    fn reservation_identity_prevents_loser_cleanup_and_drop_does_not_release() {
        let now = Instant::now();
        let config = RunAdmissionConfig::new(1, 1, 10, 10, 10, 10).expect("config");
        let mut state = RunAdmissionState::new(config, now);
        let winner = match state
            .reserve_checked("app:a", "run-1", now)
            .expect("winner")
        {
            ReserveRun::Reserved(reservation) => reservation,
            ReserveRun::AlreadyReserved => panic!("new run"),
        };
        assert_eq!(
            state
                .reserve_checked("app:a", "run-1", now)
                .expect("same run"),
            ReserveRun::AlreadyReserved
        );
        assert!(!state.release("run-1", winner.generation() + 1, now));
        assert_eq!(
            state.reserve_checked("app:b", "run-2", now),
            Err(AdmissionLimitReached)
        );
        assert!(state.release("run-1", winner.generation(), now));
        assert!(matches!(
            state.reserve_checked("app:b", "run-2", now),
            Ok(ReserveRun::Reserved(_))
        ));
    }

    #[test]
    fn watch_permits_enforce_both_limits_and_release_on_drop() {
        let config = RunAdmissionConfig::default()
            .with_watch_limits(2, 1)
            .expect("watch limits");
        let admission = WatchAdmission::new(&config);
        let first = admission.acquire("app:a").expect("first");
        assert!(admission.acquire("app:a").is_err());
        let second = admission.acquire("app:b").expect("second");
        assert!(admission.acquire("app:c").is_err());
        drop(first);
        let replacement = admission.acquire("app:a").expect("released");
        drop((second, replacement));
        assert!(admission.acquire("app:c").is_ok());
    }

    #[test]
    fn list_permits_bound_concurrent_scans_and_release_on_drop() {
        let config = RunAdmissionConfig::default()
            .with_list_limits(2, 1, 100, 100, 100, 100)
            .expect("list limits");
        let admission = ListAdmission::new(&config);
        let first = admission.acquire("app:a").expect("first");
        assert!(admission.acquire("app:a").is_err());
        let second = admission.acquire("app:b").expect("second");
        assert!(admission.acquire("app:c").is_err());
        drop(first);
        assert!(admission.acquire("app:a").is_ok());
        drop(second);
    }
}
