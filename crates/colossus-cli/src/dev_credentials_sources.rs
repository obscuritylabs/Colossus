use super::{
    dev_credentials_args::DevelopmentCredentialSources,
    dev_credentials_plan::{self as plan, Failure, Plan, Result, Source},
    public_api_admin,
};
use colossus_credentials::{DevelopmentAuthority, PlatformCredentialVault};
use colossus_home::{ColossusHome, detect_workspace_identity};
use colossus_runtime::{KeyConfig, RuntimeConfig, StorageAdapter, StorageLocation};
use fs4::fs_std::FileExt as _;
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

const MAX_SOURCES: usize = 64;

pub(super) fn make_plan(args: &DevelopmentCredentialSources, workspace: &Path) -> Result<Plan> {
    make_plan_inner(args, workspace, false)
}

fn make_plan_inner(
    args: &DevelopmentCredentialSources,
    workspace: &Path,
    resume_copy: bool,
) -> Result<Plan> {
    if args.public_api_directories.len()
        + args.client_configs.len()
        + args.journal_configs.len()
        + args.runtime_oauth_storage.len()
        + usize::from(args.desktop_vault)
        + usize::from(args.control_plane_vault)
        + usize::from(args.connector_source_home.is_some())
        > MAX_SOURCES
    {
        return Err(Failure("development source selection exceeds its bound"));
    }
    if !args.fresh_empty
        && !args.desktop_vault
        && !args.control_plane_vault
        && args.public_api_directories.is_empty()
        && args.client_configs.is_empty()
        && args.journal_configs.is_empty()
        && args.runtime_oauth_storage.is_empty()
        && args.connector_source_home.is_none()
    {
        return Err(Failure(
            "select exact existing sources or explicitly select a fresh empty home",
        ));
    }
    let home_root = plan::existing_root(&args.home)?;
    let home = ColossusHome::ensure_at(home_root.path())
        .map_err(|_| Failure("development home is unsafe"))?;
    let workspace = std::fs::canonicalize(workspace)
        .map_err(|_| Failure("workspace constraint is unavailable"))?;
    if !workspace.is_dir() {
        return Err(Failure("workspace constraint must be a directory"));
    }
    let _metadata = DevelopmentAuthority::metadata(
        home.confined_root(),
        &DevelopmentAuthority::path_for_home(home.confined_root()),
        std::slice::from_ref(&workspace),
    )
    .map_err(|_| Failure("prepared development authority is unavailable"))?;
    if args.fresh_empty {
        validate_fresh_home(home.root())?;
    }
    let mut sources = Vec::new();
    if args.desktop_vault {
        let root = plan::existing_root(&home.root().join("desktop"))?;
        let metadata = PlatformCredentialVault::key_metadata(&root, "desktop-manual")
            .map_err(|_| Failure("existing desktop vault metadata is unavailable or busy"))?
            .ok_or(Failure("selected desktop vault does not exist"))?;
        sources.push(Source::DesktopVault { metadata });
    }
    if args.control_plane_vault {
        let root = plan::existing_root(&home.root().join("cloud-connector"))?;
        let metadata = PlatformCredentialVault::key_metadata(&root, "cloud-connector")
            .map_err(|_| Failure("existing connector vault metadata is unavailable or busy"))?
            .ok_or(Failure("selected connector vault does not exist"))?;
        sources.push(Source::ControlPlaneVault { metadata });
    }
    if let Some(source_home) = &args.connector_source_home {
        let source = plan::existing_root(source_home)?;
        let canonical_source = std::fs::canonicalize(source.path())
            .map_err(|_| Failure("source home identity is unavailable"))?;
        let canonical_target = std::fs::canonicalize(home.root())
            .map_err(|_| Failure("target home identity is unavailable"))?;
        source
            .revalidate()
            .map_err(|_| Failure("source home identity changed"))?;
        home.confined_root()
            .revalidate()
            .map_err(|_| Failure("target home identity changed"))?;
        if canonical_source == canonical_target
            || canonical_source.starts_with(&canonical_target)
            || canonical_target.starts_with(&canonical_source)
        {
            return Err(Failure(
                "named enrollment copy requires separate non-overlapping source and target homes",
            ));
        }
        let name = args
            .connector_enrollment
            .as_deref()
            .ok_or(Failure("named enrollment selector is missing"))?;
        let key = colossus_connector::EnrollmentStore::credential_key(name)
            .map_err(|_| Failure("named enrollment selector is invalid"))?;
        if !resume_copy
            && home
                .root()
                .join("cloud-connector")
                .try_exists()
                .map_err(|_| Failure("isolated connector target cannot be inspected"))?
        {
            return Err(Failure(
                "named enrollment copy requires a new isolated connector target",
            ));
        }
        let source_vault = plan::existing_root(&source.path().join("cloud-connector"))?;
        let metadata =
            PlatformCredentialVault::record_metadata(&source_vault, "cloud-connector", &key)
                .map_err(|_| {
                    Failure("selected original enrollment metadata is unavailable or busy")
                })?
                .ok_or(Failure("the exact named source enrollment does not exist"))?;
        sources.push(Source::ConnectorEnrollment {
            source_home: source.path().to_owned(),
            name: name.to_owned(),
            metadata,
        });
    } else if args.connector_enrollment.is_some() {
        return Err(Failure(
            "named enrollment copy requires an explicit source home",
        ));
    }
    for directory in &args.public_api_directories {
        sources.push(public_api_source(directory)?);
    }
    for config in &args.client_configs {
        sources.push(client_source(config)?);
    }
    for config in &args.journal_configs {
        sources.push(journal_source(config, &home, &workspace)?);
    }
    for storage in &args.runtime_oauth_storage {
        plan::clean_absolute(storage)?;
        let binding = colossus_runtime::runtime_oauth_vault_binding(storage)
            .map_err(|_| Failure("existing runtime OAuth binding is unavailable or unsafe"))?;
        let root = plan::existing_root(&binding.directory)?;
        let metadata = PlatformCredentialVault::key_metadata(&root, &binding.owner_scope)
            .map_err(|_| Failure("existing runtime OAuth vault metadata is unavailable or busy"))?
            .ok_or(Failure("selected runtime OAuth vault does not exist"))?;
        sources.push(Source::RuntimeOAuth {
            storage: storage.clone(),
            directory: binding.directory,
            owner_scope: binding.owner_scope,
            metadata,
        });
    }
    let mut identities = BTreeSet::new();
    for source in &sources {
        let identity = serde_json::to_string(source)
            .map_err(|_| Failure("source metadata serialization failed"))?;
        if !identities.insert(identity) {
            return Err(Failure("duplicate development source selection is invalid"));
        }
    }
    Ok(Plan {
        schema_version: 1,
        home: home.root().to_owned(),
        workspace,
        fresh_empty: args.fresh_empty,
        sources,
    })
}

