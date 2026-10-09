//! Fault-injection proof that bootstrap does no work after a marker verification failure.
use super::*;
use colossus_cloud::storage::*;
use std::sync::atomic::{AtomicUsize, Ordering};
pub(super) struct MarkerFailure {
    inner: Arc<dyn CloudStore>,
    key: &'static str,
    writes: AtomicUsize,
    rotation: std::sync::Mutex<Option<(EntityKey, CloudTransaction)>>,
}
impl MarkerFailure {
    pub(super) fn rotating(
        inner: Arc<dyn CloudStore>,
        key: EntityKey,
        transaction: CloudTransaction,
    ) -> Self {
        Self {
            inner,
            key: "",
            writes: AtomicUsize::new(0),
            rotation: std::sync::Mutex::new(Some((key, transaction))),
        }
    }
}
#[async_trait::async_trait]
impl CloudStore for MarkerFailure {
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.inner.subscribe()
    }
    async fn readiness(&self) -> CloudResult<()> {
        self.inner.readiness().await
    }
    async fn read(&self, key: &EntityKey) -> CloudResult<EntityRecord> {
        if key.kind == EntityKind::Setting && key.id == self.key {
            Err(CloudError::Storage)
        } else {
            let record = self.inner.read(key).await?;
            let rotation = {
                let mut next = self.rotation.lock().unwrap();
                if next.as_ref().is_some_and(|(trigger, _)| trigger == key) {
                    next.take()
                } else {
                    None
                }
            };
            if let Some((_, transaction)) = rotation {
                self.inner
                    .commit(transaction)
                    .await
                    .map_err(CloudError::from)?;
            }
            Ok(record)
        }
    }
    async fn delete_entity(&self, key: &EntityKey, revision: u64) -> CloudResult<()> {
        self.inner.delete_entity(key, revision).await
    }
    async fn list(&self, q: &EntityQuery) -> CloudResult<Vec<EntityRecord>> {
        self.inner.list(q).await
    }
    async fn list_projects(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<EntityRecord>> {
        self.inner.list_projects(after, limit).await
    }
    async fn user_identities(&self, user: &str) -> CloudResult<Vec<EntityRecord>> {
        self.inner.user_identities(user).await
    }
    async fn user_accounts(&self, users: &[String]) -> CloudResult<Vec<EntityRecord>> {
        self.inner.user_accounts(users).await
    }
    async fn memberships(&self, subject: &str) -> CloudResult<Vec<EntityRecord>> {
        self.inner.memberships(subject).await
    }
    async fn thread_incomplete(&self, project: &str, thread: &str) -> CloudResult<bool> {
        self.inner.thread_incomplete(project, thread).await
    }
    async fn commit(
        &self,
        t: CloudTransaction,
    ) -> std::result::Result<(), colossus_ports::StoreError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.commit(t).await
    }
    async fn events(
        &self,
        p: &str,
        s: &str,
        after: u64,
        limit: usize,
    ) -> CloudResult<Vec<ReleasedEvent>> {
        self.inner.events(p, s, after, limit).await
    }
    async fn cursor(&self, p: &str, s: &str, c: &str) -> CloudResult<u64> {
        self.inner.cursor(p, s, c).await
    }
    async fn put_session(&self, s: AuthSession) -> CloudResult<()> {
        self.inner.put_session(s).await
    }
    async fn read_session(&self, h: &str, now: u64) -> CloudResult<AuthSession> {
        self.inner.read_session(h, now).await
    }
    async fn delete_session(&self, h: &str) -> CloudResult<()> {
        self.inner.delete_session(h).await
    }
    async fn claim_lease(
        &self,
        p: &str,
        n: &str,
        o: &str,
        now: u64,
        ttl: u64,
    ) -> CloudResult<ConnectionLease> {
        self.inner.claim_lease(p, n, o, now, ttl).await
    }
    async fn read_lease(&self, p: &str, n: &str, now: u64) -> CloudResult<ConnectionLease> {
        self.inner.read_lease(p, n, now).await
    }
    async fn renew_lease(
        &self,
        l: &ConnectionLease,
        now: u64,
        ttl: u64,
    ) -> CloudResult<ConnectionLease> {
        self.inner.renew_lease(l, now, ttl).await
    }
    async fn release_lease(&self, l: &ConnectionLease) -> CloudResult<()> {
        self.inner.release_lease(l).await
    }
    async fn verify_lease(&self, l: &ConnectionLease, now: u64) -> CloudResult<()> {
        self.inner.verify_lease(l, now).await
    }
}
#[tokio::test]
async fn bootstrap_propagates_marker_integrity_failure_without_initializing_accounts() {
    let (auth, _, server) = fixture().await;
    for marker in ["identity-bootstrap-v3", "administrator-bootstrap-v3"] {
        let store = Arc::new(MarkerFailure {
            inner: Arc::new(MemoryCloudStore::default()),
            key: marker,
            writes: AtomicUsize::new(0),
            rotation: std::sync::Mutex::new(None),
        });
        let mut config = auth.config.clone();
        if marker == "administrator-bootstrap-v3" {
            config.bootstrap_admin = Some(crate::config::BootstrapAdmin {
                display_name: "Fixture administrator".into(),
                email: None,
                oidc_subject: Some("alice".into()),
                username: None,
                password_variable: None,
            });
        }
        let port: Arc<dyn CloudStore> = store.clone();
        assert_eq!(
            super::super::bootstrap::seed_accounts(&port, &config)
                .await
                .unwrap_err(),
            CloudError::Storage
        );
        assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    }
    server.abort();
}
