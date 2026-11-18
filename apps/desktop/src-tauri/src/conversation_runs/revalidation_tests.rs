use super::*;
use crate::run_bindings::MAX_BINDINGS;
use async_trait::async_trait;
use colossus_sdk::*;

#[derive(Clone, Default)]
struct Reader {
    name: &'static str,
    owned: Vec<Run>,
    visible: HashMap<String, Run>,
    get_error: Option<ApiError>,
    list_error: Option<ApiError>,
    empty_pages: usize,
    calls: Arc<Mutex<Vec<String>>>,
}
impl Reader {
    fn owns(name: &'static str, runs: Vec<Run>, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            name,
            visible: runs
                .iter()
                .map(|run| (run.run_id.clone(), run.clone()))
                .collect(),
            owned: runs,
            calls,
            ..Self::default()
        }
    }
}
#[async_trait]
impl AgentRunClient for Reader {
    async fn get_run(&self, request: GetRunRequest) -> ApiResult<GetRunResponse> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:get:{}", self.name, request.run_id));
        if let Some(error) = &self.get_error {
            return Err(error.clone());
        }
        Ok(GetRunResponse {
            run: self
                .visible
                .get(&request.run_id)
                .cloned()
                .ok_or_else(|| ApiError::not_found(ApiErrorReason::RunNotFound, "missing"))?,
            pending_interactions: vec![],
        })
    }
    async fn list_runs(&self, request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        let page = request.page.unwrap();
        assert!(page.page_size <= MAX_PAGE_SIZE);
        self.calls.lock().unwrap().push(format!(
            "{}:list:{}:{}",
            self.name,
            request.session_id.as_deref().unwrap_or("all"),
            page.page_token
        ));
        if let Some(error) = &self.list_error {
            return Err(error.clone());
        }
        let digest = hex::encode(Sha256::digest(
            serde_json::to_vec(&(
                &request.session_id,
                &request.statuses,
                request.include_archived,
                page.page_size,
            ))
            .unwrap(),
        ));
        let start = if page.page_token.is_empty() {
            0
        } else {
            let (scope, offset) = page.page_token.split_once(':').ok_or_else(invalid)?;
            if scope != digest {
                return Err(invalid());
            }
            offset.parse::<usize>().map_err(|_| invalid())?
        };
        // Match the real owner index: scan at most 64 GLOBAL records before
        // applying a session/status/archive filter. Filtering first masks deep-history failures.
        let empty_records = self.empty_pages * 64;
        let total = empty_records + self.owned.len();
        if start > total {
            return Err(invalid());
        }
        let limit = (start + 64).min(total);
        let mut offset = start;
        let mut runs = Vec::new();
        while offset < limit && runs.len() < page.page_size as usize {
            if offset >= empty_records {
                let run = &self.owned[offset - empty_records];
                if request
                    .session_id
                    .as_ref()
                    .is_none_or(|session| session == &run.session_id)
                    && (request.statuses.is_empty() || request.statuses.contains(&run.status))
                    && (request.include_archived || !run.archived)
                {
                    runs.push(run.clone());
                }
            }
            offset += 1;
        }
        Ok(ListRunsResponse {
            runs,
            page: (offset < total).then(|| PageResponse {
                next_page_token: format!("{digest}:{offset}"),
            }),
        })
    }
    async fn create_run(&self, _: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        panic!("read proof cannot create")
    }
    async fn watch_run(&self, _: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        panic!("read proof cannot watch")
    }
    async fn cancel_run(&self, _: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        panic!("read proof cannot cancel")
    }
    async fn respond_interaction(
        &self,
        _: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        panic!("read proof cannot answer")
    }
}
#[async_trait]
impl Backend for Reader {
    fn kind(&self) -> BackendKind {
        BackendKind::Sidecar
    }
    fn agent_runs(&self) -> Arc<dyn AgentRunClient> {
        Arc::new(self.clone())
    }
    async fn close(&self) -> SdkResult<()> {
        Ok(())
    }
}
fn run(id: &str, session: &str) -> Run {
    Run {
        run_id: id.into(),
        session_id: session.into(),
        title: id.into(),
        title_revision: 0,
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
fn get(id: &str) -> GetRunRequest {
    GetRunRequest { run_id: id.into() }
}
fn query(token: Option<String>) -> ListRunsRequest {
    ListRunsRequest {
        session_id: None,
        statuses: vec![],
        include_archived: false,
        page: Some(PageRequest {
            page_size: MAX_PAGE_SIZE,
            page_token: token.unwrap_or_default(),
        }),
    }
}
fn output(run: &Run) -> &str {
    match run.terminal.as_ref().unwrap() {
        RunTerminal::Result(result) => &result.output,
        _ => panic!("result"),
    }
}
fn with_output(mut run: Run) -> Run {
    run.terminal = Some(RunTerminal::Result(RunResult {
        output: "full released output".repeat(1000),
        plan_id: None,
        plan_revision: None,
        plan_status: None,
        goal_id: None,
        profile: "default".into(),
        model_profile: "default".into(),
        provider_profile: "default".into(),
        model: "fixture".into(),
        elapsed_seconds: 1.0,
    }));
    run
}

#[tokio::test]
async fn history_beyond_binding_capacity_reopens_old_primary_and_connector_rows() {
    let broker = ConversationRuns::default();
    let calls = Arc::new(Mutex::new(vec![]));
    let rows = (0..MAX_BINDINGS + 3)
        .map(|index| run(&format!("primary-{index}"), &format!("session-{index}")))
        .collect();
    let primary: Arc<dyn AgentRunClient> = Arc::new(Reader::owns("primary", rows, calls.clone()));
    let mut remote = run("connector-old", "session-0");
    remote.created_at = "2026-10-10T00:00:00Z".into();
    let cloud: Arc<dyn AgentRunClient> =
        Arc::new(Reader::owns("connector", vec![remote], calls.clone()));
    let list = RunList::default();
    let mut token = None;
    let mut count = 0;
    loop {
        let page = broker
            .list_readers(
                "runtime".into(),
                primary.clone(),
                Some(cloud.clone()),
                &list,
                query(token),
            )
            .await
            .unwrap();
        assert!(page.runs.len() <= MAX_PAGE_SIZE as usize);
        count += page.runs.len();
        token = page.next_page_token;
        if token.is_none() {
            break;
        }
    }
    assert_eq!(count, MAX_BINDINGS + 4);
    assert_eq!(broker.sources.lock().unwrap().len(), MAX_BINDINGS);
    assert_eq!(broker.desktop_sessions.lock().unwrap().len(), MAX_BINDINGS);
    assert!(broker.source("primary-0").is_none());
    assert!(broker.source("connector-old").is_none());
    assert!(!broker.can_continue("session-0"));
    let old = broker
        .get_readers(
            primary.clone(),
            Some(cloud.clone()),
            &list,
            get("primary-0"),
        )
        .await
        .unwrap();
    assert_eq!(old.source, ConversationSource::Primary);
    assert!(old.continuable);
    let old = broker
        .get_readers(primary, Some(cloud), &list, get("connector-old"))
        .await
        .unwrap();
    assert_eq!(old.source, ConversationSource::Cloud);
    assert!(!old.source.can_control());
    assert!(old.continuable);
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .any(|call| call == "connector:get:connector-old")
    );
}

#[tokio::test]
async fn only_typed_run_absence_allows_connector_fallback() {
    for mut error in [
        ApiError::permission_denied(ApiErrorReason::ScopeDenied, "denied"),
        ApiError::not_found(ApiErrorReason::ArtifactNotFound, "other absence"),
        ApiError::not_found(ApiErrorReason::RunNotFound, "transport"),
    ] {
        if error.message == "transport" {
            error.code = ApiErrorCode::Unavailable;
        }
        let broker = ConversationRuns::default();
        let calls = Arc::new(Mutex::new(vec![]));
        let primary: Arc<dyn AgentRunClient> = Arc::new(Reader {
            name: "primary",
            get_error: Some(error.clone()),
            calls: calls.clone(),
            ..Reader::default()
        });
        let cloud: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
            "connector",
            vec![run("remote", "session")],
            calls.clone(),
        ));
        let actual = broker
            .get_readers(primary, Some(cloud), &RunList::default(), get("remote"))
            .await
            .err()
            .unwrap();
        assert_eq!(actual, error);
        assert_eq!(*calls.lock().unwrap(), ["primary:get:remote"]);
        assert!(broker.source("remote").is_none());
    }
}

