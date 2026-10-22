//! A bounded, reused blocking owner for the synchronous PostgreSQL port.
//! The private postgres reactor is never constructed or dropped inside Tokio.

use super::*;
use std::sync::mpsc;

mod ports;

type Operation = Box<dyn FnOnce(&PostgresJournalCore) + Send + 'static>;

/// Namespaced PostgreSQL journal backed by one bounded reusable database worker.
/// Database mutations are never automatically retried after uncertain delivery.
pub struct PostgresEventJournal {
    requests: mpsc::SyncSender<Operation>,
    timeout: Duration,
    diagnostic: Value,
}

impl PostgresEventJournal {
    /// Open one legacy or explicitly namespaced canonical journal.
    pub fn open(
        config: PostgresJournalConfig,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
    ) -> Result<Self, StoreError> {
        Self::open_with_tls_roots(config, keys, signer, &AdditionalRootCertificates::default())
    }

    /// Open with runtime-wide additional trusted TLS roots.
    pub fn open_with_tls_roots(
        config: PostgresJournalConfig,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
        roots: &AdditionalRootCertificates,
    ) -> Result<Self, StoreError> {
        Self::open_with_tls_roots_and_startup_verification(
            config,
            keys,
            signer,
            roots,
            StartupVerificationMode::Incremental,
        )
    }

    /// Open with an explicit bounded startup verification policy.
    pub fn open_with_tls_roots_and_startup_verification(
        config: PostgresJournalConfig,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
        roots: &AdditionalRootCertificates,
        mode: StartupVerificationMode,
    ) -> Result<Self, StoreError> {
        Self::start(config, keys, signer, roots.clone(), mode, None)
    }

    /// Open under the exact shared execution lease checked transactionally on writes.
    pub fn open_fenced(
        config: PostgresJournalConfig,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
        fence: PostgresJournalFence,
    ) -> Result<Self, StoreError> {
        fence.validate()?;
        if config.runtime_namespace.is_empty()
            || keys.payload_protection() != JournalPayloadProtection::Encrypted
        {
            return Err(StoreError::Adapter(
                "hosted journal requires a protected runtime namespace".into(),
            ));
        }
        Self::start(
            config,
            keys,
            signer,
            AdditionalRootCertificates::default(),
            StartupVerificationMode::Incremental,
            Some(fence),
        )
    }

    fn start(
        config: PostgresJournalConfig,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
        roots: AdditionalRootCertificates,
        mode: StartupVerificationMode,
        fence: Option<PostgresJournalFence>,
    ) -> Result<Self, StoreError> {
        config.validate()?;
        let timeout = Duration::from_millis(config.statement_timeout_ms.saturating_add(10_000));
        let (requests, receiver) = mpsc::sync_channel::<Operation>(16);
        let (ready, startup) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("postgres-journal".into())
            .spawn(move || {
                let journal = match PostgresJournalCore::open_internal(
                    config, keys, signer, &roots, mode, fence,
                ) {
                    Ok(journal) => journal,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                if ready.send(Ok(journal.diagnostic())).is_err() {
                    return;
                }
                while let Ok(operation) = receiver.recv() {
                    operation(&journal);
                }
            })
            .map_err(|_| StoreError::Adapter("PostgreSQL journal worker is unavailable".into()))?;
        let diagnostic = startup.recv_timeout(timeout).map_err(|_| {
            StoreError::Adapter("PostgreSQL journal startup deadline elapsed".into())
        })??;
        Ok(Self {
            requests,
            timeout,
            diagnostic,
        })
    }

    fn call<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&PostgresJournalCore) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, StoreError> {
        let (reply, receive) = mpsc::sync_channel(1);
        let deadline = Instant::now() + self.timeout;
        self.requests
            .try_send(Box::new(move |journal| {
                let result = if Instant::now() >= deadline {
                    Err(StoreError::Adapter(
                        "PostgreSQL journal queued operation expired before dispatch".into(),
                    ))
                } else {
                    operation(journal)
                };
                let _ = reply.send(result);
            }))
            .map_err(|_| {
                StoreError::Adapter("PostgreSQL journal admission is unavailable".into())
            })?;
        receive.recv_timeout(self.timeout).map_err(|_| {
            StoreError::OutcomeUnknown("PostgreSQL journal operation outcome is unknown".into())
        })?
    }

    /// Bounded reason verification entered read-only recovery mode.
    pub fn recovery_reason(&self) -> Result<Option<String>, StoreError> {
        self.call(PostgresJournalCore::recovery_reason)
    }
    /// Stable startup verification evidence without payloads or credentials.
    pub fn startup_verification_report(&self) -> Result<StartupVerificationReport, StoreError> {
        self.call(PostgresJournalCore::startup_verification_report)
    }
    /// Non-secret adapter metadata captured during verified startup.
    #[must_use]
    pub fn diagnostic(&self) -> Value {
        self.diagnostic.clone()
    }

    #[cfg(test)]
    pub(super) fn connect(&self) -> Result<Client, StoreError> {
        self.call(PostgresJournalCore::connect)
    }
    #[cfg(test)]
    pub(super) fn build_tls_connector(
        tls: &PostgresTlsConfig,
    ) -> Result<MakeRustlsConnect, StoreError> {
        PostgresJournalCore::build_tls_connector(tls)
    }
}
