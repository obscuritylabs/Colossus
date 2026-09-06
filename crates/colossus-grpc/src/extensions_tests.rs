use super::*;
use colossus_api::{
    ApiResult, ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, PluginSkillContent,
    RequestId,
};
use colossus_contracts::{PluginResourceRead, PluginTrustEvidence};
use std::sync::Mutex;

struct Inventory {
    calls: Mutex<Vec<bool>>,
    entries: Vec<PluginInventoryEntry>,
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
        Ok(self
            .entries
            .iter()
            .filter(|entry| include_disabled || entry.available)
            .cloned()
            .collect())
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
        entries: vec![plugin("active", true), plugin("disabled", false)],
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
        entries: vec![plugin("active", true), plugin("disabled", false)],
    });
    let adapter = ExtensionServiceAdapter::new(Some(inventory.clone()));
    let error = adapter
        .list_extensions(Request::new(proto::ListExtensionsRequest::default()))
        .await
        .expect_err("authentication");
    assert_eq!(error.code(), tonic::Code::Unauthenticated);
    assert!(inventory.calls.lock().expect("calls").is_empty());
}

#[tokio::test]
async fn icon_heavy_discovery_pages_stay_bounded_without_skipping_plugins() {
    let entries = (0..48)
        .map(|index| {
            let mut entry = plugin(&format!("plugin-{index:03}"), true);
            // A normalized 64 KiB icon produces 87,384 base64 characters.
            entry.icon_data_url = Some(format!("data:image/png;base64,{}", "A".repeat(87_384)));
            entry.manifest.description = Some("x".repeat(4096));
            entry
        })
        .collect::<Vec<_>>();
    let expected = entries
        .iter()
        .map(|entry| entry.manifest.name.clone())
        .collect::<Vec<_>>();
    let adapter = ExtensionServiceAdapter::new(Some(Arc::new(Inventory {
        calls: Mutex::new(vec![]),
        entries,
    })));
    let mut token = String::new();
    let mut names = Vec::new();
    let mut page_count = 0;
    let mut icon_bytes = 0;
    loop {
        let mut request = request(vec![], false);
        request.get_mut().page = Some(proto::PageRequest {
            page_size: 32,
            page_token: token,
        });
        let response = adapter
            .list_extensions(request)
            .await
            .expect("bounded page")
            .into_inner();
        assert!(response.encoded_len() <= MAX_DISCOVERY_BYTES);
        assert!(!response.plugins.is_empty());
        assert!(response.plugins.len() < 32);
        assert_eq!(response.extensions.len(), response.plugins.len());
        for (summary, plugin) in response.extensions.iter().zip(&response.plugins) {
            assert_eq!(summary.name, plugin.name);
            assert!(matches!(plugin.icon_data_url.len(), 0 | 87_406));
            icon_bytes += plugin.icon_data_url.len();
        }
        names.extend(response.plugins.into_iter().map(|plugin| plugin.name));
        token = response.page.expect("continuation").next_page_token;
        page_count += 1;
        if token.is_empty() {
            break;
        }
        assert!(page_count < 4, "discovery must make progress");
    }
    assert_eq!(page_count, 2);
    assert!(icon_bytes > 0 && icon_bytes <= MAX_CATALOG_ICON_BYTES);
    assert_eq!(names, expected);
}

#[tokio::test]
async fn a_single_oversized_plugin_fails_instead_of_returning_an_empty_continuation() {
    let mut entry = plugin("oversized", true);
    entry.manifest.description = Some("a".repeat(MAX_DISCOVERY_BYTES));
    let adapter = ExtensionServiceAdapter::new(Some(Arc::new(Inventory {
        calls: Mutex::new(vec![]),
        entries: vec![entry],
    })));
    let error = adapter
        .list_extensions(request(vec![], false))
        .await
        .expect_err("single oversized entry");
    assert_eq!(error.code(), tonic::Code::ResourceExhausted);
}
