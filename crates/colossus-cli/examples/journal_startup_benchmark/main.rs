//! Time a real embedded TUI process against a checkpointed journal of saved conversations.

mod fixture;
#[path = "../../tests/support/terminal.rs"]
#[allow(dead_code)]
mod terminal;

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use clap::{Parser, Subcommand};
use portable_pty::CommandBuilder;
use serde_json::json;

use crate::{fixture::Fixture, terminal::Terminal};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Seed isolated journals and time fresh launches of the benchmark-enabled Colossus binary.
    Measure {
        binary: PathBuf,
        scratch: PathBuf,
        #[arg(long, default_value_t = 900_000_000)]
        bytes: usize,
        #[arg(long, default_value_t = 1024)]
        sessions: usize,
        #[arg(long, default_value_t = 9, value_parser = clap::value_parser!(u32).range(1..))]
        trials: u32,
    },
    #[command(hide = true)]
    Seed { bytes: usize, sessions: usize },
}

#[derive(Clone, Copy)]
enum Launch {
    Fresh,
    FreshWithoutAnimation,
    ResumeLatest,
}

impl Launch {
    const ALL: [Self; 3] = [Self::Fresh, Self::FreshWithoutAnimation, Self::ResumeLatest];

    fn name(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::FreshWithoutAnimation => "fresh_without_animation",
            Self::ResumeLatest => "resume_latest",
        }
    }

    fn run(self, binary: &Path, fixture: &Fixture) -> Result<f64> {
        let mut command = CommandBuilder::new(binary);
        command.cwd(&fixture.workspace);
        for (name, value) in &fixture.environment {
            command.env(name, value);
        }
        command.env_remove("NO_COLOR");
        command.env(
            "COLOSSUS_TUI_ANIMATIONS",
            if matches!(self, Self::FreshWithoutAnimation) {
                "off"
            } else {
                "on"
            },
        );
        command.args(["--config", fixture::CONFIG_NAME, "--alt-screen", "tui"]);
        let resume = matches!(self, Self::ResumeLatest);
        if resume {
            command.arg("--resume");
        }
        let (mut terminal, started) = Terminal::start_timed(command);
        let description = format!("{} {} prompt", fixture.engine.name(), self.name());
        // Wait for the actual composer title and, for a resumed session, its saved last message.
        let (_, ready_at) =
            terminal.wait_until_observed(&description, Duration::from_secs(180), |screen| {
                screen.contains("Enter sends")
                    && (!resume || screen.contains(fixture::RESTORED_MESSAGE))
            });
        let draft = "startup benchmark input";
        terminal.send(format!("\x1b[200~{draft}\x1b[201~").as_bytes());
        terminal.wait_for_composer("benchmark draft accepted", |text| text == draft);
        // Clear the draft with Ctrl+C, then use the empty composer's Ctrl+D exit.
        terminal.send(b"\x03");
        terminal.wait_for_composer("benchmark draft cleared", |text| !text.contains(draft));
        terminal.send(b"\x04");
        terminal
            .wait_for_clean_exit()
            .map_err(|error| format!("{description}: {error}"))?;
        Ok(ready_at.duration_since(started).as_secs_f64() * 1_000.0)
    }
}

fn main() -> Result<()> {
    match Args::parse().command {
        Command::Measure {
            binary,
            scratch,
            bytes,
            sessions,
            trials,
        } => measure(&binary, &scratch, bytes, sessions, trials),
        Command::Seed { bytes, sessions } => {
            // Match the stack headroom of the native CLI startup on Windows.
            std::thread::Builder::new()
                .stack_size(8 * 1024 * 1024)
                .spawn(move || fixture::seed(bytes, sessions).map_err(|error| error.to_string()))?
                .join()
                .map_err(|_| "journal seeder panicked")?
                .map_err(Into::into)
        }
    }
}

fn measure(
    binary: &Path,
    scratch: &Path,
    bytes: usize,
    sessions: usize,
    trials: u32,
) -> Result<()> {
    fixture::validate_corpus_size(bytes, sessions)?;
    let binary = fs::canonicalize(binary)?;
    let scratch = fs::canonicalize(scratch)?;
    let directory = tempfile::Builder::new()
        .prefix("colossus-journal-startup-")
        .tempdir_in(scratch)?;
    let root = fs::canonicalize(directory.path())?;
    let mut fixtures = Vec::new();
    for engine in fixture::Engine::ALL {
        eprintln!("Seeding {}", engine.name());
        fixtures.push(Fixture::new(&root, engine, &binary, bytes, sessions)?);
    }
    for trial in 0..=trials {
        for (offset, _) in fixtures.iter().enumerate() {
            let fixture = &fixtures[(trial as usize + offset) % fixtures.len()];
            for launch in Launch::ALL {
                let elapsed = launch.run(&binary, fixture).or_else(|error| {
                    eprintln!("{error}. Repeating this launch.");
                    launch.run(&binary, fixture)
                })?;
                println!(
                    "{}",
                    json!({
                        "record": "sample",
                        "engine": fixture.engine.name(),
                        "trial": trial,
                        "warmup": trial == 0,
                        "launch": launch.name(),
                        "launch_to_prompt_ms": elapsed,
                    })
                );
            }
        }
    }
    drop(fixtures);
    directory.close()?;
    Ok(())
}
