//! Explicit offline migration. Verified canonical source events remain unchanged.
use crate::config::Config;
use colossus_cloud::{CloudError, CloudRepository, storage::*};
use colossus_cloud_postgres::CloudPostgresStore;
use colossus_home::ConfinedRoot;
use colossus_journal_postgres::{PostgresEventJournal, PostgresJournalConfig};
use colossus_journal_redb::{
    DisabledCheckpointSigner, Ed25519CheckpointSigner, EnvironmentKeyProvider,
    PlaintextKeyProvider, RedbEventJournal,
};
use colossus_ports::{CheckpointSigner, EventJournal, KeyProvider};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(test)]
mod tests;

const PAGE: usize = 16;
const PREFIXES: [&str; 9] = [
    "cloud.node:",
    "cloud.task:",
    "cloud.node-task:",
    "cloud.run:",
    "cloud.command:",
    "cloud.invitation:",
    "cloud.renewal:",
    "cloud.admission:",
    "cloud.output:",
];

/// Credential-free outcome of one verified, resumable offline import.
#[derive(Clone, Debug)]
pub struct MigrationReport {
    /// Verified source journal head retained by the import marker.
    pub source_sequence: u64,
    /// Original domain events imported while preserving aggregate revisions.
    pub entities: u64,
    /// Released run updates retained with their original exclusive cursors.
    pub released_events: u64,
    /// Projects imported, with unchanged namespaces.
    pub projects: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum LegacyStorage {
    Redb {
        path: PathBuf,
        key_variable: Option<String>,
    },
    Postgres {
        config: PostgresJournalConfig,
        key_variable: String,
        anchor_path: PathBuf,
    },
}
struct LegacyConfig {
    storage: LegacyStorage,
    signing: Option<String>,
    local: bool,
}

fn legacy_config(bytes: &[u8]) -> Result<LegacyConfig, &'static str> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| "legacy cloud configuration is invalid")?;
    let object = value
        .as_object()
        .ok_or("legacy cloud configuration is invalid")?;
    const FIELDS: [&str; 16] = [
        "http_bind",
        "grpc_bind",
        "public_origin",
        "grpc_endpoint",
        "ca_certificate",
        "ca_key",
        "server_certificate",
        "server_key",
        "oidc",
        "memberships",
        "web_root",
        "storage",
        "signing_key_variable",
        "local_development",
        "schema_version",
        "auth_key_variable",
    ];
    if object.keys().any(|key| !FIELDS.contains(&key.as_str())) {
        return Err("legacy cloud configuration has unsupported fields");
    }
    let storage: LegacyStorage = serde_json::from_value(
        object
            .get("storage")
            .cloned()
            .ok_or("legacy storage configuration is missing")?,
    )
    .map_err(|_| "legacy storage configuration is invalid")?;
    let signing: Option<String> = serde_json::from_value(
        object
            .get("signing_key_variable")
            .cloned()
            .unwrap_or(Value::Null),
    )
    .map_err(|_| "legacy signing reference is invalid")?;
    let local = object
        .get("local_development")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let encryption = match &storage {
        LegacyStorage::Redb { key_variable, .. } => key_variable.as_ref(),
        LegacyStorage::Postgres { key_variable, .. } => Some(key_variable),
    };
    if !local && (encryption.is_none() || signing.is_none()) {
        return Err("legacy production state requires its encryption and signing references");
    }
    if signing.as_ref().is_some_and(|key| Some(key) == encryption) {
        return Err("legacy encryption and signing references must be independent");
    }
    Ok(LegacyConfig {
        storage,
        signing,
        local,
    })
}

