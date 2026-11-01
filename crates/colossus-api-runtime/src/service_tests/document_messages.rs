use super::super::standalone_goal::{serve, text_response};
use super::*;
use colossus_api::{
    ApiError, ApiResult, ArtifactDownload, ArtifactReference, ArtifactUploadReservation,
};
use std::sync::atomic::AtomicUsize;
use tokio::net::TcpListener;

fn provider_fixture(origin: &str) -> RuntimeFixture {
    runtime_fixture_with_config(|config| {
        config.access = serde_json::from_value(serde_json::json!({
            "profile":"pinned", "tools":{"include":[]}, "actions":{"allow":["provider.openai.chat"]}
        }))
        .unwrap();
        config.providers = serde_json::from_value(serde_json::json!({"profiles":{"documents":{
            "kind":"open_ai_compatible", "baseUrl":format!("{origin}/v1"),"timeoutMs":5000
        }}}))
        .unwrap();
        config.models = serde_json::from_value(serde_json::json!({
            "profiles":{"documents":{"providerProfile":"documents","model":"fixture-documents",
                "contextWindowTokens":32768,"maxOutputTokens":4096,
                "capabilities":{"toolCalls":"off","streaming":"on","documentInputs":"on"}}},
            "roles":{"primary":"documents"}
        }))
        .unwrap();
        config.sandbox.network_destinations = vec![origin.into()];
    })
}

async fn updates(
    api: &RuntimeAgentRunApi,
    owner: &CallerContext,
    run_id: &str,
) -> Vec<colossus_api::RunUpdate> {
    api.watch_run(
        owner,
        WatchRunRequest {
            run_id: run_id.into(),
            after_sequence: 0,
        },
    )
    .await
    .unwrap()
    .collect::<Vec<_>>()
    .await
    .into_iter()
    .map(Result::unwrap)
    .collect()
}

