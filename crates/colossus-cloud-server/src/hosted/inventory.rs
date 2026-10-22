//! Personal Hosted inventory reads and explicit, idle-only runtime placement controls.
use super::*;
use colossus_cloud::{
    CloudTask,
    storage::{EntityKind, EntityOrder, EntityQuery},
};

#[derive(Default, Serialize)]
pub(super) struct Activity {
    queued: usize,
    running: usize,
    waiting: usize,
    /// Counts are lower bounds when the bounded active-work page is full.
    limited: bool,
}

pub(super) async fn activity(
    host: &HostedExecution,
    caller: &CloudCaller,
) -> std::result::Result<Activity, CloudError> {
    let records = host
        .repo
        .query_entities(caller, work_query(host, caller, true, None, 100)?)
        .await?;
    let mut activity = Activity {
        limited: records.len() == 100,
        ..Default::default()
    };
    for record in records {
        match record.value.status() {
            "queued" => activity.queued += 1,
            "waiting" => activity.waiting += 1,
            // Includes cancellation and terminal output still being reconciled.
            _ => activity.running += 1,
        }
    }
    Ok(activity)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkPage {
    #[serde(default)]
    active: bool,
    after: Option<String>,
    limit: Option<usize>,
}

fn work_query(
    host: &HostedExecution,
    caller: &CloudCaller,
    active: bool,
    after: Option<String>,
    limit: usize,
) -> std::result::Result<EntityQuery, CloudError> {
    if !(1..=100).contains(&limit) || after.as_ref().is_some_and(|cursor| cursor.len() > 2048) {
        return Err(CloudError::InvalidArgument);
    }
    Ok(EntityQuery {
        kind: EntityKind::Task,
        node_id: host.node_id(caller)?,
        status: active.then(|| "unfinished".into()),
        after,
        limit,
        order: EntityOrder::CreatedDesc,
        ..Default::default()
    })
}

pub(super) async fn tasks(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Query(page): Query<WorkPage>,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    caller.require(CloudPermission::Read)?;
    let host = service(&state)?;
    if host.project_config(&project).await?.is_none() {
        return Err(CloudError::NotFound.into());
    }
    let limit = page.limit.unwrap_or(25);
    let records = host
        .repo
        .query_entities(
            &caller,
            work_query(host, &caller, page.active, page.after, limit)?,
        )
        .await?;
    let next_cursor = (records.len() == limit)
        .then(|| records.last().and_then(|record| record.page_cursor.clone()))
        .flatten();
    let tasks = records
        .into_iter()
        .map(|record| record.value.try_into())
        .collect::<std::result::Result<Vec<CloudTask>, CloudError>>()?;
    Ok(Json(
        serde_json::json!({"tasks":tasks,"next_cursor":next_cursor}),
    ))
}

pub(super) async fn load(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Read)?;
    caller.require(CloudPermission::Execute)?;
    let host = service(&state)?;
    host.ensure(&state, &caller).await?;
    Ok(Json(serde_json::json!({"loaded":true})))
}

pub(super) async fn release(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, true).await?;
    caller.require(CloudPermission::Read)?;
    caller.require(CloudPermission::Execute)?;
    caller.require(CloudPermission::Control)?;
    let host = service(&state)?;
    if host.project_config(&project).await?.is_none() {
        return Err(CloudError::NotFound.into());
    }
    let _lifecycle = host.lifecycle.lock().await;
    // The shared gate blocks new admissions while the owning replica proves idle
    // and releases placement. Dropping it leaves the durable target available.
    let edit = host.begin_shared_edit(&state, &caller, false).await?;
    if host
        .user_is_busy(&state, &project, caller.subject())
        .await?
    {
        return Err(CloudError::Conflict.into());
    }
    if edit.is_none() {
        host.stop_user(&project, caller.subject()).await?;
    }
    Ok(Json(serde_json::json!({"released":true})))
}
