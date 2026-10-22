//! Bounded metadata reads retain ordinary project and personal thread authority.
use super::*;

#[derive(serde::Serialize)]
pub(super) struct Metadata {
    thread: colossus_cloud::CloudThread,
}

pub(super) async fn read(
    Extract(state): Extract<Arc<State>>,
    Path((project, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Metadata>> {
    let caller = state.auth.caller(&headers, &project, false).await?;
    let thread = state.repo.read_thread_metadata(&caller, &id).await?;
    Ok(Json(Metadata { thread }))
}
