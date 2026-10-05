use super::*;
use colossus_api::{ApiScope, ApplicationKind, scopes};
use colossus_api_runtime::RuntimeWorkflowApi;
use colossus_grpc::{
    ApplicationGrant, AuthenticationInterceptor, CredentialAuthenticator,
    InMemoryCredentialRepository,
};
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeOpenOptions, StorageAdapter};

struct Credential(String);
#[async_trait]
impl crate::CredentialProvider for Credential {
    async fn load(&self) -> crate::SdkResult<crate::Secret> {
        crate::Secret::new(self.0.as_bytes().to_vec())
    }
}

#[test]
fn transport_and_protocol_failures_after_mutation_are_unconfirmed_and_not_retryable() {
    for status in [
        Status::unavailable("private transport"),
        Status::deadline_exceeded("private target"),
        Status::internal("private response"),
    ] {
        let error = mutation_status(status);
        assert_eq!(error.code, ApiErrorCode::OutcomeUnknown);
        assert!(!error.retryable);
        assert_eq!(error.outcome, colossus_api::OutcomeCertainty::Unknown);
        assert!(!error.message.contains("private"));
    }
}

#[tokio::test]
async fn authenticated_sdk_roundtrip_uses_canonical_resources_and_shared_watch_headroom() {
    const YAML: &str = "apiVersion: colossus.dev/v1alpha1\nkind: Workflow\nmetadata: {name: transport-health, version: 1.0.0, description: Transport fixture}\ninputs: {type: object, additionalProperties: false}\noutputs: {type: object}\ncapabilities: []\nmaxConcurrency: 1\nstepBudget: 2\nsteps:\n  - id: result\n    type: emit\n    value: {ok: true}\n";
    let directory = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::offline_template(directory.path().join("state.redb"));
    config.storage.adapter = StorageAdapter::Ephemeral;
    config
        .models
        .profiles
        .insert("task-model".into(), config.models.profiles["echo"].clone());
    let runtime = Arc::new(
        Runtime::open_with_options(
            &config,
            Arc::new(colossus_policy::DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(directory.path()).unwrap(),
        )
        .unwrap(),
    );
    let authenticator = Arc::new(CredentialAuthenticator::new(
        [71; 32],
        Arc::new(InMemoryCredentialRepository::default()),
    ));
    let issue = |id: &str, granted: &[&str]| {
        let grant = ApplicationGrant::new(
            id,
            ApplicationKind::Enrolled,
            granted.iter().map(|value| ApiScope::new(*value).unwrap()),
            ["primary".into()],
            Vec::<String>::new(),
        )
        .unwrap();
        let issued = authenticator.issue_pending(&grant).unwrap();
        authenticator.activate(issued.credential_id()).unwrap();
        issued.expose_token().to_owned()
    };
    let scopes = &[
        scopes::WORKFLOWS_READ,
        scopes::WORKFLOWS_REGISTER,
        scopes::SCHEDULES_READ,
        scopes::SCHEDULES_CREATE,
        scopes::SCHEDULES_CONTROL,
        scopes::WORKFLOW_RUNS_READ,
        scopes::WORKFLOW_RUNS_START,
    ];
    let token = issue("app:transport", scopes);
    let other_token = issue("app:other", scopes);
    let denied_token = issue("app:read-only", &[scopes::SCHEDULES_READ]);
    let watches = Arc::new(tokio::sync::Semaphore::new(1));
    let service = proto::automation_service_server::AutomationServiceServer::with_interceptor(
        colossus_grpc::AutomationServiceAdapter::new(
            Some(Arc::new(RuntimeWorkflowApi::new(runtime.clone()))),
            watches.clone(),
        ),
        AuthenticationInterceptor::new(authenticator),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let incoming = Box::pin(futures::stream::try_unfold(
        listener,
        |listener| async move {
            let (stream, _) = listener.accept().await?;
            Ok::<_, std::io::Error>(Some((stream, listener)))
        },
    ));
    let (shutdown, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = stopped.await;
            }),
    );
    let channel = Channel::from_shared(format!("http://{address}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let client = |token: String| GrpcWorkflowClient {
        transport: Arc::new(GrpcArtifactClient {
            channel: channel.clone(),
            credential_provider: Arc::new(Credential(token)),
            closed: watch::channel(false).0,
        }),
    };
    let owner = client(token);
    let other = client(other_token);
    let denied = client(denied_token);
    let definition = owner.validate_definition(YAML.into()).await.unwrap();
    assert_eq!(
        owner
            .register_definition(
                YAML.into(),
                definition.workflow_hash.clone(),
                "import".into()
            )
            .await
            .unwrap(),
        definition
    );
    let request = CreateWorkflowScheduleRequest {
        schedule_id: "health".into(),
        workflow_id: definition.workflow_id,
        expected_hash: definition.workflow_hash,
        inputs: serde_json::json!({}),
        cadence_seconds: 60,
        calendar: None,
        task: None,
        starts_at: "2026-10-03T12:00:00Z".into(),
        misfire_policy: crate::WorkflowScheduleMisfirePolicy::Skip,
        enabled: true,
        idempotency_key: "allocation".into(),
    };
    assert_eq!(
        denied
            .create_schedule(request.clone())
            .await
            .unwrap_err()
            .code,
        ApiErrorCode::PermissionDenied
    );
    let created = owner.create_schedule(request.clone()).await.unwrap();
    assert_eq!(created.origin.as_ref().unwrap().owner.id, "app:transport");
    assert!(
        other
            .list_schedules(None, 100)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        other.get_schedule("health".into()).await.unwrap_err().code,
        ApiErrorCode::NotFound
    );
    assert!(
        owner.list_schedules(None, 1).await.unwrap().items[0]
            .record
            .inputs
            .is_null()
    );
    runtime
        .workflows()
        .tick_schedules_at("2026-10-03T12:00:00Z")
        .unwrap();
    assert_eq!(owner.create_schedule(request).await.unwrap(), created);
    let current = owner.get_schedule("health".into()).await.unwrap();
    let run_id = current.record.last_run_id.unwrap();
    assert_eq!(
        owner.get_run(run_id.clone()).await.unwrap().status,
        crate::WorkflowStatus::Queued
    );
    let calendar = crate::WorkflowCalendar {
        timezone: "America/New_York".into(),
        time: "09:00".into(),
        weekdays: vec![1, 5],
    };
    let task_request = CreateWorkflowScheduleRequest {
        schedule_id: "task-briefing".into(),
        workflow_id: String::new(),
        expected_hash: String::new(),
        inputs: serde_json::json!({}),
        cadence_seconds: 0,
        calendar: Some(calendar.clone()),
        task: Some(crate::WorkflowTask {
            name: "Briefing".into(),
            instructions: "Summarize the task result".into(),
            tools: Vec::new(),
            options: crate::WorkflowAgentOptions {
                model_profile: Some("task-model".into()),
                reasoning_effort: None,
            },
        }),
        starts_at: "2050-10-07T13:00:00Z".into(),
        misfire_policy: crate::WorkflowScheduleMisfirePolicy::Skip,
        enabled: false,
        idempotency_key: "task-create".into(),
    };
    let task = owner.create_schedule(task_request.clone()).await.unwrap();
    assert_eq!(task.record.calendar, Some(calendar));
    assert_eq!(owner.create_schedule(task_request).await.unwrap(), task);
    assert_eq!(
        owner.list_workflows(None, 100).await.unwrap().items.len(),
        1
    );
    assert_eq!(
        owner
            .list_schedules(None, 100)
            .await
            .unwrap()
            .items
            .iter()
            .find(|item| item.record.schedule_id == "task-briefing")
            .unwrap()
            .record
            .task
            .as_ref()
            .unwrap()
            .instructions,
        ""
    );
    let history = owner
        .list_runs("transport-health:1.0.0".into(), None, 100)
        .await
        .unwrap();
    assert_eq!(history.items[0].run_id, run_id);
    assert!(
        other
            .list_runs("transport-health:1.0.0".into(), None, 100)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let deletion = DeleteWorkflowScheduleRequest {
        schedule_id: "health".into(),
        etag: current.etag.clone(),
    };
    assert_eq!(
        denied
            .delete_schedule(deletion.clone())
            .await
            .unwrap_err()
            .code,
        ApiErrorCode::PermissionDenied
    );
    assert_eq!(
        other
            .delete_schedule(deletion.clone())
            .await
            .unwrap_err()
            .code,
        ApiErrorCode::NotFound
    );
    let stale = DeleteWorkflowScheduleRequest {
        etag: created.etag.clone(),
        ..deletion.clone()
    };
    assert_eq!(
        owner.delete_schedule(stale).await.unwrap_err().code,
        ApiErrorCode::Conflict
    );
    assert_eq!(
        owner
            .delete_schedule(deletion.clone())
            .await
            .unwrap()
            .schedule_id,
        "health"
    );
    assert_eq!(
        owner.delete_schedule(deletion).await.unwrap().schedule_id,
        "health"
    );
    assert_eq!(
        owner.get_schedule("health".into()).await.unwrap_err().code,
        ApiErrorCode::NotFound
    );
    assert_eq!(
        owner.list_schedules(None, 100).await.unwrap().items.len(),
        1
    );
    assert_eq!(owner.get_run(run_id.clone()).await.unwrap().run_id, run_id);
    let mut stream = AutomationServiceClient::new(channel.clone())
        .watch_workflow_run(
            owner
                .transport
                .request(proto::WatchWorkflowRunRequest {
                    workflow_run_id: run_id,
                    after_sequence: 0,
                })
                .await
                .unwrap(),
        )
        .await
        .unwrap()
        .into_inner();
    assert!(stream.message().await.unwrap().is_some());
    assert_eq!(watches.available_permits(), 0);
    assert!(
        owner.get_schedule("task-briefing".into()).await.is_ok(),
        "unary headroom remains while watching"
    );
    drop(stream);
    tokio::time::timeout(Duration::from_secs(5), async {
        while watches.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let request = crate::StartWorkflowRunRequest {
        workflow_id: format!(
            "{}:{}",
            task.record.workflow_name, task.record.workflow_version
        ),
        expected_hash: task.record.workflow_hash,
        inputs: serde_json::json!({}),
        idempotency_key: "manual-task-proof".into(),
    };
    let run = owner.start_run(request.clone()).await.unwrap();
    assert_eq!(owner.start_run(request).await.unwrap().run_id, run.run_id);
    runtime.workflows().drain().await.unwrap();
    assert_eq!(
        owner.get_run(run.run_id.clone()).await.unwrap().status,
        crate::WorkflowStatus::Completed
    );
    let events = runtime.journal().read_global(1, 1024).unwrap();
    let selected = events
        .iter()
        .filter(|event| event.event_type == "model.request.prepared.v1")
        .map(|event| runtime.journal().decrypt_payload(event).unwrap())
        .any(|payload| payload["model_profile"] == "task-model");
    assert!(
        selected,
        "the task's configured model is used by the agent loop"
    );
    shutdown.send(()).unwrap();
    server.await.unwrap().unwrap();
}