fn source(config: LegacyConfig) -> Result<Arc<dyn EventJournal>, &'static str> {
    let signer: Arc<dyn CheckpointSigner> = match config.signing {
        Some(variable) => {
            let value = zeroize::Zeroizing::new(
                std::env::var(variable).map_err(|_| "legacy signing key is unavailable")?,
            );
            let mut secret = zeroize::Zeroizing::new([0u8; 32]);
            hex::decode_to_slice(value.trim(), secret.as_mut())
                .map_err(|_| "legacy signing key is invalid")?;
            Arc::new(Ed25519CheckpointSigner::new("cloud-signing-v1", *secret))
        }
        None => Arc::new(DisabledCheckpointSigner),
    };
    let journal: Arc<dyn EventJournal> = match config.storage {
        LegacyStorage::Redb { path, key_variable } => {
            if !std::fs::symlink_metadata(&path)
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            {
                return Err("legacy journal must be an existing protected regular file");
            }
            let root = ConfinedRoot::bind(path.parent().ok_or("legacy journal path is invalid")?)
                .map_err(|_| "legacy journal parent is unavailable")?;
            let retained = root
                .open_existing_file_read_write(
                    path.file_name()
                        .ok_or("legacy journal path is invalid")?
                        .as_ref(),
                )
                .map_err(|_| "legacy journal file is unavailable")?;
            retained
                .revalidate(&root)
                .map_err(|_| "legacy journal identity changed")?;
            let keys: Arc<dyn KeyProvider> = match key_variable {
                Some(variable) => Arc::new(EnvironmentKeyProvider::new(
                    variable,
                    "cloud-v1",
                    path.with_extension("anchor"),
                )),
                None if config.local => Arc::new(PlaintextKeyProvider),
                None => return Err("legacy plaintext journal requires local development"),
            };
            Arc::new(
                RedbEventJournal::open_file_with_startup_verification(
                    retained
                        .file()
                        .try_clone()
                        .map_err(|_| "legacy journal file is unavailable")?,
                    keys,
                    signer,
                    colossus_contracts::StartupVerificationMode::Full,
                )
                .map_err(|_| "legacy journal is unavailable; stop its writer before migrating")?,
            )
        }
        LegacyStorage::Postgres {
            config,
            key_variable,
            anchor_path,
        } => Arc::new(
            PostgresEventJournal::open(
                config,
                Arc::new(EnvironmentKeyProvider::new(
                    key_variable,
                    "cloud-v1",
                    anchor_path,
                )),
                signer,
            )
            .map_err(|_| "legacy journal is unavailable; stop its writer before migrating")?,
        ),
    };
    if journal.is_recovery_mode() {
        return Err("legacy journal is in read-only recovery mode");
    }
    journal
        .verify()
        .map_err(|_| "legacy journal verification failed")?;
    Ok(journal)
}

fn marker() -> EntityKey {
    EntityKey {
        kind: EntityKind::AuthFlow,
        project_id: "__migration".into(),
        parent_id: None,
        id: "journal-import".into(),
    }
}
fn identity(stream: &str, value: &Value) -> Result<EntityKey, &'static str> {
    let (kind, tail) = stream
        .split_once(':')
        .ok_or("legacy entity identity is invalid")?;
    let kind = match kind {
        "cloud.node" => EntityKind::Node,
        "cloud.task" => EntityKind::Task,
        "cloud.node-task" => EntityKind::NodeTask,
        "cloud.run" => EntityKind::Run,
        "cloud.command" => EntityKind::Command,
        "cloud.invitation" => EntityKind::Invitation,
        "cloud.renewal" => EntityKind::Renewal,
        "cloud.admission" => EntityKind::Admission,
        _ => return Err("legacy journal contains unsupported state"),
    };
    let (project, rest) = if kind == EntityKind::Invitation {
        (
            value
                .get("project_id")
                .and_then(Value::as_str)
                .ok_or("legacy invitation project is invalid")?,
            tail,
        )
    } else {
        tail.split_once(':')
            .ok_or("legacy project identity is invalid")?
    };
    let (parent, id) = if matches!(
        kind,
        EntityKind::NodeTask | EntityKind::Run | EntityKind::Command
    ) {
        let (parent, id) = rest
            .split_once(':')
            .ok_or("legacy parent identity is invalid")?;
        (Some(parent.to_owned()), id)
    } else {
        (None, rest)
    };
    let valid = |id: &str| {
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    };
    if !valid(project) || !valid(id) || parent.as_ref().is_some_and(|id| !valid(id)) {
        return Err("legacy entity identity is invalid");
    }
    Ok(EntityKey {
        kind,
        project_id: project.into(),
        parent_id: parent,
        id: id.into(),
    })
}

async fn page(
    journal: Arc<dyn EventJournal>,
    stream: String,
    after: u64,
) -> Result<Vec<(colossus_contracts::EventEnvelope, Value)>, &'static str> {
    tokio::task::spawn_blocking(move || {
        journal
            .read_stream_from(&stream, after, PAGE)
            .map_err(|_| "legacy journal read failed")?
            .into_iter()
            .map(|event| {
                let value = journal
                    .decrypt_payload(&event)
                    .map_err(|_| "legacy journal payload verification failed")?;
                Ok((event, value))
            })
            .collect()
    })
    .await
    .map_err(|_| "legacy journal reader failed")?
}

