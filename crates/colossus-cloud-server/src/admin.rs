//! Authenticated product identity/project administration; no runtime credential access.
use crate::{http::Result, server::State};
use axum::{
    Json, Router,
    extract::{Path, Query, State as Extract},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use colossus_cloud::{
    CloudError, CloudPermission, CloudProject, CloudUser, LocalCredential, LoginIdentityMetadata,
    OidcIdentity, ProjectRole, UserAccount, hash_identity, normalize_username,
};
use serde::Deserialize;
use std::sync::Arc;
use zeroize::Zeroizing;

pub(crate) fn router() -> Router<Arc<State>> {
    Router::new()
        .route("/api/projects", get(projects).post(create_project))
        .route("/api/projects/{project}", get(project).patch(edit_project))
        .route(
            "/api/projects/{project}/members",
            get(members).post(add_member),
        )
        .route(
            "/api/projects/{project}/members/{user}",
            axum::routing::patch(edit_member).delete(remove_member),
        )
        .route("/api/projects/{project}/member-candidates", get(candidates))
        .route("/api/admin/projects", get(admin_projects))
        .route("/api/admin/users", get(users).post(create_user))
        .route("/api/admin/users/{user}", get(user).patch(edit_user))
        .route("/api/admin/users/{user}/password", post(password))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    after: Option<String>,
    limit: Option<usize>,
    query: Option<String>,
}
impl Page {
    fn limit(&self) -> Result<usize> {
        let n = self.limit.unwrap_or(100);
        if !(1..=100).contains(&n) {
            return Err(CloudError::InvalidArgument.into());
        }
        if self
            .after
            .as_ref()
            .is_some_and(|a| a.len() > 128 || a.chars().any(char::is_control))
        {
            return Err(CloudError::InvalidArgument.into());
        }
        if self
            .query
            .as_ref()
            .is_some_and(|q| q.len() > 256 || q.chars().any(char::is_control))
        {
            return Err(CloudError::InvalidArgument.into());
        }
        Ok(n)
    }
}
async fn users(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, false).await?;
    let limit = page.limit()?;
    let users = state
        .repo
        .list_users(&actor, page.after.as_deref(), page.query.as_deref(), limit)
        .await?;
    let next = (users.len() == limit)
        .then(|| users.last().map(|u| u.id.clone()))
        .flatten();
    Ok(Json(serde_json::json!({"users":users,"next_cursor":next})))
}
async fn user(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    state.auth.admin(&headers, false).await?;
    Ok(Json(
        serde_json::json!({"user":state.repo.account(&id).await?.user}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewUser {
    username: Option<String>,
    password: Option<String>,
    oidc_subject: Option<String>,
    display_name: String,
    email: Option<String>,
    #[serde(default)]
    is_admin: bool,
}
async fn create_user(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Json(mut input): Json<NewUser>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, true).await?;
    let username = input
        .username
        .as_deref()
        .map(normalize_username)
        .transpose()?;
    let provider = state.config.oidc.as_ref();
    if input.oidc_subject.is_some() && provider.is_none() {
        return Err(CloudError::InvalidArgument.into());
    }
    if username.is_some() && (state.config.local_auth.is_none() || input.password.is_none()) {
        return Err(CloudError::InvalidArgument.into());
    }
    if input.password.is_some() && username.is_none() {
        return Err(CloudError::InvalidArgument.into());
    }
    let id = if let (Some(provider), Some(subject)) = (provider, &input.oidc_subject) {
        hash_identity(&["oidc-user", &provider.issuer, subject])
    } else {
        uuid::Uuid::now_v7().simple().to_string()
    };
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| CloudError::Storage)?;
    let mut identities = Vec::new();
    let oidc = if let (Some(provider), Some(subject)) = (provider, input.oidc_subject) {
        identities.push(LoginIdentityMetadata {
            kind: "oidc".into(),
            label: provider.label.clone(),
            username: None,
            issuer: Some(provider.issuer.clone()),
            subject: Some(subject.clone()),
        });
        Some(OidcIdentity {
            user_id: id.clone(),
            issuer: provider.issuer.clone(),
            subject,
        })
    } else {
        None
    };
    let local = if let Some(username) = username {
        let hash = state
            .auth
            .password_hash(Zeroizing::new(
                input.password.take().ok_or(CloudError::InvalidArgument)?,
            ))
            .await?;
        identities.push(LoginIdentityMetadata {
            kind: "local".into(),
            label: "Local account".into(),
            username: Some(username.clone()),
            issuer: None,
            subject: None,
        });
        Some(LocalCredential {
            user_id: id.clone(),
            username,
            password_hash: hash,
        })
    } else {
        None
    };
    let account = UserAccount {
        user: CloudUser {
            id,
            display_name: input.display_name,
            email: input.email,
            active: true,
            is_admin: input.is_admin,
            revision: 1,
            created_at: now.clone(),
            updated_at: now,
            identities,
        },
        security_epoch: 1,
    };
    let user = state
        .repo
        .create_account(&actor.id, account, oidc, local)
        .await?;
    Ok(Json(serde_json::json!({"user":user})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserEdit {
    revision: u64,
    display_name: Option<String>,
    #[serde(default, deserialize_with = "nullable_email")]
    email: Option<Option<String>>,
    active: Option<bool>,
    is_admin: Option<bool>,
}
fn nullable_email<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}
async fn edit_user(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<UserEdit>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, true).await?;
    let mut user = state.repo.account(&id).await?.user;
    user.revision = input.revision;
    if let Some(name) = input.display_name {
        user.display_name = name;
    }
    if let Some(email) = input.email {
        user.email = email;
    }
    if let Some(active) = input.active {
        user.active = active;
    }
    if let Some(admin) = input.is_admin {
        user.is_admin = admin;
    }
    Ok(Json(
        serde_json::json!({"user":state.repo.update_account(&actor,user).await?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Password {
    revision: u64,
    new_password: String,
}
async fn password(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Password>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, true).await?;
    if state.config.local_auth.is_none() {
        return Err(CloudError::PermissionDenied.into());
    }
    let hash = state
        .auth
        .password_hash(Zeroizing::new(input.new_password))
        .await?;
    Ok(Json(
        serde_json::json!({"user":state.repo.reset_local_password(&actor,&id,input.revision,hash).await?}),
    ))
}
async fn projects(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<serde_json::Value>> {
    let limit = page.limit()?;
    let user = state.auth.user(&headers).await?;
    let projects = if user.is_admin {
        state
            .repo
            .projects_page(&user, page.after.as_deref(), limit)
            .await?
    } else {
        state
            .auth
            .projects(&headers)
            .await?
            .into_iter()
            .filter(|p| page.after.as_ref().is_none_or(|after| p.id > *after))
            .take(limit)
            .collect()
    };
    let next = (projects.len() == limit)
        .then(|| projects.last().map(|p| p.id.clone()))
        .flatten();
    Ok(Json(
        serde_json::json!({"projects":projects,"next_cursor":next}),
    ))
}
async fn admin_projects(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<serde_json::Value>> {
    state.auth.admin(&headers, false).await?;
    projects(Extract(state), headers, Query(page)).await
}
async fn project(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>> {
    state
        .auth
        .caller(&headers, &id, false)
        .await?
        .require(CloudPermission::Read)?;
    Ok(Json(
        serde_json::json!({"project":state.repo.project(&id).await?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewProject {
    id: Option<String>,
    name: String,
    #[serde(default)]
    description: String,
    parent_project_id: Option<String>,
}
async fn create_project(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Json(input): Json<NewProject>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, true).await?;
    let project = CloudProject {
        id: input
            .id
            .unwrap_or_else(|| uuid::Uuid::now_v7().simple().to_string()),
        name: input.name,
        description: input.description,
        parent_project_id: input.parent_project_id,
        archived: false,
        revision: 0,
        created_at: String::new(),
        updated_at: String::new(),
    };
    Ok(Json(
        serde_json::json!({"project":state.repo.save_project(&actor,project).await?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectEdit {
    revision: u64,
    name: Option<String>,
    description: Option<String>,
    #[serde(default, deserialize_with = "nullable_email")]
    parent_project_id: Option<Option<String>>,
    archived: Option<bool>,
}
async fn edit_project(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<ProjectEdit>,
) -> Result<Json<serde_json::Value>> {
    let user = state.auth.user(&headers).await?;
    let caller = state.auth.caller(&headers, &id, true).await?;
    caller.require(CloudPermission::Administer)?;
    let mut project = state.repo.project(&id).await?;
    project.revision = input.revision;
    if let Some(name) = input.name {
        project.name = name;
    }
    if let Some(description) = input.description {
        project.description = description;
    }
    if let Some(parent) = input.parent_project_id {
        project.parent_project_id = parent;
    }
    if let Some(archived) = input.archived {
        project.archived = archived;
    }
    let project = if user.is_admin {
        state.repo.save_project(&user, project).await?
    } else {
        state.repo.update_project_metadata(&caller, project).await?
    };
    Ok(Json(serde_json::json!({"project":project})))
}
async fn members(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &id, false).await?;
    let limit = page.limit()?;
    let members = state
        .repo
        .project_members(&caller, page.after.as_deref(), limit)
        .await?;
    let next = (members.len() == limit)
        .then(|| members.last().map(|m| hash_identity(&[&m.user_id])))
        .flatten();
    let ids = members
        .iter()
        .map(|m| m.user_id.clone())
        .collect::<Vec<_>>();
    let accounts = state
        .repo
        .storage()
        .user_accounts(&ids)
        .await?
        .into_iter()
        .map(|record| {
            UserAccount::try_from(record.value)
                .map(|a| (a.user.id.clone(), a.user))
                .map_err(|_| CloudError::Storage)
        })
        .collect::<colossus_cloud::CloudResult<std::collections::BTreeMap<_, _>>>()?;
    let members = members
        .into_iter()
        .map(|member| {
            let mut value = serde_json::to_value(&member).map_err(|_| CloudError::Storage)?;
            if let Some(user) = accounts.get(&member.user_id) {
                value["display_name"] = serde_json::Value::String(user.display_name.clone());
                value["active"] = serde_json::Value::Bool(user.active);
            }
            Ok(value)
        })
        .collect::<colossus_cloud::CloudResult<Vec<_>>>()?;
    Ok(Json(
        serde_json::json!({"members":members,"next_cursor":next}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    user_id: String,
    role: ProjectRole,
    revision: Option<u64>,
}
async fn add_member(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Member>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &id, true).await?;
    Ok(Json(
        serde_json::json!({"member":state.repo.save_membership(&caller,&input.user_id,input.role,input.revision.unwrap_or(0)).await?}),
    ))
}
async fn edit_member(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path((id, user)): Path<(String, String)>,
    Json(input): Json<Member>,
) -> Result<Json<serde_json::Value>> {
    if input.user_id != user {
        return Err(CloudError::InvalidArgument.into());
    }
    let caller = state.auth.caller(&headers, &id, true).await?;
    Ok(Json(
        serde_json::json!({"member":state.repo.save_membership(&caller,&user,input.role,input.revision.ok_or(CloudError::InvalidArgument)?).await?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    revision: u64,
}
async fn remove_member(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path((id, user)): Path<(String, String)>,
    Query(input): Query<Revision>,
) -> Result<StatusCode> {
    let caller = state.auth.caller(&headers, &id, true).await?;
    state
        .repo
        .remove_membership(&caller, &user, input.revision)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn candidates(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(page): Query<Page>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &id, false).await?;
    let query = page.query.ok_or(CloudError::InvalidArgument)?;
    let users = state
        .repo
        .member_candidates(&caller, &query)
        .await?
        .into_iter()
        .map(|u| serde_json::json!({"id":u.id,"display_name":u.display_name}))
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({"users":users})))
}
