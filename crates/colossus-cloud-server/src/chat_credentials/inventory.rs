use super::*;

pub(super) struct Connection {
    pub id: String,
    pub kind: ChatConnectionKind,
    pub label: String,
    pub endpoint: String,
    pub binding: ChatConnectionBinding,
}
impl Connection {
    pub fn legacy_id(&self) -> String {
        format!("legacy-{}", self.binding.record_id())
    }
}
#[derive(Clone, Serialize)]
pub(super) struct Usage {
    id: String,
    kind: ChatConnectionKind,
    label: String,
    endpoint: String,
    credential_binding: String,
    credential_source: &'static str,
}
#[derive(Serialize)]
pub(super) struct Credential {
    #[serde(flatten)]
    metadata: StoredCredentialMetadata,
    legacy: bool,
    used_by: Vec<Usage>,
}
#[derive(Serialize)]
pub(super) struct Destination {
    #[serde(flatten)]
    usage: Usage,
    credential_id: Option<String>,
}
#[derive(Serialize)]
pub(super) struct Snapshot {
    revision: u64,
    credentials: Vec<Credential>,
    connections: Vec<Destination>,
}

pub(super) fn connections(
    host: &HostedExecution,
    caller: &CloudCaller,
    config: &ChatProjectConfig,
    scope: Scope,
) -> std::result::Result<Vec<Connection>, CloudError> {
    let mut rows = Vec::new();
    for (kind, ids) in [
        (
            ChatConnectionKind::Provider,
            config.runtime.providers.profiles.keys().collect::<Vec<_>>(),
        ),
        (
            ChatConnectionKind::Mcp,
            config.runtime.mcp.servers.keys().collect(),
        ),
        (
            ChatConnectionKind::Search,
            config.runtime.search.profiles.keys().collect(),
        ),
    ] {
        for id in ids {
            let (authentication, endpoint, eligible, label) = match kind {
                ChatConnectionKind::Provider => {
                    let value = &config.runtime.providers.profiles[id];
                    (
                        config
                            .provider_authentication
                            .get(id)
                            .copied()
                            .unwrap_or_default(),
                        value.base_url.as_deref(),
                        !matches!(
                            value.kind.generation_action(),
                            "provider.echo" | "provider.openai.codex"
                        ) && !value
                            .credential_reference
                            .as_deref()
                            .is_some_and(|reference| reference.starts_with("env:")),
                        config.labels.providers.get(id).unwrap_or(id),
                    )
                }
                ChatConnectionKind::Mcp => {
                    let value = &config.runtime.mcp.servers[id];
                    (
                        config
                            .mcp_authentication
                            .get(id)
                            .copied()
                            .unwrap_or_default(),
                        value.url.as_deref(),
                        value.oauth.is_none()
                            && value.credential_headers.len() <= 1
                            && !value
                                .credential_headers
                                .values()
                                .any(|header| header.reference.starts_with("env:")),
                        id,
                    )
                }
                ChatConnectionKind::Search => {
                    let value = &config.runtime.search.profiles[id];
                    (
                        crate::global_settings::search::authentication(config, id),
                        Some(crate::global_settings::search::endpoint(value)),
                        !crate::global_settings::search::reference(value)
                            .is_some_and(|reference| reference.starts_with("env:")),
                        config.labels.search.get(id).unwrap_or(id),
                    )
                }
            };
            if authentication != scope.authentication() || !eligible {
                continue;
            }
            let binding = host
                .connection_binding(caller, config, kind, id)?
                .ok_or(CloudError::InvalidArgument)?;
            rows.push(Connection {
                id: id.clone(),
                kind,
                label: label.clone(),
                endpoint: endpoint.ok_or(CloudError::InvalidArgument)?.into(),
                binding,
            });
        }
    }
    Ok(rows)
}

pub(super) fn snapshot(
    host: &HostedExecution,
    caller: &CloudCaller,
    config: &ChatProjectConfig,
    scope: Scope,
    vault: &dyn CredentialVault,
    store: &CredentialStore,
) -> std::result::Result<Snapshot, CloudError> {
    let mut credentials: Vec<_> = store
        .metadata()
        .map(|metadata| Credential {
            metadata: metadata.clone(),
            legacy: false,
            used_by: Vec::new(),
        })
        .collect();
    let mut destinations = Vec::new();
    for row in connections(host, caller, config, scope)? {
        let instance_assigned = matches!(scope, Scope::Project)
            && crate::global_settings::credentials::assigned_vault(host, &row.binding)?.is_some();
        let usage = Usage {
            id: row.id.clone(),
            kind: row.kind,
            label: row.label.clone(),
            endpoint: row.endpoint.clone(),
            credential_binding: row.binding.record_id(),
            credential_source: if instance_assigned {
                "global"
            } else {
                "project"
            },
        };
        let id = match store.attachment(&row.binding) {
            Some(id) => id.map(str::to_owned),
            None if colossus_cloud_chat::ChatCredentialResolver::contains(vault, &row.binding)
                .map_err(|_| CloudError::Storage)? =>
            {
                let id = row.legacy_id();
                credentials.push(Credential {
                    metadata: StoredCredentialMetadata {
                        id: id.clone(),
                        label: row.label.chars().take(160).collect(),
                        kind: StoredCredentialKind::ApiKey,
                    },
                    legacy: true,
                    used_by: Vec::new(),
                });
                Some(id)
            }
            None => None,
        };
        if let Some(id) = &id {
            credentials
                .iter_mut()
                .find(|credential| &credential.metadata.id == id)
                .ok_or(CloudError::Storage)?
                .used_by
                .push(usage.clone());
        }
        destinations.push(Destination {
            usage,
            credential_id: id,
        });
    }
    Ok(Snapshot {
        revision: store.revision(),
        credentials,
        connections: destinations,
    })
}
