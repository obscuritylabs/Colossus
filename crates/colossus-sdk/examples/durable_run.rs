//! Run one durable prompt through an enrolled shared Colossus worker.
//!
//! This example deliberately loads the bearer from the OS credential store. The
//! independently enrolled instance ID and certificate pin remain separate arguments.

use colossus_sdk::{
    ApiMajor, ApprovalInteraction, ArtifactPurpose, Colossus, DaemonConnectOptions, IdempotencyKey,
    InputContentPart, InstanceId, Interaction, InteractionAnswer, InteractionContent,
    KeyringCredentialProvider, PromptAnswer, RespondInteractionRequest, RunMode, RunUpdateKind,
    TlsFingerprint, UploadArtifactRequest, WatchRunRequest,
};
use std::{env, error::Error, fs, path::PathBuf, sync::Arc};
use uuid::Uuid;

struct Options {
    public_api_dir: PathBuf,
    instance_id: InstanceId,
    certificate_pin: TlsFingerprint,
    keyring_service: String,
    keyring_account: String,
    mode: RunMode,
    approve_effects: bool,
    prompt_answer: Option<String>,
    attachment: Option<PathBuf>,
    prompt: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let options = parse_options()?;
    let credential = Arc::new(KeyringCredentialProvider::new(
        options.keyring_service.clone(),
        options.keyring_account.clone(),
    )?);
    let connect = DaemonConnectOptions::new(
        options.instance_id,
        options.public_api_dir.join("endpoint.json"),
        options.certificate_pin,
        ApiMajor::new(1)?,
        credential,
    )?
    .with_certificate_path(options.public_api_dir.join("certificate.pem"))?;
    let client = Colossus::connect_installed(connect).await?;
    let mut input = vec![InputContentPart::Text(options.prompt.clone())];
    if let Some(path) = options.attachment.as_deref() {
        if !client.capabilities().contains("attachments.run_input") {
            return Err("the authenticated runtime did not advertise run attachments".into());
        }
        let bytes = fs::read(path)?;
        if bytes.len() > 16 * 1_048_576 || std::str::from_utf8(&bytes).is_err() {
            return Err("the example accepts UTF-8 attachments up to 16 MiB".into());
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("the attachment must have a UTF-8 display name")?
            .to_owned();
        let artifact = client
            .upload_artifact(UploadArtifactRequest {
                file_name,
                media_type: attachment_media_type(path).into(),
                purpose: ArtifactPurpose::RunInput,
                bytes,
                idempotency_key: next_idempotency_key("attachment")?,
            })
            .await?;
        eprintln!(
            "uploaded attachment {} as {}",
            artifact.file_name, artifact.artifact_id
        );
        input.push(InputContentPart::Artifact(artifact.artifact_id));
    }

    let created = client
        .create_run(colossus_sdk::CreateRunRequest {
            input,
            session_id: None,
            end_user_id: None,
            role: "primary".to_owned(),
            mode: options.mode,
            research_depth: None,
            research_sources: Vec::new(),
            selected_skills: Vec::new(),
            plan_action: None,
            branch: None,
            max_turns: 12,
            idempotency_key: next_idempotency_key("create")?,
        })
        .await?;
    let run_id = created.run.run_id;
    eprintln!("run {run_id}");

    let mut updates = client
        .watch_run(WatchRunRequest {
            run_id: run_id.clone(),
            after_sequence: 0,
        })
        .await?;
    while let Some(update) = updates.next_update().await {
        let update = update?;
        match update.update {
            RunUpdateKind::ToolActivity(activity) => {
                eprintln!(
                    "tool {} {:?}: {}",
                    activity.tool_name, activity.state, activity.summary
                );
            }
            RunUpdateKind::Interaction(interaction) if interaction.respondable_by_caller => {
                respond_to_interaction(&client, interaction, &options).await?;
            }
            RunUpdateKind::Notice { reason, message } => {
                eprintln!("notice {reason}: {message}");
            }
            RunUpdateKind::Result(result) => {
                println!("{}", result.output);
            }
            RunUpdateKind::Failure { failure, .. } => {
                eprintln!(
                    "run failed: {} (reason={}, recoverable={}, outcome={:?}, http_status={:?}, retry_after_ms={:?})",
                    failure.message,
                    failure.reason,
                    failure.recoverable,
                    failure.outcome_certainty,
                    failure.http_status,
                    failure.retry_after_ms,
                );
            }
            RunUpdateKind::Cancellation(cancellation) => {
                eprintln!("run cancelled: {}", cancellation.message);
            }
            _ => {}
        }
    }

    client.close().await?;
    Ok(())
}

async fn respond_to_interaction(
    client: &Colossus,
    interaction: Interaction,
    options: &Options,
) -> Result<(), Box<dyn Error>> {
    let response = match &interaction.content {
        InteractionContent::Approval(approval) => {
            display_approval(approval, options.approve_effects);
            InteractionAnswer::Approval {
                approved: options.approve_effects,
                request_hash: approval.request_hash.clone(),
            }
        }
        InteractionContent::UserPrompt(prompt) => {
            eprintln!("user input requested: {}", prompt.question);
            for choice in &prompt.choices {
                eprintln!("  choice {}: {}", choice.choice_id, choice.label);
            }
            let answer = options
                .prompt_answer
                .clone()
                .ok_or("the run is waiting for user input; restart the example with --answer TEXT or cancel the durable run")?;
            InteractionAnswer::Prompt(PromptAnswer::FreeForm(answer))
        }
    };
    client
        .respond_interaction(RespondInteractionRequest {
            run_id: interaction.run_id,
            interaction_id: interaction.interaction_id,
            etag: interaction.etag,
            idempotency_key: next_idempotency_key("interaction")?,
            response,
        })
        .await?;
    Ok(())
}

fn display_approval(approval: &ApprovalInteraction, approved: bool) {
    eprintln!(
        "approval requested: action={} resource={} risk={:?}",
        approval.action, approval.resource, approval.risk
    );
    eprintln!("reason: {}", approval.reason);
    eprintln!(
        "decision: {}",
        if approved {
            "allow once (--approve-effects)"
        } else {
            "deny (safe default)"
        }
    );
}

fn next_idempotency_key(operation: &str) -> Result<IdempotencyKey, Box<dyn Error>> {
    Ok(IdempotencyKey::new(format!(
        "sdk-example-{operation}-{}",
        Uuid::now_v7()
    ))?)
}

fn parse_options() -> Result<Options, Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let public_api_dir = PathBuf::from(next_argument(&mut arguments, "PUBLIC_API_DIR")?);
    let instance_id = next_argument(&mut arguments, "INSTANCE_ID")?.parse()?;
    let certificate_pin =
        TlsFingerprint::from_hex(&next_argument(&mut arguments, "CERTIFICATE_SHA256")?)?;
    let keyring_service = next_argument(&mut arguments, "KEYRING_SERVICE")?;
    let keyring_account = next_argument(&mut arguments, "KEYRING_ACCOUNT")?;

