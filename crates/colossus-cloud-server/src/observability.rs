//! Scoped operational dashboards and runtime-reported policy inspection.
use crate::{
    http::{Result, now},
    server::State,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State as Extract},
    http::HeaderMap,
    routing::get,
};
use colossus_cloud::{
    CloudError,
    observability::{DailyActivity, OperationalCounts},
};
use futures::{StreamExt, TryStreamExt};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};

pub(crate) fn router() -> Router<Arc<State>> {
    Router::new()
        .route("/api/dashboard", get(dashboard))
        .route("/api/overview", get(dashboard))
        .route("/api/projects/{project}/analytics", get(project_analytics))
        .route(
            "/api/projects/{project}/nodes/{node}/analytics",
            get(node_analytics),
        )
        .route(
            "/api/projects/{project}/nodes/{node}/policy",
            get(node_policy),
        )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Window {
    #[serde(default = "default_days")]
    days: u16,
}
fn default_days() -> u16 {
    7
}
fn timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
async fn project_analytics(
    Extract(state): Extract<Arc<State>>,
    Path(project): Path<String>,
    headers: HeaderMap,
    Query(window): Query<Window>,
) -> Result<Json<serde_json::Value>> {
    analytics(state, headers, project, None, window.days).await
}
async fn node_analytics(
    Extract(state): Extract<Arc<State>>,
    Path((project, node)): Path<(String, String)>,
    headers: HeaderMap,
    Query(window): Query<Window>,
) -> Result<Json<serde_json::Value>> {
    analytics(state, headers, project, Some(node), window.days).await
}
async fn analytics(
    state: Arc<State>,
    headers: HeaderMap,
    project: String,
    node: Option<String>,
    days: u16,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    let stats = state
        .repo
        .statistics(&caller, node.as_deref(), now(), days)
        .await?;
    Ok(Json(
        serde_json::json!({"generated_at":timestamp(),"window_days":days,"counts":stats.counts,"activity":stats.activity,"usage":stats.usage,"telemetry":{"complete":stats.complete,"description":"Run outcomes and retained provider usage; failures are not evidence of a policy violation."}}),
    ))
}
async fn dashboard(
    Extract(state): Extract<Arc<State>>,
    headers: HeaderMap,
    Query(window): Query<Window>,
) -> Result<Json<serde_json::Value>> {
    let days = window.days;
    if !(1..=90).contains(&days) {
        return Err(CloudError::InvalidArgument.into());
    }
    let observed_at = now();
    let projects = state.auth.projects(&headers).await?;
    let mut counts = OperationalCounts::default();
    let mut recent = Vec::new();
    let mut activity = BTreeMap::<String, DailyActivity>::new();
    let mut complete = true;
    // A bounded four-project fan-out leaves database capacity for agent ingestion.
    // Visibility is still resolved independently before every project's reads.
    let snapshots = futures::stream::iter(projects)
        .map(|project| {
            let state = &state;
            let headers = &headers;
            async move {
                let caller = match state.auth.caller(headers, &project.id, false).await {
                    Ok(caller) => caller,
                    Err(CloudError::PermissionDenied) => return Ok(None),
                    Err(error) => return Err(error),
                };
                let stats = state
                    .repo
                    .statistics(&caller, None, observed_at, days)
                    .await?;
                let threads = state
                    .repo
                    .list_threads(&caller, None, None, Some(false), None, 5)
                    .await?;
                Ok::<_, CloudError>(Some((project.name, stats, threads)))
            }
        })
        .buffer_unordered(4)
        .try_collect::<Vec<_>>()
        .await?;
    for (project_name, stats, threads) in snapshots.into_iter().flatten() {
        counts.projects += 1;
        counts.hosts += stats.counts.hosts;
        counts.agents += stats.counts.agents;
        counts.online_agents += stats.counts.online_agents;
        counts.threads += stats.counts.threads;
        counts.runs += stats.counts.runs;
        counts.completed += stats.counts.completed;
        counts.failed += stats.counts.failed;
        counts.cancelled += stats.counts.cancelled;
        counts.interrupted += stats.counts.interrupted;
        counts.outcome_unknown += stats.counts.outcome_unknown;
        counts.active += stats.counts.active;
        counts.queued += stats.counts.queued;
        complete &= stats.complete;
        for day in stats.activity {
            let total = activity
                .entry(day.date.clone())
                .or_insert_with(|| DailyActivity {
                    date: day.date.clone(),
                    ..Default::default()
                });
            total.runs += day.runs;
            total.completed += day.completed;
            total.failed += day.failed;
            for (aggregate, value) in [
                (&mut total.input_tokens, day.input_tokens),
                (&mut total.output_tokens, day.output_tokens),
            ] {
                if let Some(value) = value {
                    *aggregate = Some(
                        aggregate
                            .unwrap_or(0)
                            .checked_add(value)
                            .ok_or(CloudError::Storage)?,
                    );
                }
            }
        }
        for thread in threads {
            recent.push(serde_json::json!({"project_name":project_name,"thread":thread}));
        }
    }
    recent.sort_by(|a, b| {
        b.pointer("/thread/updated_at")
            .and_then(serde_json::Value::as_str)
            .cmp(
                &a.pointer("/thread/updated_at")
                    .and_then(serde_json::Value::as_str),
            )
    });
    recent.truncate(10);
    Ok(Json(
        serde_json::json!({"generated_at":timestamp(),"window_days":days,"counts":{"projects":counts.projects,"hosts":counts.hosts,"agents":counts.agents,"online_agents":counts.online_agents,"threads":counts.threads,"active_runs":counts.active,"queued_tasks":counts.queued,"failed_runs":counts.failed,"outcome_unknown_runs":counts.outcome_unknown},"recent_threads":recent,"activity":activity.into_values().collect::<Vec<_>>(),"telemetry":{"complete":complete,"description":"Visible projects only. Run outcomes are not policy-violation counts; imported history may omit usage."}}),
    ))
}
async fn node_policy(
    Extract(state): Extract<Arc<State>>,
    Path((project, node)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    let node = state.repo.get_node(&caller, &node).await?;
    let current = now();
    let connected = if node.revoked || !node.runtime_ready {
        false
    } else {
        match state
            .repo
            .storage()
            .read_lease(&project, &node.node_id, current)
            .await
        {
            Ok(_) => true,
            Err(CloudError::NotFound) => false,
            Err(error) => return Err(error.into()),
        }
    };
    let observed = node.policy_observed_at;
    let stale = !connected || observed.is_none_or(|time| current.saturating_sub(time) > 60);
    let baseline = state.repo.project_policy(&caller).await?;
    let posture = node.policy.as_ref();
    let evaluation = baseline.evaluate(posture, stale);
    Ok(Json(
        serde_json::json!({"node_id":node.node_id,"observed_at":observed,"last_seen_at":observed,"connected":connected,"posture":posture,"provenance":"runtime_reported","stale":stale,"status":if posture.is_some(){"reported"}else{"unknown"},"description":"Authenticated runtime-reported configuration. Monitoring drift is not independent attestation or proof that a prohibited effect occurred.","expectation":baseline,"evaluation":evaluation}),
    ))
}