#[tokio::test]
async fn cached_connector_never_uses_primary_when_connector_is_missing_or_run_disappears() {
    for missing in [true, false] {
        let broker = ConversationRuns::default();
        broker.remember(&[run("remote", "session")], true).unwrap();
        let calls = Arc::new(Mutex::new(vec![]));
        let primary: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
            "primary",
            vec![run("remote", "session")],
            calls.clone(),
        ));
        let cloud = (!missing).then(|| {
            Arc::new(Reader {
                name: "connector",
                calls: calls.clone(),
                ..Reader::default()
            }) as Arc<dyn AgentRunClient>
        });
        assert!(
            broker
                .get_readers(primary, cloud, &RunList::default(), get("remote"))
                .await
                .is_err()
        );
        assert!(
            calls
                .lock()
                .unwrap()
                .iter()
                .all(|call| !call.starts_with("primary:"))
        );
        assert_eq!(broker.source("remote"), Some(ConversationSource::Cloud));
    }
}

#[tokio::test]
async fn a_shared_read_and_owned_continuation_do_not_grant_source_run_control() {
    let broker = ConversationRuns::default();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut primary = Reader::owns(
        "primary",
        vec![run("desktop-continuation", "shared-session")],
        calls.clone(),
    );
    primary.visible.insert(
        "shared-source".into(),
        run("shared-source", "shared-session"),
    );
    let primary: Arc<dyn AgentRunClient> = Arc::new(primary);
    let cloud: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
        "connector",
        vec![run("shared-source", "shared-session")],
        calls.clone(),
    ));
    let verified = broker
        .get_readers(
            primary,
            Some(cloud),
            &RunList::default(),
            get("shared-source"),
        )
        .await
        .unwrap();
    assert_eq!(verified.source, ConversationSource::PrimarySharedReadOnly);
    assert!(!verified.source.can_control());
    assert!(verified.continuable);
    assert!(
        calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| !call.starts_with("connector:"))
    );
    // Later authenticated connector discovery can refine an unproved owner.
    broker
        .remember(&[run("shared-source", "shared-session")], true)
        .unwrap();
    assert_eq!(
        broker.source("shared-source"),
        Some(ConversationSource::Cloud)
    );
    assert!(
        broker
            .remember(&[run("shared-source", "shared-session")], false)
            .is_err()
    );
}

