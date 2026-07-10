//! Thin terminal interface for the Rust runtime.

use clap::{Args, Parser, Subcommand};
use colossus_runtime::{Runtime, RuntimeConfig};
use reedline::{DefaultPrompt, Reedline, Signal};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    name = "colossus-rs",
    version,
    about = "Auditable Colossus workflow runtime"
)]
struct Cli {
    /// Fresh Rust YAML configuration path.
    #[arg(long, default_value = ".colossus/config.yaml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create or inspect fresh YAML configuration.
    Config(ConfigCommand),
    /// Verify and inspect the authoritative journal.
    Audit(AuditCommand),
    /// Diagnose the active built-in or OPA policy channel.
    Policy(PolicyCommand),
    /// Inspect, drain, or rebuild disposable state projections.
    Projection(ProjectionCommand),
    /// Diagnose canonical storage, lease, repositories, and projection readiness.
    State(StateCommand),
    /// Diagnose the native/OCI sandbox helper.
    Sandbox(SandboxCommand),
    /// Execute exact programs without a shell through the effect gateway.
    Process(ProcessCommand),
    /// Perform policy-allowed brokered network requests.
    Network(NetworkCommand),
    /// Validate and operate durable workflows.
    Workflow(WorkflowCommand),
    /// Inspect and diagnose configured model providers.
    Provider(ProviderCommand),
    /// Inspect model role routing.
    Models(ModelsCommand),
    /// Execute one audited model turn through the configured role.
    Run {
        /// User prompt sent as the complete logical request content.
        prompt: String,
        /// Configured model role.
        #[arg(long, default_value = "primary")]
        role: String,
        /// System/developer instructions for this turn.
        #[arg(long, default_value = "You are Colossus.")]
        instructions: String,
    },
    /// Run the credential-free, network-free echo smoke provider.
    Echo {
        /// Text returned by the deterministic provider.
        message: String,
    },
    /// Start the modern interactive terminal.
    Repl,
    /// Recover abandoned runs and drain queued resumable work.
    Worker {
        /// Recover state and return without repeatedly polling.
        #[arg(long, default_value_t = true)]
        once: bool,
    },
    /// Internal authenticated one-shot sandbox helper.
    #[command(name = "__sandbox-helper", hide = true)]
    SandboxHelper,
}

#[derive(Args)]
struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigAction,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Create a strict offline configuration without overwriting an existing file.
    Init,
    /// Parse and print the active configuration with references intact.
    Show,
}

#[derive(Args)]
struct AuditCommand {
    #[command(subcommand)]
    command: AuditAction,
}

