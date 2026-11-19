//! Live pinned-TLS and direct application API parity over canonical messages.

use super::*;
use colossus_api::{
    AgentRunApi, ApiScope, ApplicationKind, ApplicationPrincipal, CallerContext,
    EventSourcedArtifactApi, EventSourcedRunRepository, NewRun, RunRepository, scopes,
};
use colossus_api_runtime::{PublicInteractionRouter, RunAdmissionConfig, RuntimeAgentRunApi};
use colossus_grpc::{
    ApplicationGrant, BoundPublicGrpcServer, CredentialAuthenticator, FixedReadiness,
    InMemoryCredentialRepository, SystemMetadata, SystemServiceAdapter, TlsIdentity, TlsKeySeed,
};
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeOpenOptions, StorageAdapter};
use serde_json::json;

struct Credential(Zeroizing<Vec<u8>>);

#[async_trait]
impl CredentialProvider for Credential {
    async fn load(&self) -> SdkResult<crate::Secret> {
        crate::Secret::new(self.0.as_slice().to_vec())
    }
}

fn query(token: Option<String>) -> crate::ReadSessionTranscriptRequest {
    crate::ReadSessionTranscriptRequest {
        source_run_id: "transcript-source".into(),
        page_size: 1,
        page_token: token,
    }
}

