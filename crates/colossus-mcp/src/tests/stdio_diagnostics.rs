use super::*;
use colossus_contracts::{McpDiagnosticCode, McpDiagnosticStage};

struct ProcessFixture(Result<Value, &'static str>);

struct UnavailableCredentials;

impl colossus_ports::CredentialResolver for UnavailableCredentials {
    fn resolve(
        &self,
        _reference: &str,
    ) -> Result<String, colossus_ports::CredentialResolutionError> {
        Err(colossus_ports::CredentialResolutionError::Unavailable)
    }
}

#[async_trait]
impl EffectExecutor for ProcessFixture {
    async fn execute(
        &self,
        _request: &EffectRequest,
        _permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        match &self.0 {
            Ok(value) => Ok(QuarantinedEffectResult {
                media_type: "application/json".into(),
                bytes: serde_json::to_vec(value).unwrap(),
                effect_succeeded: true,
            }),
            Err(message) => Err(ExecutionError::Failed((*message).into())),
        }
    }
}

fn sandbox_result(stdout: &str) -> Value {
    json!({
        "backend": "native", "exit_code": 0, "success": true, "timed_out": false,
        "resource_limit_exceeded": null, "output_truncated": false,
        "stdout_base64": BASE64.encode(stdout), "stderr_base64": BASE64.encode("private-stderr")
    })
}

async fn probe_stdio(fixture: ProcessFixture, missing_credential: bool) -> McpDiagnosticCapture {
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    let workspace = std::env::current_dir().unwrap().canonicalize().unwrap();
    let mut server = remote_server("unused");
    server.transport = McpTransportKind::Stdio;
    server.command = executable.clone();
    server.url = None;
    if missing_credential {
        server
            .environment
            .insert("MCP_TOKEN".into(), "env:MCP_TOKEN".into());
    }
    let executor = McpExecutor::new(
        &McpConfig {
            servers: BTreeMap::from([("fixture".into(), server)]),
            ..Default::default()
        },
        &workspace,
        "native",
        Arc::new(fixture),
    )
    .unwrap()
    .with_credentials(Arc::new(UnavailableCredentials));
    let policy = BuiltInPolicy::offline_default()
        .with_action("mcp.tools", DecisionOutcome::Allow)
        .with_post_effect(false)
        .with_sandbox("native", "stdio-diagnostics", false)
        .with_environment("MCP_TOKEN")
        .with_filesystem_root(executable.display().to_string(), "execute")
        .with_filesystem_read_root(workspace.display().to_string());
    let gateway = EffectGateway::new(
        Arc::new(InMemoryEventJournal::default()),
        Arc::new(policy),
        Arc::new(AllowApproval {
            approved_by: "test".into(),
        }),
        SafetyKernel::new(["mcp.invoke".into()]),
        [42_u8; 32],
    );
    let request = executor
        .request(
            Actor {
                actor_type: ActorType::System,
                id: "test".into(),
            },
            ExecutionContext::default(),
            McpOperation::ListTools {
                server: "fixture".into(),
                cursor: None,
            },
        )
        .unwrap();
    let capture = McpDiagnosticCapture::default();
    let error = capture
        .scope(gateway.execute(request, &executor))
        .await
        .expect_err("failed discovery");
    assert!(
        matches!(error, colossus_policy::GatewayError::Execution(_)),
        "{error:?}"
    );
    capture
}

#[tokio::test]
async fn stdio_credentials_launch_and_timeout_failures_are_distinct() {
    let capture = probe_stdio(ProcessFixture(Err("must not execute")), true).await;
    assert_eq!(capture.snapshot(), (McpDiagnosticStage::Credentials, None));
    for (message, code) in [
        ("private launch failure", McpDiagnosticCode::Process),
        (
            "sandboxed process exceeded its timeout",
            McpDiagnosticCode::Timeout,
        ),
    ] {
        let capture = probe_stdio(ProcessFixture(Err(message)), false).await;
        let (stage, failure) = capture.snapshot();
        assert_eq!(stage, McpDiagnosticStage::Process);
        assert_eq!(failure.unwrap().code, code);
    }
}

#[tokio::test]
async fn stdio_protocol_and_bounded_output_failures_preserve_stage_without_output() {
    let initialized = json!({"jsonrpc": "2.0", "id": INITIALIZE_REQUEST_ID, "result": {
        "protocolVersion": "2025-11-25", "capabilities": {"tools": {}},
        "serverInfo": {"name": "fixture", "version": "1"}
    }})
    .to_string();
    for (stdout, expected_stage) in [
        (
            "private-malformed-init".to_owned(),
            McpDiagnosticStage::Initialize,
        ),
        (initialized, McpDiagnosticStage::ListTools),
    ] {
        let capture = probe_stdio(ProcessFixture(Ok(sandbox_result(&stdout))), false).await;
        assert_eq!(capture.snapshot(), (expected_stage, None));
    }
    for (field, value, expected_code) in [
        ("timed_out", json!(true), McpDiagnosticCode::Timeout),
        (
            "output_truncated",
            json!(true),
            McpDiagnosticCode::ResponseTooLarge,
        ),
        (
            "resource_limit_exceeded",
            json!("private-limit"),
            McpDiagnosticCode::Process,
        ),
    ] {
        let mut result = sandbox_result("private-stdout");
        result[field] = value;
        let capture = probe_stdio(ProcessFixture(Ok(result)), false).await;
        let (stage, failure) = capture.snapshot();
        assert_eq!(stage, McpDiagnosticStage::Process);
        assert_eq!(failure.as_ref().unwrap().code, expected_code);
        assert!(!serde_json::to_string(&failure).unwrap().contains("private"));
    }
}
