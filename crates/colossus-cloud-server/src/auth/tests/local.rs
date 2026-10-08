use super::*;
use colossus_cloud::{
    CloudProject, CloudUser, LocalCredential, LoginIdentityMetadata, ProjectRole, UserAccount,
};

const PASSWORD: &str = "fixture local password 2026";

#[tokio::test]
async fn bootstrap_existing_oidc_account_adds_local_binding_once_without_resetting_credentials() {
    const ISOLATED: &str = "COLOSSUS_BOOTSTRAP_TEST_ISOLATED";
    const SECRET: &str = "COLOSSUS_BOOTSTRAP_TEST_PASSWORD";
    if std::env::var_os(ISOLATED).is_none() {
        // A child owns its secret environment before any threads start; tests never mutate the process environment.
        let output=tokio::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","auth::tests::local::bootstrap_existing_oidc_account_adds_local_binding_once_without_resetting_credentials","--test-threads=1"]).env(ISOLATED,"1").env(SECRET,PASSWORD).output().await.unwrap();
        assert!(
            output.status.success(),
            "isolated bootstrap regression failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let (prior, _, server) = fixture().await;
    let original = prior
        .repository()
        .unwrap()
        .user_memberships(&colossus_cloud::hash_identity(&[
            "oidc-user",
            &prior.config.oidc.as_ref().unwrap().issuer,
            "alice",
        ]))
        .await
        .unwrap()
        .remove(0);
    let mut config = prior.config.clone();
    config.local_auth = Some(crate::config::LocalAuthConfig {
        session_seconds: 3600,
    });
    config.bootstrap_admin = Some(crate::config::BootstrapAdmin {
        display_name: "Existing administrator".into(),
        email: None,
        oidc_subject: Some("alice".into()),
        username: Some("promoted-admin".into()),
        password_variable: Some(SECRET.into()),
    });
    let promoted = Authentication::with_store(config.clone(), prior.store.clone())
        .await
        .unwrap();
    let headers = local_cookie(&promoted, "promoted-admin", PASSWORD).await;
    let user = promoted.admin(&headers, false).await.unwrap();
    assert_eq!(user.id, original.user_id);
    assert!(
        user.identities
            .iter()
            .any(|identity| identity.username.as_deref() == Some("promoted-admin"))
    );
    let revision = promoted
        .repository()
        .unwrap()
        .account(&user.id)
        .await
        .unwrap()
        .user
        .revision;
    config.bootstrap_admin.as_mut().unwrap().password_variable =
        Some("MISSING_BOOTSTRAP_SECRET_MUST_NOT_BE_READ".into());
    let restarted = Authentication::with_store(config, prior.store.clone())
        .await
        .unwrap();
    assert!(restarted.admin(&headers, false).await.is_ok());
    assert_eq!(
        restarted
            .repository()
            .unwrap()
            .account(&user.id)
            .await
            .unwrap()
            .user
            .revision,
        revision
    );
    server.abort();
}
async fn fixture_local() -> (Authentication, tokio::task::JoinHandle<()>) {
    let (oidc, _, server) = fixture().await;
    let mut config = oidc.config.clone();
    config.oidc = None;
    config.memberships.clear();
    config.local_auth = Some(crate::config::LocalAuthConfig {
        session_seconds: 3600,
    });
    let auth = Authentication::with_store(config, oidc.store.clone())
        .await
        .unwrap();
    for (id, username, admin) in [
        ("local-admin", "administrator", true),
        ("local-user", "operator", false),
    ] {
        let hash = auth
            .password_hash(Zeroizing::new(PASSWORD.into()))
            .await
            .unwrap();
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        let account = UserAccount {
            user: CloudUser {
                id: id.into(),
                display_name: username.into(),
                email: None,
                active: true,
                is_admin: admin,
                revision: 1,
                created_at: now.clone(),
                updated_at: now,
                identities: vec![LoginIdentityMetadata {
                    kind: "local".into(),
                    label: "Local account".into(),
                    username: Some(username.into()),
                    issuer: None,
                    subject: None,
                }],
            },
            security_epoch: 1,
        };
        auth.repository()
            .unwrap()
            .create_account(
                "fixture-provisioning",
                account,
                None,
                Some(LocalCredential {
                    user_id: id.into(),
                    username: username.into(),
                    password_hash: hash,
                }),
            )
            .await
            .unwrap();
    }
    (auth, server)
}
fn csrf() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("origin", HeaderValue::from_static("http://127.0.0.1:5180"));
    headers.insert("x-colossus-csrf", HeaderValue::from_static("1"));
    headers
}
async fn local_cookie(auth: &Authentication, username: &str, password: &str) -> HeaderMap {
    let mut headers = csrf();
    let cookie = auth
        .local_login(&headers, username.into(), Zeroizing::new(password.into()))
        .await
        .unwrap();
    headers.insert(
        "cookie",
        HeaderValue::from_str(cookie.to_str().unwrap().split(';').next().unwrap()).unwrap(),
    );
    headers
}
#[tokio::test]
async fn local_password_sessions_are_revoked_across_replicas_and_disabled_users_cannot_return() {
    let (auth, server) = fixture_local().await;
    assert_eq!(
        auth.local_login(
            &HeaderMap::new(),
            "operator".into(),
            Zeroizing::new(PASSWORD.into())
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        auth.local_login(
            &csrf(),
            "operator".into(),
            Zeroizing::new("incorrect password".into())
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        auth.local_login(
            &csrf(),
            "unknown-user".into(),
            Zeroizing::new("incorrect password".into())
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    let headers = local_cookie(&auth, "OPERATOR", PASSWORD).await;
    assert_eq!(auth.user(&headers).await.unwrap().id, "local-user");
    assert!(auth.admin(&headers, false).await.is_err());
    let replica = Authentication::with_store(auth.config.clone(), auth.store.clone())
        .await
        .unwrap();
    assert!(replica.user(&headers).await.is_ok());
    let admin_headers = local_cookie(&auth, "administrator", PASSWORD).await;
    let admin = auth.admin(&admin_headers, true).await.unwrap();
    let caller = auth
        .caller(&admin_headers, "project-a", true)
        .await
        .unwrap();
    assert_eq!(
        caller.require(CloudPermission::Execute),
        Err(CloudError::PermissionDenied)
    );
    assert_eq!(
        caller.require(CloudPermission::Approve),
        Err(CloudError::PermissionDenied)
    );
    let repo = auth.repository().unwrap();
    repo.save_membership(&caller, "local-user", ProjectRole::ProjectAdmin, 0)
        .await
        .unwrap();
    assert!(
        auth.caller(&headers, "project-a", true)
            .await
            .unwrap()
            .require(CloudPermission::Execute)
            .is_ok()
    );
    assert_eq!(
        auth.caller(&headers, "project-a", true)
            .await
            .unwrap()
            .require(CloudPermission::Approve),
        Err(CloudError::PermissionDenied)
    );
    let child = CloudProject {
        id: "private-child".into(),
        name: "Private child".into(),
        description: String::new(),
        parent_project_id: Some("project-a".into()),
        archived: false,
        revision: 0,
        created_at: String::new(),
        updated_at: String::new(),
    };
    repo.save_project(&admin, child).await.unwrap();
    assert_eq!(
        auth.caller(&headers, "private-child", false)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    let hash = auth
        .password_hash(Zeroizing::new("rotated fixture password 2026".into()))
        .await
        .unwrap();
    let user = repo.account("local-user").await.unwrap().user;
    let updated = repo
        .reset_local_password(&admin, "local-user", user.revision, hash)
        .await
        .unwrap();
    assert_eq!(
        auth.user(&headers).await.unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        replica.user(&headers).await.unwrap_err(),
        CloudError::PermissionDenied
    );
    let fresh = local_cookie(&replica, "operator", "rotated fixture password 2026").await;
    let mut disabled = updated;
    disabled.active = false;
    let mut disabled = repo.update_account(&admin, disabled).await.unwrap();
    assert_eq!(
        replica.user(&fresh).await.unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        auth.local_login(
            &csrf(),
            "operator".into(),
            Zeroizing::new("rotated fixture password 2026".into())
        )
        .await
        .unwrap_err(),
        CloudError::PermissionDenied
    );
    disabled.active = true;
    repo.update_account(&admin, disabled).await.unwrap();
    assert_eq!(
        replica.user(&fresh).await.unwrap_err(),
        CloudError::PermissionDenied
    );
    let mut last_admin = admin.clone();
    last_admin.active = false;
    assert_eq!(
        repo.update_account(&admin, last_admin).await.unwrap_err(),
        CloudError::Conflict
    );
    let admin = auth.admin(&admin_headers, false).await.unwrap();
    let mut changed = admin.clone();
    changed.is_admin = false;
    assert_eq!(
        repo.update_account(&admin, changed).await.unwrap_err(),
        CloudError::Conflict
    );
    server.abort();
}
#[tokio::test]
async fn local_password_inputs_and_shared_attempt_budgets_are_bounded() {
    let (auth, server) = fixture_local().await;
    let now = crate::http::now();
    assert!(
        auth.password_hash(Zeroizing::new("界".repeat(5)))
            .await
            .is_err()
    );
    assert!(
        auth.password_hash(Zeroizing::new("a".repeat(1025)))
            .await
            .is_err()
    );
    for _ in 0..8 {
        assert_eq!(
            auth.local_login_at(
                &csrf(),
                "rate-limited".into(),
                Zeroizing::new("incorrect password".into()),
                now,
            )
            .await
            .unwrap_err(),
            CloudError::PermissionDenied
        );
    }
    let replica = Authentication::with_store(auth.config.clone(), auth.store.clone())
        .await
        .unwrap();
    assert_eq!(
        replica
            .local_login_at(
                &csrf(),
                "rate-limited".into(),
                Zeroizing::new("incorrect password".into()),
                now,
            )
            .await
            .unwrap_err(),
        CloudError::ResourceExhausted
    );
    assert_eq!(
        replica
            .local_login_at(
                &csrf(),
                "rate-limited".into(),
                Zeroizing::new("incorrect password".into()),
                now + 60,
            )
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    server.abort();
}

#[tokio::test]
async fn exhausted_global_login_budget_does_not_create_new_username_buckets() {
    let (auth, server) = fixture_local().await;
    let now = crate::http::now();
    let bucket = (now / 60).to_string();
    let global = flow_key(&colossus_cloud::hash_identity(&[
        "local-login-global",
        &bucket,
    ]));
    auth.store
        .commit(CloudTransaction {
            entities: vec![EntityMutation {
                key: global,
                expected_revision: 0,
                value: colossus_cloud::storage::EntityValue::AuthFlow(
                    json!({"attempts":120,"expires_at":now+600}),
                ),
                actor: "fixture".into(),
                operation: "fixture.auth-budget".into(),
            }],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        auth.local_login(
            &csrf(),
            "new-username".into(),
            Zeroizing::new(PASSWORD.into())
        )
        .await
        .unwrap_err(),
        CloudError::ResourceExhausted
    );
    let personal = flow_key(&colossus_cloud::hash_identity(&[
        "local-login",
        "new-username",
        &bucket,
    ]));
    assert!(matches!(
        auth.store.read(&personal).await,
        Err(CloudError::NotFound)
    ));
    server.abort();
}

#[tokio::test]
async fn password_rotation_between_credential_read_and_verification_cannot_mint_new_epoch_cookie() {
    let (auth, server) = fixture_local().await;
    let key = colossus_cloud::identity_key(
        EntityKind::LocalCredential,
        &colossus_cloud::hash_identity(&["operator"]),
    );
    let original = auth.store.read(&key).await.unwrap();
    let mut credential: LocalCredential = LocalCredential::try_from(original.value).unwrap();
    credential.password_hash = auth
        .password_hash(Zeroizing::new("concurrently rotated password".into()))
        .await
        .unwrap();
    let mut account = auth
        .repository()
        .unwrap()
        .account("local-user")
        .await
        .unwrap();
    let version = account.user.revision;
    account.security_epoch += 1;
    account.user.revision += 1;
    let transaction = CloudTransaction {
        entities: vec![
            EntityMutation {
                key: key.clone(),
                expected_revision: original.revision,
                value: credential.into(),
                actor: "local-admin".into(),
                operation: "fixture.password-rotation".into(),
            },
            EntityMutation {
                key: colossus_cloud::identity_key(EntityKind::User, "local-user"),
                expected_revision: version,
                value: account.into(),
                actor: "local-admin".into(),
                operation: "fixture.password-rotation".into(),
            },
        ],
        ..Default::default()
    };
    let injected: Arc<dyn CloudStore> = Arc::new(super::markers::MarkerFailure::rotating(
        auth.store.clone(),
        key,
        transaction,
    ));
    let raced = Authentication::with_store(auth.config.clone(), injected)
        .await
        .unwrap();
    assert_eq!(
        raced
            .local_login(&csrf(), "operator".into(), Zeroizing::new(PASSWORD.into()))
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    assert!(
        raced
            .local_login(
                &csrf(),
                "operator".into(),
                Zeroizing::new("concurrently rotated password".into())
            )
            .await
            .is_ok()
    );
    server.abort();
}
