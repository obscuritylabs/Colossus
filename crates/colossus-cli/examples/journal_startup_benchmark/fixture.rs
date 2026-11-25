use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use colossus_contracts::{Actor, ActorType, ModelMessage, ModelMessageRole, SessionMessageAppend};
use colossus_home::ConfinedRoot;
use colossus_policy::DenyApproval;
use colossus_runtime::{Runtime, RuntimeConfig, RuntimeOpenOptions, StorageAdapter};
use serde_json::{Value, json};

use crate::Result;

pub(super) const CONFIG_NAME: &str = "benchmark.yaml";
pub(super) const RESTORED_MESSAGE: &str = "Benchmark-restored-session";
const JOURNAL_NAME: &str = "journal.db";
const MESSAGE_BYTES: usize = 8_192;
const SEED_TEXT: &str = "The workspace notes describe the scheduled review and the follow-up \
    questions. The next draft incorporates the evidence, records the decision, and checks the \
    updated examples against the saved conversation. ";

#[derive(Clone, Copy)]
pub(super) enum Engine {
    Redb,
    Sqlite,
    Turso,
}

impl Engine {
    pub(super) const ALL: [Self; 3] = [Self::Redb, Self::Sqlite, Self::Turso];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Redb => "redb",
            Self::Sqlite => "sqlite",
            Self::Turso => "turso",
        }
    }

    fn storage_adapter(self) -> StorageAdapter {
        match self {
            Self::Redb => StorageAdapter::Redb,
            Self::Sqlite => StorageAdapter::BenchmarkSqlite,
            Self::Turso => StorageAdapter::BenchmarkTurso,
        }
    }
}

pub(super) struct Fixture {
    pub(super) engine: Engine,
    pub(super) workspace: PathBuf,
    pub(super) environment: BTreeMap<&'static str, OsString>,
}

impl Fixture {
    pub(super) fn new(
        root: &Path,
        engine: Engine,
        binary: &Path,
        bytes: usize,
        sessions: usize,
    ) -> Result<Self> {
        let workspace = ConfinedRoot::bind(root.join(engine.name()))?;
        let home = workspace.prepare_directory(Path::new("home"))?;
        let app_data = workspace.prepare_directory(Path::new("home/AppData/Local/Packages"))?;
        let temporary = workspace.prepare_directory(Path::new("home/tmp"))?;
        let mut config = RuntimeConfig::offline_template(JOURNAL_NAME);
        config.storage.adapter = engine.storage_adapter();
        config.use_environment_storage(workspace.path().join("anchor.json"));
        fs::write(
            workspace.path().join(CONFIG_NAME),
            config.to_resolved_yaml()?,
        )?;

        let environment = BTreeMap::from([
            ("HOME", home.as_os_str().to_owned()),
            ("USERPROFILE", home.as_os_str().to_owned()),
            (
                "COLOSSUS_HOME",
                home.join(".colossus-home").into_os_string(),
            ),
            (
                "LOCALAPPDATA",
                app_data
                    .parent()
                    .ok_or("missing app data directory")?
                    .as_os_str()
                    .to_owned(),
            ),
            ("XDG_DATA_HOME", home.join(".data").into_os_string()),
            ("XDG_CACHE_HOME", home.join(".cache").into_os_string()),
            ("TEMP", temporary.as_os_str().to_owned()),
            ("TMP", temporary.as_os_str().to_owned()),
            ("TMPDIR", temporary.into_os_string()),
            ("COLOSSUS_JOURNAL_STARTUP_BENCHMARK", "1".into()),
            ("COLOSSUS_JOURNAL_KEY", fixture_key()?),
            ("COLOSSUS_SIGNING_KEY", fixture_key()?),
        ]);
        let fixture = Self {
            engine,
            workspace: workspace.path().to_owned(),
            environment,
        };
        let mut seeder = Command::new(env::current_exe()?);
        seeder.args(["seed", &bytes.to_string(), &sessions.to_string()]);
        let seeded = fixture.json_output(seeder)?;
        let mut doctor = Command::new(binary);
        doctor.args([
            "--config",
            CONFIG_NAME,
            "--output",
            "json",
            "state",
            "doctor",
        ]);
        let status = fixture.json_output(doctor)?;
        let verification = &status["storage"]["startup_verification"];
        if status["storage"]["adapter"] != engine.name()
            || status["storage"]["payload_protection"] != "encrypted"
            || status["recovery_mode"] != false
            || status["journal_head"] != seeded["journal_head"]
            || verification["path"] != "incremental"
            || verification["verified_through_sequence"] != seeded["journal_head"]
        {
            return Err(format!(
                "{} did not reopen at its protected checkpoint",
                engine.name()
            )
            .into());
        }
        println!(
            "{}",
            json!({
                "record": "dataset",
                "engine": engine.name(),
                "corpus": seeded,
                "closed_journal_bytes": fs::metadata(fixture.workspace.join(JOURNAL_NAME))?.len(),
                "startup_verification": verification,
            })
        );
        Ok(fixture)
    }

    fn json_output(&self, mut command: Command) -> Result<Value> {
        let output = command
            .current_dir(&self.workspace)
            .envs(&self.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "{} fixture process ended with {}",
                self.engine.name(),
                output.status
            )
            .into());
        }
        Ok(serde_json::from_slice(&output.stdout)?)
    }
}

