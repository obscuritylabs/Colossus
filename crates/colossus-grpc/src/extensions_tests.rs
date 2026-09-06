use super::*;
use colossus_api::{
    ApiResult, ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, PluginSkillContent,
    RequestId,
};
use colossus_contracts::{PluginResourceRead, PluginTrustEvidence};
use std::sync::Mutex;

struct Inventory {
    calls: Mutex<Vec<bool>>,
}

fn plugin(name: &str, enabled: bool) -> PluginInventoryEntry {
    PluginInventoryEntry {
        icon_data_url: Some("data:image/png;base64,iVBORw0KGgo=".into()),
        origin: PluginOrigin::Installed,
        available: enabled,
        unavailable_reason: (!enabled).then(|| "Disabled globally".into()),
        actions: vec![],
        manifest: serde_json::from_value(
            serde_json::json!({"$schema": colossus_contracts::AGENT_PLUGIN_SCHEMA_V1, "name": name, "version": "1.0.0", "description": "fixture"}),
        )
        .expect("manifest"),
        digest: format!("sha256:{}", "a".repeat(64)),
        source: "fixture".into(),
        status: if enabled {
            PluginStatus::Enabled
        } else {
            PluginStatus::Disabled
        },
        trust: PluginTrustEvidence {
            trusted: false,
            profile: None,
            signer: None,
            method: "digest-only".into(),
        },
        skills: vec![],
        mcp_servers: vec![],
        diagnostics: vec![],
    }
}

#[tonic::async_trait]
impl ExtensionApi for Inventory {
    async fn plugins(
        &self,
        _caller: &CallerContext,
        include_disabled: bool,
    ) -> ApiResult<Vec<PluginInventoryEntry>> {
        self.calls.lock().expect("calls").push(include_disabled);
        let mut entries = vec![plugin("active", true)];
        if include_disabled {
            entries.push(plugin("disabled", false));
        }
        Ok(entries)
    }
    async fn skill(&self, _: &CallerContext, _: &str, _: &str) -> ApiResult<PluginSkillContent> {
        unreachable!("discovery never reads instructions")
    }
    async fn resources(
        &self,
        _: &CallerContext,
        _: &str,
        _: &str,
    ) -> ApiResult<Vec<PluginResourceEntry>> {
        unreachable!("discovery never reads resources")
    }
    async fn resource(
        &self,
        _: &CallerContext,
        _: &str,
        _: &str,
        _: &str,
    ) -> ApiResult<PluginResourceRead> {
        unreachable!("discovery never reads resources")
    }
}

fn request(kinds: Vec<i32>, include_disabled: bool) -> Request<proto::ListExtensionsRequest> {
    let mut request = Request::new(proto::ListExtensionsRequest {
        kinds,
        include_disabled,
        ..Default::default()
    });
    request
        .extensions_mut()
        .insert(CallerContext::authenticated(
            ApplicationPrincipal::authenticated(
                "app:plugins",
                "credential",
                ApplicationKind::Enrolled,
                [ApiScope::new(scopes::EXTENSIONS_READ).expect("scope")],
                ["primary".to_owned()],
                Vec::<String>::new(),
            )
            .expect("application"),
            RequestId::new("plugins").expect("request"),
        ));
    request
}

#[tokio::test]
async fn plugin_discovery_forwards_disabled_flag_and_treats_unspecified_as_unfiltered() {
    let inventory = Arc::new(Inventory {
        calls: Mutex::new(vec![]),
    });
    let adapter = ExtensionServiceAdapter::new(Some(inventory.clone()));
    for kinds in [
        vec![],
        vec![proto::ExtensionKind::Unspecified as i32],
        vec![proto::ExtensionKind::AgentPlugin as i32],
        vec![
            proto::ExtensionKind::Tool as i32,
            proto::ExtensionKind::Unspecified as i32,
        ],
    ] {
        for include_disabled in [false, true] {
            let response = adapter
                .list_extensions(request(kinds.clone(), include_disabled))
                .await
                .expect("discovery")
                .into_inner();
            assert_eq!(response.plugins.len(), if include_disabled { 2 } else { 1 });
            assert_eq!(
                response.plugins[0].icon_data_url,
                "data:image/png;base64,iVBORw0KGgo="
            );
            assert_eq!(response.extensions.len(), response.plugins.len());
            assert_eq!(
                inventory.calls.lock().expect("calls").last(),
                Some(&include_disabled)
            );
        }
    }
    let response = adapter
        .list_extensions(request(vec![proto::ExtensionKind::Tool as i32], true))
        .await
        .expect("filtered")
        .into_inner();
    assert!(response.plugins.is_empty());
    assert!(response.extensions.is_empty());
}

#[tokio::test]
async fn plugin_discovery_requires_an_authenticated_read_scope() {
    let inventory = Arc::new(Inventory {
        calls: Mutex::new(vec![]),
    });
    let adapter = ExtensionServiceAdapter::new(Some(inventory.clone()));
    let error = adapter
        .list_extensions(Request::new(proto::ListExtensionsRequest::default()))
        .await
        .expect_err("authentication");
    assert_eq!(error.code(), tonic::Code::Unauthenticated);
    assert!(inventory.calls.lock().expect("calls").is_empty());
}