#[derive(Subcommand)]
enum AuditAction {
    /// Verify encryption, chain, checkpoint signature, and secure anchor.
    Verify,
    /// Show bounded envelope metadata without decrypted payload content.
    Show {
        /// First global sequence.
        #[arg(long, default_value_t = 1)]
        from: u64,
        /// Maximum records.
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Stream bounded redacted envelopes as JSON Lines to stdout.
    Export {
        /// First global sequence.
        #[arg(long, default_value_t = 1)]
        from: u64,
        /// Maximum records.
        #[arg(long, default_value_t = 1_000)]
        limit: usize,
    },
    /// Show the latest signed checkpoint and secure chain head.
    AnchorStatus,
}

#[derive(Args)]
struct PolicyCommand {
    #[command(subcommand)]
    command: PolicyAction,
}

#[derive(Subcommand)]
enum PolicyAction {
    /// Check readiness, revision metadata, and decision-log safeguards.
    Doctor,
}

#[derive(Args)]
struct ProjectionCommand {
    #[command(subcommand)]
    command: ProjectionAction,
}

#[derive(Subcommand)]
enum ProjectionAction {
    /// Show position, journal head, lag, and readiness.
    Status,
    /// Replay queued journal records into every projection.
    Drain,
    /// Delete and replay one projection, or every projection when omitted.
    Rebuild { name: Option<String> },
}

#[derive(Args)]
struct StateCommand {
    #[command(subcommand)]
    command: StateAction,
}

#[derive(Subcommand)]
enum StateAction {
    /// Check the writer lease, journal head, adapters, and projection lag.
    Doctor,
}

#[derive(Args)]
struct SandboxCommand {
    #[command(subcommand)]
    command: SandboxAction,
}

#[derive(Subcommand)]
enum SandboxAction {
    /// Report native kernel support and configured OCI fallback.
    Doctor,
}

#[derive(Args)]
struct ProcessCommand {
    #[command(subcommand)]
    command: ProcessAction,
}

#[derive(Subcommand)]
enum ProcessAction {
    /// Run one exact executable with literal arguments and an explicit environment.
    Run {
        executable: PathBuf,
        /// Absolute or repository-relative working directory.
        #[arg(long, default_value = ".")]
        cwd: PathBuf,
        /// Explicit KEY=VALUE environment entry. Repeat as needed.
        #[arg(long = "env")]
        environment: Vec<String>,
        /// Literal arguments passed after `--`; no shell interpretation occurs.
        #[arg(last = true)]
        args: Vec<String>,
    },
}

#[derive(Args)]
struct NetworkCommand {
    #[command(subcommand)]
    command: NetworkAction,
}

#[derive(Subcommand)]
enum NetworkAction {
    /// Fetch one exact HTTP(S) URL through destination enforcement and quarantine.
    Get { url: String },
}

#[derive(Args)]
struct WorkflowCommand {
    #[command(subcommand)]
    command: WorkflowAction,
}

#[derive(Subcommand)]
enum WorkflowAction {
    /// Parse and validate a strict workflow YAML file.
    Validate { path: PathBuf },
    /// Validate and register a definition with repository provenance.
    Register { path: PathBuf },
    /// List registered definition change events.
    List,
    /// Show an exact registered definition and pinned content hash.
    Show { name: String, version: String },
    /// Start a durable run.
    Run {
        name: String,
        version: String,
        /// Inline JSON or @path to a JSON document.
        #[arg(long, default_value = "{}")]
        inputs: String,
    },
    /// Show a reconstructed run.
    Status { run_id: String },
    /// Resume a waiting or interrupted run.
    Resume { run_id: String },
    /// Supply inline JSON or @path input and resume.
    Input { run_id: String, input: String },
    /// Cancel a non-terminal run.
    Cancel { run_id: String },
}

#[derive(Args)]
struct ProviderCommand {
    #[command(subcommand)]
    command: ProviderAction,
}

#[derive(Subcommand)]
enum ProviderAction {
    /// Show configured profiles without resolving credentials.
    Profiles,
    /// Exercise the profile model-catalog endpoint through policy.
    Doctor { profile: Option<String> },
    /// List normalized models through policy.
    Models { profile: Option<String> },
}

#[derive(Args)]
struct ModelsCommand {
    #[command(subcommand)]
    command: ModelsAction,
}

#[derive(Subcommand)]
enum ModelsAction {
    /// Show role-to-profile mappings.
    Routes,
}

async fn parse_json_argument(runtime: &Runtime, source: &str) -> Result<Value, Box<dyn Error>> {
    let document = if let Some(path) = source.strip_prefix('@') {
        runtime.read_text_file(path).await?
    } else {
        source.to_owned()
    };
    Ok(serde_json::from_str(&document)?)
}

fn init_config(path: &Path) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        return Err(format!("refusing to overwrite {}", path.display()).into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let state = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("state.redb");
    let config = RuntimeConfig::offline_template(state);
    fs::write(path, config.to_yaml()?)?;
    println!("created {}", path.display());
    Ok(())
}

fn print_json(value: &impl serde::Serialize) -> Result<(), Box<dyn Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn parse_environment(entries: Vec<String>) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let mut environment = BTreeMap::new();
    for entry in entries {
        let (name, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("environment entry must be KEY=VALUE: {entry}"))?;
        if name.is_empty() || environment.insert(name.into(), value.into()).is_some() {
            return Err(format!("environment name is empty or duplicated: {name}").into());
        }
    }
    Ok(environment)
}