#[tokio::test]
async fn ownership_proofs_follow_empty_pages_and_remain_bounded() {
    for empty_pages in [2, 100] {
        let broker = ConversationRuns::default();
        let calls = Arc::new(Mutex::new(vec![]));
        let mut primary = Reader::owns("primary", vec![run("old", "session")], calls.clone());
        primary.empty_pages = empty_pages;
        let verified = broker
            .get_readers(Arc::new(primary), None, &RunList::default(), get("old"))
            .await
            .unwrap();
        assert_eq!(verified.source.can_control(), empty_pages == 2);
        assert_eq!(verified.continuable, empty_pages == 2);
        let reads = calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.contains(":list:"))
            .count();
        assert_eq!(reads, if empty_pages == 2 { 3 } else { 16 });
    }
}

#[tokio::test]
async fn a_failed_ownership_probe_preserves_authorized_read_without_control() {
    let calls = Arc::new(Mutex::new(vec![]));
    for connector in [false, true] {
        let broker = ConversationRuns::default();
        let mut primary = Reader::owns(
            "primary",
            if connector {
                vec![]
            } else {
                vec![run("readable", "session")]
            },
            calls.clone(),
        );
        primary.list_error = Some(ApiError::permission_denied(
            ApiErrorReason::ScopeDenied,
            "proof unavailable",
        ));
        let cloud = connector.then(|| {
            Arc::new(Reader::owns(
                "connector",
                vec![run("readable", "session")],
                calls.clone(),
            )) as Arc<dyn AgentRunClient>
        });
        let verified = broker
            .get_readers(
                Arc::new(primary),
                cloud,
                &RunList::default(),
                get("readable"),
            )
            .await
            .unwrap();
        assert!(!verified.source.can_control());
        assert!(!verified.continuable);
    }
}

