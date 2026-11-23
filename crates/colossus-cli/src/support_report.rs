//! Local terminal support reports, built only from known host facts.

use clap::{Args, Subcommand};
use colossus_ports::RunControl;
use colossus_worker::WorkerError;
use serde::Serialize;
use std::{
    fmt,
    io::Write as _,
    path::{Path, PathBuf},
};

mod destination;
use destination::Destination;

const MAX_REPORT_BYTES: usize = 64 * 1024;

#[derive(Args)]
pub(super) struct SupportCommand {
    #[command(subcommand)]
    action: SupportAction,
}

#[derive(Subcommand)]
enum SupportAction {
    /// Save a bounded JSON support report without starting a runtime or contacting a worker.
    Report {
        /// Destination file. Relative paths use the invocation directory.
        #[arg(long, value_name = "PATH")]
        output: PathBuf,
        /// Allow replacing an existing regular file.
        #[arg(long)]
        replace: bool,
    },
}

pub(super) async fn run(command: &SupportCommand) -> Result<(), SaveError> {
    let SupportAction::Report { output, replace } = &command.action;
    let control = RunControl::default();
    let writing = save_async(
        HostFacts::Unresolved,
        output.clone(),
        *replace,
        control.clone(),
    );
    tokio::pin!(writing);
    tokio::select! {
        result = &mut writing => result,
        signal = tokio::signal::ctrl_c() => {
            if signal.is_ok() {
                control.cancel();
            }
            writing.await
        }
    }?;
    println!("Support report saved.");
    Ok(())
}

pub(super) fn redirect_line_mode(input: &str) -> bool {
    if input.split_whitespace().next() != Some("/support-report") {
        return false;
    }
    eprintln!("Use colossus support report --output PATH when terminal input is redirected.");
    true
}

/// Reports contain only build metadata, the opening mode, and closed status categories.
#[derive(Clone, Copy)]
pub(super) enum HostFacts {
    Unresolved,
    Embedded,
    Worker(WorkerObservation),
}

#[derive(Clone, Copy)]
pub(super) enum WorkerObservation {
    Unobserved,
    Available,
    Unavailable(FailureReason),
}

impl WorkerObservation {
    // Observe existing authorized requests. The report writer never makes a request.
    pub(super) fn record<T>(&mut self, outcome: &Result<T, WorkerError>) {
        let reason = match outcome {
            Ok(_) | Err(WorkerError::Remote(_)) => {
                *self = Self::Available;
                return;
            }
            Err(WorkerError::Unavailable(_)) => FailureReason::ConnectionUnavailable,
            Err(WorkerError::Io(_)) => FailureReason::TransportFailed,
            Err(WorkerError::Incompatible(_)) => FailureReason::ProtocolIncompatible,
            Err(WorkerError::Busy(_)) => FailureReason::WorkerBusy,
            Err(WorkerError::Protocol(_)) => FailureReason::ProtocolRejected,
            Err(_) => return,
        };
        *self = Self::Unavailable(reason);
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FailureReason {
    NotObserved,
    ConnectionUnavailable,
    TransportFailed,
    ProtocolIncompatible,
    WorkerBusy,
    ProtocolRejected,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum WorkerStatus {
    NotApplicable,
    LastObservedAvailable,
    Unavailable { reason: FailureReason },
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum RuntimeMode {
    Unresolved,
    Embedded,
    Worker,
}

#[derive(Serialize)]
struct Platform {
    os: &'static str,
    architecture: &'static str,
}

#[derive(Serialize)]
struct SupportReport {
    schema_version: u8,
    colossus_version: &'static str,
    platform: Platform,
    runtime_mode: RuntimeMode,
    worker: WorkerStatus,
}

impl SupportReport {
    fn new(facts: HostFacts) -> Self {
        let unobserved = WorkerStatus::Unavailable {
            reason: FailureReason::NotObserved,
        };
        let (runtime_mode, worker) = match facts {
            HostFacts::Unresolved => (RuntimeMode::Unresolved, unobserved),
            HostFacts::Embedded => (RuntimeMode::Embedded, WorkerStatus::NotApplicable),
            HostFacts::Worker(observation) => (
                RuntimeMode::Worker,
                match observation {
                    WorkerObservation::Unobserved => unobserved,
                    WorkerObservation::Available => WorkerStatus::LastObservedAvailable,
                    WorkerObservation::Unavailable(reason) => WorkerStatus::Unavailable { reason },
                },
            ),
        };
        Self {
            schema_version: 1,
            colossus_version: env!("CARGO_PKG_VERSION"),
            platform: Platform {
                os: std::env::consts::OS,
                architecture: std::env::consts::ARCH,
            },
            runtime_mode,
            worker,
        }
    }

    fn encode(&self) -> Result<Vec<u8>, SaveError> {
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|_| SaveError::Save)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_REPORT_BYTES {
            return Err(SaveError::Save);
        }
        Ok(bytes)
    }
}

#[derive(Debug)]
pub(super) enum SaveError {
    Exists,
    Cancelled,
    Destination,
    Save,
}

impl fmt::Display for SaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Exists => {
                "The support report already exists. Choose a new file or use --replace."
            }
            Self::Cancelled => "Support report save cancelled.",
            Self::Destination => "The support report destination is unavailable.",
            Self::Save => "The support report could not be saved.",
        })
    }
}

impl std::error::Error for SaveError {}

pub(super) async fn save_interactive(
    facts: HostFacts,
    output: PathBuf,
    replace: bool,
    control: RunControl,
) -> Result<(), String> {
    save_async(facts, output, replace, control)
        .await
        .map_err(|error| error.to_string())
}

async fn save_async(
    facts: HostFacts,
    output: PathBuf,
    replace: bool,
    control: RunControl,
) -> Result<(), SaveError> {
    tokio::task::spawn_blocking(move || save(facts, &output, replace, &control))
        .await
        .map_err(|_| SaveError::Save)?
}

fn save(
    facts: HostFacts,
    output: &Path,
    replace: bool,
    control: &RunControl,
) -> Result<(), SaveError> {
    check_cancelled(control)?;
    let bytes = SupportReport::new(facts).encode()?;
    let destination = Destination::open(output)?;
    destination.check_leaf(replace)?;
    check_cancelled(control)?;
    let mut staged = destination.stage()?;
    let file = staged.file_mut();
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| SaveError::Save)?;
    staged.publish(replace, control)
}

fn check_cancelled(control: &RunControl) -> Result<(), SaveError> {
    if control.is_cancelled() {
        return Err(SaveError::Cancelled);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
