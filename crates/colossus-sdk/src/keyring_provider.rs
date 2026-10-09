use crate::{CredentialProvider, SdkError, SdkResult, Secret};
use async_trait::async_trait;
use std::{fmt, sync::Arc, time::Duration};
use tokio::sync::{Mutex, Semaphore};

const MAX_KEYRING_ID_BYTES: usize = 256;
const MAX_KEYRING_WAIT: Duration = Duration::from_secs(30);

/// OS-keyring credential source for one enrolled application.
///
/// This provider has no environment, argv, or file fallback. Enrollment writes the
/// bearer directly to the same service/account entry; each request loads a fresh owned
/// secret and the SDK clears that allocation after constructing sensitive metadata.
pub struct KeyringCredentialProvider {
    service: String,
    account: String,
    access: Arc<Mutex<()>>,
    read_slots: Option<Arc<Semaphore>>,
}

impl KeyringCredentialProvider {
    /// Select one exact platform credential-store entry.
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> SdkResult<Self> {
        let service = service.into();
        let account = account.into();
        validate_keyring_id(&service)?;
        validate_keyring_id(&account)?;
        Ok(Self {
            service,
            account,
            access: Arc::new(Mutex::new(())),
            read_slots: None,
        })
    }

    /// Share a native-read concurrency limit with other credential providers.
    ///
    /// The blocking platform read owns its permit until it finishes, even when the
    /// asynchronous caller times out or is canceled. Waiting for a permit is included
    /// in the provider's bounded credential-load deadline.
    #[must_use]
    pub fn with_read_limit(mut self, slots: Arc<Semaphore>) -> Self {
        self.read_slots = Some(slots);
        self
    }

    async fn read_with(
        &self,
        wait: Duration,
        reader: impl FnOnce() -> SdkResult<Secret> + Send + 'static,
    ) -> SdkResult<Secret> {
        let access = Arc::clone(&self.access);
        let read_slots = self.read_slots.clone();
        tokio::time::timeout(wait, async move {
            let guard = access.lock_owned().await;
            let slot = match read_slots {
                Some(slots) => Some(
                    slots
                        .acquire_owned()
                        .await
                        .map_err(|_| SdkError::Authentication)?,
                ),
                None => None,
            };
            tokio::task::spawn_blocking(move || {
                // Cancellation cannot abort a running native read. Its guard must
                // stay in that blocking task, not in the canceled async caller.
                let _guard = guard;
                let _slot = slot;
                reader()
            })
            .await
            .map_err(|_| SdkError::Authentication)?
        })
        .await
        .map_err(|_| SdkError::Authentication)?
    }
}

impl fmt::Debug for KeyringCredentialProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeyringCredentialProvider")
            .field("service", &self.service)
            .field("account", &"[REDACTED]")
            .finish()
    }
}

#[async_trait]
impl CredentialProvider for KeyringCredentialProvider {
    async fn load(&self) -> SdkResult<Secret> {
        let service = self.service.clone();
        let account = self.account.clone();
        self.read_with(MAX_KEYRING_WAIT, move || {
            let bytes = keyring::Entry::new(&service, &account)
                .and_then(|entry| entry.get_secret())
                .map_err(|_| SdkError::Authentication)?;
            // Even if the caller has canceled, the completed task owns a
            // zeroizing secret; no detached plaintext Vec outlives this read.
            Secret::new(bytes)
        })
        .await
    }
}

