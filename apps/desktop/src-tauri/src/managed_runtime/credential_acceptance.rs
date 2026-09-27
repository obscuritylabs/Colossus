//! Opt-in native backend acceptance; this does not simulate or claim native UI input.
//! Run with an unlocked Windows/macOS credential store, loopback access, and an absolute
//! `COLOSSUS_ACCEPTANCE_SIDECAR` pointing to the matching built sidecar executable.

use super::*;
use colossus_contracts::HostSecret;
use colossus_home::ConfinedRoot;
use colossus_sdk::{Sha256Digest, VerifiedExecutable};
use futures_util::FutureExt as _;
use keyring_core::api::CredentialStoreApi as _;
use redb::{ReadableDatabase as _, TableDefinition};
use serde_json::{Value, json};
use std::{
    io::Read as _,
    path::PathBuf,
    process::{Command, Stdio},
    time::Instant,
};
use uuid::Uuid;

#[cfg(target_os = "macos")]
#[path = "credential_acceptance_cleanup.rs"]
mod cleanup;
#[path = "credential_acceptance_fixture.rs"]
mod fixture;

const CHILD: &str = "managed_runtime::credential_acceptance::native_backend_process_child";
const ROOT_PREFIX: &str = ".colossus-desktop-credential-acceptance-";
const CREDENTIAL_ID: &str = "synthetic-large-token";
const KEY_SERVICE: &str = "com.obscuritylabs.colossus.credentials.v1";

#[test]
#[ignore = "requires native credential store, loopback, and COLOSSUS_ACCEPTANCE_SIDECAR; run in Desktop native credential CI"]
fn native_vault_restarts_reach_managed_sidecar_mcp() {
    let executable = sidecar_executable();
    let private = PrivateAcceptanceRoot::new();
    // The sandbox may grant workspace traversal; it must not own the vault's parent.
    let workspace = PrivateAcceptanceRoot::new();
    let server = fixture::Server::start();
    run_child(
        &private.0,
        &workspace.0,
        &executable,
        &server.origin,
        "save",
    );
    run_child(
        &private.0,
        &workspace.0,
        &executable,
        &server.origin,
        "8192",
    );
    run_child(
        &private.0,
        &workspace.0,
        &executable,
        &server.origin,
        "65536",
    );
    server.assert_roundtrips();
    assert_no_plaintext_files(&private.0);
    eprintln!(
        "native backend acceptance: separate processes reopened 8192 and 65536 bytes; exact provider and MCP discovery/tool bearer verified"
    );
}

fn sidecar_executable() -> PathBuf {
    let path = PathBuf::from(std::env::var_os("COLOSSUS_ACCEPTANCE_SIDECAR").expect(
        "set COLOSSUS_ACCEPTANCE_SIDECAR to the matching built colossus-sidecar executable",
    ));
    assert!(path.is_absolute());
    path.canonicalize().expect("canonical sidecar executable")
}

