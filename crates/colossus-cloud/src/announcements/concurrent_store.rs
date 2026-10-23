//! Deterministic concurrent writer fixture: both edits read before either commits.
use crate::{CloudResult, storage::*};
use std::sync::Arc;

pub(super) struct ConcurrentUpdates {
    inner: Arc<dyn CloudStore>,
    gate: tokio::sync::Barrier,
}
impl ConcurrentUpdates {
    pub(super) fn new(inner: Arc<dyn CloudStore>) -> Self {
        Self {
            inner,
            gate: tokio::sync::Barrier::new(2),
        }
    }
}
#[async_trait::async_trait]
impl CloudStore for ConcurrentUpdates {
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.inner.subscribe()
    }
    async fn readiness(&self) -> CloudResult<()> {
        self.inner.readiness().await
    }
    async fn read(&self, key: &EntityKey) -> CloudResult<EntityRecord> {
        self.inner.read(key).await
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
    async fn memberships_page(
        &self,
        subject: &str,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<EntityRecord>> {
        self.inner.memberships_page(subject, after, limit).await
    }
    async fn thread_incomplete(&self, project: &str, thread: &str) -> CloudResult<bool> {
        self.inner.thread_incomplete(project, thread).await
    }
    async fn commit(
        &self,
        t: CloudTransaction,
    ) -> std::result::Result<(), colossus_ports::StoreError> {
        if t.entities.iter().any(|mutation| {
            mutation.key.kind == EntityKind::Announcement && mutation.expected_revision == 1
        }) {
            self.gate.wait().await;
        }
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