#[tokio::test]
async fn get_rejects_wrong_run_and_changed_discovered_session() {
    for changed_session in [false, true] {
        let broker = ConversationRuns::default();
        broker
            .remember(&[run("requested", "session")], false)
            .unwrap();
        let returned = if changed_session {
            run("requested", "another-session")
        } else {
            run("another-run", "session")
        };
        let reader = Reader {
            visible: HashMap::from([("requested".into(), returned)]),
            ..Reader::default()
        };
        let error = broker
            .get_readers(
                Arc::new(reader),
                None,
                &RunList::default(),
                get("requested"),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
    }
}

#[tokio::test]
async fn history_and_pending_cursors_drop_output_while_get_keeps_full_result() {
    let broker = ConversationRuns::default();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut local = with_output(run("local", "local-session"));
    local.created_at = "2026-10-10T00:00:00Z".into();
    let primary: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
        "primary",
        vec![local.clone(), with_output(run("older", "older-session"))],
        calls.clone(),
    ));
    let cloud: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
        "connector",
        vec![with_output(run("remote", "remote-session"))],
        calls,
    ));
    let mut request = query(None);
    request.page.as_mut().unwrap().page_size = 1;
    let page = broker
        .list_readers(
            "runtime".into(),
            primary.clone(),
            Some(cloud.clone()),
            &RunList::default(),
            request,
        )
        .await
        .unwrap();
    assert_eq!(page.sources["local"], ConversationSource::Primary);
    assert_eq!(output(&page.runs[0]), "");
    {
        let cursors = broker.cursors.lock().unwrap();
        let cursor = &cursors.entries[&page.next_page_token.unwrap()].0;
        assert_eq!(output(cursor.cloud.pending.front().unwrap()), "");
    }
    let selected = broker
        .get_readers(primary, Some(cloud), &RunList::default(), get("local"))
        .await
        .unwrap();
    assert_eq!(output(&selected.response.run), output(&local));
    assert!(selected.source.can_control());
    assert!(selected.continuable);
}

#[tokio::test]
async fn an_evicted_active_control_reauthenticates_against_its_selected_native_target() {
    use crate::state::{AppState, TargetConsentContext};
    let state = AppState::default();
    let calls = Arc::new(Mutex::new(vec![]));
    let old = run("active-control", "active-session");
    let reader = Reader::owns("selected", vec![old.clone()], calls.clone());
    state
        .replace_target(
            "selected",
            Colossus::from_backend(reader),
            TargetConsentContext::ManagedLocal,
        )
        .await;
    state.select_target(Some("selected".into())).await;
    let lease = state.selected_target("selected").await.unwrap();
    lease.target.remember_primary_conversation(&old).unwrap();
    state.bind_runs(&lease, vec![old.run_id.clone()]).await;
    for index in 0..MAX_BINDINGS {
        let discovered = run(&format!("discovered-{index}"), &format!("session-{index}"));
        lease
            .target
            .remember_primary_conversation(&discovered)
            .unwrap();
        state.bind_runs(&lease, vec![discovered.run_id]).await;
    }
    assert!(!state.run_is_bound(&lease, &old.run_id).await);
    assert!(lease.target.conversation_source(&old.run_id).is_none());
    let source = crate::commands::require_run_binding(&state, &lease, &old.run_id)
        .await
        .unwrap();
    assert!(source.can_control());
    assert!(state.run_is_bound(&lease, &old.run_id).await);
    assert_eq!(
        *calls.lock().unwrap(),
        [
            "selected:get:active-control",
            "selected:list:active-session:"
        ]
    );
}

