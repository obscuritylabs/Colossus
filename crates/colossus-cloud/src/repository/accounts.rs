use super::*;
use crate::{
    CloudUser, IDENTITY_NAMESPACE, LocalCredential, OidcIdentity, ProjectMembership, ProjectRole,
    UserAccount, hash_identity, identity_key, membership_key, validate_user,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub(super) fn timestamp() -> CloudResult<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| CloudError::Storage)
}
pub(super) fn mutation<T: Serialize>(
    actor: &str,
    key: EntityKey,
    revision: u64,
    operation: &str,
    value: &T,
) -> CloudResult<EntityMutation> {
    Ok(EntityMutation {
        key,
        expected_revision: revision,
        value: serde_json::to_value(value).map_err(|_| CloudError::InvalidArgument)?,
        actor: actor.into(),
        operation: operation.into(),
    })
}
pub(super) fn administrator(user: &CloudUser) -> CloudResult<()> {
    if user.active && user.is_admin {
        Ok(())
    } else {
        Err(CloudError::PermissionDenied)
    }
}

impl CloudRepository {
    /// Host-only identity lookup after authentication; never releases credentials.
    pub async fn account(&self, id: &str) -> CloudResult<UserAccount> {
        crate::validate_identifier(id)?;
        let record = self.store.read(&identity_key(EntityKind::User, id)).await?;
        let mut account: UserAccount =
            serde_json::from_value(record.value).map_err(|_| CloudError::Storage)?;
        if account.user.id != id {
            return Err(CloudError::Storage);
        }
        account.user.revision = record.revision;
        Ok(account)
    }
    /// Atomically provision an explicit account and its selected login bindings.
    pub async fn create_account(
        &self,
        actor: &str,
        mut account: UserAccount,
        oidc: Option<OidcIdentity>,
        local: Option<LocalCredential>,
    ) -> CloudResult<CloudUser> {
        validate_user(&account.user)?;
        if oidc.is_none() && local.is_none() {
            return Err(CloudError::InvalidArgument);
        }
        account.user.revision = 1;
        let mut entities = vec![mutation(
            actor,
            identity_key(EntityKind::User, &account.user.id),
            0,
            "cloud.user.created.v3",
            &account,
        )?];
        if let Some(binding) = oidc {
            if binding.user_id != account.user.id
                || binding.subject.is_empty()
                || binding.subject.len() > 256
                || binding.subject.chars().any(char::is_control)
            {
                return Err(CloudError::InvalidArgument);
            }
            entities.push(mutation(
                actor,
                identity_key(
                    EntityKind::OidcIdentity,
                    &hash_identity(&[&binding.issuer, &binding.subject]),
                ),
                0,
                "cloud.user.oidc-bound.v3",
                &binding,
            )?);
        }
        if let Some(credential) = local {
            if credential.user_id != account.user.id
                || credential.username != crate::normalize_username(&credential.username)?
                || credential.password_hash.len() > 1024
            {
                return Err(CloudError::InvalidArgument);
            }
            entities.push(mutation(
                actor,
                identity_key(
                    EntityKind::LocalCredential,
                    &hash_identity(&[&credential.username]),
                ),
                0,
                "cloud.user.password-created.v3",
                &credential,
            )?);
        }
        self.store
            .commit(CloudTransaction {
                entities,
                ..Default::default()
            })
            .await?;
        Ok(account.user)
    }
    /// Update retained account metadata and retire sessions after disable/reenable.
    pub async fn update_account(
        &self,
        actor: &CloudUser,
        mut user: CloudUser,
    ) -> CloudResult<CloudUser> {
        administrator(actor)?;
        validate_user(&user)?;
        let current = self.account(&user.id).await?;
        if current.user.revision != user.revision {
            return Err(CloudError::Conflict);
        }
        user.created_at = current.user.created_at;
        user.identities = current.user.identities;
        user.updated_at = timestamp()?;
        let epoch = current.security_epoch + u64::from(user.active != current.user.active);
        let revision = user.revision;
        user.revision += 1;
        let account = UserAccount {
            user: user.clone(),
            security_epoch: epoch,
        };
        self.store
            .commit(CloudTransaction {
                entities: vec![mutation(
                    &actor.id,
                    identity_key(EntityKind::User, &user.id),
                    revision,
                    "cloud.user.updated.v3",
                    &account,
                )?],
                ..Default::default()
            })
            .await?;
        Ok(user)
    }
    /// Credential rotation and session retirement commit together.
    pub async fn reset_local_password(
        &self,
        actor: &CloudUser,
        user_id: &str,
        expected: u64,
        password_hash: String,
    ) -> CloudResult<CloudUser> {
        administrator(actor)?;
        let mut account = self.account(user_id).await?;
        if account.user.revision != expected {
            return Err(CloudError::Conflict);
        }
        // Login binding lookup is indexed by user_id through the dedicated port query.
        let records = self.store.user_identities(user_id).await?;
        let record = records
            .into_iter()
            .find(|r| r.key.kind == EntityKind::LocalCredential)
            .ok_or(CloudError::NotFound)?;
        let mut credential: LocalCredential =
            serde_json::from_value(record.value).map_err(|_| CloudError::Storage)?;
        credential.password_hash = password_hash;
        account.security_epoch += 1;
        account.user.revision += 1;
        account.user.updated_at = timestamp()?;
        self.store
            .commit(CloudTransaction {
                entities: vec![
                    mutation(
                        &actor.id,
                        record.key,
                        record.revision,
                        "cloud.user.password-reset.v3",
                        &credential,
                    )?,
                    mutation(
                        &actor.id,
                        identity_key(EntityKind::User, user_id),
                        expected,
                        "cloud.user.sessions-retired.v3",
                        &account,
                    )?,
                ],
                ..Default::default()
            })
            .await?;
        Ok(account.user)
    }
    /// Bounded account listing for a verified global administrator.
    pub async fn list_users(
        &self,
        actor: &CloudUser,
        after: Option<&str>,
        query: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<CloudUser>> {
        administrator(actor)?;
        let mut options = EntityQuery::new(EntityKind::User, IDENTITY_NAMESPACE.into());
        options.after = after.map(str::to_owned);
        options.query = query.map(str::to_owned);
        options.limit = limit.min(100);
        self.store
            .list(&options)
            .await?
            .into_iter()
            .map(|r| {
                let mut account: UserAccount =
                    serde_json::from_value(r.value).map_err(|_| CloudError::Storage)?;
                account.user.revision = r.revision;
                Ok(account.user)
            })
            .collect()
    }
    /// Explicit account membership lookup, independent of visual project ancestry.
    pub async fn user_memberships(&self, user: &str) -> CloudResult<Vec<ProjectMembership>> {
        self.store
            .memberships(user)
            .await?
            .into_iter()
            .map(|r| {
                let mut member: ProjectMembership =
                    serde_json::from_value(r.value).map_err(|_| CloudError::Storage)?;
                member.revision = r.revision;
                Ok(member)
            })
            .collect()
    }
    /// Assign a single explicit role. Caller authority is resolved by host authentication.
    pub async fn save_membership(
        &self,
        caller: &CloudCaller,
        user: &str,
        role: ProjectRole,
        expected: u64,
    ) -> CloudResult<ProjectMembership> {
        caller.require(CloudPermission::Administer)?;
        let _ = self.account(user).await?;
        let expected = if expected == 0 {
            match self
                .store
                .read(&membership_key(caller.project_id(), user))
                .await
            {
                Ok(r)
                    if r.value
                        .get("permissions")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(Vec::is_empty) =>
                {
                    r.revision
                }
                Ok(_) => return Err(CloudError::Conflict),
                Err(CloudError::NotFound) => 0,
                Err(e) => return Err(e),
            }
        } else {
            expected
        };
        let member = ProjectMembership {
            project_id: caller.project_id().into(),
            user_id: user.into(),
            subject: user.into(),
            role,
            permissions: role.permissions(),
            revision: expected + 1,
        };
        self.store
            .commit(CloudTransaction {
                entities: vec![mutation(
                    caller.subject(),
                    membership_key(caller.project_id(), user),
                    expected,
                    "cloud.membership.role-assigned.v3",
                    &member,
                )?],
                ..Default::default()
            })
            .await?;
        Ok(member)
    }
    /// Remove explicit project access without deleting user or retained project history.
    pub async fn remove_membership(
        &self,
        caller: &CloudCaller,
        user: &str,
        expected: u64,
    ) -> CloudResult<()> {
        caller.require(CloudPermission::Administer)?;
        let key = membership_key(caller.project_id(), user);
        let record = self.store.read(&key).await?;
        if record.revision != expected {
            return Err(CloudError::Conflict);
        }
        let mut member: ProjectMembership =
            serde_json::from_value(record.value).map_err(|_| CloudError::Storage)?;
        member.permissions.clear();
        member.revision += 1;
        self.store
            .commit(CloudTransaction {
                entities: vec![mutation(
                    caller.subject(),
                    key,
                    expected,
                    "cloud.membership.removed.v3",
                    &member,
                )?],
                ..Default::default()
            })
            .await?;
        Ok(())
    }
    /// Bounded membership administration for the exact project.
    pub async fn project_members(
        &self,
        caller: &CloudCaller,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<ProjectMembership>> {
        caller.require(CloudPermission::Administer)?;
        let mut query = EntityQuery::new(EntityKind::Membership, caller.project_id().into());
        query.after = after.map(str::to_owned);
        query.limit = limit.min(100);
        query.status = Some("active".into());
        self.store
            .list(&query)
            .await?
            .into_iter()
            .filter(|r| {
                r.value.get("user_id").is_some()
                    && r.value
                        .get("permissions")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|p| !p.is_empty())
            })
            .map(|r| {
                let mut member: ProjectMembership =
                    serde_json::from_value(r.value).map_err(|_| CloudError::Storage)?;
                member.revision = r.revision;
                Ok(member)
            })
            .collect()
    }
    /// Minimal directory search to add an existing account, constrained to project administration.
    pub async fn member_candidates(
        &self,
        caller: &CloudCaller,
        text: &str,
    ) -> CloudResult<Vec<CloudUser>> {
        caller.require(CloudPermission::Administer)?;
        if !(2..=128).contains(&text.len()) || text.chars().any(char::is_control) {
            return Err(CloudError::InvalidArgument);
        }
        let mut query = EntityQuery::new(EntityKind::User, IDENTITY_NAMESPACE.into());
        query.query = Some(text.into());
        query.limit = 20;
        query.status = Some("active".into());
        self.store
            .list(&query)
            .await?
            .into_iter()
            .map(|r| {
                serde_json::from_value::<UserAccount>(r.value)
                    .map(|a| a.user)
                    .map_err(|_| CloudError::Storage)
            })
            .filter(|r| !matches!(r,Ok(user) if !user.active))
            .collect()
    }
}
