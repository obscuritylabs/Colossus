use super::RuntimeExtensionApi;
use colossus_api::{
    ApiErrorCode, ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext, ExtensionApi,
    RequestId, scopes,
};
use colossus_policy::DenyApproval;
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeOpenOptions, StorageAdapter};
use std::{fs, sync::Arc};

fn caller(authorized: bool) -> CallerContext {
    CallerContext::authenticated(
        ApplicationPrincipal::authenticated(
            "app:plugin-test",
            "test-credential",
            ApplicationKind::Enrolled,
            authorized.then(|| ApiScope::new(scopes::EXTENSIONS_READ).expect("scope")),
            ["primary".to_owned()],
            Vec::<String>::new(),
        )
        .expect("application"),
        RequestId::new("plugin-reads").expect("request"),
    )
}

#[tokio::test]
async fn public_plugin_reads_require_scope_pin_digests_and_never_release_credentials_or_roots() {
    let temporary = crate::service_tests::runtime_tempdir();
    let root = temporary.path().canonicalize().expect("canonical root");
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).expect("workspace");
    let home = root.join("home");
    fs::create_dir(&home).expect("home");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).expect("owner-private home");
    }
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.storage.adapter = StorageAdapter::Ephemeral;
    let runtime = Arc::new(
        Runtime::open_with_options(
            &config,
            Arc::new(DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(&workspace)
                .expect("workspace binding")
                .with_colossus_home(&home)
                .expect("explicit home"),
        )
        .expect("runtime"),
    );
    let api = RuntimeExtensionApi::new(Arc::clone(&runtime));
    let unauthorized = caller(false);
    assert_eq!(
        api.plugins(&unauthorized, false)
            .await
            .expect_err("scope")
            .code,
        ApiErrorCode::PermissionDenied
    );
    assert_eq!(
        api.skill(&unauthorized, "invalid", "invalid")
            .await
            .expect_err("scope before input")
            .code,
        ApiErrorCode::PermissionDenied
    );
    let authorized = caller(true);
    assert_eq!(
        api.plugins(&unauthorized, true)
            .await
            .expect_err("inventory scope")
            .code,
        ApiErrorCode::PermissionDenied
    );
    let plugins = api.plugins(&authorized, false).await.expect("catalog");
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].skills.len(), 4);
    assert!(plugins[0].actions.is_empty());
    let encoded = serde_json::to_string(&plugins).expect("inventory");
    assert!(!encoded.contains("instructions"));
    assert!(!encoded.contains(&root.display().to_string()));
    let id = "colossus/plugin-authoring";
    let digest = &plugins[0].digest;
    let skill = api
        .skill(&authorized, id, digest)
        .await
        .expect("explicit instruction read");
    assert_eq!(&skill.digest, digest);
    assert!(!skill.instructions.is_empty());
    assert!(
        !serde_json::to_string(&skill)
            .expect("public skill")
            .contains(&root.display().to_string())
    );
    assert_eq!(
        api.skill(&authorized, id, &format!("sha256:{}", "0".repeat(64)))
            .await
            .expect_err("exact digest")
            .code,
        ApiErrorCode::FailedPrecondition
    );
    assert_eq!(
        api.skill(&authorized, "coding", digest)
            .await
            .expect_err("qualified identity")
            .code,
        ApiErrorCode::InvalidArgument
    );
    let resources = api
        .resources(&authorized, id, digest)
        .await
        .expect("resources");
    assert!(
        resources
            .iter()
            .any(|entry| entry.path.ends_with("plugin.json"))
    );
    let reference = resources
        .iter()
        .find(|entry| entry.text)
        .expect("text resource");
    assert!(
        !api.resource(&authorized, id, digest, &reference.path)
            .await
            .expect("bounded preview")
            .content
            .is_empty()
    );
    assert!(
        api.resource(&authorized, id, digest, "../../../../state.redb")
            .await
            .is_err()
    );
    runtime
        .manage_plugin(colossus_contracts::PluginManagementRequest::Disable {
            name: "colossus".into(),
        })
        .await
        .expect("disable");
    assert!(
        api.plugins(&authorized, false)
            .await
            .expect("fresh public catalog")
            .is_empty()
    );
    assert!(!runtime.plugin_inventory().expect("management inventory")[0].available);
    let disabled = api
        .plugins(&authorized, true)
        .await
        .expect("live disabled inventory");
    assert_eq!(disabled.len(), 1);
    assert!(!disabled[0].available);
    assert_ne!(
        disabled[0].status,
        colossus_contracts::PluginStatus::Enabled
    );
    assert!(disabled[0].actions.is_empty());
    assert!(disabled[0].manifest.extensions.is_empty());
    let encoded = serde_json::to_string(&disabled).expect("disabled metadata");
    assert!(!encoded.contains("instructions"));
    assert!(!encoded.contains(&root.display().to_string()));
    assert!(api.skill(&authorized, id, digest).await.is_err());
    runtime
        .manage_plugin(colossus_contracts::PluginManagementRequest::Enable {
            name: "colossus".into(),
            digest: digest.clone(),
            allow_untrusted: false,
        })
        .await
        .expect("restore global core activation");
    let excluded_workspace = root.join("excluded-workspace");
    fs::create_dir(&excluded_workspace).expect("excluded workspace");
    config.plugins.exclude = vec!["colossus".into()];
    let excluded = RuntimeExtensionApi::new(Arc::new(
        Runtime::open_with_options(
            &config,
            Arc::new(DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(&excluded_workspace)
                .expect("workspace binding")
                .with_colossus_home(&home)
                .expect("shared home"),
        )
        .expect("excluded workspace runtime"),
    ));
    assert!(
        excluded
            .plugins(&authorized, false)
            .await
            .expect("effective catalog")
            .is_empty()
    );
    let unavailable = excluded
        .plugins(&authorized, true)
        .await
        .expect("workspace-unavailable inventory");
    assert_eq!(unavailable.len(), 1);
    assert_eq!(
        unavailable[0].status,
        colossus_contracts::PluginStatus::Enabled
    );
    assert!(!unavailable[0].available);
    assert!(
        unavailable[0]
            .unavailable_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("excluded"))
    );
    assert!(excluded.skill(&authorized, id, digest).await.is_err());
}