async fn streams(
    journal: Arc<dyn EventJournal>,
    prefix: &str,
    after: Option<String>,
) -> Result<Vec<String>, &'static str> {
    let prefix = prefix.to_owned();
    tokio::task::spawn_blocking(move || {
        journal
            .list_stream_ids(&prefix, after.as_deref(), 64)
            .map_err(|_| "legacy journal discovery failed")
    })
    .await
    .map_err(|_| "legacy journal reader failed")?
}

async fn projects(
    journal: Arc<dyn EventJournal>,
    head: (u64, String),
) -> Result<BTreeSet<String>, &'static str> {
    let mut projects = BTreeSet::new();
    let mut after = 1;
    loop {
        let source = journal.clone();
        let events = tokio::task::spawn_blocking(move || {
            source
                .read_global(after, PAGE)
                .map_err(|_| "legacy journal read failed")
        })
        .await
        .map_err(|_| "legacy journal reader failed")??;
        if events.is_empty() {
            break;
        }
        for event in events {
            if event.global_sequence != after
                || event.event_version != 1
                || !event.event_type.starts_with("cloud.")
                || !PREFIXES
                    .iter()
                    .any(|prefix| event.stream_id.starts_with(prefix))
            {
                return Err("legacy journal contains unsupported or inconsistent state");
            }
            let value = journal
                .decrypt_payload(&event)
                .map_err(|_| "legacy journal payload verification failed")?;
            let project = if let Some(output) = event.stream_id.strip_prefix("cloud.output:") {
                output
                    .split_once(':')
                    .ok_or("legacy output identity is invalid")?
                    .0
                    .to_owned()
            } else {
                identity(&event.stream_id, &value)?.project_id
            };
            projects.insert(project);
            if projects.len() > 4096 {
                return Err("legacy project count exceeds migration bound");
            }
            after = after
                .checked_add(1)
                .ok_or("legacy sequence exceeds migration bound")?;
        }
    }
    if after.saturating_sub(1) != head.0
        || journal
            .head()
            .map_err(|_| "legacy journal head is unavailable")?
            != head
    {
        return Err("legacy journal changed during migration");
    }
    Ok(projects)
}

async fn replay(
    journal: Arc<dyn EventJournal>,
    store: Arc<dyn CloudStore>,
    head: (u64, String),
) -> Result<MigrationReport, &'static str> {
    let project_ids = projects(journal.clone(), head.clone()).await?;
    for project in &project_ids {
        let key = EntityKey {
            kind: EntityKind::Project,
            project_id: project.clone(),
            parent_id: None,
            id: project.clone(),
        };
        if matches!(store.read(&key).await, Err(CloudError::NotFound)) {
            store
                .commit(CloudTransaction {
                    entities: vec![EntityMutation {
                        key,
                        expected_revision: 0,
                        value: json!({"project_id":project,"label":project,"revision":1}),
                        actor: "journal-import".into(),
                        operation: "cloud.project.imported.v2".into(),
                    }],
                    ..Default::default()
                })
                .await
                .map_err(|_| "migration project initialization failed")?;
        }
    }
    let mut report = MigrationReport {
        source_sequence: head.0,
        entities: 0,
        released_events: 0,
        projects: project_ids.into_iter().collect(),
    };
    for prefix in PREFIXES {
        let mut after = None;
        loop {
            let listed = streams(journal.clone(), prefix, after.clone()).await?;
            if listed.is_empty() {
                break;
            }
            for stream in &listed {
                let mut version = 0;
                loop {
                    let events = page(journal.clone(), stream.clone(), version).await?;
                    if events.is_empty() {
                        break;
                    }
                    for (event, value) in events {
                        if event.stream_version != version + 1 {
                            return Err("legacy entity revision is inconsistent");
                        }
                        if let Some(output) = stream.strip_prefix("cloud.output:") {
                            let (project, scope) = output
                                .split_once(':')
                                .ok_or("legacy output identity is invalid")?;
                            let update: colossus_sdk::RunUpdate =
                                serde_json::from_value(value.clone())
                                    .map_err(|_| "legacy released event is invalid")?;
                            if update.sequence != event.stream_version {
                                return Err("legacy released event cursor is inconsistent");
                            }
                            store
                                .commit(CloudTransaction {
                                    events: vec![ReleasedEvent {
                                        project_id: project.into(),
                                        scope_id: scope.into(),
                                        sequence: event.stream_version,
                                        value,
                                    }],
                                    ..Default::default()
                                })
                                .await
                                .map_err(|_| "legacy released event import failed")?;
                            report.released_events += 1;
                        } else {
                            let key = identity(stream, &value)?;
                            let current = store.read(&key).await;
                            let needed = match current {
                                Ok(record) if record.revision > event.stream_version => false,
                                Ok(record)
                                    if record.revision == event.stream_version
                                        && record.value == value =>
                                {
                                    false
                                }
                                Ok(record) if record.revision == version => true,
                                Err(CloudError::NotFound) if version == 0 => true,
                                _ => {
                                    return Err(
                                        "legacy entity target revision conflicts with source",
                                    );
                                }
                            };
                            if needed {
                                store
                                    .commit(CloudTransaction {
                                        entities: vec![EntityMutation {
                                            key,
                                            expected_revision: version,
                                            value,
                                            actor: event.actor.id,
                                            operation: event.event_type,
                                        }],
                                        ..Default::default()
                                    })
                                    .await
                                    .map_err(|_| "legacy entity import failed")?;
                            }
                            report.entities += 1;
                        }
                        version = event.stream_version;
                    }
                }
            }
            after = listed.last().cloned();
        }
    }
    if journal
        .head()
        .map_err(|_| "legacy journal head is unavailable")?
        != head
    {
        return Err("legacy journal changed during migration");
    }
    Ok(report)
}

