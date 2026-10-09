//! Audited one-time migration and explicit first-administrator provisioning.
use super::*;
use colossus_cloud::{
    CloudPermission, CloudProject, CloudRepository, CloudUser, IDENTITY_NAMESPACE, LocalCredential,
    LoginIdentityMetadata, OidcIdentity, ProjectMembership, ProjectRole, UserAccount,
    hash_identity, identity_key, membership_key, normalize_username, project_key,
};
fn timestamp() -> CloudResult<String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| CloudError::Storage)
}
fn operation(
    key: EntityKey,
    revision: u64,
    value: colossus_cloud::storage::EntityValue,
    name: &str,
) -> EntityMutation {
    EntityMutation {
        key,
        expected_revision: revision,
        value,
        actor: "operator-bootstrap".into(),
        operation: name.into(),
    }
}
fn metadata(
    oidc: Option<&crate::config::OidcConfig>,
    subject: Option<&str>,
    username: Option<&str>,
) -> Vec<LoginIdentityMetadata> {
    let mut identities = Vec::new();
    if let Some(username) = username {
        identities.push(LoginIdentityMetadata {
            kind: "local".into(),
            label: "Local account".into(),
            username: Some(username.into()),
            issuer: None,
            subject: None,
        });
    }
    if let (Some(provider), Some(subject)) = (oidc, subject) {
        identities.push(LoginIdentityMetadata {
            kind: "oidc".into(),
            label: provider.label.clone(),
            username: None,
            issuer: Some(provider.issuer.clone()),
            subject: Some(subject.into()),
        });
    }
    identities
}
pub(super) async fn seed_accounts(store: &Arc<dyn CloudStore>, config: &Config) -> CloudResult<()> {
    let repo = CloudRepository::new(store.clone())?;
    // Explicit bootstrap may be supplied later to upgrade a previously configured-only deployment.
    bootstrap_administrator(&repo, config).await?;
    let marker = identity_key(EntityKind::Setting, "identity-bootstrap-v3");
    match store.read(&marker).await {
        Ok(_) => return Ok(()),
        Err(CloudError::NotFound) => {}
        Err(error) => return Err(error),
    }
    for member in &config.memberships {
        let provider = config.oidc.as_ref().ok_or(CloudError::InvalidArgument)?;
        let user_id = hash_identity(&["oidc-user", &provider.issuer, &member.subject]);
        let now = timestamp()?;
        if matches!(repo.account(&user_id).await, Err(CloudError::NotFound)) {
            let account = UserAccount {
                user: CloudUser {
                    id: user_id.clone(),
                    display_name: member.subject.clone(),
                    email: None,
                    active: true,
                    is_admin: false,
                    revision: 1,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    identities: metadata(Some(provider), Some(&member.subject), None),
                },
                security_epoch: 1,
            };
            let binding = OidcIdentity {
                user_id: user_id.clone(),
                issuer: provider.issuer.clone(),
                subject: member.subject.clone(),
            };
            match repo
                .create_account("operator-bootstrap", account, Some(binding), None)
                .await
            {
                Ok(_) | Err(CloudError::Conflict) => {}
                Err(e) => return Err(e),
            }
        }
        if matches!(
            store.read(&project_key(&member.project_id)).await,
            Err(CloudError::NotFound)
        ) {
            let project = CloudProject {
                id: member.project_id.clone(),
                name: member.project_id.clone(),
                description: String::new(),
                parent_project_id: None,
                archived: false,
                revision: 1,
                created_at: now.clone(),
                updated_at: now,
            };
            match store
                .commit(CloudTransaction {
                    entities: vec![operation(
                        project_key(&project.id),
                        0,
                        project.into(),
                        "cloud.project.bootstrap.v3",
                    )],
                    ..Default::default()
                })
                .await
            {
                Ok(()) | Err(colossus_ports::StoreError::Conflict { .. }) => {}
                Err(e) => return Err(e.into()),
            }
        }
        let key = membership_key(&member.project_id, &user_id);
        if matches!(store.read(&key).await, Err(CloudError::NotFound)) {
            let role = if member.permissions.contains(&CloudPermission::Administer) {
                ProjectRole::ProjectAdmin
            } else if member.permissions.contains(&CloudPermission::Approve) {
                ProjectRole::Approver
            } else if member.permissions.contains(&CloudPermission::Execute) {
                ProjectRole::Operator
            } else {
                ProjectRole::Viewer
            };
            let value = ProjectMembership {
                project_id: member.project_id.clone(),
                user_id: user_id.clone(),
                subject: user_id,
                role,
                permissions: member.permissions.clone(),
                revision: 1,
            };
            match store
                .commit(CloudTransaction {
                    entities: vec![operation(
                        key,
                        0,
                        value.into(),
                        "cloud.membership.bootstrap.v3",
                    )],
                    ..Default::default()
                })
                .await
            {
                Ok(()) | Err(colossus_ports::StoreError::Conflict { .. }) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    let users = store
        .list(&colossus_cloud::storage::EntityQuery::new(
            EntityKind::User,
            IDENTITY_NAMESPACE.into(),
        ))
        .await?;
    if users.is_empty() {
        return Err(CloudError::PermissionDenied);
    }
    match store
        .commit(CloudTransaction {
            entities: vec![operation(
                marker,
                0,
                colossus_cloud::storage::BootstrapMarker {
                    completed: true,
                    version: 3,
                }
                .into(),
                "cloud.identity.bootstrap-completed.v3",
            )],
            ..Default::default()
        })
        .await
    {
        Ok(()) | Err(colossus_ports::StoreError::Conflict { .. }) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
async fn bootstrap_administrator(repo: &CloudRepository, config: &Config) -> CloudResult<()> {
    let Some(bootstrap) = &config.bootstrap_admin else {
        return Ok(());
    };
    let marker = identity_key(EntityKind::Setting, "administrator-bootstrap-v3");
    match repo.storage().read(&marker).await {
        Ok(_) => return Ok(()),
        Err(CloudError::NotFound) => {}
        Err(error) => return Err(error),
    }
    let mut administrators =
        colossus_cloud::storage::EntityQuery::new(EntityKind::User, IDENTITY_NAMESPACE.into());
    administrators.status = Some("administrator".into());
    administrators.limit = 1;
    if !repo.storage().list(&administrators).await?.is_empty() {
        return Ok(());
    }
    let username = bootstrap
        .username
        .as_deref()
        .map(normalize_username)
        .transpose()?;
    let provider = config.oidc.as_ref();
    let binding = bootstrap
        .oidc_subject
        .as_ref()
        .map(|subject| {
            let provider = provider.ok_or(CloudError::InvalidArgument)?;
            Ok::<_, CloudError>((provider, subject))
        })
        .transpose()?;
    let id = if let Some((provider, subject)) = binding {
        hash_identity(&["oidc-user", &provider.issuer, subject])
    } else if let Some(username) = &username {
        hash_identity(&["bootstrap-local", username])
    } else {
        return Err(CloudError::InvalidArgument);
    };
    let now = timestamp()?;
    let existing = match repo.account(&id).await {
        Ok(account) => Some(account),
        Err(CloudError::NotFound) => None,
        Err(error) => return Err(error),
    };
    if let Some(mut account) = existing {
        let revision = account.user.revision;
        account.user.is_admin = true;
        account.user.updated_at = now;
        let mut bindings = Vec::new();
        if let Some(username) = &username {
            let key = identity_key(EntityKind::LocalCredential, &hash_identity(&[username]));
            match repo.storage().read(&key).await {
                Ok(record) => {
                    let credential: LocalCredential = record.value.try_into()?;
                    if credential.user_id != id {
                        return Err(CloudError::Conflict);
                    }
                }
                Err(CloudError::NotFound) => {
                    let credential =
                        bootstrap_local_credential(config, bootstrap, &id, username).await?;
                    bindings.push(operation(
                        key,
                        0,
                        credential.into(),
                        "cloud.user.bootstrap-local-bound.v3",
                    ));
                    account
                        .user
                        .identities
                        .extend(metadata(None, None, Some(username)));
                }
                Err(error) => return Err(error),
            }
        }
        bindings.push(operation(
            identity_key(EntityKind::User, &id),
            revision,
            account.into(),
            "cloud.user.bootstrap-administrator.v3",
        ));
        match repo
            .storage()
            .commit(CloudTransaction {
                entities: bindings,
                ..Default::default()
            })
            .await
        {
            Ok(()) | Err(colossus_ports::StoreError::Conflict { .. }) => {}
            Err(error) => return Err(error.into()),
        };
    } else {
        let account = UserAccount {
            user: CloudUser {
                id: id.clone(),
                display_name: bootstrap.display_name.clone(),
                email: bootstrap.email.clone(),
                active: true,
                is_admin: true,
                revision: 1,
                created_at: now.clone(),
                updated_at: now,
                identities: metadata(
                    provider,
                    bootstrap.oidc_subject.as_deref(),
                    username.as_deref(),
                ),
            },
            security_epoch: 1,
        };
        let oidc = binding.map(|(provider, subject)| OidcIdentity {
            user_id: id.clone(),
            issuer: provider.issuer.clone(),
            subject: subject.clone(),
        });
        let local = if let Some(username) = username {
            Some(bootstrap_local_credential(config, bootstrap, &id, &username).await?)
        } else {
            None
        };
        match repo
            .create_account("operator-bootstrap", account, oidc, local)
            .await
        {
            Ok(_) | Err(CloudError::Conflict) => {}
            Err(e) => return Err(e),
        }
    }
    let account = repo.account(&id).await?;
    if !account.user.is_admin || !account.user.active {
        return Err(CloudError::PermissionDenied);
    }
    match repo
        .storage()
        .commit(CloudTransaction {
            entities: vec![operation(
                marker,
                0,
                colossus_cloud::storage::BootstrapMarker {
                    completed: true,
                    version: 3,
                }
                .into(),
                "cloud.administrator.bootstrap-completed.v3",
            )],
            ..Default::default()
        })
        .await
    {
        Ok(()) | Err(colossus_ports::StoreError::Conflict { .. }) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

async fn bootstrap_local_credential(
    config: &Config,
    bootstrap: &crate::config::BootstrapAdmin,
    user: &str,
    username: &str,
) -> CloudResult<LocalCredential> {
    if config.local_auth.is_none() {
        return Err(CloudError::InvalidArgument);
    }
    let variable = bootstrap
        .password_variable
        .as_ref()
        .ok_or(CloudError::InvalidArgument)?;
    let password =
        Zeroizing::new(std::env::var(variable).map_err(|_| CloudError::PermissionDenied)?);
    let password_hash =
        super::local::hash_password(Arc::new(tokio::sync::Semaphore::new(1)), password).await?;
    Ok(LocalCredential {
        user_id: user.into(),
        username: username.into(),
        password_hash,
    })
}