fn run_child(root: &Path, workspace: &Path, executable: &Path, origin: &str, stage: &str) {
    let mut child = Command::new(std::env::current_exe().expect("Desktop test executable"))
        .args(["--exact", CHILD, "--ignored", "--nocapture"])
        .env("COLOSSUS_ACCEPTANCE_ROOT", root)
        .env("COLOSSUS_ACCEPTANCE_WORKSPACE", workspace)
        .env("COLOSSUS_ACCEPTANCE_SIDECAR", executable)
        .env("COLOSSUS_ACCEPTANCE_ORIGIN", origin)
        .env("COLOSSUS_ACCEPTANCE_STAGE", stage)
        .stdin(Stdio::null())
        .spawn()
        .expect("spawn independent Desktop backend process");
    let deadline = Instant::now() + Duration::from_secs(150);
    loop {
        if let Some(status) = child.try_wait().expect("wait for Desktop child") {
            assert!(status.success(), "native backend process failed at {stage}");
            break;
        }
        if Instant::now() >= deadline {
            child.kill().expect("stop timed out Desktop child");
            child.wait().expect("reap Desktop child");
            panic!("native backend process timed out at {stage}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[tokio::test]
#[ignore = "subprocess helper for native_vault_restarts_reach_managed_sidecar_mcp"]
#[allow(clippy::too_many_lines)]
async fn native_backend_process_child() {
    let Some(root) = std::env::var_os("COLOSSUS_ACCEPTANCE_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    validate_fixture_root(&root);
    let confined = ConfinedRoot::bind(root.clone()).expect("private acceptance root");
    let desktop = confined.prepare_directory(Path::new("desktop")).unwrap();
    let settings = SettingsStore::open(desktop).expect("production Desktop settings root");
    let state = AppState::default();
    let credentials = DesktopCredentials::for_settings(&state, &settings).unwrap();
    let phase = std::env::var("COLOSSUS_ACCEPTANCE_STAGE").unwrap();
    if phase == "save" {
        credentials
            .write(CREDENTIAL_ID, fixture::token(8192))
            .await
            .unwrap();
        return;
    }
    let size = phase.parse::<usize>().expect("bounded acceptance size");
    assert!([8192, 65536].contains(&size));
    let recovered = credentials.read(CREDENTIAL_ID).await.unwrap();
    assert!(
        recovered.expose() == fixture::token(size).expose(),
        "reopened token changed"
    );
    let origin = std::env::var("COLOSSUS_ACCEPTANCE_ORIGIN").unwrap();
    let resolved = resolved_configuration(&origin, size);
    let metadata = credentials
        .availability(vec![CREDENTIAL_ID.into()])
        .await
        .unwrap();
    fixture::assert_no_secret(&serde_json::to_string(&metadata).unwrap());
    let host_credentials = provider_host_credentials(&resolved, &credentials)
        .await
        .unwrap();
    assert_eq!(
        host_credentials.len(),
        1,
        "shared provider/MCP credential is deduplicated"
    );
    let workspace = PathBuf::from(std::env::var_os("COLOSSUS_ACCEPTANCE_WORKSPACE").unwrap());
    validate_fixture_root(&workspace);
    let workspace = crate::desktop_settings::validate_workspace(&workspace).unwrap();
    let instance = confined.prepare_directory(Path::new("instance")).unwrap();
    let home = confined
        .prepare_directory(Path::new("runtime-home"))
        .unwrap();
    let worker_key = [0x63; 32];
    let bootstrap = managed_bootstrap(
        &workspace.path,
        workspace.identity.unwrap(),
        &resolved,
        host_credentials,
        approval_broker_grant().unwrap(),
        &worker_key,
        &ManagedBootstrapPaths {
            ca_bundle: None,
            codex_auth: None,
            colossus_home: &home,
        },
    )
    .unwrap();
    let executable = sidecar_executable();
    let mut file = std::fs::File::open(&executable).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let options = SidecarOptions::new(
        InstanceId::from_uuid(
            Uuid::parse_str(
                root.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .strip_prefix(ROOT_PREFIX)
                    .unwrap(),
            )
            .unwrap(),
        ),
        AppPrivateInstanceDir::new(&instance).unwrap(),
        VerifiedExecutable::new(
            executable,
            Sha256Digest::from_bytes(digest.finalize().into()),
        )
        .unwrap(),
        ApiMajor::new(1).unwrap(),
    )
    .unwrap();
    let lifecycle = NativeSidecarLifecycle::new(bootstrap);
    let client = Colossus::start_sidecar(&lifecycle, options)
        .await
        .expect("native managed sidecar");
    let worker = WorkerControlClient::new(
        managed_worker_endpoint(&instance).unwrap(),
        zeroize::Zeroizing::new(worker_key),
    )
    .unwrap();
    let discovered = worker.mcp_tools(Some("large-token")).await;
    let run = std::panic::AssertUnwindSafe(async {
        let tools = discovered.expect("authenticated MCP discovery");
        assert!(
            tools
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "credential_roundtrip")
        );
        run_roundtrip(&client).await;
    })
    .catch_unwind()
    .await;
    client.close().await.expect("graceful native sidecar close");
    if let Err(panic) = run {
        std::panic::resume_unwind(panic);
    }
    if size == 8192 {
        credentials
            .write(CREDENTIAL_ID, fixture::token(65536))
            .await
            .unwrap();
    }
    eprintln!(
        "native backend process: exact {size}-byte vault read and managed MCP tool completed"
    );
}

async fn run_roundtrip(client: &Colossus) {
    let created = client
        .create_run(CreateRunRequest {
            plugin_skill_ids: Vec::new(),
            input: vec![InputContentPart::Text(
                "Call credential_roundtrip with the large-token MCP server.".into(),
            )],
            session_id: None,
            end_user_id: None,
            role: "primary".into(),
            mode: RunMode::Execute,
            research_depth: None,
            research_sources: Vec::new(),
            plan_action: None,
            branch: None,
            max_turns: 3,
            idempotency_key: IdempotencyKey::new(Uuid::now_v7().to_string()).unwrap(),
        })
        .await
        .expect("native run request");
    tokio::time::timeout(Duration::from_mins(1), async {
        loop {
            let run = client
                .get_run(GetRunRequest {
                    run_id: created.run.run_id.clone(),
                })
                .await
                .unwrap()
                .run;
            if !matches!(run.status, RunStatus::Queued | RunStatus::Running) {
                fixture::assert_no_secret(&format!("{run:?}"));
                assert_eq!(run.status, RunStatus::Completed, "MCP run did not complete");
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("native tool roundtrip deadline");
}

fn resolved_configuration(origin: &str, size: usize) -> ResolvedSpaceConfiguration {
    use crate::desktop_settings::{ModelCapabilitiesSetting, ModelSetting, ProviderSetting};
    use crate::managed_configuration::{McpCredentialHeaderSetting, McpServerSetting};
    ResolvedSpaceConfiguration {
        access_profile: AccessProfileSetting::AllowAll,
        execution_boundary: ExecutionBoundarySetting::WorkspaceIsolated,
        terminal_enabled: false,
        field_overrides: Vec::new(),
        providers: vec![ProviderSetting {
            profile: "fixture".into(),
            kind: ProviderKindSetting::Compatible,
            base_url: format!("{origin}/provider/{size}"),
            credential_id: Some(CREDENTIAL_ID.into()),
            timeout_ms: Some(30_000),
        }],
        models: vec![ModelSetting {
            profile: "primary".into(),
            provider_profile: "fixture".into(),
            model: "synthetic-acceptance".into(),
            context_window_tokens: 32768,
            max_output_tokens: 1024,
            capabilities: ModelCapabilitiesSetting {
                tool_calls: true,
                streaming: false,
                image_inputs: false,
            },
            reasoning_effort: None,
        }],
        model_roles: BTreeMap::from([("primary".into(), "primary".into())]),
        search_providers: Vec::new(),
        search_roles: BTreeMap::new(),
        telemetry: None,
        mcp_servers: vec![McpServerSetting {
            name: "large-token".into(),
            transport: McpTransportSetting::StreamableHttp,
            command: None,
            args: Vec::new(),
            working_directory: None,
            environment_credentials: BTreeMap::new(),
            url: Some(format!("{origin}/mcp/{size}")),
            headers: BTreeMap::new(),
            credential_headers: BTreeMap::from([(
                "Authorization".into(),
                McpCredentialHeaderSetting {
                    scheme: Some("Bearer".into()),
                    credential_id: CREDENTIAL_ID.into(),
                },
            )]),
            allow_stateless: true,
            oauth: None,
            allowed_tools: vec!["credential_roundtrip".into()],
            research_tools: Vec::new(),
            timeout_ms: Some(30_000),
            max_output_bytes: Some(65536),
        }],
    }
}

struct PrivateAcceptanceRoot(PathBuf);

fn user_profile() -> PathBuf {
    #[cfg(windows)]
    let variable = "USERPROFILE";
    #[cfg(target_os = "macos")]
    let variable = "HOME";
    PathBuf::from(std::env::var_os(variable).expect("native user profile"))
}

fn validate_fixture_root(root: &Path) {
    assert_eq!(root.parent(), Some(user_profile().as_path()));
    let name = root.file_name().unwrap().to_str().unwrap();
    Uuid::parse_str(
        name.strip_prefix(ROOT_PREFIX)
            .expect("generated acceptance root"),
    )
    .unwrap();
}

impl PrivateAcceptanceRoot {
    fn new() -> Self {
        let path = user_profile().join(format!("{ROOT_PREFIX}{}", Uuid::now_v7()));
        #[cfg(windows)]
        colossus_windows_native::create_private_directory(&path).unwrap();
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
        }
        Self(path)
    }
}

impl PrivateAcceptanceRoot {
    fn cleanup(&self) {
        validate_fixture_root(&self.0);
        let vault = self.0.join("desktop/credentials-v1.redb");
        if vault.exists() {
            let database = redb::Database::open(vault).expect("closed acceptance vault");
            let read = database.begin_read().unwrap();
            let table = read
                .open_table(TableDefinition::<&str, &[u8]>::new(
                    "credential_vault_metadata",
                ))
                .unwrap();
            let metadata: Value =
                serde_json::from_slice(table.get("state").unwrap().unwrap().value()).unwrap();
            assert_eq!(
                metadata["owner_scope_hash"],
                hex::encode(Sha256::digest(b"desktop-manual"))
            );
            let valid_id = |field: &str| {
                let value = metadata[field].as_str().unwrap();
                assert!(value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit()));
                value
            };
            let account = format!("v1.{}.{}", valid_id("vault_id"), valid_id("key_id"));
            #[cfg(windows)]
            let store = windows_native_keyring_store::Store::new().unwrap();
            #[cfg(target_os = "macos")]
            let store = apple_native_keyring_store::keychain::Store::new().unwrap();
            let entry = store.build(KEY_SERVICE, &account, None).unwrap();
            if let Err(error) = entry.delete_credential() {
                assert!(
                    matches!(error, keyring_core::Error::NoEntry),
                    "remove only acceptance vault master key"
                );
            }
        }
        // Immutable plugin snapshots intentionally leave owner-read-only directories.
        // Restore only this stopped test's owned directories before removing its home.
        #[cfg(target_os = "macos")]
        cleanup::prepare_removal(&self.0).expect("prepare generated acceptance directories");
        std::fs::remove_dir_all(&self.0).expect("remove generated private acceptance files");
    }
}

impl Drop for PrivateAcceptanceRoot {
    fn drop(&mut self) {
        if let Err(panic) = std::panic::catch_unwind(|| self.cleanup()) {
            if std::thread::panicking() {
                eprintln!("acceptance cleanup incomplete; generated test root retained");
            } else {
                std::panic::resume_unwind(panic);
            }
        }
    }
}

fn assert_no_plaintext_files(root: &Path) {
    for entry in std::fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        assert!(
            !kind.is_symlink(),
            "acceptance root contains unexpected link"
        );
        if kind.is_dir() {
            assert_no_plaintext_files(&entry.path());
        } else if kind.is_file() {
            let mut file = std::fs::File::open(entry.path()).unwrap();
            let mut previous = Vec::new();
            let mut chunk = [0; 8192];
            loop {
                let count = file.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                previous.extend_from_slice(&chunk[..count]);
                assert!(
                    !previous
                        .windows(fixture::PREFIX.len())
                        .any(|window| window == fixture::PREFIX.as_bytes()),
                    "synthetic credential persisted outside authenticated encryption"
                );
                let keep = previous.len().saturating_sub(fixture::PREFIX.len());
                previous.drain(..keep);
            }
        }
    }
}
