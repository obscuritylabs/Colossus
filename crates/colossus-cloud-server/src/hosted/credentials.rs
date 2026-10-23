use super::*;
use colossus_cloud_chat::{
    ChatAuthentication, ChatConnectionBinding, ChatConnectionKind, ChatCredentialResolver,
};
use colossus_contracts::{CredentialError, HostSecret, VaultRecord};
use colossus_credentials::{EnvironmentKeyStore, PlatformCredentialVault};
use colossus_ports::{CredentialKey, CredentialVault};
use sha2::{Digest as _, Sha256};
use std::path::Path as FsPath;
use zeroize::Zeroizing;

impl HostedExecution {
    /// Construct tenant OAuth custody only for the exact live Hosted generation.
    pub(crate) fn execution_vault(
        &self,
        project: &str,
        user: &str,
        lease: Option<colossus_cloud_postgres::ChatLease>,
    ) -> std::result::Result<Arc<dyn CredentialVault>, CloudError> {
        let identity =
            ChatRuntime::identity_for(project, user).map_err(|_| CloudError::InvalidArgument)?;
        let shared = self.shared.as_ref().ok_or(CloudError::InvalidArgument)?;
        let lease = lease.ok_or(CloudError::Conflict)?;
        if lease.namespace != identity || lease.project_id != project || lease.user_id != user {
            return Err(CloudError::PermissionDenied);
        }
        shared.sql.verify_lease(&lease)?;
        Ok(shared.vault(&format!("user/{project}/{user}/oauth"), Some(lease)))
    }

    fn vault(
        &self,
        path: &str,
        owner: &str,
    ) -> std::result::Result<Arc<dyn CredentialVault>, CloudError> {
        let cache_key = format!("{path}\0{owner}");
        let mut cache = self.vaults.lock().map_err(|_| CloudError::Storage)?;
        if let Some(vault) = cache.get(&cache_key).and_then(std::sync::Weak::upgrade) {
            return Ok(vault);
        }
        cache.retain(|_, vault| vault.strong_count() > 0);
        if cache.len() >= 1024 {
            return Err(CloudError::ResourceExhausted);
        }
        let path = self
            .root
            .prepare_directory(FsPath::new(path))
            .map_err(|_| CloudError::Storage)?;
        let root = ConfinedRoot::bind(path).map_err(|_| CloudError::Storage)?;
        let keys =
            EnvironmentKeyStore::new(root.clone(), self.deployment.wrapping_key_variable.clone())
                .map_err(|_| CloudError::Storage)?;
        let vault: Arc<dyn CredentialVault> = Arc::new(
            PlatformCredentialVault::with_key_store(root, owner.to_owned(), Arc::new(keys))
                .map_err(|_| CloudError::Storage)?,
        );
        cache.insert(cache_key, Arc::downgrade(&vault));
        Ok(vault)
    }

    pub(crate) fn project_vault(
        &self,
        project: &str,
    ) -> std::result::Result<Arc<dyn CredentialVault>, CloudError> {
        colossus_cloud::validate_identifier(project)?;
        if let Some(shared) = &self.shared {
            return Ok(shared.vault(&format!("project/{project}/connections"), None));
        }
        let id = hex::encode(Sha256::digest(project.as_bytes()));
        self.vault(
            &format!("projects/{id}/credentials"),
            &format!("cloud-chat-project-{id}"),
        )
    }

    pub(crate) fn instance_vault(
        &self,
    ) -> std::result::Result<Arc<dyn CredentialVault>, CloudError> {
        if let Some(shared) = &self.shared {
            return Ok(shared.vault("instance/connections", None));
        }
        self.vault("instance/credentials", "cloud-hosted-instance")
    }

    pub(crate) fn user_vault(
        &self,
        project: &str,
        user: &str,
    ) -> std::result::Result<Arc<dyn CredentialVault>, CloudError> {
        if let Some(shared) = &self.shared {
            return Ok(shared.vault(&format!("user/{project}/{user}/connections"), None));
        }
        let id =
            ChatRuntime::identity_for(project, user).map_err(|_| CloudError::InvalidArgument)?;
        self.vault(
            &format!("users/{id}/connections"),
            &format!("cloud-chat-user-{id}"),
        )
    }

