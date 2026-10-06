use super::*;
use colossus_cloud::{
    CloudPermission, CloudProject, CloudRepository, CloudUser, ProjectMembership, UserAccount,
    membership_key,
};
use std::collections::BTreeSet;

impl Authentication {
    pub(super) fn repository(&self) -> CloudResult<CloudRepository> {
        CloudRepository::new(self.store.clone())
    }
    pub(crate) fn csrf(&self, headers: &HeaderMap) -> CloudResult<()> {
        if headers.get("origin").and_then(|v| v.to_str().ok())
            != Some(self.config.public_origin.trim_end_matches('/'))
            || headers.get("x-colossus-csrf").and_then(|v| v.to_str().ok()) != Some("1")
        {
            return Err(CloudError::PermissionDenied);
        }
        Ok(())
    }
    pub(super) async fn session(
        &self,
        headers: &HeaderMap,
    ) -> CloudResult<(AuthSession, UserAccount)> {
        let token = cookie(headers, "colossus_session")
            .filter(|t| t.len() <= 256)
            .ok_or(CloudError::PermissionDenied)?;
        let now = crate::http::now();
        let mut session = None;
        if self.config.oidc.is_some() {
            session = self
                .store
                .read_session(&self.authority_hash(&token), now)
                .await
                .ok();
        }
        if session.is_none() && self.config.local_auth.is_some() {
            session = self
                .store
                .read_session(&self.local_session_hash(&token), now)
                .await
                .ok();
        }
        let session = session.ok_or(CloudError::PermissionDenied)?;
        let account = self
            .repository()?
            .account(&session.subject)
            .await
            .map_err(|_| CloudError::PermissionDenied)?;
        if !account.user.active || session.security_epoch != account.security_epoch {
            return Err(CloudError::PermissionDenied);
        }
        Ok((session, account))
    }
    pub(crate) async fn user(&self, headers: &HeaderMap) -> CloudResult<CloudUser> {
        Ok(self.session(headers).await?.1.user)
    }
    pub(crate) async fn admin(
        &self,
        headers: &HeaderMap,
        mutation: bool,
    ) -> CloudResult<CloudUser> {
        if mutation {
            self.csrf(headers)?;
        }
        let user = self.user(headers).await?;
        if !user.is_admin {
            return Err(CloudError::PermissionDenied);
        }
        Ok(user)
    }
    pub async fn caller(
        &self,
        headers: &HeaderMap,
        project: &str,
        mutation: bool,
    ) -> CloudResult<CloudCaller> {
        self.caller_with_expiry(headers, project, mutation)
            .await
            .map(|(caller, _)| caller)
    }
    pub async fn caller_with_expiry(
        &self,
        headers: &HeaderMap,
        project: &str,
        mutation: bool,
    ) -> CloudResult<(CloudCaller, u64)> {
        if mutation {
            self.csrf(headers)?;
        }
        let (session, account) = self.session(headers).await?;
        let mut permissions = BTreeSet::new();
        // Global management/read visibility grants no implicit task execution or approval.
        if account.user.is_admin {
            permissions.extend([CloudPermission::Read, CloudPermission::Administer]);
        }
        match self
            .store
            .read(&membership_key(project, &account.user.id))
            .await
        {
            Ok(record) => {
                let member: ProjectMembership =
                    serde_json::from_value(record.value).map_err(|_| CloudError::Storage)?;
                if member.project_id != project
                    || member.user_id != account.user.id
                    || member.subject != account.user.id
                {
                    return Err(CloudError::PermissionDenied);
                }
                permissions.extend(member.permissions);
            }
            Err(CloudError::NotFound) => {}
            Err(error) => return Err(error),
        }
        if permissions.is_empty() {
            return Err(CloudError::PermissionDenied);
        }
        let metadata = self.repository()?.project(project).await?;
        if metadata.archived {
            permissions
                .retain(|p| matches!(p, CloudPermission::Read | CloudPermission::Administer));
        }
        Ok((
            CloudCaller::new(account.user.id, project.into(), permissions)?,
            session.expires_at,
        ))
    }
    pub async fn memberships(&self, headers: &HeaderMap) -> CloudResult<Vec<ProjectMembership>> {
        let user = self.user(headers).await?;
        Ok(self
            .repository()?
            .user_memberships(&user.id)
            .await?
            .into_iter()
            .filter(|m| !m.permissions.is_empty())
            .collect())
    }
    pub(crate) async fn projects(&self, headers: &HeaderMap) -> CloudResult<Vec<CloudProject>> {
        let user = self.user(headers).await?;
        let repo = self.repository()?;
        let mut projects = Vec::new();
        if user.is_admin {
            let mut after = None;
            loop {
                let page = repo.projects_page(&user, after.as_deref(), 100).await?;
                let count = page.len();
                after = page.last().map(|p| p.id.clone());
                projects.extend(page);
                if count < 100 {
                    break;
                }
                if projects.len() > 1024 {
                    return Err(CloudError::ResourceExhausted);
                }
            }
        } else {
            for member in repo.user_memberships(&user.id).await? {
                if member.permissions.contains(&CloudPermission::Read) {
                    projects.push(repo.project(&member.project_id).await?);
                }
            }
        }
        projects.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(projects)
    }
    pub(crate) fn public_configuration(&self) -> serde_json::Value {
        serde_json::json!({"local_enabled":self.config.local_auth.is_some(),"oidc":self.config.oidc.as_ref().map(|provider|serde_json::json!({"label":provider.label,"login_url":"/auth/login"}))})
    }
}
