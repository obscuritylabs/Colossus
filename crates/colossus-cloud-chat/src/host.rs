use crate::{ChatCredentialResolver, ChatError, connections::valid_id};
use colossus_api::{
    ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, RequestId, scopes,
};
use colossus_api_runtime::{PublicInteractionRouter, RuntimeAgentRunApi};
use colossus_credentials::{EnvironmentKeyStore, PlatformCredentialVault};
use colossus_home::ConfinedRoot;
use colossus_policy::DenyApproval;
use colossus_runtime::{
    CodexAuthStore, KeyConfig, Runtime, RuntimeConfig, RuntimeOpenOptions, SemanticMemoryConfig,
    StorageAdapter, StorageLocation,
};
use colossus_sdk::{AgentRunClient, ContextBoundAgentRunClient};
use sha2::{Digest as _, Sha256};
use std::{path::Path, sync::Arc, time::Duration};

/// Trusted host inputs; this type is deliberately not deserializable from an HTTP request.
pub struct ChatRuntimeOptions {
    /// Verified project identity from the Control Plane caller.
    pub project_id: String,
    /// Verified stable user identity from the Control Plane caller.
    pub user_id: String,
    /// Owner-private service root. Tenant paths are derived, never supplied by browsers.
    pub root: ConfinedRoot,
    /// Administrator-reviewed runtime connection/configuration snapshot.
    pub config: RuntimeConfig,
    /// Explicit headless wrapping key reference for private per-user OAuth vaults.
    pub wrapping_key_variable: String,
    /// Exact late-bound credentials selected under current connection policy.
    pub credentials: Arc<ChatCredentialResolver>,
    /// Optional trusted existing Codex connection; no browser path or ambient account selection.
    pub codex_auth: Option<CodexAuthStore>,
    /// Shared SQL journal, live distributed owner and shared encrypted OAuth custody.
    /// Absent only for the retained single-host development fixture.
    pub shared: Option<ChatSharedRuntime>,
}

/// Trusted hosted persistence; neither browsers nor agents can supply these objects.
pub struct ChatSharedRuntime {
    /// Fenced SQL canonical state with independently protected journal keys/anchors.
    pub durability: colossus_runtime::RuntimeSharedStorage,
    /// Tenant-bound encrypted custody shared across API replicas.
    pub oauth_vault: Arc<dyn colossus_ports::CredentialVault>,
}

/// One user's hosted runtime, retaining lifecycle ownership independently from browser connections.
pub struct ChatRuntime {
    runtime: Arc<Runtime>,
    api: Arc<RuntimeAgentRunApi>,
    runs: Arc<dyn AgentRunClient>,
    identity: String,
}
impl ChatRuntime {
    /// Derive the stable private execution identity from authenticated tenant/account IDs.
    /// This identifies custody and enrollment state without opening either one.
    pub fn identity_for(project_id: &str, user_id: &str) -> Result<String, ChatError> {
        if !valid_id(project_id) || !valid_id(user_id) {
            return Err(ChatError::InvalidConfiguration);
        }
        let encoded = serde_json::to_vec(&("cloud-chat-v1", project_id, user_id))
            .map_err(|_| ChatError::InvalidConfiguration)?;
        Ok(hex::encode(Sha256::digest(encoded)))
    }