pub(super) async fn actual_file_only_and_mixed_create_watch_reload_keep_ordered_durable_artifacts()
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let provider = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(20),
            serve(
                listener,
                vec![
                    text_response("Read the document."),
                    text_response("Read the follow-up."),
                ],
            ),
        )
        .await
        .unwrap()
    });
    let fixture = provider_fixture(&origin);
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner = caller_with_exact_scopes(
        "app:document-e2e",
        "document-create",
        &[
            scopes::RUNS_EXECUTE,
            scopes::ARTIFACTS_READ,
            scopes::ARTIFACTS_WRITE,
        ],
    );
    let reader =
        caller_with_exact_scopes("app:document-e2e", "document-reader", &[scopes::RUNS_READ]);
    let artifacts = EventSourcedArtifactApi::new(fixture.runtime.journal());
    let bytes = b"%PDF-1.7\nPRIVATE_E2E_DOCUMENT\n%%EOF\n".to_vec();
    let document = upload(
        &artifacts,
        &owner,
        "durable.pdf",
        "application/pdf",
        bytes.clone(),
    )
    .await;
    let mut session_id = None;
    for (index, input) in [
        vec![document.clone()],
        vec![
            ContentPart::Text {
                text: "before".into(),
            },
            document.clone(),
            ContentPart::Text {
                text: "after".into(),
            },
        ],
    ]
    .into_iter()
    .enumerate()
    {
        let mut request = request(&format!("document-e2e-{index}"), "");
        request.input = input;
        request.session_id = session_id.clone();
        let created = api.create_run(&owner, request.clone()).await.unwrap();
        let terminal = wait_terminal(&api, &reader, &created.run.id).await;
        assert_eq!(terminal.status, RunStatus::Completed, "{terminal:?}");
        session_id = Some(terminal.session_id.clone());
        let first = updates(&api, &reader, &created.run.id).await;
        let replay = api.create_run(&owner, request.clone()).await.unwrap();
        assert_eq!(replay.run.id, created.run.id);
        let reloaded = service(fixture.runtime.clone(), RunAdmissionConfig::default());
        assert_eq!(updates(&reloaded, &reader, &created.run.id).await, first);
        let messages = first
            .iter()
            .filter_map(|update| {
                if let RunUpdateKind::Message { message } = &update.kind {
                    Some((update.sequence, message))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].1.content.len(), request.input.len());
        let canonical = fixture
            .runtime
            .session_messages(&terminal.session_id)
            .unwrap()
            .into_iter()
            .find(|entry| {
                entry.run_id == created.run.id && entry.message.role == ModelMessageRole::User
            })
            .unwrap();
        assert_eq!(messages[0].1.sequence, canonical.sequence);
        assert_eq!(messages[0].1.created_at, canonical.created_at);
        assert!(
            first
                .iter()
                .filter(|update| matches!(update.kind, RunUpdateKind::OutputDelta { .. }))
                .all(|update| update.sequence > messages[0].0)
        );
        let released = messages[0]
            .1
            .content
            .iter()
            .find_map(|part| {
                if let colossus_api::ReleasedContentPart::Artifact { artifact } = part {
                    Some(artifact)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(released.file_name, "durable.pdf");
        assert_eq!(released.size_bytes, bytes.len() as u64);
        if index == 1 {
            assert!(
                matches!(&messages[0].1.content[0],colossus_api::ReleasedContentPart::Text {text} if text=="before")
            );
            assert!(
                matches!(&messages[0].1.content[2],colossus_api::ReleasedContentPart::Text {text} if text=="after")
            );
        }
        assert!(
            !serde_json::to_string(&first)
                .unwrap()
                .contains("PRIVATE_E2E_DOCUMENT")
        );
    }
    let requests = provider.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        serde_json::to_string(&requests)
            .unwrap()
            .contains("data:application/pdf;base64,")
    );
}

type StartedEffects = Vec<(String, String, Option<String>)>;

fn started_effects(journal: &dyn EventJournal) -> StartedEffects {
    journal
        .read_global(1, 512)
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "effect.started.v1")
        .map(|event| {
            let first = journal.read_stream_from(&event.stream_id, 0, 1).unwrap();
            let metadata = journal.decrypt_payload(&first[0]).unwrap();
            (
                event.stream_id,
                metadata["action"]
                    .as_str()
                    .expect("audited action")
                    .to_owned(),
                event.context.run_id,
            )
        })
        .collect()
}

struct MetadataFault {
    inner: EventSourcedArtifactApi,
    journal: Arc<dyn EventJournal>,
    at_failure: std::sync::Mutex<Option<StartedEffects>>,
    at_repair: std::sync::Mutex<Option<StartedEffects>>,
    calls: AtomicUsize,
    panic_once: bool,
}

#[async_trait::async_trait]
impl colossus_api::ArtifactApi for MetadataFault {
    async fn create_upload(
        &self,
        caller: &CallerContext,
        request: CreateArtifactUploadRequest,
    ) -> ApiResult<ArtifactUploadReservation> {
        self.inner.create_upload(caller, request).await
    }
    async fn upload(
        &self,
        caller: &CallerContext,
        id: &str,
        chunks: Vec<ArtifactChunk>,
    ) -> ApiResult<ArtifactReference> {
        self.inner.upload(caller, id, chunks).await
    }
    async fn get(&self, caller: &CallerContext, id: &str) -> ApiResult<ArtifactReference> {
        let count = self.calls.fetch_add(1, Ordering::AcqRel) + 1;
        if count == 3 {
            *self.at_failure.lock().unwrap() = Some(started_effects(self.journal.as_ref()));
        }
        if count == 4 {
            *self.at_repair.lock().unwrap() = Some(started_effects(self.journal.as_ref()));
        }
        if count == 3 && self.panic_once {
            panic!("injected first post-canonical metadata read panic");
        }
        if count >= 3 && !self.panic_once {
            return Err(ApiError::failed_precondition(
                colossus_api::ApiErrorReason::ArtifactUnavailable,
                "fixture metadata unavailable",
            ));
        }
        self.inner.get(caller, id).await
    }
    async fn download(
        &self,
        caller: &CallerContext,
        id: &str,
        offset: u64,
    ) -> ApiResult<ArtifactDownload> {
        self.inner.download(caller, id, offset).await
    }
}

pub(super) async fn outer_execution_fallback_repairs_panics_but_preserves_unavailable_input_gaps() {
    for panic_once in [true, false] {
        let fixture = message_fixture();
        let router = Arc::new(PublicInteractionRouter::new(Arc::new(DenyApproval), None));
        let fault = Arc::new(MetadataFault {
            inner: EventSourcedArtifactApi::new(fixture.runtime.journal()),
            journal: fixture.runtime.journal(),
            at_failure: std::sync::Mutex::new(None),
            at_repair: std::sync::Mutex::new(None),
            calls: AtomicUsize::new(0),
            panic_once,
        });
        let api = RuntimeAgentRunApi::new(fixture.runtime.clone(), router, "primary", "fixture")
            .with_artifacts_for_test(fault.clone());
        let owner = caller_with_additional_scopes(
            "app:metadata-fault",
            "metadata-fault",
            &[scopes::ARTIFACTS_READ, scopes::ARTIFACTS_WRITE],
        );
        let artifacts = EventSourcedArtifactApi::new(fixture.runtime.journal());
        let mut request = request("metadata-fault", "");
        request.input = vec![
            upload(
                &artifacts,
                &owner,
                "fault.pdf",
                "application/pdf",
                b"%PDF-1.7\nfixture\n%%EOF\n".to_vec(),
            )
            .await,
        ];
        let created = api.create_run(&owner, request).await.unwrap();
        tokio::time::timeout(Duration::from_secs(20), async {
            while api.has_pending_executions() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let repository = EventSourcedRunRepository::new(fixture.runtime.journal());
        let current = repository
            .recoverable_run(&owner, &created.run.id)
            .unwrap()
            .unwrap()
            .0;
        let released = repository
            .updates_after(&owner, &created.run.id, 0, 32)
            .unwrap();
        if panic_once {
            assert_eq!(current.status, RunStatus::OutcomeUnknown);
            let message = released
                .iter()
                .find(|update| matches!(update.kind, RunUpdateKind::Message { .. }))
                .unwrap();
            let terminal = released
                .iter()
                .find(|update| matches!(update.kind, RunUpdateKind::Failure { .. }))
                .unwrap();
            assert!(message.sequence < terminal.sequence);
        } else {
            assert_eq!(current.status, RunStatus::Running);
            assert!(!released.iter().any(|update| matches!(
                update.kind,
                RunUpdateKind::Message { .. } | RunUpdateKind::Failure { .. }
            )));
            assert!(
                api.get_run(
                    &owner,
                    GetRunRequest {
                        run_id: created.run.id.clone()
                    }
                )
                .await
                .is_err()
            );
        }
        if panic_once {
            assert_eq!(
                api.get_run(
                    &owner,
                    GetRunRequest {
                        run_id: created.run.id.clone()
                    }
                )
                .await
                .unwrap()
                .status,
                RunStatus::OutcomeUnknown
            );
        }
        // A callback error makes Runtime settle its original scheduled attempt
        // before returning. The first API repair (get4) is the no-replay boundary;
        // the earlier callback failure (get3) need not precede that initial start.
        let at_failure = fault.at_failure.lock().unwrap().clone().unwrap();
        let at_repair = fault.at_repair.lock().unwrap().clone().unwrap();
        assert!(at_failure.iter().all(|effect| at_repair.contains(effect)));
        assert!(
            at_repair.len() <= 1
                && at_repair
                    .iter()
                    .all(|(_, action, run_id)| action == "provider.openai.responses"
                        && run_id.as_deref() == Some(created.run.id.as_str())),
            "initial settlement must contain at most the original generation: {at_repair:?}"
        );
        assert_eq!(
            started_effects(fixture.runtime.journal().as_ref()),
            at_repair,
            "fallback/recovery started another effect (ID, action, run metadata); panic_once={panic_once}"
        );
    }
}

pub(super) async fn actual_text_create_and_idempotent_replay_keep_execute_only_creator_authority() {
    let fixture = runtime_fixture();
    let api = service(fixture.runtime.clone(), RunAdmissionConfig::default());
    let owner = caller_with_exact_scopes(
        "app:execute-only-input",
        "execute-only-create",
        &[scopes::RUNS_EXECUTE],
    );
    let reader = caller_with_exact_scopes(
        "app:execute-only-input",
        "execute-only-reader",
        &[scopes::RUNS_READ],
    );
    let request = request("execute-only-input", "hello");
    let created = api.create_run(&owner, request.clone()).await.unwrap();
    assert_eq!(
        wait_terminal(&api, &reader, &created.run.id).await.status,
        RunStatus::Completed
    );
    assert_eq!(
        api.create_run(&owner, request).await.unwrap().run.id,
        created.run.id
    );
    let released = updates(&api, &reader, &created.run.id).await;
    assert_eq!(released.iter().filter(|update| matches!(&update.kind,RunUpdateKind::Message {message} if message.content==vec![colossus_api::ReleasedContentPart::Text {text:"hello".into()}])).count(),1);
}