async fn workflow_command(
    runtime: &Runtime,
    command: WorkflowAction,
) -> Result<(), Box<dyn Error>> {
    match command {
        WorkflowAction::Validate { path } => {
            let validated = runtime.validate_workflow_path(&path).await?;
            print_json(&json!({
                "valid": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
            }))?;
        }
        WorkflowAction::Register { path } => {
            let provenance = format!("repo:{}", path.display());
            let validated = runtime.register_workflow_path(&path).await?;
            print_json(&json!({
                "registered": true,
                "name": validated.definition.metadata.name,
                "version": validated.definition.metadata.version,
                "content_hash": validated.content_hash,
                "provenance": provenance,
            }))?;
        }
        WorkflowAction::List => {
            let journal = runtime.journal();
            let definitions = journal
                .read_global(1, usize::MAX)?
                .into_iter()
                .filter(|event| event.event_type.starts_with("workflow.definition."))
                .map(|event| {
                    json!({
                        "event_id": event.event_id,
                        "event_type": event.event_type,
                        "stream_id": event.stream_id,
                        "occurred_at": event.occurred_at,
                        "record_hash": event.record_hash,
                    })
                })
                .collect::<Vec<_>>();
            print_json(&definitions)?;
        }
        WorkflowAction::Show { name, version } => {
            let (definition, content_hash) = runtime
                .workflow_repository()
                .definition(&name, &version)?
                .ok_or_else(|| format!("workflow {name}:{version} is not registered"))?;
            print_json(&json!({
                "definition": definition,
                "content_hash": content_hash,
            }))?;
        }
        WorkflowAction::Run {
            name,
            version,
            inputs,
        } => {
            let run = runtime
                .workflows()
                .start_run(
                    &name,
                    &version,
                    parse_json_argument(runtime, &inputs).await?,
                )
                .await?;
            print_json(&run)?;
        }
        WorkflowAction::Status { run_id } => {
            print_json(&runtime.workflows().get_run(&run_id)?)?;
        }
        WorkflowAction::Resume { run_id } => {
            print_json(&runtime.workflows().resume_run(&run_id).await?)?;
        }
        WorkflowAction::Input { run_id, input } => {
            print_json(
                &runtime
                    .workflows()
                    .provide_input(&run_id, parse_json_argument(runtime, &input).await?)
                    .await?,
            )?;
        }
        WorkflowAction::Cancel { run_id } => {
            print_json(&runtime.workflows().cancel_run(&run_id)?)?;
        }
    }
    Ok(())
}