    pub(crate) fn connection_binding(
        &self,
        caller: &CloudCaller,
        configuration: &ChatProjectConfig,
        kind: ChatConnectionKind,
        id: &str,
    ) -> std::result::Result<Option<ChatConnectionBinding>, CloudError> {
        let (endpoint, authentication) = match kind {
            ChatConnectionKind::Provider => {
                let profile = configuration
                    .runtime
                    .providers
                    .profiles
                    .get(id)
                    .ok_or(CloudError::NotFound)?;
                (
                    profile
                        .base_url
                        .as_deref()
                        .ok_or(CloudError::InvalidArgument)?,
                    configuration
                        .provider_authentication
                        .get(id)
                        .copied()
                        .unwrap_or_default(),
                )
            }
            ChatConnectionKind::Mcp => {
                let server = configuration
                    .runtime
                    .mcp
                    .servers
                    .get(id)
                    .ok_or(CloudError::NotFound)?;
                (
                    server.url.as_deref().ok_or(CloudError::InvalidArgument)?,
                    configuration
                        .mcp_authentication
                        .get(id)
                        .copied()
                        .unwrap_or_default(),
                )
            }
            ChatConnectionKind::Search => {
                let profile = configuration
                    .runtime
                    .search
                    .profiles
                    .get(id)
                    .ok_or(CloudError::NotFound)?;
                (
                    crate::global_settings::search::endpoint(profile),
                    crate::global_settings::search::authentication(configuration, id),
                )
            }
        };
        let binding = ChatConnectionBinding::new(
            caller.project_id(),
            caller.subject(),
            kind,
            id,
            configuration.generation(kind, id),
            endpoint,
            authentication,
        )
        .map_err(|_| CloudError::InvalidArgument)?;
        let Some(binding) = binding else {
            return Ok(None);
        };
        let (provenance, generations) = match kind {
            ChatConnectionKind::Provider => (
                &configuration.inheritance.provenance.providers,
                &configuration.inheritance.global_generations.providers,
            ),
            ChatConnectionKind::Mcp => (
                &configuration.inheritance.provenance.mcp,
                &configuration.inheritance.global_generations.mcp,
            ),
            ChatConnectionKind::Search => (
                &configuration.inheritance.provenance.search,
                &configuration.inheritance.global_generations.search,
            ),
        };
        let binding = if provenance.get(id).is_some_and(|source| *source == "global") {
            binding
                .with_catalog_generation(*generations.get(id).ok_or(CloudError::Storage)?)
                .map_err(|_| CloudError::InvalidArgument)?
        } else {
            binding
        };
        Ok(Some(binding))
    }

