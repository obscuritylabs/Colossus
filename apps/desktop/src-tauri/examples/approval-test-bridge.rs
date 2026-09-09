//! Opt-in stdio acceptance host. Never built or invoked by the shipped application.
//! Only test-owned confirmation replaces OS UI; challenge refetch and broker stay real.
#[path = "../src/approval_adapter.rs"]
mod approval_adapter;
#[path = "approval-test-bridge/diagnostics.rs"]
mod diagnostics;

use anyhow::Context as _;
use colossus_sdk::{
    ApiMajor, ApiScope, AppPrivateInstanceDir, CancelRunRequest, Colossus, CreateRunRequest,
    GetRunRequest, IdempotencyKey, InputContentPart, InstanceId, InteractionAnswer,
    InteractionContent, MacosCodeSigningRequirement, ManagedAccessProfile, ManagedProviderKind,
    ManagedRuntimeConfig, NativeSidecarLifecycle, RespondInteractionRequest, RunMode, RunStatus,
    Secret, Sha256Digest, SidecarApplicationGrant, SidecarApprovalBrokerGrant,
    SidecarBootstrapConfig, SidecarOptions, VerifiedExecutable, scopes,
};
use colossus_worker_protocol::{WorkerApprovalMode, WorkerControlClient, worker_ipc_endpoint};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    io::{BufRead as _, Read as _, Write as _},
    path::Path,
    time::Duration,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    anyhow::ensure!(
        args.len() == 3,
        "expected sidecar, private fixture root, loopback provider URL"
    );
    let root = std::fs::canonicalize(&args[1])?;
    let workspace = std::fs::canonicalize(root.join("work"))?;
    let instance = std::fs::canonicalize(root.join("instance"))?;
    let executable = std::fs::canonicalize(&args[0])?;
    let digest = executable_digest(&executable)?;
    let mut runtime = ManagedRuntimeConfig::echo(ManagedAccessProfile::Development);
    runtime.providers[0].kind = ManagedProviderKind::OpenAiCompatible;
    runtime.providers[0].base_url = Some(args[2].clone());
    runtime.providers[0].timeout_ms = 5000;
    runtime.models[0].model = "approval-fixture".into();
    let grant = SidecarApplicationGrant::new(
        "app:approval-acceptance",
        [
            scopes::RUNS_EXECUTE,
            scopes::RUNS_READ,
            scopes::RUNS_CONTROL,
        ]
        .into_iter()
        .map(ApiScope::new)
        .collect::<Result<Vec<_>, _>>()?,
        ["primary".into()],
        ["shell.run".into()],
    )?;
    let bootstrap = SidecarBootstrapConfig::new(&workspace, runtime, grant)?
        .with_colossus_home(root.join("home"))?
        .with_plaintext_journal_for_development()
        .with_approval_broker_grant(SidecarApprovalBrokerGrant::new(
            "app:approval-acceptance",
            ["primary".into()],
        )?)?
        .with_worker_ipc_authentication(Secret::new(vec![0x5a; 32])?)?;
    let options = SidecarOptions::new(
        InstanceId::from_uuid(uuid::Uuid::new_v4()),
        AppPrivateInstanceDir::new(&instance)?,
        VerifiedExecutable::new(executable, Sha256Digest::from_bytes(digest))?
            .with_macos_code_signing_requirement(
                MacosCodeSigningRequirement::AdHocDeveloperPreview,
            ),
        ApiMajor::new(1)?,
    )?;
    let client = Colossus::start_sidecar(&NativeSidecarLifecycle::new(bootstrap), options).await?;
    let result = serve(&client, &instance).await;
    client.close().await?;
    result
}