#[tokio::test]
async fn live_tls_transcript_matches_direct_pages_with_read_only_authority() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = RuntimeConfig::offline_template(directory.path().join("state.redb"));
    config.storage.adapter = StorageAdapter::Ephemeral;
    let runtime = Arc::new(
        Runtime::open_with_options(
            &config,
            Arc::new(colossus_policy::DenyApproval),
            None,
            RuntimeOpenOptions::for_workspace(directory.path()).unwrap(),
        )
        .unwrap(),
    );
    let owner = CallerContext::authenticated(
        ApplicationPrincipal::authenticated(
            "app:transcript-sdk",
            "fixture-owner",
            ApplicationKind::Enrolled,
            [
                scopes::RUNS_READ,
                scopes::RUNS_EXECUTE,
                scopes::RUNS_CONTROL,
            ]
            .into_iter()
            .map(|scope| ApiScope::new(scope).unwrap()),
            ["primary".into()],
            Vec::<String>::new(),
        )
        .unwrap(),
        RequestId::new("transcript-seed").unwrap(),
    );
    let create: colossus_api::CreateRunRequest = serde_json::from_value(json!({
        "input":[{"type":"text","text":"opening question"}], "role":"primary",
        "max_turns":1,"idempotency_key":"transcript-seed"
    }))
    .unwrap();
    let repository = EventSourcedRunRepository::new(runtime.journal());
    repository
        .create_run(
            &owner,
            &create,
            &NewRun::from_request(
                "transcript-source",
                "transcript-session",
                "primary",
                &create,
            )
            .unwrap(),
        )
        .unwrap();
    runtime
        .create_application_session(
            "transcript-session",
            Some("Full retained transcript"),
            owner.actor(),
        )
        .unwrap();
    let large = "```rust\nlet quoted = \"🦀\\é\";\n```\n".repeat(5000);
    for (role, text, run) in [
        ("user", "opening question", "transcript-source"),
        ("assistant", large.as_str(), "transcript-source"),
        ("user", "second run question", "transcript-second"),
        ("assistant", "second run answer", "transcript-second"),
    ] {
        runtime
            .append_application_message(
                "transcript-session",
                run,
                serde_json::from_value(json!({"role":role,"content":text,
                "tool_call_id":null,"tool_calls":[]}))
                .unwrap(),
                owner.actor(),
            )
            .unwrap();
    }
    let api = Arc::new(RuntimeAgentRunApi::with_admission(
        runtime.clone(),
        Arc::new(PublicInteractionRouter::new(
            Arc::new(colossus_policy::DenyApproval),
            None,
        )),
        "primary",
        "No provider request is needed for export.",
        RunAdmissionConfig::default()
            .with_list_limits(4, 1, 1000, 1000, 1000, 1000)
            .unwrap(),
    ));
    let authenticator = Arc::new(CredentialAuthenticator::new(
        [0x63; 32],
        Arc::new(InMemoryCredentialRepository::default()),
    ));
    let grant = ApplicationGrant::new(
        "app:transcript-sdk",
        ApplicationKind::Enrolled,
        [ApiScope::new(scopes::RUNS_READ).unwrap()],
        ["primary".into()],
        Vec::<String>::new(),
    )
    .unwrap();
    let issued = authenticator.issue_pending(&grant).unwrap();
    authenticator.activate(issued.credential_id()).unwrap();
    let direct_caller = CallerContext::authenticated(
        authenticator
            .authenticate_authorization(&format!("Bearer {}", issued.expose_token()))
            .unwrap(),
        RequestId::new("transcript-direct").unwrap(),
    );
    assert!(!direct_caller.principal().has_scope(scopes::RUNS_EXECUTE));
    assert!(!direct_caller.principal().has_scope(scopes::RUNS_CONTROL));
    let tls = TlsIdentity::from_seed(TlsKeySeed::new([0x64; 32])).unwrap();
    let certificate = tls.certificate_pem().to_vec();
    let fingerprint = TlsFingerprint::from_hex(tls.certificate_sha256()).unwrap();
    let instance = Uuid::now_v7();
    let server = BoundPublicGrpcServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        tls,
        authenticator,
        SystemServiceAdapter::new(
            SystemMetadata {
                instance_id: instance.to_string(),
                server_version: "0.9.0".into(),
                deployment_mode: proto::DeploymentMode::SharedDaemon,
            },
            Arc::new(FixedReadiness::ready()),
        ),
        api.clone(),
        Arc::new(EventSourcedArtifactApi::new(runtime.journal())),
    )
    .await
    .unwrap();
    let address = server.local_addr();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server_task = tokio::spawn(server.serve(async {
        let _ = shutdown_rx.await;
    }));
    let backend = GrpcBackend::connect(
        GrpcConnectOptions::new(
            BackendKind::Daemon,
            crate::InstanceId::from_uuid(instance),
            crate::ApiMajor::new(1).unwrap(),
            Url::parse(&format!("https://{address}/")).unwrap(),
            fingerprint,
            certificate,
            Arc::new(Credential(Zeroizing::new(
                issued.expose_token().as_bytes().to_vec(),
            ))),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let client = crate::Colossus::from_backend(backend);
    assert!(
        client
            .capabilities()
            .contains(crate::SESSION_TRANSCRIPT_CAPABILITY)
    );
    assert!(!client.capabilities().contains("agent_runs.create"));
    assert!(!client.capabilities().contains("agent_runs.cancel"));
    let first = client.read_session_transcript(query(None)).await.unwrap();
    assert_eq!(first.messages.len(), 1);
    let direct_first = api
        .read_session_transcript(&direct_caller, query(None))
        .await
        .unwrap();
    assert_eq!(direct_first.messages, first.messages);
    assert_eq!(
        direct_first.snapshot.stream_version,
        first.snapshot.stream_version
    );
    let snapshot = first.snapshot.clone();
    let mut token = first.next_cursor;
    let mut exported = first.messages;
    let mut pages = 1;
    while let Some(cursor) = token {
        let request = query(Some(cursor));
        let direct = api
            .read_session_transcript(&direct_caller, request.clone())
            .await
            .unwrap();
        #[cfg(feature = "embedded")]
        {
            let embedded =
                crate::ContextBoundAgentRunClient::new(api.clone(), direct_caller.clone());
            assert_eq!(
                embedded
                    .read_session_transcript(request.clone())
                    .await
                    .unwrap(),
                direct
            );
        }
        let transported = client.read_session_transcript(request).await.unwrap();
        assert_eq!(
            transported, direct,
            "same caller-bound cursor has identical direct and TLS contents"
        );
        assert_eq!(transported.snapshot, snapshot);
        token = transported.next_cursor;
        exported.extend(transported.messages);
        pages += 1;
        assert!(pages <= 8, "cursor must terminate");
    }
    assert!(pages >= 4);
    assert_eq!(
        exported
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(
        exported[1].parts,
        vec![crate::TranscriptPart::Text { text: large }]
    );
    assert_eq!(exported[3].run_id, "transcript-second");
    client.close().await.unwrap();
    drop(client);
    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), server_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
