use axum::Router;
use colossus_sdk::AgentRunClient;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Semaphore;

/// Authenticated peer's fixed SDK connection and execution profile.
pub struct PeerProfile {
    pub(crate) runs: Arc<dyn AgentRunClient>,
    pub(crate) role: String,
    pub(crate) max_turns: u32,
    pub(crate) permits: Arc<Semaphore>,
}
impl PeerProfile {
    /// Bind an independently enrolled application; callers cannot choose a workspace or grant.
    pub fn new(
        runs: Arc<dyn AgentRunClient>,
        role: String,
        max_turns: u32,
    ) -> Result<Self, &'static str> {
        if !crate::wire::token(&role) || !(1..=100).contains(&max_turns) {
            return Err("invalid A2A execution profile");
        }
        Ok(Self {
            runs,
            role,
            max_turns,
            permits: Arc::new(Semaphore::new(8)),
        })
    }
}

pub(crate) struct State {
    pub(crate) public_url: String,
    pub(crate) peers: HashMap<String, Arc<PeerProfile>>,
    pub(crate) permits: Arc<Semaphore>,
}
/// Independently hosted authenticated application listener; it owns no mailbox database.
pub struct A2aListener {
    state: Arc<State>,
}
impl A2aListener {
    /// Compose explicit token SHA-256/profile bindings; raw bearer secrets are never retained.
    pub fn new(
        public_url: String,
        peers: Vec<(String, PeerProfile)>,
    ) -> Result<Self, &'static str> {
        let url = url::Url::parse(&public_url).map_err(|_| "invalid A2A public URL")?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
            || peers.is_empty()
            || peers.len() > 32
        {
            return Err("invalid A2A listener configuration");
        }
        let mut bindings = HashMap::new();
        for (digest, profile) in peers {
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                || bindings.insert(digest, Arc::new(profile)).is_some()
            {
                return Err("invalid A2A peer binding");
            }
        }
        Ok(Self {
            state: Arc::new(State {
                public_url: public_url.trim_end_matches('/').into(),
                peers: bindings,
                permits: Arc::new(Semaphore::new(32)),
            }),
        })
    }
    /// Build HTTP routes for an HTTPS host. The production binary always terminates TLS.
    pub fn router(&self) -> Router {
        crate::http::router(Arc::clone(&self.state))
    }
}
pub(crate) fn credential_digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
