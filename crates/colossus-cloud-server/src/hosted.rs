//! Tenant-bound Hosted runtime lifecycle, dispatch and managed artifacts.
use crate::{
    config::{ChatConfig, ChatProjectConfig},
    http::{Result, now},
    server::State,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State as Extract},
    http::HeaderMap,
    routing::{get, post},
};
use colossus_cloud::{CloudCaller, CloudError, CloudPermission};
use colossus_cloud_chat::ChatRuntime;
use colossus_home::ConfinedRoot;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::sync::{Mutex, RwLock, watch};

pub(crate) mod catalog;
mod context;
mod credentials;
pub(crate) mod files;
mod host;
mod inventory;
pub(crate) mod management;
pub(crate) mod oauth;
mod oauth_shared;
mod pool;
pub(crate) mod shared;
mod title_recovery;
mod worker;

pub(crate) fn router() -> Router<Arc<State>> {
    files::router()
        .route("/api/projects/{project}/hosted", get(context::discovery))
        .route(
            "/api/projects/{project}/hosted/tasks",
            get(inventory::tasks),
        )
        .route(
            "/api/projects/{project}/hosted/runtime/load",
            post(inventory::load),
        )
        .route(
            "/api/projects/{project}/hosted/runtime/release",
            post(inventory::release),
        )
}

pub(crate) struct HostedExecution {
    pub(crate) deployment: ChatConfig,
    pub(crate) root: ConfinedRoot,
    pub(crate) repo: colossus_cloud::CloudRepository,
    pub(crate) projects: RwLock<BTreeMap<String, ChatProjectConfig>>,
    pub(crate) live: Mutex<BTreeMap<String, Arc<LiveRuntime>>>,
    pub(crate) lifecycle: Mutex<()>,
    pub(super) shared: Option<Arc<shared::SharedChat>>,
    closing: AtomicBool,
    wake: tokio::sync::Notify,
    pub(crate) supervisor: Mutex<Option<tokio::task::JoinHandle<()>>>,
    oauth: Mutex<BTreeMap<String, oauth::PendingOAuth>>,
    vaults:
        std::sync::Mutex<BTreeMap<String, std::sync::Weak<dyn colossus_ports::CredentialVault>>>,
}

pub(crate) struct LiveRuntime {
    pub(crate) project_id: String,
    pub(crate) user_id: String,
    pub(crate) runtime: Arc<ChatRuntime>,
    pub(crate) node_id: String,
    shutdown: watch::Sender<bool>,
    worker: Mutex<Option<tokio::task::JoinHandle<std::result::Result<(), &'static str>>>>,
    lease: Option<colossus_cloud_postgres::ChatLease>,
    management: Mutex<Option<tokio::task::JoinHandle<()>>>,
    last_used: std::sync::Mutex<std::time::Instant>,
    operations: std::sync::atomic::AtomicUsize,
}

pub(crate) fn service(state: &State) -> std::result::Result<&Arc<HostedExecution>, CloudError> {
    state.hosted.as_ref().ok_or(CloudError::NotFound)
}
impl LiveRuntime {
    fn touch(&self) {
        if let Ok(mut last) = self.last_used.lock() {
            *last = std::time::Instant::now();
        }
    }
    fn pin(&self) -> RuntimeOperation<'_> {
        self.operations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.touch();
        RuntimeOperation(self)
    }
}
struct RuntimeOperation<'a>(&'a LiveRuntime);
impl Drop for RuntimeOperation<'_> {
    fn drop(&mut self) {
        self.0.touch();
        self.0
            .operations
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