pub(super) fn validate_fresh_home(home: &Path) -> Result<()> {
    for entry in
        std::fs::read_dir(home).map_err(|_| Failure("fresh home could not be inspected"))?
    {
        let entry = entry.map_err(|_| Failure("fresh home could not be inspected"))?;
        if entry.file_name() != "development-credentials" {
            return Err(Failure(
                "fresh activation refuses existing home state; explicitly plan its rewrap",
            ));
        }
    }
    for scope in [
        "desktop-vault",
        "control-plane-vault",
        "public-api",
        "journal",
        "runtime-oauth-vault",
    ] {
        let root = plan::existing_root(&home.join("development-credentials").join(scope))?;
        if std::fs::read_dir(root.path())
            .map_err(|_| Failure("prepared store could not be inspected"))?
            .next()
            .is_some()
        {
            return Err(Failure(
                "fresh activation refuses existing or partial sealed custody",
            ));
        }
    }
    Ok(())
}

pub(super) fn rebuild(plan: &Plan) -> Result<Plan> {
    let mut args = DevelopmentCredentialSources {
        home: plan.home.clone(),
        desktop_vault: false,
        control_plane_vault: false,
        connector_source_home: None,
        connector_enrollment: None,
        public_api_directories: Vec::new(),
        client_configs: Vec::new(),
        journal_configs: Vec::new(),
        runtime_oauth_storage: Vec::new(),
        fresh_empty: plan.fresh_empty,
    };
    for source in &plan.sources {
        match source {
            Source::DesktopVault { .. } => args.desktop_vault = true,
            Source::ControlPlaneVault { .. } => args.control_plane_vault = true,
            Source::ConnectorEnrollment {
                source_home, name, ..
            } => {
                if args.connector_source_home.is_some() {
                    return Err(Failure("only one exact named enrollment may be copied"));
                }
                args.connector_source_home = Some(source_home.clone());
                args.connector_enrollment = Some(name.clone());
            }
            Source::RuntimeOAuth { storage, .. } => {
                args.runtime_oauth_storage.push(storage.clone())
            }
            Source::PublicApi { directory, .. } => {
                args.public_api_directories.push(directory.clone())
            }
            Source::Client { config, .. } => args.client_configs.push(config.clone()),
            Source::Journal { config, .. } => args.journal_configs.push(config.clone()),
            Source::PlaintextJournal { config, .. } => args.journal_configs.push(config.clone()),
        }
    }
    if args.fresh_empty
        && (!args.public_api_directories.is_empty()
            || !args.client_configs.is_empty()
            || !args.journal_configs.is_empty()
            || !args.runtime_oauth_storage.is_empty()
            || args.desktop_vault
            || args.control_plane_vault
            || args.connector_source_home.is_some())
    {
        return Err(Failure(
            "fresh activation cannot contain existing source selectors",
        ));
    }
    if args.control_plane_vault && args.connector_source_home.is_some() {
        return Err(Failure(
            "whole-vault rewrap and named copy cannot share the same connector target",
        ));
    }
    make_plan_inner(&args, &plan.workspace, plan::copy_receipt_matches(plan)?)
}

