use super::*;
use crate::{
    PluginClient, PluginInventoryEntry, PluginResourceEntry, PluginResourceRead,
    PluginSkillContent, PluginSkillMetadata,
};
use colossus_api::{
    AgentPluginManifest, PluginComponentDiagnostic, PluginComponentKind, PluginMcpInventoryEntry,
    PluginMcpTransport, PluginOrigin, PluginStatus, PluginTrustEvidence,
};
use proto::extension_service_client::ExtensionServiceClient;

#[path = "grpc_plugin_icons.rs"]
mod icons;

const MAX_CATALOG_METADATA_BYTES: usize = 8 * 1024 * 1024;
const MAX_CATALOG_ICON_BYTES: usize = 2 * 1024 * 1024;
const MAX_CATALOG_RESPONSE_BYTES: usize = MAX_CATALOG_METADATA_BYTES + MAX_CATALOG_ICON_BYTES;

#[derive(Default)]
struct InventoryBudget {
    response_bytes: usize,
    metadata_bytes: usize,
    icon_bytes: usize,
}

impl InventoryBudget {
    fn append(
        &mut self,
        output: &mut Vec<PluginInventoryEntry>,
        response: proto::ListExtensionsResponse,
    ) -> ApiResult<String> {
        self.response_bytes = self.response_bytes.saturating_add(response.encoded_len());
        let icon_payload_bytes = response
            .plugins
            .iter()
            .map(|plugin| plugin.icon_data_url.len())
            .sum::<usize>();
        self.metadata_bytes = self
            .metadata_bytes
            .saturating_add(response.encoded_len().saturating_sub(icon_payload_bytes));
        if self.response_bytes > MAX_CATALOG_RESPONSE_BYTES
            || self.metadata_bytes > MAX_CATALOG_METADATA_BYTES
            || output.len().saturating_add(response.plugins.len()) > 10_000
        {
            return Err(protocol_error());
        }
        for plugin in response.plugins {
            let mut plugin = plugin_from_proto(plugin)?;
            if let Some(icon) = &plugin.icon_data_url {
                if self.icon_bytes.saturating_add(icon.len()) > MAX_CATALOG_ICON_BYTES {
                    // Display assets must not make an otherwise bounded catalog unreadable.
                    plugin.icon_data_url = None;
                } else {
                    self.icon_bytes += icon.len();
                }
            }
            output.push(plugin);
        }
        Ok(response.page.unwrap_or_default().next_page_token)
    }
}

pub(super) struct GrpcPluginClient {
    pub(super) transport: Arc<GrpcArtifactClient>,
}

impl GrpcPluginClient {
    fn client(&self) -> ExtensionServiceClient<Channel> {
        ExtensionServiceClient::new(self.transport.channel.clone())
            .max_decoding_message_size(MAX_MESSAGE_BYTES)
            .max_encoding_message_size(16 * 1024)
    }
}

#[async_trait]
impl PluginClient for GrpcPluginClient {
    async fn list(&self) -> ApiResult<Vec<PluginInventoryEntry>> {
        let mut output = Vec::new();
        let mut token = String::new();
        let mut seen = std::collections::BTreeSet::new();
        let mut budget = InventoryBudget::default();
        for _ in 0..1_000 {
            let request = self
                .transport
                .request(proto::ListExtensionsRequest {
                    kinds: vec![proto::ExtensionKind::AgentPlugin as i32],
                    include_disabled: false,
                    page: Some(proto::PageRequest {
                        page_size: 32,
                        page_token: token,
                    }),
                })
                .await?;
            let response = self
                .client()
                .list_extensions(request)
                .await
                .map_err(api_error_from_status)?
                .into_inner();
            token = budget.append(&mut output, response)?;
            if token.is_empty() {
                return Ok(output);
            }
            if !seen.insert(token.clone()) {
                return Err(protocol_error());
            }
        }
        Err(protocol_error())
    }

