use super::*;
use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use colossus_sdk::*;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;

const TOKEN: &str = "test-peer-secret-with-at-least-32-bytes";
const OTHER: &str = "other-peer-secret-with-at-least-32-bytes";

struct Client {
    id: &'static str,
    status: Mutex<AgentTaskStatus>,
    inputs: Mutex<Vec<SubmitAgentTaskMessageRequest>>,
    queries: Mutex<Vec<ListAgentTasksRequest>>,
    complete_on_watch: bool,
    read_throttles: AtomicUsize,
    reads: AtomicUsize,
}
impl Client {
    fn new(id: &'static str, complete_on_watch: bool) -> Self {
        Self {
            id,
            status: Mutex::new(AgentTaskStatus::Queued),
            inputs: Mutex::default(),
            queries: Mutex::default(),
            complete_on_watch,
            read_throttles: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
        }
    }
    fn task(&self, history: u32) -> AgentTaskSnapshot {
        let status = *self.status.lock().unwrap();
        AgentTaskSnapshot {
            task_id: self.id.into(),
            context_id: format!("context-{}", self.id),
            status,
            status_updated_at: "2026-10-09T12:00:00Z".into(),
            last_sequence: if status.is_terminal() { 2 } else { 1 },
            output: status.is_terminal().then(|| "Released result".into()),
            failure: None,
            waiting_kind: None,
            history: if history == 0 {
                Vec::new()
            } else {
                self.inputs
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .take(history as usize)
                    .map(|input| AgentTaskInput {
                        message_id: input.message_id.clone(),
                        task_id: self.id.into(),
                        context_id: format!("context-{}", self.id),
                        text: input.text.clone(),
                        accepted_at: "2026-10-09T12:00:00Z".into(),
                        inbox_message_id: None,
                    })
                    .collect()
            },
        }
    }
}
fn unavailable() -> ApiError {
    ApiError::not_found(ApiErrorReason::RunNotFound, "Task not found")
}
#[async_trait]
impl AgentRunClient for Client {
    async fn submit_agent_task_message(
        &self,
        request: SubmitAgentTaskMessageRequest,
    ) -> ApiResult<AgentTaskSnapshot> {
        if request.task_id.as_ref().is_some_and(|id| id != self.id) {
            return Err(unavailable());
        }
        self.inputs.lock().unwrap().push(request);
        Ok(self.task(0))
    }
    async fn get_agent_task(&self, request: GetAgentTaskRequest) -> ApiResult<AgentTaskSnapshot> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if request.task_id != self.id {
            return Err(unavailable());
        }
        if self
            .read_throttles
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(ApiError::resource_exhausted(
                ApiErrorReason::CapacityExceeded,
                "Read admission throttled",
            ));
        }
        Ok(self.task(request.history_length))
    }
    async fn list_agent_tasks(
        &self,
        request: ListAgentTasksRequest,
    ) -> ApiResult<ListAgentTasksResponse> {
        let mut task = self.task(request.history_length);
        if !request.include_output {
            task.output = None;
        }
        self.queries.lock().unwrap().push(request);
        Ok(ListAgentTasksResponse {
            tasks: vec![task],
            next_page_token: None,
            total_size: 1,
        })
    }
    async fn watch_run(&self, _request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        if self.complete_on_watch {
            *self.status.lock().unwrap() = AgentTaskStatus::Completed;
            Ok(Box::pin(futures::stream::iter([Ok(RunUpdate {
                run_id: self.id.into(),
                sequence: 2,
                created_at: "2026-10-09T12:00:00Z".into(),
                update: RunUpdateKind::State(RunStatus::Completed),
            })])))
        } else {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
    async fn create_run(&self, _: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        Err(unavailable())
    }
    async fn get_run(&self, _: GetRunRequest) -> ApiResult<GetRunResponse> {
        Err(unavailable())
    }
    async fn list_runs(&self, _: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        Err(unavailable())
    }
    async fn cancel_run(&self, _: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        Err(unavailable())
    }
    async fn respond_interaction(
        &self,
        _: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        panic!("peer input must never answer an interaction")
    }
}

#[tokio::test]
async fn read_backpressure_does_not_replay_submission() {
    let client = Arc::new(Client::new("task-1", true));
    client.read_throttles.store(1, Ordering::Relaxed);
    let (_, body) = call(
        &router(client.clone()),
        Some(TOKEN),
        Some("1.0"),
        "SendMessage",
        message(),
    )
    .await;
    assert_eq!(
        value(&body)["result"]["task"]["status"]["state"],
        "TASK_STATE_COMPLETED"
    );
    assert_eq!(client.inputs.lock().unwrap().len(), 1);
    assert_eq!(client.reads.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn premature_stream_end_reconciles_and_preserves_the_accepted_task() {
    let client = Arc::new(Client::new("task-1", false));
    let (_, body) = call(
        &router(client),
        Some(TOKEN),
        Some("1.0"),
        "SendStreamingMessage",
        message(),
    )
    .await;
    let events = body
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(value)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0]["result"]["task"]["status"]["state"],
        "TASK_STATE_SUBMITTED"
    );
    assert_eq!(events[1]["error"]["data"]["taskId"], "task-1");
    assert_eq!(events[1]["error"]["data"]["accepted"], true);
    assert!(!body.contains("TASK_STATE_COMPLETED"));
}
fn router(client: Arc<Client>) -> axum::Router {
    A2aListener::new(
        "https://agent.example".into(),
        vec![
            (
                crate::service::credential_digest(TOKEN),
                PeerProfile::new(client, "primary".into(), 2).unwrap(),
            ),
            (
                crate::service::credential_digest(OTHER),
                PeerProfile::new(
                    Arc::new(Client::new("other-task", false)),
                    "limited".into(),
                    1,
                )
                .unwrap(),
            ),
        ],
    )
    .unwrap()
    .router()
}
async fn call(
    router: &axum::Router,
    token: Option<&str>,
    version: Option<&str>,
    method: &str,
    params: Value,
) -> (StatusCode, String) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/")
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(version) = version {
        request = request.header("A2A-Version", version);
    }
    let response = router.clone().oneshot(request.body(Body::from(json!({"jsonrpc": "2.0", "id": "request-1", "method": method, "params": params}).to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}
fn message() -> Value {
    json!({"message": {"messageId": "message-1", "role": "ROLE_USER", "parts": [{"text": "Review this code"}]}})
}
fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn authentication_and_version_fail_before_task_allocation() {
    let client = Arc::new(Client::new("task-1", false));
    let router = router(client.clone());
    assert_eq!(
        call(&router, None, Some("1.0"), "SendMessage", message())
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &router,
            Some("wrong-credential-with-at-least-32-bytes"),
            Some("1.0"),
            "SendMessage",
            message()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (_, text) = call(&router, Some(TOKEN), None, "SendMessage", message()).await;
    assert_eq!(value(&text)["error"]["code"], -32009);
    assert!(client.inputs.lock().unwrap().is_empty());
    let card = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/.well-known/agent-card.json")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(card.into_body(), 64 * 1024).await.unwrap();
    let card: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(card["supportedInterfaces"][0]["protocolVersion"], "1.0");
    assert_eq!(card["capabilities"]["streaming"], true);
    assert_eq!(card["capabilities"]["pushNotifications"], false);
    assert!(card["securityRequirements"][0]["schemes"]["peerBearer"]["list"].is_array());
    assert!(!String::from_utf8(bytes.to_vec()).unwrap().contains(TOKEN));
}

#[tokio::test]
async fn metadata_cannot_choose_authority_and_identifiers_do_not_cross_peers() {
    let client = Arc::new(Client::new("task-1", false));
    let router = router(client.clone());
    let mut params = message();
    params["configuration"] = json!({"returnImmediately": true, "historyLength": 0});
    params["message"]["metadata"] = json!({"role": "administrator", "workspace": "/private", "maxTurns": 999, "approval": true});
    let (_, text) = call(&router, Some(TOKEN), Some("1.0"), "SendMessage", params).await;
    let result = value(&text);
    assert_eq!(result["result"]["task"]["id"], "task-1");
    assert!(result["result"]["task"].get("history").is_none());
    {
        let inputs = client.inputs.lock().unwrap();
        assert_eq!(inputs[0].role, "primary");
        assert_eq!(inputs[0].max_turns, 2);
    }
    let (_, text) = call(
        &router,
        Some(OTHER),
        Some("1.0"),
        "GetTask",
        json!({"id": "task-1"}),
    )
    .await;
    assert_eq!(value(&text)["error"]["code"], -32001);
}

#[tokio::test]
async fn unsupported_content_and_features_never_submit_work() {
    let client = Arc::new(Client::new("task-1", false));
    let router = router(client.clone());
    for (parts, code) in [
        (json!([{"raw": "ZXZpbA=="}]), -32005),
        (json!([{"url": "https://private.example"}]), -32005),
        (json!([{"data": {"command": "run"}}]), -32005),
        (
            json!([{"text": "x", "mediaType": "application/json"}]),
            -32005,
        ),
        (json!([{"text": "x".repeat(16 * 1024 + 1)}]), -32602),
    ] {
        let mut params = message();
        params["message"]["parts"] = parts;
        let (_, text) = call(&router, Some(TOKEN), Some("1.0"), "SendMessage", params).await;
        assert_eq!(value(&text)["error"]["code"], code);
    }
    let mut params = message();
    params["message"]["role"] = json!("ROLE_AGENT");
    assert_eq!(
        value(
            &call(&router, Some(TOKEN), Some("1.0"), "SendMessage", params)
                .await
                .1
        )["error"]["code"],
        -32602
    );
    let mut params = message();
    params["configuration"] =
        json!({"taskPushNotificationConfig": {"url": "https://callback.example"}});
    assert_eq!(
        value(
            &call(&router, Some(TOKEN), Some("1.0"), "SendMessage", params)
                .await
                .1
        )["error"]["code"],
        -32004
    );
    assert!(client.inputs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn default_blocking_waits_for_completion_and_interrupted_watch_preserves_task() {
    let client = Arc::new(Client::new("task-1", true));
    let (_, text) = call(
        &router(client),
        Some(TOKEN),
        Some("1.0"),
        "SendMessage",
        message(),
    )
    .await;
    let result = value(&text);
    assert_eq!(
        result["result"]["task"]["status"]["state"],
        "TASK_STATE_COMPLETED"
    );
    assert_eq!(
        result["result"]["task"]["artifacts"][0]["parts"][0]["text"],
        "Released result"
    );
    let (_, text) = call(
        &router(Arc::new(Client::new("task-2", false))),
        Some(TOKEN),
        Some("1.0"),
        "SendMessage",
        message(),
    )
    .await;
    assert_eq!(
        value(&text)["error"]["data"],
        json!({"taskId": "task-2", "accepted": true})
    );
}

#[tokio::test]
async fn streaming_starts_with_task_and_emits_artifact_before_terminal_status() {
    let (_, text) = call(
        &router(Arc::new(Client::new("task-1", true))),
        Some(TOKEN),
        Some("1.0"),
        "SendStreamingMessage",
        message(),
    )
    .await;
    let events = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(value)
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 3);
    assert_eq!(
        events[0]["result"]["task"]["status"]["state"],
        "TASK_STATE_SUBMITTED"
    );
    assert_eq!(events[1]["result"]["artifactUpdate"]["lastChunk"], true);
    assert_eq!(
        events[2]["result"]["statusUpdate"]["status"]["state"],
        "TASK_STATE_COMPLETED"
    );
    assert!(!text.contains("toolActivity"));
}

#[tokio::test]
async fn listing_forwards_status_filters_and_omits_artifacts_by_default() {
    let client = Arc::new(Client::new("task-1", false));
    *client.status.lock().unwrap() = AgentTaskStatus::Completed;
    let router = router(client.clone());
    let (_, text) = call(
        &router,
        Some(TOKEN),
        Some("1.0"),
        "ListTasks",
        json!({"status": "TASK_STATE_COMPLETED", "historyLength": 0}),
    )
    .await;
    let result = value(&text);
    assert_eq!(result["result"]["nextPageToken"], "");
    assert_eq!(result["result"]["totalSize"], 1);
    assert!(result["result"]["tasks"][0].get("artifacts").is_none());
    assert_eq!(
        client.queries.lock().unwrap()[0].statuses,
        vec![AgentTaskStatus::Completed]
    );
    let (_, text) = call(
        &router,
        Some(TOKEN),
        Some("1.0"),
        "ListTasks",
        json!({"status": "TASK_STATE_REJECTED"}),
    )
    .await;
    assert_eq!(value(&text)["result"]["totalSize"], 0);
}
