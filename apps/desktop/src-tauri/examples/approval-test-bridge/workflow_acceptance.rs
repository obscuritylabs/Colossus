//! Opt-in renderer acceptance over Desktop's production managed SDK and real sidecar.
use super::*;

pub(super) async fn serve(client: &Colossus, instance: &Path) -> anyhow::Result<()> {
    // This isolated fixture deliberately authorizes agent.run so model-option
    // acceptance can reach the loopback provider. Production policy is unchanged.
    WorkerControlClient::new(
        worker_ipc_endpoint(&instance.join("state.redb"))?,
        zeroize::Zeroizing::new([0x5a; 32]),
    )?
    .set_approval_mode(WorkerApprovalMode::FullAccess)
    .await?;
    let (send, mut receive) = tokio::sync::mpsc::channel(2);
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(std::io::stdin().take(512 * 1024)).lines() {
            let Ok(line) = line else { break };
            if line.len() > 256 * 1024 || send.blocking_send(line).is_err() {
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
        let result = match handle(client, &message).await {
            Ok(result) => json!({"result": result}),
            Err(error) => json!({"error": error.to_string()}),
        };
        println!("{result}");
        std::io::stdout().flush()?;
    }
    Ok(())
}
async fn handle(client: &Colossus, message: &Value) -> anyhow::Result<Value> {
    let api = client
        .workflows()
        .context("workflow resources must be advertised")?;
    let args = &message["args"];
    let text = |field| {
        args[field]
            .as_str()
            .map(str::to_owned)
            .context("required workflow field")
    };
    Ok(match message["command"].as_str() {
        Some("workflow_context") => {
            json!({"selection_epoch": 1, "workflows_read": true, "workflows_register": true, "schedules_read": true, "schedules_create": true, "schedules_control": true, "workflow_runs_read": true, "workflow_runs_start": client.capabilities().contains("workflow_runs.start"), "workflow_run_history": client.capabilities().contains("workflow_runs.history"), "calendar_schedules": client.capabilities().contains("schedules.calendar"), "task_schedules": client.capabilities().contains("schedules.tasks"), "managed": true})
        }
        Some("validate_workflow_definition") => {
            serde_json::to_value(api.validate_definition(text("yaml")?).await?)?
        }
        Some("register_workflow_definition") => serde_json::to_value(
            api.register_definition(
                text("yaml")?,
                text("expectedHash")?,
                text("idempotencyKey")?,
            )
            .await?,
        )?,
        Some("list_registered_workflows") => serde_json::to_value(
            api.list_workflows(args["after"].as_str().map(str::to_owned), 32)
                .await?,
        )?,
        Some("get_registered_workflow") => {
            serde_json::to_value(api.get_workflow(text("workflowId")?).await?)?
        }
        Some("list_workflow_schedules") => serde_json::to_value(
            api.list_schedules(args["after"].as_str().map(str::to_owned), 32)
                .await?,
        )?,
        Some("get_workflow_schedule") => {
            serde_json::to_value(api.get_schedule(text("scheduleId")?).await?)?
        }
        Some("create_workflow_schedule") => serde_json::to_value(
            api.create_schedule(serde_json::from_value(args["request"].clone())?)
                .await?,
        )?,
        Some("set_workflow_schedule_enabled") => serde_json::to_value(
            api.set_schedule_enabled(serde_json::from_value(args["request"].clone())?)
                .await?,
        )?,
        Some("list_workflow_runs") => serde_json::to_value(
            api.list_runs(
                text("workflowId")?,
                args["after"].as_str().map(str::to_owned),
                16,
            )
            .await?,
        )?,
        Some("start_workflow_run") => serde_json::to_value(
            api.start_run(serde_json::from_value(args["request"].clone())?)
                .await?,
        )?,
        Some("get_scheduled_workflow_run") => {
            serde_json::to_value(api.get_run(text("runId")?).await?)?
        }
        Some("active_work") => json!({"active": api.has_active_work().await?}),
        _ => anyhow::bail!("unsupported workflow acceptance operation"),
    })
}
