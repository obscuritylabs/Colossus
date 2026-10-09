//! Opt-in private-fixture driver for real process lifecycle acceptance.
//! Decisions apply only to commands supplied by the loopback test provider.
use super::*;
use colossus_sdk::{
    ListProcessSessionsRequest, ReadProcessSessionRequest, StopProcessSessionRequest,
};

pub(super) async fn serve(client: &Colossus, instance: &Path) -> anyhow::Result<()> {
    WorkerControlClient::new(
        worker_ipc_endpoint(&instance.join("state.redb"))?,
        zeroize::Zeroizing::new([0x5a; 32]),
    )?
    .set_approval_mode(WorkerApprovalMode::Ask)
    .await?;
    let (send, mut receive) = tokio::sync::mpsc::channel(2);
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(std::io::stdin().take(256 * 1024)).lines() {
            let Ok(line) = line else { break };
            if line.len() > 16 * 1024 || send.blocking_send(line).is_err() {
                break;
            }
        }
    });
    println!("{}", json!({"ready": true}));
    std::io::stdout().flush()?;
    while let Some(line) = receive.recv().await {
        let message: Value = serde_json::from_str(&line)?;
        if message["command"] == "close" {
            break;
        }
        let result = dispatch(client, &message).await;
        println!(
            "{}",
            match result {
                Ok(value) => json!({"result":value}),
                Err(error) => json!({"error": error.to_string()}),
            }
        );
        std::io::stdout().flush()?;
    }
    Ok(())
}

async fn dispatch(client: &Colossus, message: &Value) -> anyhow::Result<Value> {
    let args = &message["args"];
    match message["command"].as_str() {
        Some("run") => run(client, args["sessionId"].as_str().map(str::to_owned)).await,
        Some("list_shell_sessions") => Ok(serde_json::to_value(
            client
                .list_process_sessions(ListProcessSessionsRequest { after: None })
                .await?,
        )?),
        Some("read_shell_session") => Ok(serde_json::to_value(
            client
                .read_process_session(ReadProcessSessionRequest {
                    session_id: args["sessionId"]
                        .as_str()
                        .context("session required")?
                        .into(),
                    after_sequence: args["afterSequence"].as_u64().unwrap_or(0),
                    wait_ms: args["waitMs"].as_u64().unwrap_or(0),
                    max_output_bytes: 65536,
                })
                .await?,
        )?),
        Some("stop_shell_session") => Ok(serde_json::to_value(
            client
                .stop_process_session(StopProcessSessionRequest {
                    session_id: args["sessionId"]
                        .as_str()
                        .context("session required")?
                        .into(),
                })
                .await?,
        )?),
        _ => anyhow::bail!("unsupported process acceptance command"),
    }
}

pub(super) async fn run(client: &Colossus, session_id: Option<String>) -> anyhow::Result<Value> {
    let created = client
        .create_run(CreateRunRequest {
            input: vec![InputContentPart::Text(
                "Execute the next isolated process acceptance case.".into(),
            )],
            plugin_skill_ids: vec![],
            session_id,
            end_user_id: None,
            role: "primary".into(),
            mode: RunMode::Execute,
            goal_max_iterations: 0,
            research_depth: None,
            research_sources: vec![],
            plan_action: None,
            branch: None,
            max_turns: 4,
            idempotency_key: IdempotencyKey::new(uuid::Uuid::new_v4().to_string())?,
        })
        .await?;
    let run_id = created.run.run_id.clone();
    let mut approvals = 0;
    let details = tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let details = client
                .get_run(GetRunRequest {
                    run_id: run_id.clone(),
                })
                .await?;
            if let Some(interaction) = details.pending_interactions.first() {
                let InteractionContent::Approval(approval) = &interaction.content else {
                    anyhow::bail!("unexpected interaction")
                };
                client
                    .respond_interaction(RespondInteractionRequest {
                        run_id: run_id.clone(),
                        interaction_id: interaction.interaction_id.clone(),
                        etag: interaction.etag.clone(),
                        idempotency_key: IdempotencyKey::new(uuid::Uuid::new_v4().to_string())?,
                        response: InteractionAnswer::Approval {
                            approved: true,
                            request_hash: approval.request_hash.clone(),
                        },
                    })
                    .await?;
                approvals += 1;
            }
            if matches!(
                details.run.status,
                RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled
            ) {
                return Ok::<_, anyhow::Error>(details);
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
    .await??;
    let activity = released_activity(client, &run_id).await?;
    Ok(
        json!({"run": { "session_id": details.run.session_id, "run_id": details.run.run_id }, "activity": activity, "approvals": approvals}),
    )
}
