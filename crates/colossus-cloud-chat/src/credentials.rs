use crate::{ChatConnectionBinding, ChatCredentialScope, ChatError, connections::valid_id};
use colossus_contracts::{HostSecret, VaultRecord};
use colossus_ports::{
    CredentialKey, CredentialResolutionError, CredentialResolver, CredentialVault,
};
use std::{collections::BTreeMap, sync::Arc};

/// Exact late-bound credential references belonging to one hosted runtime.
///
/// There is no ambient environment lookup or shared-account fallback. Trusted host
/// composition chooses each binding from the current project policy and caller.
pub struct ChatCredentialResolver {
    project_id: String,
    user_id: String,
    entries: BTreeMap<String, (Arc<dyn CredentialVault>, CredentialKey)>,
}
impl ChatCredentialResolver {
    /// Bind only the selected user's personal records and explicitly shared project records.
    pub fn new(
        project_id: &str,
        user_id: &str,
        entries: impl IntoIterator<Item = (ChatConnectionBinding, Arc<dyn CredentialVault>)>,
    ) -> Result<Self, ChatError> {
        if !valid_id(project_id) || !valid_id(user_id) {
            return Err(ChatError::InvalidConfiguration);
        }
        let mut selected = BTreeMap::new();
        for (binding, vault) in entries {
            let matches = match binding.scope() {
                ChatCredentialScope::Personal { project, user } => {
                    project == project_id && user == user_id
                }
                ChatCredentialScope::Project { project } => project == project_id,
            };
            if !matches {
                return Err(ChatError::InvalidConfiguration);
            }
            let id = binding.record_id();
            let key = CredentialKey::new("cloud-chat", &id)
                .map_err(|_| ChatError::InvalidConfiguration)?;
            if selected
                .insert(format!("host:chat-{id}"), (vault, key))
                .is_some()
            {
                return Err(ChatError::InvalidConfiguration);
            }
        }
        Ok(Self {
            project_id: project_id.into(),
            user_id: user_id.into(),
            entries: selected,
        })
    }

    pub(crate) fn matches_tenant(&self, project_id: &str, user_id: &str) -> bool {
        self.project_id == project_id && self.user_id == user_id
    }

    /// Nonsecret reference to put in a host-composed provider or MCP configuration.
    pub fn reference(binding: &ChatConnectionBinding) -> String {
        format!("host:chat-{}", binding.record_id())
    }

    /// Store a complete user-entered credential under an already authorized binding.
    pub fn save(
        vault: &dyn CredentialVault,
        binding: &ChatConnectionBinding,
        secret: &HostSecret,
    ) -> Result<(), ChatError> {
        let key = CredentialKey::new("cloud-chat", &binding.record_id())
            .map_err(|_| ChatError::InvalidConfiguration)?;
        let record = VaultRecord::new(secret.expose().as_bytes().to_vec())
            .map_err(|_| ChatError::InvalidConfiguration)?;
        vault
            .write(&key, &record)
            .map_err(|_| ChatError::StateUnavailable)
    }

