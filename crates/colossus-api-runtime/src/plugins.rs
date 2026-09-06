//! Public plugin reads preserve application attribution and the runtime policy boundary.

use async_trait::async_trait;
use colossus_api::{
    ApiError, ApiErrorReason, ApiResult, CallerContext, ExtensionApi, PluginSkillContent, scopes,
};
use colossus_contracts::{
    PluginInventoryEntry, PluginOperation, PluginResourceEntry, PluginResourceRead,
    PluginSkillMetadata, PluginSkillRecord,
};
use colossus_runtime::Runtime;
use serde::de::DeserializeOwned;
use std::sync::Arc;

/// Authenticated read-only adapter for the selected workspace's effective catalog.
pub struct RuntimeExtensionApi {
    runtime: Arc<Runtime>,
}

impl RuntimeExtensionApi {
    /// Bind one already owned runtime; never chooses an ambient Colossus home.
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self { runtime }
    }

    async fn read<T: DeserializeOwned>(
        &self,
        caller: &CallerContext,
        operation: PluginOperation,
        digest: Option<&str>,
    ) -> ApiResult<T> {
        caller.require_scope(scopes::EXTENSIONS_READ)?;
        match &operation {
            PluginOperation::SkillRead { skill_id }
            | PluginOperation::ListResources { skill_id }
            | PluginOperation::ReadResource { skill_id, .. }
                if !colossus_contracts::valid_plugin_skill_id(skill_id) =>
            {
                return Err(ApiError::invalid(
                    ApiErrorReason::InvalidArgument,
                    "skill_id",
                    "select a qualified plugin/skill identifier",
                ));
            }
            _ => {}
        }
        if let Some(digest) = digest
            && (digest.len() != 71
                || !digest.starts_with("sha256:")
                || !digest[7..]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
        {
            return Err(ApiError::invalid(
                ApiErrorReason::InvalidArgument,
                "digest",
                "an exact sha256 OCI manifest digest is required",
            ));
        }
        let value = self
            .runtime
            .read_plugin_as(operation, digest, caller.actor())
            .await
            .map_err(plugin_read_error)?;
        serde_json::from_value(value).map_err(|_| {
            ApiError::failed_precondition(
                ApiErrorReason::InternalInvariant,
                "plugin response failed validation",
            )
        })
    }
}

fn plugin_read_error(error: colossus_runtime::RuntimeError) -> ApiError {
    match error {
        colossus_runtime::RuntimeError::Gateway(
            colossus_policy::GatewayError::Denied(_) | colossus_policy::GatewayError::Approval(_),
        ) => ApiError::permission_denied(
            ApiErrorReason::ToolDenied,
            "workspace policy did not authorize this plugin read",
        ),
        _ => ApiError::failed_precondition(
            ApiErrorReason::InvalidArgument,
            "plugin content is unavailable; refresh the inventory",
        ),
    }
}

#[async_trait]
impl ExtensionApi for RuntimeExtensionApi {
    async fn plugins(
        &self,
        caller: &CallerContext,
        include_disabled: bool,
    ) -> ApiResult<Vec<PluginInventoryEntry>> {
        caller.require_scope(scopes::EXTENSIONS_READ)?;
        let mut entries: Vec<PluginInventoryEntry> = if include_disabled {
            let value = self
                .runtime
                .read_plugin_inventory_as(caller.actor())
                .await
                .map_err(plugin_read_error)?;
            serde_json::from_value(value).map_err(|_| {
                ApiError::failed_precondition(
                    ApiErrorReason::InternalInvariant,
                    "plugin response failed validation",
                )
            })?
        } else {
            self.read(caller, PluginOperation::List, None).await?
        };
        for entry in &mut entries {
            // Public discovery cannot grant local management or disclose opaque client data.
            entry.actions.clear();
            entry.manifest.extensions.clear();
        }
        Ok(entries)
    }

    async fn skill(
        &self,
        caller: &CallerContext,
        id: &str,
        digest: &str,
    ) -> ApiResult<PluginSkillContent> {
        let skill: PluginSkillRecord = self
            .read(
                caller,
                PluginOperation::SkillRead {
                    skill_id: id.into(),
                },
                Some(digest),
            )
            .await?;
        Ok(PluginSkillContent {
            digest: digest.into(),
            instructions: skill.instructions,
            skill: PluginSkillMetadata {
                id: skill.id,
                plugin: skill.plugin,
                name: skill.manifest.name,
                description: skill.manifest.description,
                compatibility: skill.manifest.compatibility,
                allowed_tools: skill.manifest.allowed_tools,
            },
        })
    }

    async fn resources(
        &self,
        caller: &CallerContext,
        id: &str,
        digest: &str,
    ) -> ApiResult<Vec<PluginResourceEntry>> {
        self.read(
            caller,
            PluginOperation::ListResources {
                skill_id: id.into(),
            },
            Some(digest),
        )
        .await
    }

    async fn resource(
        &self,
        caller: &CallerContext,
        id: &str,
        digest: &str,
        path: &str,
    ) -> ApiResult<PluginResourceRead> {
        self.read(
            caller,
            PluginOperation::ReadResource {
                skill_id: id.into(),
                path: path.into(),
            },
            Some(digest),
        )
        .await
    }
}