fn validate_keyring_id(value: &str) -> SdkResult<()> {
    if value.is_empty()
        || value.len() > MAX_KEYRING_ID_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(SdkError::InvalidConfiguration(
            "keyring service and account must be bounded non-empty text",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_account_and_invalid_identifiers_fail_closed() {
        let provider =
            KeyringCredentialProvider::new("colossus.api", "private-app-account").expect("valid");
        let debug = format!("{provider:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("private-app-account"));
        assert!(KeyringCredentialProvider::new("", "account").is_err());
        assert!(KeyringCredentialProvider::new("service", " account").is_err());
    }

    #[tokio::test]
    async fn canceled_reads_keep_the_shared_limit_until_native_work_finishes() {
        use std::sync::{
            Condvar,
            atomic::{AtomicUsize, Ordering},
        };

        let slots = Arc::new(Semaphore::new(1));
        let first = Arc::new(
            KeyringCredentialProvider::new("test", "first")
                .unwrap()
                .with_read_limit(Arc::clone(&slots)),
        );
        let second = KeyringCredentialProvider::new("test", "second")
            .unwrap()
            .with_read_limit(Arc::clone(&slots));
        let calls = Arc::new(AtomicUsize::new(0));
        let release = Arc::new((std::sync::Mutex::new(false), Condvar::new()));
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let running = tokio::spawn({
            let calls = Arc::clone(&calls);
            let release = Arc::clone(&release);
            async move {
                first
                    .read_with(Duration::from_secs(5), move || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        let _ = entered_tx.send(());
                        let mut guard = release.0.lock().unwrap();
                        while !*guard {
                            guard = release.1.wait(guard).unwrap();
                        }
                        let _ = finished_tx.send(());
                        Secret::new(b"first-token".to_vec())
                    })
                    .await
            }
        });
        entered_rx.await.unwrap();
        running.abort();
        let canceled = running.await;
        let exhausted = slots.try_acquire().is_err();
        let waiting = second
            .read_with(Duration::from_millis(30), {
                let calls = Arc::clone(&calls);
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Secret::new(b"must-not-start".to_vec())
                }
            })
            .await;
        // Release before assertions so a failure cannot strand a native thread.
        *release.0.lock().unwrap() = true;
        release.1.notify_one();
        finished_rx.await.unwrap();

        assert!(canceled.unwrap_err().is_cancelled());
        assert!(exhausted, "the blocking read still owns the shared permit");
        assert!(matches!(waiting, Err(SdkError::Authentication)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let fresh = second
            .read_with(Duration::from_secs(1), || {
                Secret::new(b"fresh-token".to_vec())
            })
            .await
            .unwrap();
        assert_eq!(fresh.expose(), b"fresh-token");
        assert_eq!(slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn canceled_native_read_keeps_its_guard_and_next_completed_read_is_fresh() {
        use std::sync::{
            Condvar,
            atomic::{AtomicUsize, Ordering},
        };
        let provider = Arc::new(KeyringCredentialProvider::new("test", "test").unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let release = Arc::new((std::sync::Mutex::new(false), Condvar::new()));
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
        let first = {
            let provider = Arc::clone(&provider);
            let calls = Arc::clone(&calls);
            let release = Arc::clone(&release);
            tokio::spawn(async move {
                provider
                    .read_with(Duration::from_secs(5), move || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        let _ = entered_tx.send(());
                        let (mutex, condition) = &*release;
                        let mut guard = mutex.lock().unwrap();
                        while !*guard {
                            guard = condition.wait(guard).unwrap();
                        }
                        let _ = finished_tx.send(());
                        Secret::new(b"first-token".to_vec())
                    })
                    .await
            })
        };
        entered_rx.await.unwrap();
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        let result = {
            let calls = Arc::clone(&calls);
            provider
                .read_with(Duration::from_millis(30), move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Secret::new(b"must-not-start".to_vec())
                })
                .await
        };
        // Release before assertions so a failure cannot strand a native thread.
        *release.0.lock().unwrap() = true;
        release.1.notify_one();
        finished_rx.await.unwrap();
        assert!(matches!(result, Err(SdkError::Authentication)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let fresh = {
            let calls = Arc::clone(&calls);
            provider
                .read_with(Duration::from_secs(1), move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Secret::new(b"rotated-token".to_vec())
                })
                .await
                .unwrap()
        };
        assert_eq!(fresh.expose(), b"rotated-token");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