/// Import a verified, offline legacy cloud journal into an empty or matching resumable database.
/// Stop both source and destination controllers first. Failed imports retain a running
/// marker that prevents normal server startup; retry uses the same verified source head.
pub async fn migrate_journal(
    legacy_config_path: &Path,
    config: &Config,
) -> Result<MigrationReport, &'static str> {
    config.validate()?;
    let mut bytes = Vec::new();
    std::fs::File::open(legacy_config_path)
        .map_err(|_| "legacy cloud configuration is unavailable")?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "legacy cloud configuration is unavailable")?;
    if bytes.len() > 65536 {
        return Err("legacy cloud configuration exceeds its bound");
    }
    let legacy = legacy_config(&bytes)?;
    if matches!(&legacy.storage, LegacyStorage::Postgres{config:source,..} if source.schema == config.database.schema)
    {
        return Err("legacy and destination PostgreSQL schemas must be distinct");
    }
    let journal = tokio::task::spawn_blocking(move || source(legacy))
        .await
        .map_err(|_| "legacy journal initialization failed")??;
    let head = journal
        .head()
        .map_err(|_| "legacy journal head is unavailable")?;
    projects(journal.clone(), head.clone()).await?;
    let database = CloudPostgresStore::open(
        config.database.clone(),
        &colossus_network::AdditionalRootCertificates::default(),
    )
    .await
    .map_err(|_| "migration database is unavailable")?;
    let prior = database
        .begin_journal_import(&head.1, head.0)
        .await
        .map_err(|_| "migration target must be empty or match this exact source head")?;
    if prior.value.get("status").and_then(Value::as_str) == Some("complete") {
        return Ok(MigrationReport {
            source_sequence: head.0,
            entities: prior
                .value
                .get("entities")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            released_events: prior
                .value
                .get("released_events")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            projects: serde_json::from_value(
                prior.value.get("projects").cloned().unwrap_or(json!([])),
            )
            .map_err(|_| "migration completion marker is invalid")?,
        });
    }
    let store: Arc<dyn CloudStore> = Arc::new(database);
    let report = replay(journal, store.clone(), head.clone()).await?;
    let repo =
        CloudRepository::new(store.clone()).map_err(|_| "migration repository is unavailable")?;
    for project in &report.projects {
        repo.bootstrap_migrated_project(project)
            .await
            .map_err(|_| "legacy conversation projection import failed")?;
    }
    let current = store
        .read(&marker())
        .await
        .map_err(|_| "migration marker is unavailable")?;
    store.commit(CloudTransaction{entities:vec![EntityMutation{key:marker(),expected_revision:current.revision,value:json!({"source_head_hash":head.1,"source_sequence":head.0,"status":"complete","entities":report.entities,"released_events":report.released_events,"projects":report.projects}),actor:"journal-import".into(),operation:"cloud.journal-import.completed.v2".into()}],..Default::default()}).await.map_err(|_|"migration completion could not be committed")?;
    Ok(report)
}