    pub(super) fn credentials(
        &self,
        caller: &CloudCaller,
        configuration: &ChatProjectConfig,
        lease: Option<colossus_cloud_postgres::ChatLease>,
    ) -> std::result::Result<
        (colossus_runtime::RuntimeConfig, Arc<ChatCredentialResolver>),
        CloudError,
    > {
        let mut config = configuration.runtime.clone();
        let mut entries = Vec::new();
        for (id, profile) in &mut config.providers.profiles {
            if matches!(
                profile.kind.generation_action(),
                "provider.echo" | "provider.openai.codex"
            ) {
                continue;
            }
            let authentication = configuration
                .provider_authentication
                .get(id)
                .copied()
                .unwrap_or_default();
            if authentication == ChatAuthentication::None {
                if profile.credential_reference.is_some() {
                    return Err(CloudError::InvalidArgument);
                }
                continue;
            }
            let reference = profile
                .credential_reference
                .as_deref()
                .unwrap_or("host:managed");
            let binding = self
                .connection_binding(caller, configuration, ChatConnectionKind::Provider, id)?
                .ok_or(CloudError::InvalidArgument)?;
            let vault = self.credential_source(caller, &binding, authentication, reference)?;
            profile.credential_reference = Some(ChatCredentialResolver::reference(&binding));
            entries.push((binding, vault));
        }
        for (id, profile) in &mut config.search.profiles {
            let authentication = crate::global_settings::search::authentication(configuration, id);
            if authentication == ChatAuthentication::None {
                if crate::global_settings::search::reference(profile).is_some() {
                    return Err(CloudError::InvalidArgument);
                }
                continue;
            }
            let reference = crate::global_settings::search::reference(profile)
                .unwrap_or("host:managed")
                .to_owned();
            let binding = self
                .connection_binding(caller, configuration, ChatConnectionKind::Search, id)?
                .ok_or(CloudError::InvalidArgument)?;
            let vault = self.credential_source(caller, &binding, authentication, &reference)?;
            crate::global_settings::search::set_reference(
                profile,
                Some(ChatCredentialResolver::reference(&binding)),
            )?;
            entries.push((binding, vault));
        }
        for (id, server) in &mut config.mcp.servers {
            let authentication = configuration
                .mcp_authentication
                .get(id)
                .copied()
                .unwrap_or_default();
            if server.oauth.is_some() {
                config.mcp.oauth_credential_store =
                    colossus_mcp::McpOAuthCredentialStoreKind::Platform;
            } else if authentication != ChatAuthentication::None
                && server.credential_headers.is_empty()
            {
                server.credential_headers.insert(
                    "Authorization".into(),
                    colossus_mcp::McpCredentialHeaderConfig {
                        scheme: Some("Bearer".into()),
                        reference: "host:managed".into(),
                    },
                );
            }
            for header in server.credential_headers.values_mut() {
                let binding = self
                    .connection_binding(caller, configuration, ChatConnectionKind::Mcp, id)?
                    .ok_or(CloudError::InvalidArgument)?;
                let vault =
                    self.credential_source(caller, &binding, authentication, &header.reference)?;
                header.reference = ChatCredentialResolver::reference(&binding);
                entries.push((binding, vault));
            }
        }
        let original = std::mem::take(&mut config.mcp.servers);
        config.mcp.servers = original
            .into_iter()
            .map(|(id, server)| (runtime_mcp_name(configuration, &id), server))
            .collect();
        if let (Some(shared), Some(lease)) = (&self.shared, lease) {
            entries = entries
                .into_iter()
                .map(|(binding, vault)| (binding, shared.fence_reader(vault, lease.clone())))
                .collect();
        }
        let resolver = ChatCredentialResolver::new(caller.project_id(), caller.subject(), entries)
            .map_err(|_| CloudError::InvalidArgument)?;
        Ok((config, Arc::new(resolver)))
    }

    pub(crate) fn selected_vault(
        &self,
        caller: &CloudCaller,
        authentication: ChatAuthentication,
    ) -> std::result::Result<Arc<dyn CredentialVault>, CloudError> {
        match authentication {
            ChatAuthentication::Personal => self.user_vault(caller.project_id(), caller.subject()),
            ChatAuthentication::Shared => self.project_vault(caller.project_id()),
            ChatAuthentication::None => Err(CloudError::InvalidArgument),
        }
    }

    pub(crate) fn connection_credential_configured(
        &self,
        caller: &CloudCaller,
        binding: &ChatConnectionBinding,
        authentication: ChatAuthentication,
        operator_configured: bool,
    ) -> std::result::Result<bool, CloudError> {
        if authentication == ChatAuthentication::Shared {
            if let Some(vault) = crate::global_settings::credentials::assigned_vault(self, binding)?
            {
                return ChatCredentialResolver::contains(vault.as_ref(), binding)
                    .map_err(|_| CloudError::Storage);
            }
            if operator_configured {
                return Ok(true);
            }
        }
        let vault = self.selected_vault(caller, authentication)?;
        ChatCredentialResolver::contains(vault.as_ref(), binding).map_err(|_| CloudError::Storage)
    }

