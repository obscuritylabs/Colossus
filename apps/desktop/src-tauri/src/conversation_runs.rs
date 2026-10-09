//! Read-only reconciliation of Desktop and Control Plane runs in one selected runtime.
use crate::run_list::RunList;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use colossus_sdk::{AgentRunClient, ApiError, ApiErrorReason, Colossus, ListRunsRequest, Run};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

const PREFIX: &str = "desktop-conversations-v1:";
const MAX_BINDINGS: usize = 4096;
#[derive(Clone, Default)]
pub(crate) struct ConversationRuns {
    sources: Arc<Mutex<HashMap<String, bool>>>,
    desktop_sessions: Arc<Mutex<HashSet<String>>>,
}
pub(crate) struct ConversationPage {
    pub(crate) runs: Vec<Run>,
    pub(crate) next_page_token: Option<String>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    scope: String,
    primary: Option<String>,
    cloud: Option<String>,
    primary_done: bool,
    cloud_done: bool,
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
            .unwrap_or_else(|e| e.into_inner())
            .get(run_id)
            .copied()
            .unwrap_or(false)
    }
    pub(crate) fn can_continue(&self, session_id: &str) -> bool {
        self.desktop_sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(session_id)
    }
    fn remember(&self, runs: &[Run], cloud: bool) -> Result<(), ApiError> {
        let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
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
            .unwrap_or_else(|e| e.into_inner());
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
        let mut query = request.clone();
        query.page = Some(colossus_sdk::PageRequest {
            page_size: query.page.as_ref().map_or(0, |page| page.page_size),
            page_token: String::new(),
        });
        let scope = hex::encode(Sha256::digest(
            serde_json::to_vec(&(instance, query)).map_err(|_| invalid())?,
        ));
        let mut cursor = match request
            .page
            .as_mut()
            .map(|page| std::mem::take(&mut page.page_token))
            .filter(|token| !token.is_empty())
        {
            Some(token) => {
                let encoded = token.strip_prefix(PREFIX).ok_or_else(invalid)?;
                let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
                let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
                if cursor.scope != scope {
                    return Err(invalid());
                }
                cursor
            }
            None => Cursor {
                scope,
                ..Cursor::default()
            },
        };
        let mut runs = Vec::new();
        if !cursor.primary_done {
            set_token(&mut request, cursor.primary.take());
            let page = list.list_client(primary.as_ref(), request.clone()).await?;
            self.remember(&page.runs, false)?;
            runs.extend(page.runs);
            cursor.primary = page
                .page
                .map(|page| page.next_page_token)
                .filter(|token| !token.is_empty());
            cursor.primary_done = cursor.primary.is_none();
        }
        if !cursor.cloud_done {
            if let Some(cloud) = cloud_reader {
                set_token(&mut request, cursor.cloud.take());
                let page = list.list_client(cloud.as_ref(), request).await?;
                self.remember(&page.runs, true)?;
                runs.extend(page.runs);
                cursor.cloud = page
                    .page
                    .map(|page| page.next_page_token)
                    .filter(|token| !token.is_empty());
                cursor.cloud_done = cursor.cloud.is_none();
            } else {
                cursor.cloud_done = true;
            }
        }
        runs.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.run_id.cmp(&b.run_id))
        });
        let next_page_token = if cursor.primary_done && cursor.cloud_done {
            None
        } else {
            Some(format!(
                "{PREFIX}{}",
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).map_err(|_| invalid())?)
            ))
        };
        Ok(ConversationPage {
            runs,
            next_page_token,
        })
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
                page_size: 50,
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
        });
        let cloud: Arc<dyn AgentRunClient> = Arc::new(Reader {
            id: "cloud",
            calls: calls.clone(),
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
}
