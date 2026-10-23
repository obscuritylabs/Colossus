//! Source-bound branch routes share ordinary thread authentication and lifecycle.
use super::*;
use colossus_cloud::{CloudBranchKind, CreateCloudThreadBranch};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Page {
    kind: Option<CloudBranchKind>,
    archived: Option<bool>,
    after: Option<String>,
    limit: Option<usize>,
}

pub(super) async fn create(
    Extract(state): Extract<Arc<State>>,
    Path((project, parent)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<CreateCloudThreadBranch>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    let _hosted_admission = if let Some(hosted) = &state.hosted {
        let detail = state.repo.get_thread(&caller, &parent).await?;
        hosted
            .admission(&caller, &detail.thread.node_id, &input.request)
            .await?
    } else {
        None
    };
    let (thread, task) = state
        .repo
        .create_thread_branch(&caller, &parent, input)
        .await?;
    Ok(Json(serde_json::json!({"thread":thread,"task":task})))
}

pub(super) async fn list(
    Extract(state): Extract<Arc<State>>,
    Path((project, parent)): Path<(String, String)>,
    headers: HeaderMap,
    Query(page): Query<Page>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    let (threads, next_cursor) = state
        .repo
        .query_thread_branches(
            &caller,
            &parent,
            page.kind,
            page.archived,
            page.after,
            page.limit.unwrap_or(25),
        )
        .await?;
    Ok(Json(
        serde_json::json!({"threads":threads,"next_cursor":next_cursor}),
    ))
}
