//! Authenticated Agent Plugin discovery and explicit bounded reads.

use crate::{status::api_status, system::caller_context};
use colossus_api::{ExtensionApi, scopes};
use colossus_api_proto::v1alpha1::{self as proto, extension_service_server::ExtensionService};
use colossus_contracts::{
    PluginComponentKind, PluginInventoryEntry, PluginMcpTransport, PluginOrigin,
    PluginResourceEntry, PluginSkillMetadata, PluginStatus,
};
use prost::Message as _;
use sha2::{Digest as _, Sha256};
use std::sync::Arc;
use tonic::{Request, Response, Status};

const MAX_DISCOVERY_BYTES: usize = 2 * 1024 * 1024;
const MAX_CATALOG_ICON_BYTES: usize = 2 * 1024 * 1024;

#[cfg(test)]
#[path = "extensions_tests.rs"]
mod tests;

/// Transport-only adapter; all reads retain application and runtime policy checks.
#[derive(Clone)]
pub struct ExtensionServiceAdapter {
    api: Option<Arc<dyn ExtensionApi>>,
}

impl ExtensionServiceAdapter {
    /// A missing implementation reports explicit capability unavailability.
    pub fn new(api: Option<Arc<dyn ExtensionApi>>) -> Self {
        Self { api }
    }

    fn api(&self) -> Result<&dyn ExtensionApi, Status> {
        self.api.as_deref().ok_or_else(|| {
            Status::unimplemented("this target does not support Agent Plugin discovery")
        })
    }
}

#[tonic::async_trait]
impl ExtensionService for ExtensionServiceAdapter {
    async fn get_extension(
        &self,
        request: Request<proto::GetExtensionRequest>,
    ) -> Result<Response<proto::GetExtensionResponse>, Status> {
        let caller = caller_context(&request)?;
        caller
            .require_scope(scopes::EXTENSIONS_READ)
            .map_err(api_status)?;
        let name = request
            .get_ref()
            .extension_id
            .strip_prefix("plugin:")
            .ok_or_else(|| {
                Status::invalid_argument("expected a plugin:<name> extension identity")
            })?;
        let plugin = self
            .api()?
            .plugins(caller, false)
            .await
            .map_err(api_status)?
            .into_iter()
            .find(|plugin| plugin.manifest.name == name)
            .ok_or_else(|| Status::not_found("plugin is not available in this workspace"))?;
        bounded(proto::GetExtensionResponse {
            extension: Some(summary(&plugin)),
            plugin: Some(plugin_to_proto(plugin)),
        })
    }

    async fn list_extensions(
        &self,
        request: Request<proto::ListExtensionsRequest>,
    ) -> Result<Response<proto::ListExtensionsResponse>, Status> {
        let caller = caller_context(&request)?;
        caller
            .require_scope(scopes::EXTENSIONS_READ)
            .map_err(api_status)?;
        let mut plugins = self
            .api()?
            .plugins(caller, request.get_ref().include_disabled)
            .await
            .map_err(api_status)?;
        let request = request.get_ref();
        if !includes_plugins(&request.kinds) {
            return bounded(proto::ListExtensionsResponse::default());
        }
        plugins.sort_by(|left, right| {
            (&left.manifest.name, &left.digest).cmp(&(&right.manifest.name, &right.digest))
        });
        let mut icon_bytes = 0_usize;
        for plugin in &mut plugins {
            if let Some(icon) = &plugin.icon_data_url {
                if icon_bytes.saturating_add(icon.len()) > MAX_CATALOG_ICON_BYTES {
                    plugin.icon_data_url = None;
                } else {
                    icon_bytes += icon.len();
                }
            }
        }
        // A continuation is bound to the exact catalog, so an activation between pages
        // cannot silently skip or duplicate entries.
        let identity: String = plugins
            .iter()
            .map(|entry| {
                format!(
                    "{}@{}:{}:{}\n",
                    entry.manifest.name,
                    entry.digest,
                    entry.status == PluginStatus::Enabled,
                    entry.available
                )
            })
            .collect();
        let identity = format!("{:x}", Sha256::digest(identity.as_bytes()));
        let page = request.page.clone().unwrap_or_default();
        let start = if page.page_token.is_empty() {
            0
        } else {
            let (revision, offset) = page
                .page_token
                .split_once(':')
                .ok_or_else(|| Status::invalid_argument("invalid plugin page token"))?;
            if revision != identity {
                return Err(Status::failed_precondition(
                    "plugin catalog changed; restart discovery",
                ));
            }
            offset
                .parse::<usize>()
                .map_err(|_| Status::invalid_argument("invalid plugin page token"))?
        };
        if start > plugins.len() {
            return Err(Status::invalid_argument("invalid plugin page offset"));
        }
        let limit = if page.page_size == 0 {
            32
        } else {
            page.page_size.clamp(1, 100) as usize
        };
        let total = plugins.len();
        let mut response = proto::ListExtensionsResponse::default();
        for plugin in plugins.into_iter().skip(start).take(limit) {
            response.extensions.push(summary(&plugin));
            response.plugins.push(plugin_to_proto(plugin));
            response.page = Some(continuation(
                &identity,
                start + response.plugins.len(),
                total,
            ));
            // Icons and component metadata vary in size. Return a shorter page with
            // a continuation instead of rejecting an otherwise readable catalog.
            if response.encoded_len() > MAX_DISCOVERY_BYTES {
                response.extensions.pop();
                response.plugins.pop();
                if response.plugins.is_empty() {
                    return Err(Status::resource_exhausted(
                        "a plugin exceeds the discovery response bound",
                    ));
                }
                break;
            }
        }
        response.page = Some(continuation(
            &identity,
            start + response.plugins.len(),
            total,
        ));
        bounded(response)
    }