    async fn skill(&self, id: &str, digest: &str) -> ApiResult<PluginSkillContent> {
        let request = self
            .transport
            .request(proto::ReadPluginSkillRequest {
                skill_id: id.into(),
                digest: digest.into(),
            })
            .await?;
        let value = self
            .client()
            .read_plugin_skill(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        let skill = required(value.skill)?;
        if value.digest != digest || skill.id != id {
            return Err(protocol_error());
        }
        Ok(PluginSkillContent {
            skill: skill_from_proto(skill)?,
            digest: value.digest,
            instructions: value.instructions,
        })
    }

    async fn resources(&self, id: &str, digest: &str) -> ApiResult<Vec<PluginResourceEntry>> {
        let request = self
            .transport
            .request(proto::ListPluginResourcesRequest {
                skill_id: id.into(),
                digest: digest.into(),
            })
            .await?;
        let value = self
            .client()
            .list_plugin_resources(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        if value.resources.len() > 10_000
            || value.resources.iter().any(|entry| entry.skill_id != id)
        {
            return Err(protocol_error());
        }
        value
            .resources
            .into_iter()
            .map(resource_from_proto)
            .collect()
    }

    async fn resource(&self, id: &str, digest: &str, path: &str) -> ApiResult<PluginResourceRead> {
        let request = self
            .transport
            .request(proto::ReadPluginResourceRequest {
                skill_id: id.into(),
                digest: digest.into(),
                path: path.into(),
            })
            .await?;
        let value = self
            .client()
            .read_plugin_resource(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        let resource = resource_from_proto(required(value.resource)?)?;
        if value.digest != digest
            || resource.skill_id != id
            || resource.path != path
            || !resource.text
        {
            return Err(protocol_error());
        }
        Ok(PluginResourceRead {
            skill_id: resource.skill_id,
            path: resource.path,
            size: resource.size,
            content: value.content,
        })
    }
}

fn skill_from_proto(skill: proto::PluginSkill) -> ApiResult<PluginSkillMetadata> {
    let plugin = skill
        .id
        .split_once('/')
        .filter(|(_, name)| *name == skill.name)
        .map(|(plugin, _)| plugin.to_owned())
        .ok_or_else(protocol_error)?;
    Ok(PluginSkillMetadata {
        id: skill.id,
        plugin,
        name: skill.name,
        description: skill.description,
        compatibility: (!skill.compatibility.is_empty()).then_some(skill.compatibility),
        allowed_tools: (!skill.allowed_tools.is_empty()).then_some(skill.allowed_tools),
    })
}

fn resource_from_proto(value: proto::PluginResource) -> ApiResult<PluginResourceEntry> {
    if value.path.is_empty()
        || value.path.contains('\\')
        || value
            .path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'))
    {
        return Err(protocol_error());
    }
    Ok(PluginResourceEntry {
        skill_id: value.skill_id,
        path: value.path,
        size: value.size_bytes,
        text: value.text_preview_available,
    })
}

fn plugin_from_proto(value: proto::AgentPlugin) -> ApiResult<PluginInventoryEntry> {
    let trust = required(value.trust)?;
    let skills = value
        .skills
        .into_iter()
        .map(skill_from_proto)
        .collect::<ApiResult<Vec<_>>>()?;
    if skills.iter().any(|skill| skill.plugin != value.name) {
        return Err(protocol_error());
    }
    Ok(PluginInventoryEntry {
        icon_data_url: icons::validated(value.icon_data_url)?,
        origin: if value.bundled {
            PluginOrigin::Bundled
        } else {
            PluginOrigin::Installed
        },
        available: value.available,
        unavailable_reason: (!value.unavailable_reason.is_empty())
            .then_some(value.unavailable_reason),
        actions: Vec::new(),
        manifest: AgentPluginManifest {
            schema: "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json".into(),
            name: value.name,
            version: (!value.version.is_empty()).then_some(value.version),
            description: (!value.description.is_empty()).then_some(value.description),
            author: None,
            homepage: None,
            repository: None,
            license: None,
            keywords: Vec::new(),
            extensions: Default::default(),
        },
        digest: value.digest,
        source: value.source,
        status: if value.globally_active {
            PluginStatus::Enabled
        } else {
            PluginStatus::Disabled
        },
        trust: PluginTrustEvidence {
            trusted: trust.trusted,
            profile: (!trust.profile.is_empty()).then_some(trust.profile),
            signer: (!trust.signer.is_empty()).then_some(trust.signer),
            method: trust.method,
        },
        skills,
        mcp_servers: value
            .mcp_servers
            .into_iter()
            .map(|server| {
                Ok(PluginMcpInventoryEntry {
                    id: server.id,
                    name: server.name,
                    enabled: server.enabled,
                    status: server.status,
                    transport: match server.transport.as_str() {
                        "stdio" => PluginMcpTransport::Stdio,
                        "streamable-http" => PluginMcpTransport::StreamableHttp,
                        "sse" => PluginMcpTransport::Sse,
                        _ => return Err(protocol_error()),
                    },
                })
            })
            .collect::<ApiResult<Vec<_>>>()?,
        diagnostics: value
            .diagnostics
            .into_iter()
            .map(|diagnostic| {
                Ok(PluginComponentDiagnostic {
                    kind: match diagnostic.kind.as_str() {
                        "plugin" => PluginComponentKind::Plugin,
                        "skill" => PluginComponentKind::Skill,
                        "mcp_server" => PluginComponentKind::McpServer,
                        _ => return Err(protocol_error()),
                    },
                    name: (!diagnostic.name.is_empty()).then_some(diagnostic.name),
                    code: diagnostic.code,
                    detail: diagnostic.detail,
                })
            })
            .collect::<ApiResult<Vec<_>>>()?,
    })
}

#[cfg(test)]
#[path = "grpc_plugins_tests.rs"]
mod tests;