    /// Compose an isolated controlled-tools runtime using the existing durable run API.
    /// Persistent hosted state requires explicit headless journal keys; no OS-store fallback exists.
    pub fn open(mut options: ChatRuntimeOptions) -> Result<Self, ChatError> {
        if !options
            .credentials
            .matches_tenant(&options.project_id, &options.user_id)
        {
            return Err(ChatError::InvalidConfiguration);
        }
        let identity = Self::identity_for(&options.project_id, &options.user_id)?;
        if options
            .shared
            .as_ref()
            .is_some_and(|shared| shared.durability.identity() != identity)
        {
            return Err(ChatError::InvalidConfiguration);
        }
        // Remote semantic indexes have their own credential adapters and collection
        // ownership. Do not reuse an operator's shared projection for personal history.
        if !matches!(
            options.config.memory.semantic,
            SemanticMemoryConfig::Disabled
        ) {
            return Err(ChatError::InvalidConfiguration);
        }
        let home_path = options
            .root
            .prepare_directory(Path::new(&format!("users/{identity}")))
            .map_err(|_| ChatError::StateUnavailable)?;
        let home = ConfinedRoot::bind(home_path).map_err(|_| ChatError::StateUnavailable)?;
        let workspace = home
            .prepare_directory(Path::new("workspace"))
            .map_err(|_| ChatError::StateUnavailable)?;
        let state = home
            .prepare_directory(Path::new("state"))
            .map_err(|_| ChatError::StateUnavailable)?;
        if options.shared.is_none() {
            options.config.storage.adapter = StorageAdapter::Redb;
            options.config.storage.postgres = None;
        } else if options.config.storage.adapter != StorageAdapter::Postgres
            || options
                .config
                .storage
                .postgres
                .as_ref()
                .is_none_or(|postgres| postgres.runtime_namespace != identity)
        {
            return Err(ChatError::InvalidConfiguration);
        }
        options.config.storage.location = StorageLocation::Workspace;
        options.config.storage.path = state.join("runtime-state");
        options.config.memory.index_path = Some(state.join("memory-index"));
        match &mut options.config.storage.keys {
            KeyConfig::Environment {
                anchor_path,
                journal_key_id,
                ..
            } => {
                *anchor_path = state.join("journal.anchor");
                *journal_key_id = format!("chat-{identity}");
            }
            _ => return Err(ChatError::InvalidConfiguration),
        }
        let workflows = home
            .prepare_directory(Path::new("workflows"))
            .map_err(|_| ChatError::StateUnavailable)?;
        options.config.workflows.repository = workflows.clone();
        options.config.workflows.user = workflows;
        let context = RuntimeOpenOptions::for_controlled_chat(&workspace)
            .map_err(|_| ChatError::InvalidConfiguration)?
            .with_colossus_home(home.path())
            .map_err(|_| ChatError::StateUnavailable)?;
        let context = if let Some(shared) = options.shared {
            context
                .with_oauth_credential_vault(shared.oauth_vault)
                .with_shared_storage(shared.durability)
                .map_err(|_| ChatError::StateUnavailable)?
        } else {
            options.config.storage.path = state.join("runtime.redb");
            let oauth_path = home
                .prepare_directory(Path::new("oauth"))
                .map_err(|_| ChatError::StateUnavailable)?;
            let oauth = ConfinedRoot::bind(oauth_path).map_err(|_| ChatError::StateUnavailable)?;
            let key_store = EnvironmentKeyStore::new(oauth.clone(), options.wrapping_key_variable)
                .map_err(|_| ChatError::InvalidConfiguration)?;
            let vault = Arc::new(
                PlatformCredentialVault::with_key_store(
                    oauth,
                    format!("cloud-chat-oauth-{identity}"),
                    Arc::new(key_store),
                )
                .map_err(|_| ChatError::StateUnavailable)?,
            );
            context.with_oauth_credential_vault(vault)
        };
        let interactions = Arc::new(PublicInteractionRouter::new(Arc::new(DenyApproval), None));
        let runtime = Arc::new(
            Runtime::open_with_provider_credentials_and_codex_auth(
                &options.config,
                interactions.clone(),
                Some(interactions.clone()),
                context,
                options.credentials,
                options.codex_auth,
            )
            .map_err(|_| ChatError::RuntimeUnavailable)?,
        );
        let api = Arc::new(RuntimeAgentRunApi::new(
            runtime.clone(),
            interactions,
            "primary",
            "You are Colossus, a cloud assistant. Use only the configured tools available in this conversation. No provisioned computer or coding environment is attached.",
        ));
        let principal = ApplicationPrincipal::authenticated(
            format!("app:cloud-chat-{identity}"),
            format!("cloud-chat-{identity}"),
            ApplicationKind::Enrolled,
            [
                scopes::RUNS_READ,
                scopes::RUNS_EXECUTE,
                scopes::RUNS_CONTROL,
                scopes::PROMPTS_RESPOND,
                scopes::APPROVALS_RESPOND,
            ]
            .into_iter()
            .map(ApiScope::new)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ChatError::InvalidConfiguration)?,
            options.config.models.roles.keys().cloned(),
            runtime
                .tool_catalog()
                .into_iter()
                .filter_map(|tool| tool["name"].as_str().map(str::to_owned)),
        )
        .map_err(|_| ChatError::InvalidConfiguration)?;
        let caller = CallerContext::authenticated(
            principal,
            RequestId::new("hosted-chat").map_err(|_| ChatError::InvalidConfiguration)?,
        );
        let runs = Arc::new(ContextBoundAgentRunClient::new(api.clone(), caller));
        Ok(Self {
            runtime,
            api,
            runs,
            identity,
        })
    }

    /// Stable opaque tenant identity; no user name, credential or filesystem path.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Ordinary public SDK bound once to the exact user's private runtime application.
    pub fn runs(&self) -> Arc<dyn AgentRunClient> {
        self.runs.clone()
    }

    /// Host-only runtime management. Never expose this object or its methods as arbitrary Web RPC.
    pub fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }

    /// Inspect owned execution/recovery state for trusted host lifecycle management.
    /// This never consumes a public SDK read quota or exposes execution metadata.
    pub fn has_pending_executions(&self) -> bool {
        self.api.has_pending_executions()
    }

    /// Stop admission, cancel/drain owned runs and release private state after positive completion.
    pub async fn shutdown(&self, timeout: Duration) -> bool {
        self.api.shutdown_and_wait(timeout).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_runtime_identity_separates_projects_and_users() {
        let first = ChatRuntime::identity_for("project", "alice").unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(
            first,
            ChatRuntime::identity_for("project", "alice").unwrap()
        );
        assert_ne!(first, ChatRuntime::identity_for("project", "bob").unwrap());
        assert_ne!(first, ChatRuntime::identity_for("other", "alice").unwrap());
        for (project, user) in [
            ("../project", "alice"),
            ("project", ""),
            ("project", ".."),
            ("project", "alice/../bob"),
        ] {
            assert_eq!(
                ChatRuntime::identity_for(project, user),
                Err(ChatError::InvalidConfiguration)
            );
        }
    }
}

#[cfg(test)]
mod custody_tests {
    use super::*;

    #[test]
    fn runtime_rejects_another_users_resolver_before_opening_private_state() {
        let temporary = tempfile::tempdir().unwrap();
        let root =
            ConfinedRoot::bind(temporary.path().canonicalize().unwrap().join("home")).unwrap();
        let resolver = Arc::new(ChatCredentialResolver::new("project", "alice", []).unwrap());
        let result = ChatRuntime::open(ChatRuntimeOptions {
            project_id: "project".into(),
            user_id: "bob".into(),
            root: root.clone(),
            config: RuntimeConfig::controlled_chat_template(root.path().join("unused.redb")),
            wrapping_key_variable: "COLOSSUS_UNUSED_TEST_KEY".into(),
            credentials: resolver,
            codex_auth: None,
            shared: None,
        });
        assert!(matches!(result, Err(ChatError::InvalidConfiguration)));
        assert!(!root.path().join("users").exists());
    }
}
