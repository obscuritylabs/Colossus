//! Reproducible comparison of Colossus journals on freshly created local databases.

use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use colossus_contracts::{StartupVerificationMode, StartupVerificationReport};
use colossus_journal_redb::{
    DisabledCheckpointSigner, Ed25519CheckpointSigner, PlaintextKeyProvider, RedbEventJournal,
    StaticKeyProvider,
};
use colossus_journal_sql::{SqlEventJournal, SqlJournalBackend};
use colossus_ports::{CheckpointSigner, EventJournal, KeyProvider, ProjectionStore, StoreError};
use serde_json::json;

mod workload;

use workload::{Corpus, Latencies, PAGE_SIZE, STREAMS};

type BenchResult<T> = Result<T, Box<dyn Error>>;

const DEFAULT_PAYLOAD_BYTES: u64 = 900_000_000;
const DEFAULT_BATCH_SIZE: usize = 128;
const SINGLE_COMMITS: usize = 256;
const REOPEN_TAIL: usize = 64;

trait Journal: EventJournal + ProjectionStore {
    fn startup_report(&self) -> Result<StartupVerificationReport, StoreError>;
}

impl Journal for RedbEventJournal {
    fn startup_report(&self) -> Result<StartupVerificationReport, StoreError> {
        self.startup_verification_report()
    }
}

impl Journal for SqlEventJournal {
    fn startup_report(&self) -> Result<StartupVerificationReport, StoreError> {
        Ok(self.startup_verification_report().clone())
    }
}

struct Config {
    backend: String,
    scratch: PathBuf,
    payload_bytes: u64,
    encrypted: bool,
    batch_size: usize,
}

impl Config {
    fn parse() -> BenchResult<Self> {
        let mut args = std::env::args_os().skip(1);
        let backend = next_string(&mut args)?.ok_or("missing backend")?;
        let scratch = args.next().ok_or("missing scratch directory")?.into();
        let payload_bytes = next_string(&mut args)?
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(DEFAULT_PAYLOAD_BYTES);
        let encrypted = match next_string(&mut args)?.as_deref() {
            None | Some("encrypted") => true,
            Some("plaintext") => false,
            _ => return Err("protection must be encrypted or plaintext".into()),
        };
        let batch_size = next_string(&mut args)?
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or(DEFAULT_BATCH_SIZE);
        if args.next().is_some()
            || !matches!(backend.as_str(), "redb" | "sqlite" | "turso")
            || payload_bytes < 1024
            || batch_size == 0
        {
            return Err("expected <redb|sqlite|turso> <scratch-dir> [payload-bytes >= 1024] [encrypted|plaintext] [positive-batch-size]".into());
        }
        Ok(Self {
            backend,
            scratch,
            payload_bytes,
            encrypted,
            batch_size,
        })
    }

    fn open(
        &self,
        path: &Path,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
    ) -> BenchResult<Box<dyn Journal>> {
        let mode = StartupVerificationMode::Incremental;
        let journal: Box<dyn Journal> = match self.backend.as_str() {
            "redb" => Box::new(RedbEventJournal::open_with_startup_verification(
                path, keys, signer, mode,
            )?),
            name => {
                let engine = if name == "sqlite" {
                    SqlJournalBackend::Sqlite
                } else {
                    SqlJournalBackend::Turso
                };
                Box::new(SqlEventJournal::open(path, engine, keys, signer, mode)?)
            }
        };
        if journal.is_recovery_mode() {
            return Err(format!("{} opened in recovery mode", self.backend).into());
        }
        Ok(journal)
    }
}

fn next_string(args: &mut impl Iterator<Item = std::ffi::OsString>) -> BenchResult<Option<String>> {
    args.next()
        .map(|arg| {
            arg.into_string()
                .map_err(|_| "argument must be UTF-8".into())
        })
        .transpose()
}

