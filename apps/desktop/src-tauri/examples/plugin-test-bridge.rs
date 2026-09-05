//! Explicitly opt-in, stdio-only acceptance driver. Never a shipped application target.
//! The test owner supplies paths/consent; renderer arguments cannot supply either.
#[path = "../src/plugin_adapter.rs"]
mod plugin_adapter;

use colossus_worker_protocol::{
    PluginManagementRequest, WorkerControlClient, WorkerControlError, worker_ipc_endpoint,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::Read as _,
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Invocation {
    command: String,
    args: Value,
    #[serde(default)]
    paths: Vec<Option<String>>,
    #[serde(default)]
    approve: bool,
    #[serde(default)]
    cancel: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let state = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("expected isolated worker state path"))?;
    let auth = zeroize::Zeroizing::new(std::fs::read_to_string(format!("{state}.worker-auth"))?);
    let encoded = auth
        .trim()
        .strip_prefix("colossus-worker-auth-v1:")
        .ok_or_else(|| anyhow::anyhow!("invalid isolated worker binding"))?;
    let mut key = zeroize::Zeroizing::new([0_u8; 32]);
    hex::decode_to_slice(encoded, key.as_mut())
        .map_err(|_| anyhow::anyhow!("invalid isolated worker key"))?;
    let worker = WorkerControlClient::new(worker_ipc_endpoint(Path::new(&state))?, key)?;
    let mut input = String::new();
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_string(&mut input)?;
    anyhow::ensure!(input.len() <= 1024 * 1024, "test invocation exceeds limit");
    let invocation: Invocation = serde_json::from_str(&input)?;
    let prompts = Arc::new(Mutex::new(Vec::<Value>::new()));
    let result = invoke(&worker, invocation, Arc::clone(&prompts)).await;
    let response = match result {
        Ok(value) => json!({"result": value}),
        Err(error) => {
            json!({"error": {"code": "plugin_operation_failed", "message": match error { WorkerControlError::Remote(message) => message, WorkerControlError::Protocol(message) => format!("Test bridge protocol: {message}"), WorkerControlError::Busy => "Test bridge worker timed out".into(), WorkerControlError::Io(error) => format!("Test bridge transport: {:?}", error.kind()), WorkerControlError::Unavailable => "Test worker unavailable".into() }, "retryable": false, "outcomeUnknown": false, "violations": []}})
        }
    };
    println!(
        "{}",
        json!({"response": response, "prompts": *prompts.lock().map_err(|_| anyhow::anyhow!("test prompt lock"))?})
    );
    Ok(())
}

async fn invoke(
    worker: &WorkerControlClient,
    input: Invocation,
    prompts: Arc<Mutex<Vec<Value>>>,
) -> Result<Value, WorkerControlError> {
    if input.args.get("targetId").and_then(Value::as_str) != Some("local") {
        return Err(WorkerControlError::Remote(
            "The test worker belongs to Managed Local only".into(),
        ));
    }
    match input.command.as_str() {
        "get_plugin_inventory" => {
            serde_json::to_value(plugin_adapter::inventory(worker).await?).map_err(invalid)
        }
        "read_plugin_preview" => {
            plugin_adapter::preview(
                worker,
                serde_json::from_value(input.args["request"].clone()).map_err(invalid)?,
            )
            .await
        }
        "manage_plugin" => {
            let request: PluginManagementRequest =
                serde_json::from_value(input.args["input"]["request"].clone()).map_err(invalid)?;
            let mut paths = input.paths.into_iter();
            let Some(request) = plugin_adapter::select_paths(
                request,
                input.args["input"]["verifyArchive"]
                    .as_bool()
                    .unwrap_or(false),
                |_, _| Ok::<_, WorkerControlError>(paths.next().flatten()),
            )?
            else {
                return Ok(json!({"cancelled": true}));
            };
            let (_cancel, cancelled) = tokio::sync::watch::channel(input.cancel);
            plugin_adapter::manage(worker, request, cancelled, move |prompt| {
                prompts.lock().expect("test prompt lock").push(json!({"title": prompt.title, "question": prompt.question, "details": prompt.details, "choices": prompt.choices}));
                async move { input.approve.then(|| "Allow once".into()) }
            }).await
        }
        _ => Err(WorkerControlError::Remote(
            "Unsupported acceptance command".into(),
        )),
    }
}

fn invalid(_: serde_json::Error) -> WorkerControlError {
    WorkerControlError::Protocol("invalid acceptance input".into())
}