    async fn read_plugin_skill(
        &self,
        request: Request<proto::ReadPluginSkillRequest>,
    ) -> Result<Response<proto::ReadPluginSkillResponse>, Status> {
        let caller = caller_context(&request)?;
        let value = request.get_ref();
        let skill = self
            .api()?
            .skill(caller, &value.skill_id, &value.digest)
            .await
            .map_err(api_status)?;
        bounded(proto::ReadPluginSkillResponse {
            skill: Some(skill_to_proto(skill.skill)),
            digest: skill.digest,
            instructions: skill.instructions,
        })
    }

    async fn list_plugin_resources(
        &self,
        request: Request<proto::ListPluginResourcesRequest>,
    ) -> Result<Response<proto::ListPluginResourcesResponse>, Status> {
        let caller = caller_context(&request)?;
        let value = request.get_ref();
        let resources = self
            .api()?
            .resources(caller, &value.skill_id, &value.digest)
            .await
            .map_err(api_status)?;
        bounded(proto::ListPluginResourcesResponse {
            resources: resources.into_iter().map(resource_to_proto).collect(),
        })
    }

    async fn read_plugin_resource(
        &self,
        request: Request<proto::ReadPluginResourceRequest>,
    ) -> Result<Response<proto::ReadPluginResourceResponse>, Status> {
        let caller = caller_context(&request)?;
        let value = request.get_ref();
        let resource = self
            .api()?
            .resource(caller, &value.skill_id, &value.digest, &value.path)
            .await
            .map_err(api_status)?;
        bounded(proto::ReadPluginResourceResponse {
            resource: Some(proto::PluginResource {
                skill_id: resource.skill_id,
                path: resource.path,
                size_bytes: resource.size,
                text_preview_available: true,
            }),
            digest: value.digest.clone(),
            content: resource.content,
        })
    }
}

fn includes_plugins(kinds: &[i32]) -> bool {
    kinds.is_empty()
        || kinds.contains(&(proto::ExtensionKind::Unspecified as i32))
        || kinds.contains(&(proto::ExtensionKind::AgentPlugin as i32))
}

fn continuation(identity: &str, end: usize, total: usize) -> proto::PageResponse {
    proto::PageResponse {
        next_page_token: if end < total {
            format!("{identity}:{end}")
        } else {
            String::new()
        },
    }
}

fn bounded<T: prost::Message>(value: T) -> Result<Response<T>, Status> {
    if value.encoded_len() > MAX_DISCOVERY_BYTES {
        return Err(Status::resource_exhausted(
            "plugin discovery response exceeds its bound; request a smaller page",
        ));
    }
    Ok(Response::new(value))
}

fn summary(plugin: &PluginInventoryEntry) -> proto::ExtensionSummary {
    proto::ExtensionSummary {
        extension_id: format!("plugin:{}", plugin.manifest.name),
        kind: proto::ExtensionKind::AgentPlugin as i32,
        name: plugin.manifest.name.clone(),
        version: plugin.manifest.version.clone().unwrap_or_default(),
        enabled: plugin.available,
        trusted: plugin.trust.trusted,
        required_scopes: vec![scopes::EXTENSIONS_READ.into()],
    }
}

fn plugin_to_proto(plugin: PluginInventoryEntry) -> proto::AgentPlugin {
    proto::AgentPlugin {
        icon_data_url: plugin.icon_data_url.unwrap_or_default(),
        name: plugin.manifest.name,
        version: plugin.manifest.version.unwrap_or_default(),
        description: plugin.manifest.description.unwrap_or_default(),
        digest: plugin.digest,
        source: plugin.source,
        bundled: plugin.origin == PluginOrigin::Bundled,
        globally_active: plugin.status == PluginStatus::Enabled,
        available: plugin.available,
        unavailable_reason: plugin.unavailable_reason.unwrap_or_default(),
        trust: Some(proto::PluginTrust {
            trusted: plugin.trust.trusted,
            profile: plugin.trust.profile.unwrap_or_default(),
            method: plugin.trust.method,
            signer: plugin.trust.signer.unwrap_or_default(),
        }),
        skills: plugin.skills.into_iter().map(skill_to_proto).collect(),
        mcp_servers: plugin
            .mcp_servers
            .into_iter()
            .map(|server| proto::PluginMcpServer {
                id: server.id,
                name: server.name,
                enabled: server.enabled,
                status: server.status,
                transport: match server.transport {
                    PluginMcpTransport::Stdio => "stdio",
                    PluginMcpTransport::StreamableHttp => "streamable-http",
                    PluginMcpTransport::Sse => "sse",
                }
                .into(),
            })
            .collect(),
        diagnostics: plugin
            .diagnostics
            .into_iter()
            .map(|diagnostic| proto::PluginDiagnostic {
                kind: match diagnostic.kind {
                    PluginComponentKind::Plugin => "plugin",
                    PluginComponentKind::Skill => "skill",
                    PluginComponentKind::McpServer => "mcp_server",
                }
                .into(),
                name: diagnostic.name.unwrap_or_default(),
                code: diagnostic.code,
                detail: diagnostic.detail,
            })
            .collect(),
    }
}

fn skill_to_proto(skill: PluginSkillMetadata) -> proto::PluginSkill {
    proto::PluginSkill {
        id: skill.id,
        name: skill.name,
        description: skill.description,
        compatibility: skill.compatibility.unwrap_or_default(),
        allowed_tools: skill.allowed_tools.unwrap_or_default(),
    }
}

fn resource_to_proto(resource: PluginResourceEntry) -> proto::PluginResource {
    proto::PluginResource {
        skill_id: resource.skill_id,
        path: resource.path,
        size_bytes: resource.size,
        text_preview_available: resource.text,
    }
}
