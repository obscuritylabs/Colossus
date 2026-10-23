//! Administrator account inventories retain the real viewer's personal visibility.
use super::{CloudRepository, EntityKind, EntityOrder, EntityQuery, accounts::administrator};
use crate::storage::EntityValue;
use crate::{
    AdminUserProject, AdminUserResourcePage, CloudCaller, CloudError, CloudPermission, CloudResult,
    CloudThread, CloudUser, CloudWorkspace, ProjectMembership, UserResourceVisibility,
    membership_key, validate_identifier,
};

impl CloudRepository {
    /// Page active explicit project memberships for a verified instance administrator.
    /// Project ancestry and global administration never fabricate a membership.
    pub async fn admin_user_projects(
        &self,
        actor: &CloudUser,
        user: &str,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<(Vec<AdminUserProject>, Option<String>)> {
        administrator(actor)?;
        self.account(user).await?;
        validate_page(after, limit)?;
        if let Some(after) = after {
            validate_identifier(after)?;
        }
        let records = self.store.memberships_page(user, after, limit).await?;
        let next = (records.len() == limit)
            .then(|| records.last().map(|record| record.key.project_id.clone()))
            .flatten();
        let mut projects = Vec::with_capacity(records.len());
        for record in records {
            let mut membership: ProjectMembership = record.value.try_into()?;
            if membership.user_id != user
                || membership.subject != user
                || membership.permissions.is_empty()
            {
                return Err(CloudError::Storage);
            }
            membership.revision = record.revision;
            let project = self.project(&membership.project_id).await?;
            projects.push(AdminUserProject {
                project,
                membership,
            });
        }
        Ok((projects, next))
    }

    /// Page personally owned advertised workspaces without exposing another user's inventory.
    pub async fn admin_user_workspaces(
        &self,
        actor: &CloudUser,
        caller: &CloudCaller,
        user: &str,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<AdminUserResourcePage<CloudWorkspace>> {
        self.admin_user_resources(actor, caller, user, EntityKind::Workspace, after, limit)
            .await
    }

    /// Page owned conversation metadata; messages, tasks and runtime history are never loaded.
    pub async fn admin_user_chats(
        &self,
        actor: &CloudUser,
        caller: &CloudCaller,
        user: &str,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<AdminUserResourcePage<CloudThread>> {
        self.admin_user_resources(actor, caller, user, EntityKind::Thread, after, limit)
            .await
    }

    async fn admin_user_resources<T>(
        &self,
        actor: &CloudUser,
        caller: &CloudCaller,
        user: &str,
        kind: EntityKind,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<AdminUserResourcePage<T>>
    where
        T: TryFrom<EntityValue, Error = CloudError>,
    {
        administrator(actor)?;
        caller.require(CloudPermission::Read)?;
        if caller.subject() != actor.id {
            return Err(CloudError::PermissionDenied);
        }
        self.account(user).await?;
        validate_page(after, limit)?;
        let membership: ProjectMembership = self
            .store
            .read(&membership_key(caller.project_id(), user))
            .await?
            .value
            .try_into()?;
        if membership.user_id != user
            || membership.subject != user
            || membership.project_id != caller.project_id()
        {
            return Err(CloudError::Storage);
        }
        if membership.permissions.is_empty() {
            return Err(CloudError::NotFound);
        }
        if user != caller.subject() {
            return Ok(AdminUserResourcePage {
                items: Vec::new(),
                next_cursor: None,
                visibility: UserResourceVisibility::PersonalResourcesPrivate,
            });
        }
        let records = self
            .query_entities(
                caller,
                EntityQuery {
                    kind,
                    after: after.map(str::to_owned),
                    limit,
                    owned_by: Some(user.into()),
                    order: if kind == EntityKind::Thread {
                        EntityOrder::UpdatedDesc
                    } else {
                        EntityOrder::IdAsc
                    },
                    ..Default::default()
                },
            )
            .await?;
        let next_cursor = (records.len() == limit)
            .then(|| records.last().and_then(|record| record.page_cursor.clone()))
            .flatten();
        let items = records
            .into_iter()
            .map(|record| record.value.try_into())
            .collect::<CloudResult<_>>()?;
        Ok(AdminUserResourcePage {
            items,
            next_cursor,
            visibility: UserResourceVisibility::Visible,
        })
    }
}

fn validate_page(after: Option<&str>, limit: usize) -> CloudResult<()> {
    if !(1..=100).contains(&limit)
        || after.is_some_and(|after| after.len() > 2048 || after.chars().any(char::is_control))
    {
        return Err(CloudError::InvalidArgument);
    }
    if let Some(after) = after.filter(|after| !after.starts_with("cp1_")) {
        validate_identifier(after)?;
    }
    Ok(())
}