    fn credential_source(
        &self,
        caller: &CloudCaller,
        binding: &ChatConnectionBinding,
        authentication: ChatAuthentication,
        reference: &str,
    ) -> std::result::Result<Arc<dyn CredentialVault>, CloudError> {
        if authentication == ChatAuthentication::Shared
            && let Some(vault) = crate::global_settings::credentials::assigned_vault(self, binding)?
        {
            return Ok(vault);
        }
        if authentication == ChatAuthentication::Shared
            && let Some(variable) = reference.strip_prefix("env:")
        {
            if variable.is_empty()
                || variable.len() > 256
                || !variable
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                || !(variable.as_bytes()[0].is_ascii_alphabetic() || variable.starts_with('_'))
            {
                return Err(CloudError::InvalidArgument);
            }
            return Ok(Arc::new(ReadOnlyEnvironmentVault {
                variable: variable.into(),
                key: CredentialKey::new("cloud-chat", &binding.record_id())
                    .map_err(|_| CloudError::Storage)?,
            }));
        }
        self.selected_vault(caller, authentication)
    }
}

pub(crate) fn runtime_mcp_name(configuration: &ChatProjectConfig, id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(id.as_bytes());
    hash.update([0]);
    hash.update(
        configuration
            .generation(ChatConnectionKind::Mcp, id)
            .to_be_bytes(),
    );
    if configuration
        .inheritance
        .provenance
        .mcp
        .get(id)
        .is_some_and(|source| *source == "global")
    {
        hash.update(b"instance-catalog-generation");
        hash.update(
            configuration
                .inheritance
                .global_generations
                .mcp
                .get(id)
                .copied()
                .unwrap_or(0)
                .to_be_bytes(),
        );
    }
    let digest = hex::encode(hash.finalize());
    format!("{}-g{digest}", id.chars().take(16).collect::<String>())
}

struct ReadOnlyEnvironmentVault {
    variable: String,
    key: CredentialKey,
}

impl CredentialVault for ReadOnlyEnvironmentVault {
    fn read(
        &self,
        key: &CredentialKey,
    ) -> std::result::Result<Option<VaultRecord>, CredentialError> {
        if key.purpose() == "cloud-chat-credentials" && key.id() == "catalog-v1" {
            return Ok(None); // Deployment environment credentials do not use the named store.
        }
        if key != &self.key {
            return Err(CredentialError::InvalidInput);
        }
        let value = Zeroizing::new(
            std::env::var(&self.variable).map_err(|_| CredentialError::Unavailable)?,
        );
        let secret = HostSecret::new(value.as_str())?;
        Ok(Some(VaultRecord::new(secret.expose().as_bytes().to_vec())?))
    }
    fn write(
        &self,
        _: &CredentialKey,
        _: &VaultRecord,
    ) -> std::result::Result<(), CredentialError> {
        Err(CredentialError::InvalidInput)
    }
    fn delete(&self, _: &CredentialKey) -> std::result::Result<(), CredentialError> {
        Err(CredentialError::InvalidInput)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_mcp_oauth_identity_tracks_connection_generation_without_catalog_churn() {
        let mut configuration = ChatProjectConfig::new(
            colossus_runtime::RuntimeConfig::controlled_chat_template("unused.redb"),
        );
        let local = runtime_mcp_name(&configuration, "remote");
        configuration
            .inheritance
            .provenance
            .mcp
            .insert("remote".into(), "global");
        configuration
            .inheritance
            .global_generations
            .mcp
            .insert("remote".into(), 2);
        let inherited = runtime_mcp_name(&configuration, "remote");
        assert_ne!(local, inherited);
        configuration.inheritance.global_revision = 25;
        configuration
            .labels
            .models
            .insert("unrelated".into(), "Renamed model".into());
        assert_eq!(runtime_mcp_name(&configuration, "remote"), inherited);
        configuration
            .inheritance
            .global_generations
            .mcp
            .insert("remote".into(), 3);
        assert_ne!(runtime_mcp_name(&configuration, "remote"), inherited);
        configuration
            .inheritance
            .provenance
            .mcp
            .insert("remote".into(), "project");
        assert_eq!(runtime_mcp_name(&configuration, "remote"), local);
    }
}