async fn scan_history_receipt(
    broker: &ConversationRuns,
    primary: Arc<dyn AgentRunClient>,
    cloud: Arc<dyn AgentRunClient>,
    list: &RunList,
    run_id: &str,
) -> (usize, String) {
    let mut token = None;
    let mut receipt = None;
    let mut count = 0;
    loop {
        let page = broker
            .list_readers(
                "runtime".into(),
                primary.clone(),
                Some(cloud.clone()),
                list,
                query(token),
            )
            .await
            .unwrap();
        assert!(page.runs.len() <= 3);
        receipt = page.proofs.get(run_id).cloned().or(receipt);
        count += page.runs.len();
        token = page.next_page_token;
        if token.is_none() {
            break;
        }
    }
    (count, receipt.unwrap())
}

#[tokio::test]
async fn deep_global_history_receipt_recovers_evicted_exact_ownership_and_shared_continuation() {
    let broker = ConversationRuns::default();
    let calls = Arc::new(Mutex::new(vec![]));
    let deep_id = format!("primary-{}", MAX_BINDINGS + 1);
    let deep_session = format!("session-{}", MAX_BINDINGS + 1);
    let rows = (0..MAX_BINDINGS * 2 + 6)
        .map(|index| run(&format!("primary-{index}"), &format!("session-{index}")))
        .collect();
    let primary: Arc<dyn AgentRunClient> = Arc::new(Reader::owns("primary", rows, calls.clone()));
    let remote = run("cloud-source", &deep_session);
    let cloud: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
        "connector",
        vec![remote.clone()],
        calls.clone(),
    ));
    let list = RunList::default();
    let (count, receipt) =
        scan_history_receipt(&broker, primary.clone(), cloud.clone(), &list, &deep_id).await;
    assert_eq!(count, MAX_BINDINGS * 2 + 7);
    let parsed = DiscoveryProof::decode(&receipt, &broker.proof_scope.lock().unwrap()).unwrap();
    assert!(
        !parsed
            .page
            .request
            .page
            .as_ref()
            .unwrap()
            .page_token
            .is_empty()
    );
    assert!(broker.source(&deep_id).is_none());
    assert!(!broker.can_continue(&deep_session));
    // Without the receipt, sixteen session-filtered global scans cannot reach it.
    let unproved = broker
        .get_readers(primary.clone(), Some(cloud.clone()), &list, get(&deep_id))
        .await
        .unwrap();
    assert_eq!(unproved.source, ConversationSource::PrimarySharedReadOnly);
    assert!(!unproved.continuable);
    calls.lock().unwrap().clear();
    let recovered = broker
        .get_readers_with_proofs(
            primary.clone(),
            Some(cloud.clone()),
            &list,
            get(&deep_id),
            ConversationProofs {
                discovery: Some(receipt.clone()),
                continuation: None,
            },
        )
        .await
        .unwrap();
    assert!(recovered.source.can_control());
    assert!(recovered.continuable);
    assert_eq!(recovered.discovery_proof, Some(receipt.clone()));
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.contains(":list:"))
            .count(),
        1
    );
    // Expire session metadata again, then use a different exact owned row only
    // as continuation evidence for the shared source, never source-run control.
    broker.desktop_sessions.lock().unwrap().clear();
    calls.lock().unwrap().clear();
    let shared = broker
        .get_readers_with_proofs(
            primary,
            Some(cloud),
            &list,
            get("cloud-source"),
            ConversationProofs {
                discovery: None,
                continuation: Some(receipt),
            },
        )
        .await
        .unwrap();
    assert!(!shared.source.can_control());
    assert!(shared.continuable);
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.contains(":list:"))
            .count(),
        1
    );
}

