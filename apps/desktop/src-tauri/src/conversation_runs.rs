//! Read-only reconciliation of Desktop and Control Plane runs in one selected runtime.
use crate::run_list::RunList;
use colossus_sdk::{AgentRunClient, ApiError, ApiErrorReason, Colossus, ListRunsRequest, Run};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{self, Write},
    sync::{Arc, Mutex, PoisonError},
};

const PREFIX: &str = "desktop-conversations-v2:";
// Match the public run discovery ceiling; each owner returns at most three runs.
const MAX_PAGE_SIZE: u32 = 3;
const MAX_CURSORS: usize = 32;
const MAX_CURSOR_BYTES: usize = 8 * 1024 * 1024;
const MAX_BINDINGS: usize = 4096;
#[derive(Clone, Default)]
pub(crate) struct ConversationRuns {
    sources: Arc<Mutex<HashMap<String, bool>>>,
    desktop_sessions: Arc<Mutex<HashSet<String>>>,
    cursors: Arc<Mutex<CursorCache>>,
}
pub(crate) struct ConversationPage {
    pub(crate) runs: Vec<Run>,
    pub(crate) next_page_token: Option<String>,
}
#[derive(Clone, Default, PartialEq, Serialize)]
struct SourcePage {
    token: Option<String>,
    done: bool,
    pending: VecDeque<Run>,
}
#[derive(Clone, Default, PartialEq, Serialize)]
struct Cursor {
    scope: String,
    primary: SourcePage,
    cloud: SourcePage,
}
#[derive(Default)]
struct CursorCache {
    entries: HashMap<String, (Cursor, usize)>,
    order: VecDeque<String>,
    bytes: usize,
}
struct CursorSize(usize);
impl Write for CursorSize {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_CURSOR_BYTES)
            .ok_or_else(|| io::Error::other("conversation cursor capacity exceeded"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl CursorCache {
    fn insert(&mut self, cursor: Cursor) -> Result<String, ApiError> {
        // Periodic first-page reconciliation must not evict an unchanged
        // foreground continuation just by issuing the same discovery read.
        if let Some(token) = self
            .entries
            .iter()
            .find_map(|(token, entry)| (entry.0 == cursor).then(|| token.clone()))
        {
            self.order.retain(|key| key != &token);
            self.order.push_back(token.clone());
            return Ok(token);
        }
        let mut size = CursorSize(0);
        serde_json::to_writer(&mut size, &cursor).map_err(|_| {
            ApiError::resource_exhausted(
                ApiErrorReason::CapacityExceeded,
                "Reload conversations before loading more history.",
            )
        })?;
        while self.entries.len() >= MAX_CURSORS || self.bytes + size.0 > MAX_CURSOR_BYTES {
            let key = self.order.pop_front().ok_or_else(invalid)?;
            if let Some((_, bytes)) = self.entries.remove(&key) {
                self.bytes -= bytes;
            }
        }
        let token = format!("{PREFIX}{}", uuid::Uuid::new_v4());
        self.bytes += size.0;
        self.order.push_back(token.clone());
        self.entries.insert(token.clone(), (cursor, size.0));
        Ok(token)
    }
}
fn set_token(request: &mut ListRunsRequest, token: Option<String>) {
    request
        .page
        .get_or_insert_with(|| colossus_sdk::PageRequest {
            page_size: 0,
            page_token: String::new(),
        })
        .page_token = token.unwrap_or_default();
}
fn invalid() -> ApiError {
    ApiError::invalid(
        ApiErrorReason::InvalidArgument,
        "page_token",
        "The conversation page is no longer valid.",
    )
}
impl ConversationRuns {
    pub(crate) fn is_cloud(&self, run_id: &str) -> bool {
        self.sources
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .copied()
            .unwrap_or(false)
    }
    pub(crate) fn can_continue(&self, session_id: &str) -> bool {
        self.desktop_sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(session_id)
    }
    fn remember(&self, runs: &[Run], cloud: bool) -> Result<(), ApiError> {
        let mut sources = self.sources.lock().unwrap_or_else(PoisonError::into_inner);
        if runs.iter().any(|run| {
            sources
                .get(&run.run_id)
                .is_some_and(|source| *source != cloud)
        }) {
            return Err(ApiError::failed_precondition(
                ApiErrorReason::InternalInvariant,
                "Conversation source identity changed.",
            ));
        }
        if sources.len()
            + runs
                .iter()
                .filter(|run| !sources.contains_key(&run.run_id))
                .count()
            > MAX_BINDINGS
        {
            return Err(ApiError::resource_exhausted(
                ApiErrorReason::CapacityExceeded,
                "Reconnect the selected workspace before loading more conversations.",
            ));
        }
        let mut sessions = self
            .desktop_sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for run in runs {
            sources.insert(run.run_id.clone(), cloud);
            if !cloud {
                sessions.insert(run.session_id.clone());
            }
        }
        Ok(())
    }
    pub(crate) fn reader(
        &self,
        client: &Colossus,
        run_id: &str,
    ) -> Result<Arc<dyn AgentRunClient>, ApiError> {
        if self.is_cloud(run_id) {
            client.connector_runs().ok_or_else(|| {
                ApiError::failed_precondition(
                    ApiErrorReason::InternalInvariant,
                    "The Control Plane conversation reader is unavailable.",
                )
            })
        } else {
            Ok(client.agent_runs())
        }
    }
    pub(crate) async fn list(
        &self,
        client: &Colossus,
        list: &RunList,
        request: ListRunsRequest,
    ) -> Result<ConversationPage, ApiError> {
        self.list_readers(
            client
                .instance_id()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            client.agent_runs(),
            client.connector_runs(),
            list,
            request,
        )
        .await
    }

    async fn list_readers(
        &self,
        instance: String,
        primary: Arc<dyn AgentRunClient>,
        cloud_reader: Option<Arc<dyn AgentRunClient>>,
        list: &RunList,
        mut request: ListRunsRequest,
    ) -> Result<ConversationPage, ApiError> {
        let page_size = request.page.as_ref().map_or(MAX_PAGE_SIZE, |page| {
            if page.page_size == 0 {
                MAX_PAGE_SIZE
            } else {
                page.page_size.min(MAX_PAGE_SIZE)
            }
        });
        let token = request
            .page
            .as_mut()
            .map(|page| std::mem::take(&mut page.page_token))
            .filter(|token| !token.is_empty());
        request.page = Some(colossus_sdk::PageRequest {
            page_size,
            page_token: String::new(),
        });
        let scope = hex::encode(Sha256::digest(
            serde_json::to_vec(&(instance, &request, cloud_reader.is_some()))
                .map_err(|_| invalid())?,
        ));
        // Pending rows stay in native custody. A renderer cannot forge a row or
        // source binding by editing a serialized continuation token.
        let mut cursor = if let Some(token) = token {
            let cache = self.cursors.lock().unwrap_or_else(PoisonError::into_inner);
            let cursor = cache
                .entries
                .get(&token)
                .map(|entry| entry.0.clone())
                .ok_or_else(invalid)?;
            if cursor.scope != scope {
                return Err(invalid());
            }
            cursor
        } else {
            Cursor {
                scope,
                cloud: SourcePage {
                    done: cloud_reader.is_none(),
                    ..SourcePage::default()
                },
                ..Cursor::default()
            }
        };
        let mut runs = Vec::with_capacity(page_size as usize);
        for _ in 0..page_size {
            if !self
                .refill(&mut cursor.primary, primary.as_ref(), list, &request, false)
                .await?
            {
                break;
            }
            if let Some(cloud) = cloud_reader.as_ref()
                && !self
                    .refill(&mut cursor.cloud, cloud.as_ref(), list, &request, true)
                    .await?
            {
                break;
            }
            // Upstream cursors preserve run creation/discovery order, not live
            // update order. Keep that order and prefer Desktop for equal times.
            let source = match (cursor.primary.pending.front(), cursor.cloud.pending.front()) {
                (Some(left), Some(right)) if left.created_at >= right.created_at => {
                    &mut cursor.primary
                }
                (_, Some(_)) => &mut cursor.cloud,
                (Some(_), None) => &mut cursor.primary,
                (None, None) => break,
            };
            if let Some(run) = source.pending.pop_front() {
                runs.push(run);
            }
        }
        let more = !cursor.primary.done
            || !cursor.cloud.done
            || !cursor.primary.pending.is_empty()
            || !cursor.cloud.pending.is_empty();
        let next_page_token = if more {
            Some(
                self.cursors
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(cursor)?,
            )
        } else {
            None
        };
        Ok(ConversationPage {
            runs,
            next_page_token,
        })
    }

    async fn refill(
        &self,
        source: &mut SourcePage,
        reader: &dyn AgentRunClient,
        list: &RunList,
        request: &ListRunsRequest,
        cloud: bool,
    ) -> Result<bool, ApiError> {
        // Filtered upstream scans can legitimately produce empty pages. Bound
        // admission work and return a continuation without guessing their order.
        for _ in 0..4 {
            if !source.pending.is_empty() || source.done {
                return Ok(true);
            }
            let mut request = request.clone();
            set_token(&mut request, source.token.clone());
            let page = list.list_client(reader, request).await?;
            let next = page
                .page
                .map(|page| page.next_page_token)
                .filter(|token| !token.is_empty());
            if next.is_some() && next == source.token {
                return Err(invalid());
            }
            self.remember(&page.runs, cloud)?;
            source.pending.extend(page.runs);
            source.token = next;
            source.done = source.token.is_none();
        }
        Ok(!source.pending.is_empty() || source.done)
    }
}

/// Fetch only released user text for this exact discovered cloud run, never private worker data.
pub(crate) async fn initial_prompt(reader: &dyn AgentRunClient, run: &Run) -> Option<String> {
    let mut page_token = None;
    for _ in 0..16 {
        let page = reader
            .list_session_activity(colossus_sdk::ListSessionActivityRequest {
                source_run_id: run.run_id.clone(),
                query: String::new(),
                lanes: vec![colossus_sdk::SessionActivityLane::Agent],
                kinds: vec![colossus_sdk::SessionActivityKind::User],
                statuses: vec![],
                page: Some(colossus_sdk::PageRequest {
                    page_size: 100,
                    page_token: page_token.unwrap_or_default(),
                }),
            })
            .await
            .ok()?;
        for activity in page.activities {
            if activity.run_id.as_deref() == Some(run.run_id.as_str())
                && let Some(input) = activity.input
                && input.format == "text"
                && !input.value.is_empty()
            {
                return Some(input.value);
            }
        }
        page_token = page
            .page
            .map(|page| page.next_page_token)
            .filter(|token| !token.is_empty());
        if page_token.is_none() {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use colossus_sdk::*;
    struct Reader {
        id: &'static str,
        calls: Arc<Mutex<Vec<String>>>,
        rows: Option<Vec<Run>>,
    }
    fn run(id: &str, session: &str) -> Run {
        Run {
            run_id: id.into(),
            session_id: session.into(),
            title: id.into(),
            role: "primary".into(),
            mode: RunMode::Execute,
            status: RunStatus::Completed,
            created_at: "2026-10-09T00:00:00Z".into(),
            updated_at: "2026-10-09T00:00:00Z".into(),
            started_at: None,
            finished_at: None,
            last_sequence: 1,
            pending_interaction_count: 0,
            terminal: None,
            etag: "etag".into(),
            archived: false,
            plugin_skill_ids: vec![],
        }
    }
    #[async_trait]
    impl AgentRunClient for Reader {
        async fn list_runs(&self, request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
            let token = request
                .page
                .as_ref()
                .map_or("", |page| page.page_token.as_str());
            self.calls
                .lock()
                .unwrap()
                .push(format!("{}:{token}", self.id));
            if let Some(rows) = &self.rows {
                let start = if token.is_empty() {
                    0
                } else {
                    token.parse::<usize>().unwrap()
                };
                let size = request.page.as_ref().unwrap().page_size as usize;
                let end = (start + size).min(rows.len());
                return Ok(ListRunsResponse {
                    runs: rows[start..end].to_vec(),
                    page: (end < rows.len()).then(|| PageResponse {
                        next_page_token: end.to_string(),
                    }),
                });
            }
            Ok(ListRunsResponse {
                runs: vec![run(&format!("{}{}", self.id, token), "same-session")],
                page: (self.id == "cloud" && token.is_empty()).then(|| PageResponse {
                    next_page_token: "next".into(),
                }),
            })
        }
        async fn create_run(&self, _: CreateRunRequest) -> ApiResult<CreateRunResponse> {
            panic!("read broker cannot create")
        }
        async fn get_run(&self, _: GetRunRequest) -> ApiResult<GetRunResponse> {
            panic!("not used")
        }
        async fn watch_run(&self, _: WatchRunRequest) -> ApiResult<RunUpdateStream> {
            panic!("not used")
        }
        async fn cancel_run(&self, _: CancelRunRequest) -> ApiResult<CancelRunResponse> {
            panic!("read broker cannot cancel")
        }
        async fn respond_interaction(
            &self,
            _: RespondInteractionRequest,
        ) -> ApiResult<RespondInteractionResponse> {
            panic!("read broker cannot answer")
        }
    }
    fn request(token: Option<String>) -> ListRunsRequest {
        ListRunsRequest {
            session_id: None,
            statuses: vec![],
            include_archived: false,
            page: Some(PageRequest {
                page_size: 2,
                page_token: token.unwrap_or_default(),
            }),
        }
    }
    #[tokio::test]
    async fn pages_both_owners_without_repeating_a_finished_source_and_fences_queries() {
        let broker = ConversationRuns::default();
        let calls = Arc::new(Mutex::new(vec![]));
        let primary: Arc<dyn AgentRunClient> = Arc::new(Reader {
            id: "desktop",
            calls: calls.clone(),
            rows: None,
        });
        let cloud: Arc<dyn AgentRunClient> = Arc::new(Reader {
            id: "cloud",
            calls: calls.clone(),
            rows: None,
        });
        let list = RunList::default();
        let first = broker
            .list_readers(
                "runtime-a".into(),
                primary.clone(),
                Some(cloud.clone()),
                &list,
                request(None),
            )
            .await
            .unwrap();
        assert_eq!(first.runs.len(), 2);
        assert!(!broker.is_cloud("desktop"));
        assert!(broker.is_cloud("cloud"));
        assert!(broker.can_continue("same-session"));
        let token = first.next_page_token.unwrap();
        assert!(
            broker
                .list_readers(
                    "runtime-b".into(),
                    primary.clone(),
                    Some(cloud.clone()),
                    &list,
                    request(Some(token.clone()))
                )
                .await
                .is_err()
        );
        let mut changed = request(Some(token.clone()));
        changed.session_id = Some("foreign".into());
        assert!(
            broker
                .list_readers(
                    "runtime-a".into(),
                    primary.clone(),
                    Some(cloud.clone()),
                    &list,
                    changed
                )
                .await
                .is_err()
        );
        let second = broker
            .list_readers(
                "runtime-a".into(),
                primary,
                Some(cloud),
                &list,
                request(Some(token)),
            )
            .await
            .unwrap();
        assert!(second.next_page_token.is_none());
        assert_eq!(second.runs[0].run_id, "cloudnext");
        assert_eq!(*calls.lock().unwrap(), ["desktop:", "cloud:", "cloud:next"]);
    }
    #[test]
    fn a_discovered_run_cannot_change_source_and_a_cloud_only_session_cannot_continue() {
        let broker = ConversationRuns::default();
        broker.remember(&[run("web", "cloud-only")], true).unwrap();
        assert!(!broker.can_continue("cloud-only"));
        assert!(broker.remember(&[run("web", "cloud-only")], false).is_err());
        assert!(broker.is_cloud("web"));
    }
    fn rows(prefix: &str, newest: u32) -> Vec<Run> {
        (0..4)
            .map(|offset| {
                let mut run = run(&format!("{prefix}{offset}"), prefix);
                run.created_at = format!("2026-10-09T00:00:{:02}Z", newest - offset);
                run.updated_at = run.created_at.clone();
                run
            })
            .collect()
    }
    #[tokio::test]
    async fn merged_pages_emit_only_the_global_boundary_and_keep_unconsumed_rows_native() {
        let broker = ConversationRuns::default();
        let calls = Arc::new(Mutex::new(vec![]));
        let primary: Arc<dyn AgentRunClient> = Arc::new(Reader {
            id: "desktop",
            calls: calls.clone(),
            rows: Some(rows("local", 20)),
        });
        let cloud: Arc<dyn AgentRunClient> = Arc::new(Reader {
            id: "cloud",
            calls: calls.clone(),
            rows: Some(rows("remote", 10)),
        });
        let list = RunList::default();
        let mut query = request(None);
        query.page.as_mut().unwrap().page_size = 3;
        let first = broker
            .list_readers(
                "runtime".into(),
                primary.clone(),
                Some(cloud.clone()),
                &list,
                query.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            first
                .runs
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            ["local0", "local1", "local2"]
        );
        let token = first.next_page_token.unwrap();
        assert!(!token.contains("local") && !token.contains("remote"));
        let mut forged = query.clone();
        forged.page.as_mut().unwrap().page_token = format!("{PREFIX}forged-rows");
        assert!(
            broker
                .list_readers(
                    "runtime".into(),
                    primary.clone(),
                    Some(cloud.clone()),
                    &list,
                    forged
                )
                .await
                .is_err()
        );
        let mut second_query = query.clone();
        second_query.page.as_mut().unwrap().page_token = token;
        let second = broker
            .list_readers(
                "runtime".into(),
                primary.clone(),
                Some(cloud.clone()),
                &list,
                second_query.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            second
                .runs
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            ["local3", "remote0", "remote1"]
        );
        let replay = broker
            .list_readers(
                "runtime".into(),
                primary.clone(),
                Some(cloud.clone()),
                &list,
                second_query,
            )
            .await
            .unwrap();
        assert_eq!(replay.runs, second.runs);
        query.page.as_mut().unwrap().page_token = second.next_page_token.unwrap();
        let final_page = broker
            .list_readers("runtime".into(), primary, Some(cloud), &list, query)
            .await
            .unwrap();
        assert_eq!(
            final_page
                .runs
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            ["remote2", "remote3"]
        );
        assert!(final_page.next_page_token.is_none());
        assert!(broker.is_cloud("remote3"));
        assert!(!broker.can_continue("remote"));
    }

    #[test]
    fn native_cursor_custody_is_bounded_and_expired_handles_are_removed() {
        let mut cache = CursorCache::default();
        let first = cache.insert(Cursor::default()).unwrap();
        assert_eq!(cache.insert(Cursor::default()).unwrap(), first);
        for index in 0..MAX_CURSORS {
            cache
                .insert(Cursor {
                    scope: index.to_string(),
                    ..Cursor::default()
                })
                .unwrap();
        }
        assert_eq!(cache.entries.len(), MAX_CURSORS);
        assert!(!cache.entries.contains_key(&first));
        let mut oversized = Cursor::default();
        let mut run = run("large", "session");
        run.title = "x".repeat(MAX_CURSOR_BYTES + 1);
        oversized.primary.pending.push_back(run);
        assert!(cache.insert(oversized).is_err());
    }
}