    /// Disconnect exactly this binding; no other user or shared credential is removed.
    pub fn disconnect(
        vault: &dyn CredentialVault,
        binding: &ChatConnectionBinding,
    ) -> Result<(), ChatError> {
        let key = CredentialKey::new("cloud-chat", &binding.record_id())
            .map_err(|_| ChatError::InvalidConfiguration)?;
        vault.delete(&key).map_err(|_| ChatError::StateUnavailable)
    }
}
impl CredentialResolver for ChatCredentialResolver {
    fn resolve(&self, reference: &str) -> Result<String, CredentialResolutionError> {
        let (vault, key) = self
            .entries
            .get(reference)
            .ok_or(CredentialResolutionError::InvalidReference)?;
        let record = vault
            .read(key)
            .map_err(|_| CredentialResolutionError::Unavailable)?
            .ok_or(CredentialResolutionError::Unavailable)?;
        let value = std::str::from_utf8(record.expose())
            .map_err(|_| CredentialResolutionError::InvalidValue)?;
        if value.is_empty() || value.len() > 64 * 1024 || value.contains('\0') {
            return Err(CredentialResolutionError::InvalidValue);
        }
        Ok(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatAuthentication, ChatConnectionKind};
    use colossus_contracts::CredentialError;
    use std::sync::Mutex;

    #[derive(Default)]
    pub(super) struct Vault {
        values: Mutex<BTreeMap<String, Vec<u8>>>,
        fail: bool,
    }
    impl CredentialVault for Vault {
        fn read(&self, key: &CredentialKey) -> Result<Option<VaultRecord>, CredentialError> {
            if self.fail {
                return Err(CredentialError::Corrupt);
            }
            self.values
                .lock()
                .unwrap()
                .get(key.id())
                .cloned()
                .map(VaultRecord::new)
                .transpose()
        }
        fn write(&self, key: &CredentialKey, record: &VaultRecord) -> Result<(), CredentialError> {
            self.values
                .lock()
                .unwrap()
                .insert(key.id().into(), record.expose().to_vec());
            Ok(())
        }
        fn delete(&self, key: &CredentialKey) -> Result<(), CredentialError> {
            self.values.lock().unwrap().remove(key.id());
            Ok(())
        }
    }
    fn binding(user: &str, auth: ChatAuthentication) -> ChatConnectionBinding {
        ChatConnectionBinding::new(
            "project",
            user,
            ChatConnectionKind::Mcp,
            "mcp",
            1,
            "https://mcp.example/",
            auth,
        )
        .unwrap()
        .unwrap()
    }
    #[test]
    fn token_lookup_cannot_select_other_user_environment_or_shared_identity() {
        let vault: Arc<dyn CredentialVault> = Arc::new(Vault::default());
        let alice = binding("alice", ChatAuthentication::Personal);
        let bob = binding("bob", ChatAuthentication::Personal);
        let shared = binding("alice", ChatAuthentication::Shared);
        ChatCredentialResolver::save(
            vault.as_ref(),
            &alice,
            &HostSecret::new("alice-token").unwrap(),
        )
        .unwrap();
        ChatCredentialResolver::save(
            vault.as_ref(),
            &shared,
            &HostSecret::new("service-token").unwrap(),
        )
        .unwrap();
        let resolver =
            ChatCredentialResolver::new("project", "alice", [(alice.clone(), vault.clone())])
                .unwrap();
        assert_eq!(
            resolver
                .resolve(&ChatCredentialResolver::reference(&alice))
                .unwrap(),
            "alice-token"
        );
        for reference in [
            ChatCredentialResolver::reference(&bob),
            ChatCredentialResolver::reference(&shared),
            "env:OPENAI_API_KEY".into(),
        ] {
            assert_eq!(
                resolver.resolve(&reference),
                Err(CredentialResolutionError::InvalidReference)
            );
        }
        ChatCredentialResolver::disconnect(vault.as_ref(), &alice).unwrap();
        assert_eq!(
            resolver.resolve(&ChatCredentialResolver::reference(&alice)),
            Err(CredentialResolutionError::Unavailable)
        );
        let shared_resolver =
            ChatCredentialResolver::new("project", "alice", [(shared.clone(), vault)]).unwrap();
        assert_eq!(
            shared_resolver
                .resolve(&ChatCredentialResolver::reference(&shared))
                .unwrap(),
            "service-token"
        );
    }
    #[test]
    fn corrupt_personal_custody_fails_without_identity_fallback() {
        let personal = binding("alice", ChatAuthentication::Personal);
        let reference = ChatCredentialResolver::reference(&personal);
        let resolver = ChatCredentialResolver::new(
            "project",
            "alice",
            [(
                personal,
                Arc::new(Vault {
                    fail: true,
                    ..Default::default()
                }) as Arc<dyn CredentialVault>,
            )],
        )
        .unwrap();
        assert_eq!(
            resolver.resolve(&reference),
            Err(CredentialResolutionError::Unavailable)
        );
    }
}

#[cfg(test)]
mod tenant_tests {
    use super::*;
    use crate::{ChatAuthentication, ChatConnectionKind};

    #[test]
    fn resolver_rejects_foreign_personal_and_shared_bindings_before_opening_vault() {
        let make = |project, user, auth| {
            ChatConnectionBinding::new(
                project,
                user,
                ChatConnectionKind::Mcp,
                "mcp",
                1,
                "https://mcp.example/",
                auth,
            )
            .unwrap()
            .unwrap()
        };
        let vault: Arc<dyn CredentialVault> = Arc::new(super::tests::Vault::default());
        for binding in [
            make("project", "bob", ChatAuthentication::Personal),
            make("foreign", "alice", ChatAuthentication::Personal),
            make("foreign", "alice", ChatAuthentication::Shared),
        ] {
            assert!(matches!(
                ChatCredentialResolver::new("project", "alice", [(binding, vault.clone())]),
                Err(ChatError::InvalidConfiguration)
            ));
        }
        let bound = ChatCredentialResolver::new(
            "project",
            "alice",
            [
                (
                    make("project", "alice", ChatAuthentication::Personal),
                    vault.clone(),
                ),
                (make("project", "bob", ChatAuthentication::Shared), vault),
            ],
        )
        .unwrap();
        assert!(bound.matches_tenant("project", "alice"));
        assert!(!bound.matches_tenant("project", "bob"));
    }
}