fn executable_digest(path: &Path) -> anyhow::Result<[u8; 32]> {
    let mut file = std::fs::File::open(path)?;
    anyhow::ensure!(
        file.metadata()?.len() <= 512 * 1024 * 1024,
        "fixture executable too large"
    );
    let mut hash = Sha256::new();
    let mut bytes = vec![0; 64 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(hash.finalize().into())
}

async fn serve(client: &Colossus, instance: &Path) -> anyhow::Result<()> {
    let request = start_run(client, instance).await?;
    review_loop(client, request).await
}

async fn start_run(
    client: &Colossus,
    instance: &Path,
) -> anyhow::Result<RespondInteractionRequest> {
    let worker = WorkerControlClient::new(
        worker_ipc_endpoint(&instance.join("state.redb"))?,
        zeroize::Zeroizing::new([0x5a; 32]),
    )?;
    worker
        .set_approval_mode(WorkerApprovalMode::Ask)
        .await
        .context("authenticated worker approval-mode setup failed")?;
    let run = client
        .create_run(CreateRunRequest {
            input: vec![InputContentPart::Text(
                "Verify the command approval marker.".into(),
            )],
            plugin_skill_ids: vec![],
            session_id: None,
            end_user_id: None,
            role: "primary".into(),
            mode: RunMode::Execute,
            research_depth: None,
            research_sources: vec![],
            plan_action: None,
            branch: None,
            max_turns: 4,
            idempotency_key: IdempotencyKey::new("approval-acceptance")?,
        })
        .await?;
    tokio::time::timeout(Duration::from_secs(30), async {
        // Approval publication and buffered tool events have separate writers.
        // Wait for the queued start event before capturing the run's etag; a
        // slower journal can otherwise change it between the first two reads.
        // No decision is made here, and the production adapter still refetches
        // and validates the exact frozen challenge before every review action.
        let mut updates = client
            .watch_run(colossus_sdk::WatchRunRequest {
                run_id: run.run.run_id.clone(),
                after_sequence: 0,
            })
            .await?;
        let mut command_started = false;
        while let Some(update) = updates.next_update().await {
            if let colossus_sdk::RunUpdateKind::ToolActivity(activity) = update?.update
                && activity.tool_name == "shell.run"
                && activity.state == colossus_sdk::ToolActivityState::Started
            {
                command_started = true;
                break;
            }
        }
        anyhow::ensure!(
            command_started,
            "run ended before command activity was released"
        );
        drop(updates);
        loop {
            let details = client
                .get_run(GetRunRequest {
                    run_id: run.run.run_id.clone(),
                })
                .await?;
            if let Some(interaction) = details.pending_interactions.first() {
                let InteractionContent::Approval(approval) = &interaction.content else {
                    anyhow::bail!("expected approval");
                };
                return Ok::<_, anyhow::Error>(RespondInteractionRequest {
                    run_id: interaction.run_id.clone(),
                    interaction_id: interaction.interaction_id.clone(),
                    etag: interaction.etag.clone(),
                    idempotency_key: IdempotencyKey::new("approval-response")?,
                    response: InteractionAnswer::Approval {
                        approved: true,
                        request_hash: approval.request_hash.clone(),
                    },
                });
            }
            anyhow::ensure!(
                !matches!(details.run.status, RunStatus::Failed | RunStatus::Completed),
                "run ended before approval: {:?}",
                details.run.status
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await?
}

async fn review_loop(client: &Colossus, request: RespondInteractionRequest) -> anyhow::Result<()> {
    let began = std::time::Instant::now();
    let frozen = match approval_adapter::pending(client, &request).await {
        Ok(frozen) => frozen,
        Err(error) => {
            // Categorical fixture diagnostics only: no command, credential,
            // interaction identifier, etag, or approval binding is printed.
            eprintln!(
                "initial command review lookup failed after {:?}",
                began.elapsed()
            );
            if let Ok(Ok(details)) = tokio::time::timeout(
                Duration::from_secs(5),
                client.get_run(GetRunRequest {
                    run_id: request.run_id.clone(),
                }),
            )
            .await
            {
                let current = details
                    .pending_interactions
                    .iter()
                    .find(|item| item.interaction_id == request.interaction_id);
                eprintln!(
                    "run_status={:?} pending_count={} interaction_present={} etag_matches={} respondable={}",
                    details.run.status,
                    details.pending_interactions.len(),
                    current.is_some(),
                    current.is_some_and(|item| item.etag == request.etag),
                    current.is_some_and(|item| item.respondable_by_caller),
                );
            }
            return Err(error.into());
        }
    };
    let review_id = uuid::Uuid::new_v4().to_string();
    println!("{}", json!({"ready": true}));
    std::io::stdout().flush()?;
    let (send, mut receive) = tokio::sync::mpsc::channel(2);
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(std::io::stdin().take(256 * 1024)).lines() {
            let Ok(line) = line else {
                break;
            };
            if line.len() > 16 * 1024 || send.blocking_send(line).is_err() {
                break;
            }
        }
    });
    let mut submitted = false;
    while let Some(line) = receive.recv().await {
        let value: Value = serde_json::from_str(&line)?;
        if value["command"] == "close" {
            break;
        }
        let outcome = async {
            if value["command"] == "released_activity" {
                return released_activity(client, &request.run_id).await;
            }
            if value["command"] == "cancel_run" {
                client.cancel_run(CancelRunRequest { run_id: request.run_id.clone(), idempotency_key: IdempotencyKey::new("approval-cancellation")? }).await?;
                return Ok::<_, anyhow::Error>(Value::Null);
            }
            anyhow::ensure!(approval_adapter::pending(client, &request).await? == frozen, "stale challenge");
            match value["command"].as_str() {
                Some("command_review_context") => Ok(json!({"reviewId": review_id, "target": "Managed Local — isolated acceptance workspace", "commandContext": approval_adapter::CommandApprovalContextDto::from(frozen.command_context.clone().ok_or_else(|| anyhow::anyhow!("missing command context"))?)})),
                Some("finish_command_review") => {
                    anyhow::ensure!(!submitted && value["args"]["reviewId"] == review_id, "stale review identity");
                    let approved = value["args"]["approved"].as_bool().ok_or_else(|| anyhow::anyhow!("missing decision"))?;
                    let mut response = request.clone();
                    let InteractionAnswer::Approval { approved: decision, .. } = &mut response.response else { unreachable!() };
                    *decision = approved;
                    client.respond_interaction(response).await?;
                    submitted = true;
                    Ok(Value::Null)
                }
                _ => anyhow::bail!("unsupported test command"),
            }
        }.await;
        let result = match outcome {
            Ok(result) => json!({"result": result}),
            Err(_) if value["command"] == "released_activity" => {
                activity_failure(client, &request.run_id).await
            }
            Err(_) => json!({"error": "stale or unavailable command review"}),
        };
        println!("{result}");
        std::io::stdout().flush()?;
    }
    Ok(())
}

async fn released_activity(client: &Colossus, run_id: &str) -> anyhow::Result<Value> {
    // The normal process budget is 30 seconds. Collection must cover that
    // budget plus bounded cleanup/provider/journal work, not shorten execution.
    // This acceptance-only deadline does not change approval or effect limits.
    tokio::time::timeout(Duration::from_secs(45), async {
        let mut updates = client
            .watch_run(colossus_sdk::WatchRunRequest {
                run_id: run_id.into(),
                after_sequence: 0,
            })
            .await?;
        let mut activity = Vec::new();
        while let Some(update) = updates.next_update().await {
            if let colossus_sdk::RunUpdateKind::ToolActivity(tool) = update?.update {
                activity.push(
                    json!({"name": tool.tool_name, "state": format!("{:?}", tool.state),
                    "input": tool.input, "preview": tool.preview}),
                );
            }
        }
        // Terminal errors can stop the agent before another provider request.
        // Retain only the already-public categorical outcome in that case.
        let details = client
            .get_run(GetRunRequest {
                run_id: run_id.into(),
            })
            .await?;
        Ok::<_, anyhow::Error>(json!({
            "activity": activity,
            "terminal": diagnostics::terminal(
                details.run.status,
                details.run.terminal.as_ref(),
            ),
        }))
    })
    .await?
}

async fn activity_failure(client: &Colossus, run_id: &str) -> Value {
    // Never serialize the underlying error or challenge. Only categorical run
    // state is needed to distinguish a slow effect from another pending approval.
    let diagnostics = match tokio::time::timeout(
        Duration::from_secs(5),
        client.get_run(GetRunRequest {
            run_id: run_id.into(),
        }),
    )
    .await
    {
        Ok(Ok(details)) => json!({
            "run_status": format!("{:?}", details.run.status),
            "pending_count": details.pending_interactions.len(),
        }),
        _ => json!({"run_status": "unavailable"}),
    };
    json!({
        "error": "released run activity collection failed or timed out",
        "diagnostics": diagnostics,
    })
}