#[tokio::test]
async fn discovery_receipts_never_grant_authority_without_an_exact_owned_page_match() {
    for case in [
        "run",
        "session",
        "cursor",
        "query",
        "source",
        "scope",
        "denied",
        "transport",
    ] {
        let broker = ConversationRuns::default();
        let calls = Arc::new(Mutex::new(vec![]));
        let owned = run("owned", "session");
        let mut reader = Reader::owns(
            "primary",
            vec![run("different-owned", "different-session")],
            calls.clone(),
        );
        reader.visible.insert(owned.run_id.clone(), owned.clone());
        let page = DiscoveryPage {
            scope: broker.proof_scope.lock().unwrap().clone(),
            connector: false,
            request: query(None),
        };
        let mut proof = DiscoveryProof::decode(
            &DiscoveryProof::encode(&page, &owned).unwrap(),
            &broker.proof_scope.lock().unwrap(),
        )
        .unwrap();
        match case {
            "run" => proof.run_id = "another".into(),
            "session" => proof.session_id = "another-session".into(),
            "cursor" => proof.page.request.page.as_mut().unwrap().page_token = "forged".into(),
            "query" => {
                proof.page.request.page.as_mut().unwrap().page_token =
                    format!("{}:0", hex::encode(Sha256::digest(b"different-query")));
                proof.page.request.include_archived = true;
            }
            "source" => proof.page.connector = true,
            "scope" => proof.page.scope = "another-target".into(),
            "denied" => {
                reader.list_error = Some(ApiError::permission_denied(
                    ApiErrorReason::ScopeDenied,
                    "denied",
                ));
            }
            "transport" => {
                let mut error = ApiError::not_found(ApiErrorReason::RunNotFound, "transport");
                error.code = ApiErrorCode::Unavailable;
                reader.list_error = Some(error);
            }
            _ => unreachable!(),
        }
        let result = broker
            .get_readers_with_proofs(
                Arc::new(reader),
                None,
                &RunList::default(),
                get("owned"),
                ConversationProofs {
                    discovery: Some(serde_json::to_string(&proof).unwrap()),
                    continuation: None,
                },
            )
            .await;
        if let Ok(read) = result {
            assert!(!read.source.can_control(), "{case}");
            assert!(!read.continuable, "{case}");
            assert!(read.discovery_proof.is_none(), "{case}");
        }
        assert!(
            !broker
                .source("owned")
                .is_some_and(ConversationSource::can_control),
            "{case}"
        );
        assert!(!broker.can_continue("session"), "{case}");
    }
}

#[tokio::test]
async fn foreground_head_ownership_survives_eviction_without_background_poll_replacement() {
    let broker = ConversationRuns::default();
    let calls = Arc::new(Mutex::new(vec![]));
    let initial: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
        "primary",
        vec![run("first", "original")],
        calls.clone(),
    ));
    broker
        .list_readers_with_history(
            "runtime".into(),
            initial.clone(),
            None,
            &RunList::default(),
            query(None),
            true,
        )
        .await
        .unwrap();
    // Newer records move an empty incoming page token; ordinary native LRU
    // eviction must not destroy the foreground chain's first-page ownership.
    let mut rows = (0..MAX_BINDINGS + 100)
        .map(|index| run(&format!("new-{index}"), &format!("new-session-{index}")))
        .collect::<Vec<_>>();
    rows.push(run("first", "original"));
    let current: Arc<dyn AgentRunClient> = Arc::new(Reader::owns("primary", rows, calls.clone()));
    let mut token = None;
    loop {
        let page = broker
            .list_readers(
                "runtime".into(),
                current.clone(),
                None,
                &RunList::default(),
                query(token),
            )
            .await
            .unwrap();
        token = page.next_page_token;
        if token.is_none() {
            break;
        }
    }
    // Force normal metadata past the original row after the background scan.
    for index in 0..MAX_BINDINGS {
        broker
            .remember_primary(&run(
                &format!("other-{index}"),
                &format!("other-session-{index}"),
            ))
            .unwrap();
    }
    assert!(broker.sources.lock().unwrap().get("first").is_none());
    assert!(
        broker
            .desktop_sessions
            .lock()
            .unwrap()
            .get("original")
            .is_none()
    );
    calls.lock().unwrap().clear();
    let first = broker
        .get_readers(current.clone(), None, &RunList::default(), get("first"))
        .await
        .unwrap();
    assert!(first.source.can_control());
    assert!(first.continuable);
    assert_eq!(*calls.lock().unwrap(), ["primary:get:first"]);
    assert!(broker.head_bindings.lock().unwrap().len() <= 3);
    broker
        .list_readers_with_history(
            "runtime".into(),
            current,
            None,
            &RunList::default(),
            query(None),
            true,
        )
        .await
        .unwrap();
    assert!(!broker.head_bindings.lock().unwrap().contains_key("first"));
}