    let mut mode = RunMode::Execute;
    let mut approve_effects = false;
    let mut prompt_answer = None;
    let mut attachment = None;
    let mut prompt_parts = Vec::new();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--plan" => mode = RunMode::Plan,
            "--approve-effects" => approve_effects = true,
            "--answer" => {
                prompt_answer = Some(next_argument(&mut arguments, "TEXT after --answer")?);
            }
            "--attach" => {
                attachment = Some(PathBuf::from(next_argument(
                    &mut arguments,
                    "PATH after --attach",
                )?));
            }
            _ => prompt_parts.push(argument),
        }
    }
    if prompt_parts.is_empty() {
        return Err(usage().into());
    }
    Ok(Options {
        public_api_dir,
        instance_id,
        certificate_pin,
        keyring_service,
        keyring_account,
        mode,
        approve_effects,
        prompt_answer,
        attachment,
        prompt: prompt_parts.join(" "),
    })
}

fn next_argument(
    arguments: &mut impl Iterator<Item = String>,
    name: &str,
) -> Result<String, Box<dyn Error>> {
    arguments
        .next()
        .ok_or_else(|| format!("missing {name}\n{}", usage()).into())
}

fn usage() -> &'static str {
    "usage: durable_run PUBLIC_API_DIR INSTANCE_ID CERTIFICATE_SHA256 \
KEYRING_SERVICE KEYRING_ACCOUNT [--plan] [--approve-effects] [--answer TEXT] \
[--attach PATH] PROMPT..."
}

fn attachment_media_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => "application/json",
        Some("md") | Some("markdown") => "text/markdown",
        Some("yaml") | Some("yml") => "application/yaml",
        Some("toml") => "application/toml",
        Some("csv") => "text/csv",
        _ => "text/plain",
    }
}
