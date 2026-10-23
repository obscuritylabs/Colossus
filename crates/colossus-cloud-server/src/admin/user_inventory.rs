//! Read-only account associations; administrator status never impersonates an owner.
use super::*;

pub(super) fn router() -> Router<Arc<State>> {
    Router::new()
        .route("/api/admin/users/{user}/projects", get(projects))
        .route(
            "/api/admin/users/{user}/projects/{project}/workspaces",
            get(workspaces),
        )
        .route(
            "/api/admin/users/{user}/projects/{project}/chats",
            get(chats),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryPage {
    after: Option<String>,
    limit: Option<usize>,
}
impl InventoryPage {
    fn limit(&self) -> Result<usize> {
        let limit = self.limit.unwrap_or(25);
        if !(1..=100).contains(&limit)
            || self
                .after
                .as_ref()
                .is_some_and(|after| after.len() > 2048 || after.chars().any(char::is_control))
        {
            return Err(CloudError::InvalidArgument.into());
        }
        Ok(limit)
    }
}

async fn projects(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path(user): Path<String>,
    Query(page): Query<InventoryPage>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, false).await?;
    let (projects, next_cursor) = state
        .repo
        .admin_user_projects(&actor, &user, page.after.as_deref(), page.limit()?)
        .await?;
    Ok(Json(
        serde_json::json!({"projects":projects,"next_cursor":next_cursor}),
    ))
}

async fn workspaces(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path((user, project)): Path<(String, String)>,
    Query(page): Query<InventoryPage>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, false).await?;
    let caller = state.auth.caller(&headers, &project, false).await?;
    let result = state
        .repo
        .admin_user_workspaces(&actor, &caller, &user, page.after.as_deref(), page.limit()?)
        .await?;
    Ok(Json(
        serde_json::json!({"workspaces":result.items,"next_cursor":result.next_cursor,"visibility":result.visibility}),
    ))
}

async fn chats(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Path((user, project)): Path<(String, String)>,
    Query(page): Query<InventoryPage>,
) -> Result<Json<serde_json::Value>> {
    let actor = state.auth.admin(&headers, false).await?;
    let caller = state.auth.caller(&headers, &project, false).await?;
    let result = state
        .repo
        .admin_user_chats(&actor, &caller, &user, page.after.as_deref(), page.limit()?)
        .await?;
    Ok(Json(
        serde_json::json!({"chats":result.items,"next_cursor":result.next_cursor,"visibility":result.visibility}),
    ))
}