#[tokio::test]
async fn receipts_survive_same_instance_reconnection_and_reject_another_instance() {
    let original = ConversationRuns::default();
    original.use_instance_scope(Some("authenticated-runtime".into()));
    let calls = Arc::new(Mutex::new(vec![]));
    let reader: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
        "primary",
        vec![run("owned", "session")],
        calls.clone(),
    ));
    let page = original
        .list_readers(
            "authenticated-runtime".into(),
            reader.clone(),
            None,
            &RunList::default(),
            query(None),
        )
        .await
        .unwrap();
    let receipt = page.proofs["owned"].clone();
    let reconnected = ConversationRuns::default();
    reconnected.use_instance_scope(Some("authenticated-runtime".into()));
    let recovered = reconnected
        .get_readers_with_proofs(
            reader.clone(),
            None,
            &RunList::default(),
            get("owned"),
            ConversationProofs {
                discovery: Some(receipt.clone()),
                continuation: None,
            },
        )
        .await
        .unwrap();
    assert!(recovered.source.can_control());
    let another = ConversationRuns::default();
    another.use_instance_scope(Some("another-runtime".into()));
    calls.lock().unwrap().clear();
    assert!(
        another
            .get_readers_with_proofs(
                reader,
                None,
                &RunList::default(),
                get("owned"),
                ConversationProofs {
                    discovery: Some(receipt),
                    continuation: None
                }
            )
            .await
            .is_err()
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn failed_foreground_refresh_preserves_both_owners_pinned_first_pages() {
    let broker = ConversationRuns::default();
    let calls = Arc::new(Mutex::new(vec![]));
    let primary: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
        "primary",
        (0..3)
            .map(|index| run(&format!("local-{index}"), &format!("local-session-{index}")))
            .collect(),
        calls.clone(),
    ));
    let cloud: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
        "connector",
        (0..3)
            .map(|index| {
                run(
                    &format!("remote-{index}"),
                    &format!("remote-session-{index}"),
                )
            })
            .collect(),
        calls.clone(),
    ));
    broker
        .list_readers_with_history(
            "runtime".into(),
            primary.clone(),
            Some(cloud.clone()),
            &RunList::default(),
            query(None),
            true,
        )
        .await
        .unwrap();
    assert_eq!(broker.head_bindings.lock().unwrap().len(), 6);
    assert!(!broker.can_continue("remote-session-0"));
    let replacement: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
        "primary",
        vec![run("replacement", "replacement-session")],
        calls.clone(),
    ));
    let failing: Arc<dyn AgentRunClient> = Arc::new(Reader {
        name: "connector",
        list_error: Some(ApiError::permission_denied(
            ApiErrorReason::ScopeDenied,
            "unavailable",
        )),
        calls: calls.clone(),
        ..Reader::default()
    });
    assert!(
        broker
            .list_readers_with_history(
                "runtime".into(),
                replacement,
                Some(failing),
                &RunList::default(),
                query(None),
                true
            )
            .await
            .is_err()
    );
    broker.sources.lock().unwrap().clear();
    broker.desktop_sessions.lock().unwrap().clear();
    calls.lock().unwrap().clear();
    let local = broker
        .get_readers(
            primary.clone(),
            Some(cloud.clone()),
            &RunList::default(),
            get("local-0"),
        )
        .await
        .unwrap();
    assert!(local.source.can_control());
    assert!(local.continuable);
    let remote = broker
        .get_readers(primary, Some(cloud), &RunList::default(), get("remote-0"))
        .await
        .unwrap();
    assert_eq!(remote.source, ConversationSource::Cloud);
    assert!(!remote.continuable);
    assert_eq!(broker.head_bindings.lock().unwrap().len(), 6);
    assert!(
        !broker
            .head_bindings
            .lock()
            .unwrap()
            .contains_key("replacement")
    );
}