pub(super) fn validate_corpus_size(bytes: usize, sessions: usize) -> Result<()> {
    if sessions == 0 || bytes / sessions < 1024 {
        return Err("provide at least one session and 1,024 bytes per session".into());
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
pub(super) async fn seed(bytes: usize, sessions: usize) -> Result<()> {
    validate_corpus_size(bytes, sessions)?;
    let config = RuntimeConfig::from_path(CONFIG_NAME)?;
    let home = colossus_home::ColossusHome::resolve_and_ensure()?;
    let options =
        RuntimeOpenOptions::for_workspace(env::current_dir()?)?.with_colossus_home(home.root())?;
    let runtime =
        Runtime::open_with_options(&config, std::sync::Arc::new(DenyApproval), None, options)?;
    let repository = runtime.session_repository();
    let source = SEED_TEXT.repeat((2 * MESSAGE_BYTES).div_ceil(SEED_TEXT.len()) + 1);
    let messages_per_session = (bytes / sessions / (2 * MESSAGE_BYTES)).max(1) * 2;
    for index in 0..sessions {
        let title = format!("Benchmark conversation {index:04}");
        let session = runtime.create_session(Some(&title))?;
        let budget = bytes / sessions + usize::from(index < bytes % sessions);
        let budget = budget - serde_json::to_vec(&json!({"title": title}))?.len();
        let run_id = format!("benchmark-run-{index:04}");
        let messages = (0..messages_per_session)
            .map(|message_index| {
                let marker = index + 1 == sessions && message_index + 1 == messages_per_session;
                let bytes = budget / messages_per_session
                    + usize::from(message_index < budget % messages_per_session);
                let offset = (index * 3 + message_index * 7) % SEED_TEXT.len();
                message(&run_id, message_index, bytes, &source[offset..], marker)
            })
            .collect::<Result<Vec<_>>>()?;
        repository.append_messages(&session.id, &run_id, messages)?;
        if (index + 1) % 128 == 0 {
            eprintln!("  Saved {} of {sessions} conversations", index + 1);
        }
    }
    eprintln!("  Catching up projections and signing the checkpoint");
    runtime.checkpoint()?;
    if runtime
        .projection_status()?
        .iter()
        .any(|status| !status.ready)
    {
        return Err("seeded projections are behind the checkpoint".into());
    }
    eprintln!("  Verifying the corpus size from the stored session events");
    let journal = runtime.journal();
    let (head, _) = journal.head()?;
    let mut from = 1;
    let mut stored_sessions = 0;
    let mut stored_messages = 0;
    let mut canonical_payload_bytes = 0;
    while from <= head {
        let page = journal.read_global(from, 256)?;
        let last = page.last().ok_or("missing page in seeded journal")?;
        for event in &page {
            if event.stream_id.starts_with("session:") {
                match event.event_type.as_str() {
                    "session.created.v1" => stored_sessions += 1,
                    "session.message.appended.v1" => stored_messages += 1,
                    other => return Err(format!("unexpected seeded session event: {other}").into()),
                }
                canonical_payload_bytes +=
                    serde_json::to_vec(&journal.decrypt_payload(event)?)?.len();
            }
        }
        from = last.global_sequence + 1;
    }
    if stored_sessions != sessions
        || stored_messages != sessions * messages_per_session
        || canonical_payload_bytes != bytes
    {
        return Err(format!(
            "seeded corpus mismatch: {stored_sessions} sessions, {stored_messages} messages, \
             {canonical_payload_bytes} canonical payload bytes"
        )
        .into());
    }
    println!(
        "{}",
        json!({
            "sessions": stored_sessions,
            "messages": stored_messages,
            "canonical_payload_bytes": canonical_payload_bytes,
            "journal_head": head,
        })
    );
    Ok(())
}

fn message(
    run_id: &str,
    index: usize,
    payload_bytes: usize,
    source: &str,
    mark_restored: bool,
) -> Result<SessionMessageAppend> {
    let user = index.is_multiple_of(2);
    let mut message = ModelMessage {
        agent_message_origin: None,
        role: if user {
            ModelMessageRole::User
        } else {
            ModelMessageRole::Assistant
        },
        content: "".into(),
        tool_call_id: None,
        tool_calls: Vec::new(),
    };
    let overhead = serde_json::to_vec(&json!({
        "run_id": run_id,
        "sequence": index + 1,
        "message": message,
    }))?
    .len();
    let suffix = if mark_restored {
        format!(" {RESTORED_MESSAGE}")
    } else {
        String::new()
    };
    let body_bytes = payload_bytes
        .checked_sub(overhead + suffix.len())
        .ok_or("message budget is too small for its JSON metadata")?;
    let mut body = source
        .get(..body_bytes)
        .ok_or("message budget exceeds seed text")?
        .to_owned();
    body.push_str(&suffix);
    message.content = body.into();
    Ok(SessionMessageAppend {
        message,
        actor: Actor {
            actor_type: if user {
                ActorType::User
            } else {
                ActorType::Model
            },
            id: if user {
                "benchmark-user"
            } else {
                "benchmark-model"
            }
            .into(),
        },
    })
}

fn fixture_key() -> Result<OsString> {
    let mut key = [0; 32];
    getrandom::fill(&mut key)?;
    Ok(hex::encode(key).into())
}