async fn repl(runtime: &Runtime) -> Result<(), Box<dyn Error>> {
    let mut editor = Reedline::create();
    let prompt = DefaultPrompt::default();
    println!("Colossus Rust alpha. /help for commands; Ctrl-D to exit.");
    loop {
        match editor.read_line(&prompt)? {
            Signal::Success(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if matches!(line, "/quit" | "/exit") {
                    break;
                }
                if line == "/help" {
                    println!(
                        "/workflow list | /workflow status RUN_ID | /audit verify | /projection status | /exit"
                    );
                    println!("Any other line is sent through the configured primary model role.");
                } else if line == "/workflow list" {
                    workflow_command(runtime, WorkflowAction::List).await?;
                } else if let Some(run_id) = line.strip_prefix("/workflow status ") {
                    workflow_command(
                        runtime,
                        WorkflowAction::Status {
                            run_id: run_id.trim().into(),
                        },
                    )
                    .await?;
                } else if line == "/audit verify" {
                    print_json(&runtime.journal().verify()?)?;
                } else if line == "/projection status" {
                    print_json(&runtime.projection_status()?)?;
                } else {
                    let result = runtime
                        .run_model("primary", "You are Colossus.", line)
                        .await?;
                    println!("{}", result.output);
                }
            }
            Signal::CtrlD | Signal::CtrlC => break,
            _ => continue,
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    if matches!(cli.command, Command::SandboxHelper) {
        colossus_sandbox::run_helper_stdio()?;
        return Ok(());
    }
    if let Command::Config(ConfigCommand {
        command: ConfigAction::Init,
    }) = &cli.command
    {
        return init_config(&cli.config);
    }
    let config = RuntimeConfig::from_path(&cli.config)?;
    if matches!(
        cli.command,
        Command::Config(ConfigCommand {
            command: ConfigAction::Show
        })
    ) {
        print!("{}", config.to_yaml()?);
        return Ok(());
    }
    let runtime = Runtime::open(&config)?;
    match cli.command {
        Command::Config(_) => unreachable!("handled before runtime construction"),
        Command::Audit(command) => match command.command {
            AuditAction::Verify | AuditAction::AnchorStatus => {
                print_json(&runtime.journal().verify()?)?;
            }
            AuditAction::Show { from, limit } => {
                print_json(&runtime.journal().read_global(from, limit)?)?;
            }
            AuditAction::Export { from, limit } => {
                for event in runtime.journal().read_global(from, limit)? {
                    println!("{}", serde_json::to_string(&event)?);
                }
            }
        },
        Command::Policy(command) => match command.command {
            PolicyAction::Doctor => print_json(&runtime.policy_doctor().await?)?,
        },
        Command::Projection(command) => match command.command {
            ProjectionAction::Status => print_json(&runtime.projection_status()?)?,
            ProjectionAction::Drain => print_json(&runtime.drain_projections()?)?,
            ProjectionAction::Rebuild { name } => {
                print_json(&runtime.rebuild_projection(name.as_deref())?)?;
            }
        },
        Command::State(command) => match command.command {
            StateAction::Doctor => print_json(&runtime.state_doctor()?)?,
        },
        Command::Sandbox(command) => match command.command {
            SandboxAction::Doctor => print_json(&runtime.sandbox_doctor())?,
        },
        Command::Process(command) => match command.command {
            ProcessAction::Run {
                executable,
                cwd,
                environment,
                args,
            } => print_json(
                &runtime
                    .run_process(executable, cwd, args, parse_environment(environment)?)
                    .await?,
            )?,
        },
        Command::Network(command) => match command.command {
            NetworkAction::Get { url } => {
                let result = runtime.http_get(&url).await?;
                println!("{}", String::from_utf8_lossy(&result.bytes));
            }
        },
        Command::Workflow(command) => workflow_command(&runtime, command.command).await?,
        Command::Provider(command) => match command.command {
            ProviderAction::Profiles => print_json(&runtime.provider_profiles())?,
            ProviderAction::Doctor { profile } => {
                print_json(&runtime.provider_doctor(profile.as_deref()).await?)?;
            }
            ProviderAction::Models { profile } => {
                print_json(&runtime.provider_models(profile.as_deref()).await?)?;
            }
        },
        Command::Models(command) => match command.command {
            ModelsAction::Routes => print_json(&runtime.provider_routes())?,
        },
        Command::Run {
            prompt,
            role,
            instructions,
        } => print_json(&runtime.run_model(&role, &instructions, &prompt).await?)?,
        Command::Echo { message } => {
            let result = runtime.echo(&message).await?;
            println!("{}", String::from_utf8_lossy(&result.bytes));
        }
        Command::Repl => repl(&runtime).await?,
        Command::Worker { once } => {
            let recovered = runtime.workflows().recover_interrupted()?;
            let drained = runtime.workflows().drain().await?;
            let projections = runtime.drain_projections()?;
            print_json(&json!({
                "once": once,
                "recovered": recovered,
                "projections": projections,
                "drained": drained,
            }))?;
        }
        Command::SandboxHelper => unreachable!("handled before runtime construction"),
    }
    runtime.checkpoint()?;
    Ok(())
}
