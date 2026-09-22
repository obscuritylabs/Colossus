//! Exercise the same authenticated worker channel used by Desktop MCP diagnostics.

use super::*;
use colossus_sdk::{ManagedMcpServerConfig, Secret};
use colossus_worker_protocol::{WorkerControlClient, worker_ipc_endpoint};
use serde_json::json;
use zeroize::Zeroizing;

#[tokio::test]
#[ignore = "requires Windows Credential Manager and access to docs.mcp.cloudflare.com"]
async fn cloudflare_docs_discovery_through_windows_managed_sidecar() {
    let workspace = PrivateDirectory::in_user_profile();
    let instance = PrivateDirectory::in_user_profile();
    let instance_id = Uuid::now_v7();
    let _cleanup = KeyringCleanup { instance_id };
    let executable = std::fs::canonicalize(env!("CARGO_BIN_EXE_colossus-sidecar"))
        .expect("canonical sidecar executable");
    let grant = SidecarApplicationGrant::new(
        "app:windows-mcp-acceptance",
        [ApiScope::new(scopes::RUNS_READ).expect("read scope")],
        ["primary".into()],
        Vec::<String>::new(),
    )
    .expect("application grant");
    let mut runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::Development)
        .with_execution_boundary(ManagedExecutionBoundary::WorkspaceIsolated);
    let server: ManagedMcpServerConfig = serde_json::from_value(json!({
        "name": "cloudflare-docs",
        "transport": "streamable_http",
        "command": null,
        "args": [],
        "working_directory": null,
        "environment_credentials": {},
        "url": "https://docs.mcp.cloudflare.com/mcp",
        "headers": {},
        "credential_headers": {},
        "allow_stateless": true,
        "oauth": null,
        "allowed_tools": ["search_cloudflare_documentation"],
        "research_tools": [],
        "timeout_ms": 30000,
        "max_output_bytes": 1048576
    }))
    .expect("managed MCP configuration");
    runtime.mcp_servers.push(server);
    let bootstrap = SidecarBootstrapConfig::new(&workspace.path, runtime, grant)
        .expect("bootstrap")
        .with_colossus_home(instance.path.join("home"))
        .expect("isolated home")
        .with_expected_workspace_identity(workspace_identity(&workspace.path))
        .expect("workspace identity")
        .without_automatic_agent_instructions_for_diagnostics()
        .with_worker_ipc_authentication(Secret::new(vec![0x5a; 32]).expect("worker key"))
        .expect("worker authentication");
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    let options = SidecarOptions::new(
        InstanceId::from_uuid(instance_id),
        AppPrivateInstanceDir::new(&instance.path).expect("instance directory"),
        VerifiedExecutable::new(
            &executable,
            Sha256Digest::from_bytes(executable_digest(&executable)),
        )
        .expect("verified executable"),
        ApiMajor::new(1).expect("API major"),
    )
    .expect("sidecar options");
    let client = Colossus::start_sidecar(&lifecycle, options)
        .await
        .expect("start Windows sidecar with remote MCP");

    // Desktop receives ordinary DOS paths, whereas the sidecar binds a canonical
    // Windows verbatim path. Hash exactly the path used by the running worker.
    let state = instance
        .path
        .join("state.redb")
        .canonicalize()
        .expect("state");
    let worker = WorkerControlClient::new(
        worker_ipc_endpoint(&state).expect("worker endpoint"),
        Zeroizing::new([0x5a; 32]),
    )
    .expect("authenticated worker");
    let discovered = worker.mcp_tools(Some("cloudflare-docs")).await;
    client.close().await.expect("graceful close");
    let tools = discovered.expect("Cloudflare MCP discovery");
    assert!(tools.as_array().expect("tool array").iter().any(|tool| {
        tool["server"] == "cloudflare-docs" && tool["name"] == "search_cloudflare_documentation"
    }));
}