pub(super) fn public_api_source(directory: &Path) -> Result<Source> {
    let root = plan::existing_root(directory)?;
    let lock = root
        .open_existing_file_read_write(Path::new(".public-api.lock"))
        .map_err(|_| Failure("existing public API source lease is unavailable"))?;
    if !lock
        .file()
        .try_lock_exclusive()
        .map_err(|_| Failure("public API source lease failed"))?
    {
        return Err(Failure(
            "stop the source public API before planning or rewrapping its custody",
        ));
    }
    let certificate_file_sha256 =
        match std::fs::symlink_metadata(root.path().join("certificate.pem")) {
            Ok(_) => Some(plan::sha256(
                &plan::read_private(&root.path().join("certificate.pem"), 16 * 1024)?.1,
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                return Err(Failure(
                    "existing public API publication metadata is unavailable",
                ));
            }
        };
    let source = Source::PublicApi {
        directory: root.path().to_owned(),
        certificate_file_sha256,
        service: public_api_admin::namespace_service(root.path()),
    };
    lock.revalidate(&root)
        .map_err(|_| Failure("public API source identity changed"))?;
    Ok(source)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientConfig {
    descriptor: PathBuf,
    certificate: PathBuf,
    instance_id: String,
    certificate_sha256: String,
    keyring_service: String,
    keyring_account: String,
    #[serde(default)]
    headless_authority: Option<HeadlessReference>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadlessReference {
    directory: PathBuf,
    key_variable: String,
}

fn client_source(path: &Path) -> Result<Source> {
    let (config_path, bytes) = plan::read_private(path, 16 * 1024)?;
    let config: ClientConfig = serde_json::from_slice(&bytes)
        .map_err(|_| Failure("local credential reference is malformed or has unknown fields"))?;
    if let Some(headless) = config.headless_authority {
        let _ = (headless.directory, headless.key_variable);
        return Err(Failure(
            "this offline plan only selects existing platform custody, not another headless authority",
        ));
    }
    // Worker shutdown removes publication leaves. Retained client references
    // carry the original instance and independently provisioned pin; reopening
    // a worker to republish them would violate this offline operation.
    plan::clean_absolute(&config.certificate)?;
    plan::clean_absolute(&config.descriptor)?;
    uuid::Uuid::parse_str(&config.instance_id)
        .map_err(|_| Failure("local reference instance is invalid"))?;
    validate_identifier(&config.keyring_service, false)?;
    validate_identifier(&config.keyring_account, true)?;
    if !valid_hex(&config.certificate_sha256, 64) {
        return Err(Failure("local reference certificate pin is invalid"));
    }
    if config.keyring_service == "com.obscuritylabs.colossus.desktop.external"
        && config.keyring_account
            != format!(
                "daemon-{}-{}",
                config.instance_id, config.certificate_sha256
            )
    {
        return Err(Failure(
            "Desktop external custody must bind the exact existing instance and certificate pin",
        ));
    }
    Ok(Source::Client {
        config: config_path,
        config_sha256: plan::sha256(&bytes),
        service: config.keyring_service,
        account: config.keyring_account,
        instance_id: config.instance_id,
        certificate_sha256: config.certificate_sha256,
    })
}

fn journal_source(path: &Path, home: &ColossusHome, workspace: &Path) -> Result<Source> {
    let (config_path, bytes) = plan::read_private(path, 1024 * 1024)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| Failure("journal source configuration is not UTF-8"))?;
    let config = RuntimeConfig::from_yaml(text)
        .map_err(|_| Failure("journal source configuration is invalid"))?;
    if config.storage.adapter != StorageAdapter::Redb {
        return Err(Failure(
            "offline development journal rewrap supports frozen existing redb sources only",
        ));
    }
    let journal_path = if config.storage.path.is_absolute() {
        config.storage.path.clone()
    } else {
        let base = match config.storage.location {
            StorageLocation::Workspace => workspace.to_owned(),
            StorageLocation::HomeWorkspace => {
                let identity = detect_workspace_identity(workspace)
                    .map_err(|_| Failure("journal workspace identity is unavailable"))?;
                let partition = home
                    .workspace_partition_id(workspace, identity.as_ref())
                    .map_err(|_| Failure("journal workspace partition is invalid"))?;
                // The existing CLI surface only: never call workspace_surface_dir,
                // which would create state during this read-only plan.
                home.root().join("workspaces").join(partition).join("cli")
            }
        };
        base.join(&config.storage.path)
    };
    let (root, journal_file) = plan::existing_file(&journal_path)?;
    let metadata =
        colossus_journal_redb::journal_key_metadata(journal_file.path()).map_err(|_| {
            Failure("existing journal metadata is unavailable, busy, or needs recovery")
        })?;
    if matches!(config.storage.keys, KeyConfig::None) {
        if !metadata.encryption_key_ids.is_empty() || metadata.checkpoint_key_id.is_some() {
            return Err(Failure(
                "plaintext journal selection does not match retained protection metadata",
            ));
        }
        journal_file
            .revalidate(&root)
            .map_err(|_| Failure("journal source identity changed"))?;
        return Ok(Source::PlaintextJournal {
            config: config_path,
            config_sha256: plan::sha256(&bytes),
            journal: journal_file.path().to_owned(),
            journal_sha256: plan::file_digest(journal_file.path())?,
        });
    }
    let KeyConfig::Platform {
        service,
        journal_key_id,
        signing_key_id,
    } = &config.storage.keys
    else {
        return Err(Failure(
            "journal rewrap does not select another environment authority",
        ));
    };
    validate_identifier(service, false)?;
    validate_identifier(journal_key_id, true)?;
    validate_identifier(signing_key_id, true)?;
    let mut encryption = BTreeSet::from([journal_key_id.clone()]);
    encryption.extend(metadata.encryption_key_ids);
    let mut signing = BTreeSet::from([signing_key_id.clone()]);
    signing.extend(metadata.checkpoint_key_id);
    if encryption.len() > 256 || signing.len() > 256 {
        return Err(Failure(
            "journal key identifiers exceed offline planning bounds",
        ));
    }
    for id in encryption.iter().chain(signing.iter()) {
        validate_identifier(id, true)?;
    }
    journal_file
        .revalidate(&root)
        .map_err(|_| Failure("journal source identity changed"))?;
    Ok(Source::Journal {
        config: config_path,
        config_sha256: plan::sha256(&bytes),
        journal: journal_file.path().to_owned(),
        journal_sha256: plan::file_digest(journal_file.path())?,
        service: service.clone(),
        encryption_key_ids: encryption.into_iter().collect(),
        signing_key_ids: signing.into_iter().collect(),
        anchor_key_id: journal_key_id.clone(),
    })
}

pub(super) fn validate_identifier(value: &str, account: bool) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'-')
                || account && byte == b':'
        })
    {
        return Err(Failure(
            "existing credential reference is invalid or exceeds its bound",
        ));
    }
    Ok(())
}
pub(super) fn valid_hex(value: &str, count: usize) -> bool {
    value.len() == count
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
