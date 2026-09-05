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
        api.plugins(&unauthorized).await.expect_err("scope").code,
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
    let plugins = api.plugins(&authorized).await.expect("catalog");
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
        api.plugins(&authorized)
            .await
            .expect("fresh public catalog")
            .is_empty()
    );
    assert!(!runtime.plugin_inventory().expect("management inventory")[0].available);
    assert!(api.skill(&authorized, id, digest).await.is_err());
}
