//! Live profile recovery through native persistence, sidecar restarts and worker IPC.

use super::*;
use futures_util::FutureExt as _;

#[tokio::test]
#[ignore = "requires prepared debug Desktop sidecars, native key storage and loopback; no external provider request or computer input"]
async fn native_reused_provider_profiles_restart_and_expose_authenticated_inventory() {
    let parent = std::env::var_os("LOCALAPPDATA").expect("owned fixture parent");
    let root = tempfile::Builder::new()
        .prefix("ColossusProfileTest-")
        .tempdir_in(parent)
        .expect("owned fixture root");
    let home = root.path().join("home");
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).expect("fixture workspace");
    let store = SettingsStore::open_test_home(home.clone()).expect("private Desktop home");
    let mut settings = DesktopSettings {
        access_profile: AccessProfileSetting::Minimal,
        execution_boundary: crate::desktop_settings::ExecutionBoundarySetting::WorkspaceIsolated,
        providers: vec![ProviderSetting {
            profile: "fixture-provider".into(),
            kind: ProviderKindSetting::Compatible,
            base_url: provider_base_url(ProviderKindSetting::Compatible).into(),
            credential_required: false,
            credential_id: None,
            timeout_ms: Some(1_000),
        }],
        models: vec![ModelSetting {
            profile: "primary".into(),
            provider_profile: "fixture-provider".into(),
            model: "profile-fixture".into(),
            context_window_tokens: 32_768,
            max_output_tokens: 1_024,
            capabilities: ModelCapabilitiesSetting {
                tool_calls: true.into(),
                streaming: false.into(),
                image_inputs: false.into(),
                ..Default::default()
            },
            reasoning_effort: None,
        }],
        model_roles: [("primary".into(), "primary".into())].into(),
        ..DesktopSettings::default()
    };
    settings
        .add_space(validate_workspace(&workspace).expect("native workspace identity"))
        .expect("fixture workspace settings");
    let settings =
        crate::desktop_settings::normalized_settings_snapshot(&settings).expect("initial catalog");
    store
        .save(&settings)
        .expect("persist initial Minimal settings");
    let state = AppState::default();
    let result = std::panic::AssertUnwindSafe(async {
        check_profiles(&state, &store).await;
    })
    .catch_unwind()
    .await;
    let closed = state.close_all_settled().await;
    let cleaned = crate::uninstall::cleanup(&home);
    closed.expect("acknowledged sidecar cleanup");
    cleaned.expect("remove only generated runtime keys and home");
    assert!(!home.exists(), "generated profile home survived cleanup");
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

async fn check_profiles(state: &AppState, store: &SettingsStore) {
    for (index, profile) in [
        AccessProfileSetting::Minimal,
        AccessProfileSetting::Development,
        AccessProfileSetting::Minimal,
        AccessProfileSetting::AllowAll,
    ]
    .into_iter()
    .enumerate()
    {
        let mut settings = store.load().expect("reopen persisted profile settings");
        let space_id = settings.selected_space_id.clone().expect("selected space");
        let mut request = ConfigureManagedRuntimeInput {
            workspace_id: space_id.clone(),
            provider_kind: ProviderKindSetting::Compatible,
            model: "profile-fixture".into(),
            access_profile: profile,
            execution_boundary:
                crate::desktop_settings::ExecutionBoundarySetting::WorkspaceIsolated,
            replace_credential: false,
            base_url: None,
            credential_id: None,
            no_credential: false,
            model_metadata: None,
        };
        persist_reused_provider_configuration(&mut settings, &mut request, |value| {
            store.save(value)
        })
        .expect("persist legacy reused-provider change");
        let reopened = store.load().expect("read the settings the host will use");
        let resolved = crate::managed_configuration_commands::resolved_for(&reopened, &space_id)
            .expect("resolve persisted workspace catalog");
        assert_eq!(resolved.access_profile, profile);
        state.select_target(Some(space_id.clone())).await;
        tokio::time::timeout(
            Duration::from_secs(90),
            managed_runtime::start(state, store, &reopened, index > 0),
        )
        .await
        .expect("bounded real sidecar restart")
        .expect("start selected profile in the verified sidecar");
        assert!(state.managed_lifecycle_ready_for(&space_id).await);
        let worker = state
            .managed_worker_for(&space_id)
            .await
            .expect("authenticated worker");
        for (source, result) in [
            ("tools", worker.tools().await),
            ("mcp", worker.mcp_servers().await),
            ("search", worker.search_profiles().await),
            ("plugins", worker.plugins().await),
        ] {
            let inventory = result.expect("authenticated live inventory");
            assert!(
                inventory.is_array(),
                "{source} must return a real inventory"
            );
        }
    }
}
