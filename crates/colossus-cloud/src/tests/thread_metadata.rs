//! Metadata discovery remains usable when retained history is unavailable.
use super::*;
use crate::{CloudResult, storage::*};
use std::sync::Arc;

pub(super) struct MetadataOnlyStore {
    inner: Arc<dyn CloudStore>,
}
impl MetadataOnlyStore {
    pub(super) fn new(inner: Arc<dyn CloudStore>) -> Self {
        Self { inner }
    }
}
#[async_trait::async_trait]
impl CloudStore for MetadataOnlyStore {
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
    async fn list(&self, _query: &EntityQuery) -> CloudResult<Vec<EntityRecord>> {
        Err(CloudError::Storage)
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
async fn thread_metadata_requires_no_retained_history_list() {
    let (mut repo, caller, node) = fixture().await;
    let (thread, _) = repo
        .create_personal_thread(&caller, &node.node_id, None, request())
        .await
        .unwrap();
    repo.store = Arc::new(MetadataOnlyStore::new(repo.store.clone()));
    let reader = super::caller(caller.project_id(), &[CloudPermission::Read]);
    assert_eq!(
        repo.read_thread_metadata(&reader, &thread.thread_id)
            .await
            .unwrap(),
        thread
    );
    assert_eq!(
        repo.get_thread(&reader, &thread.thread_id)
            .await
            .unwrap_err(),
        CloudError::Storage,
        "the same store deliberately rejects history collection reads"
    );
}