#[tokio::test]
async fn an_archived_page_receipt_does_not_revoke_a_still_authorized_run_read() {
    let broker = ConversationRuns::default();
    let calls = Arc::new(Mutex::new(vec![]));
    let initial = run("owned", "session");
    let page = DiscoveryPage {
        scope: broker.proof_scope.lock().unwrap().clone(),
        connector: false,
        request: query(None),
    };
    let proof = DiscoveryProof::encode(&page, &initial).unwrap();
    let mut archived = initial;
    archived.archived = true;
    let reader: Arc<dyn AgentRunClient> = Arc::new(Reader::owns("primary", vec![archived], calls));
    let read = broker
        .get_readers_with_proofs(
            reader,
            None,
            &RunList::default(),
            get("owned"),
            ConversationProofs {
                discovery: Some(proof),
                continuation: None,
            },
        )
        .await
        .unwrap();
    assert!(read.response.run.archived);
    assert!(read.source.can_control());
    assert!(read.discovery_proof.is_none());
}

#[tokio::test]
async fn current_control_binding_uses_native_authority_without_a_preflight_read() {
    use crate::state::{AppState, TargetConsentContext};
    let state = AppState::default();
    let calls = Arc::new(Mutex::new(vec![]));
    let owned = run("owned", "session");
    state
        .replace_target(
            "selected",
            Colossus::from_backend(Reader::owns("selected", vec![owned.clone()], calls.clone())),
            TargetConsentContext::ManagedLocal,
        )
        .await;
    state.select_target(Some("selected".into())).await;
    let lease = state.selected_target("selected").await.unwrap();
    lease.target.remember_primary_conversation(&owned).unwrap();
    state.bind_runs(&lease, vec![owned.run_id.clone()]).await;
    let source = crate::commands::require_run_binding_with_proof(
        &state,
        &lease,
        &owned.run_id,
        Some("untrusted but unused locator".into()),
    )
    .await
    .unwrap();
    assert!(source.can_control());
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn pinned_source_identity_cannot_be_shadowed_after_ordinary_metadata_eviction() {
    for connector in [false, true] {
        let broker = ConversationRuns::default();
        let calls = Arc::new(Mutex::new(vec![]));
        let owned = run("pinned", "session");
        let primary: Arc<dyn AgentRunClient> = Arc::new(Reader::owns(
            "primary",
            if connector {
                vec![]
            } else {
                vec![owned.clone()]
            },
            calls.clone(),
        ));
        let cloud = connector.then(|| {
            Arc::new(Reader::owns("connector", vec![owned.clone()], calls))
                as Arc<dyn AgentRunClient>
        });
        broker
            .list_readers_with_history(
                "runtime".into(),
                primary,
                cloud,
                &RunList::default(),
                query(None),
                true,
            )
            .await
            .unwrap();
        broker.sources.lock().unwrap().clear();
        assert!(
            broker
                .remember(std::slice::from_ref(&owned), !connector)
                .is_err()
        );
        assert_eq!(
            broker.source("pinned"),
            Some(if connector {
                ConversationSource::Cloud
            } else {
                ConversationSource::Primary
            })
        );
        let changed_session = run("pinned", "changed-session");
        assert!(broker.remember(&[changed_session], connector).is_err());
    }
}