fn main() -> BenchResult<()> {
    let config = Config::parse()?;
    let directory = tempfile::Builder::new()
        .prefix("colossus-journal-")
        .tempdir_in(&config.scratch)?;
    let path = directory.path().join("journal.db");
    let keys: Arc<dyn KeyProvider> = if config.encrypted {
        Arc::new(StaticKeyProvider::new("benchmark", [7; 32]))
    } else {
        Arc::new(PlaintextKeyProvider)
    };
    let signer: Arc<dyn CheckpointSigner> = if config.encrypted {
        Arc::new(Ed25519CheckpointSigner::new("benchmark", [5; 32]))
    } else {
        Arc::new(DisabledCheckpointSigner)
    };
    let begin = Instant::now();
    let journal = config.open(&path, keys.clone(), signer.clone())?;
    let create_ms = milliseconds(begin);
    let mut corpus = Corpus::new(config.payload_bytes);

    eprintln!(
        "{}: appending {} canonical payload bytes",
        config.backend, config.payload_bytes
    );
    let ingest = workload::ingest(journal.as_ref(), &mut corpus, config.batch_size)?;
    let seed_events = corpus.event_count();
    if journal.head()?.0 != seed_events {
        return Err("ingest head does not match corpus".into());
    }

    let mut singles = Latencies::default();
    for _ in 0..SINGLE_COMMITS {
        let event = corpus.followup();
        let begin = Instant::now();
        journal.append(event)?;
        singles.record(begin);
    }
    let begin = Instant::now();
    let checkpoint = journal.checkpoint()?;
    let checkpoint_ms = milliseconds(begin);
    if checkpoint.is_some() != config.encrypted {
        return Err("checkpoint does not match journal protection".into());
    }
    let tail = (0..REOPEN_TAIL).map(|_| corpus.followup()).collect();
    journal.append_batch(tail)?;
    let head = journal.head()?;
    if head.0 != corpus.event_count() {
        return Err("journal head differs from appended event count".into());
    }

    eprintln!(
        "{}: measuring close, reopen, reads, projection replay and full audit",
        config.backend
    );
    let live_bytes = storage_bytes(directory.path())?;
    let begin = Instant::now();
    drop(journal);
    let close_ms = milliseconds(begin);
    let closed_bytes = storage_bytes(directory.path())?;
    let begin = Instant::now();
    let journal = config.open(&path, keys, signer)?;
    let reopen_ms = milliseconds(begin);
    let startup = journal.startup_report()?;
    if journal.head()? != head {
        return Err("reopened journal head differs from durable result".into());
    }

    let indexed_reads = workload::indexed_reads(journal.as_ref(), seed_events)?;
    let begin = Instant::now();
    workload::replay(journal.as_ref(), head.0)?;
    let global_replay_ms = milliseconds(begin);
    let (outbox_replay_ms, projection_apply_ms) = workload::project(journal.as_ref(), head.0)?;
    let begin = Instant::now();
    let report = journal.verify()?;
    let audit_ms = milliseconds(begin);
    if (report.last_sequence, report.last_hash) != head || report.event_count != head.0 {
        return Err("full audit does not match announced durable head".into());
    }

    let metrics = json!({
        "backend": config.backend,
        "protection": if config.encrypted { "encrypted" } else { "plaintext" },
        "payload_bytes": config.payload_bytes,
        "seed_events": seed_events,
        "total_events": head.0,
        "streams": STREAMS,
        "batch_size": config.batch_size,
        "read_page_size": PAGE_SIZE,
        "create_ms": create_ms,
        "ingest": ingest,
        "ingest_mb_s": config.payload_bytes as f64 / (ingest.total_ms * 1000.0),
        "single_append": singles.finish(),
        "checkpoint_ms": checkpoint_ms,
        "close_ms": close_ms,
        "reopen_ms": reopen_ms,
        "startup": startup,
        "indexed_reads": indexed_reads,
        "global_replay_ms": global_replay_ms,
        "outbox_replay_ms": outbox_replay_ms,
        "projection_apply_ms": projection_apply_ms,
        "full_audit_ms": audit_ms,
        "live_file_bytes": live_bytes,
        "closed_file_bytes": closed_bytes,
    });
    drop(journal);
    directory.close()?;
    println!("{}", serde_json::to_string(&metrics)?);
    Ok(())
}

fn milliseconds(begin: Instant) -> f64 {
    begin.elapsed().as_secs_f64() * 1000.0
}

fn storage_bytes(path: &Path) -> std::io::Result<u64> {
    fs::read_dir(path)?.try_fold(0, |total, entry| Ok(total + entry?.metadata()?.len()))
}
