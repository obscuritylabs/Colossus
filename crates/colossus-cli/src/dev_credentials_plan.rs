use colossus_credentials::{VaultKeyMetadata, VaultRecordMetadata};
use colossus_home::{ConfinedFile, ConfinedRoot};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    io::{Read as _, Write as _},
    path::{Component, Path, PathBuf},
};
use zeroize::Zeroizing;

pub(super) type Result<T> = std::result::Result<T, Failure>;

#[derive(Debug)]
pub(super) struct Failure(pub(super) &'static str);
impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}
impl std::error::Error for Failure {}

/// Metadata only. No field can represent an envelope, bearer, seed, or key value.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Plan {
    pub(super) schema_version: u16,
    pub(super) home: PathBuf,
    pub(super) workspace: PathBuf,
    pub(super) fresh_empty: bool,
    pub(super) sources: Vec<Source>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Source {
    DesktopVault {
        metadata: VaultKeyMetadata,
    },
    ControlPlaneVault {
        metadata: VaultKeyMetadata,
    },
    ConnectorEnrollment {
        source_home: PathBuf,
        name: String,
        metadata: VaultRecordMetadata,
    },
    RuntimeOAuth {
        storage: PathBuf,
        directory: PathBuf,
        owner_scope: String,
        metadata: VaultKeyMetadata,
    },
    PublicApi {
        directory: PathBuf,
        certificate_file_sha256: Option<String>,
        service: String,
    },
    Client {
        config: PathBuf,
        config_sha256: String,
        service: String,
        account: String,
        instance_id: String,
        certificate_sha256: String,
    },
    Journal {
        config: PathBuf,
        config_sha256: String,
        journal: PathBuf,
        journal_sha256: String,
        service: String,
        encryption_key_ids: Vec<String>,
        signing_key_ids: Vec<String>,
        anchor_key_id: String,
    },
    PlaintextJournal {
        config: PathBuf,
        config_sha256: String,
        journal: PathBuf,
        journal_sha256: String,
    },
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(super) fn plan_digest(plan: &Plan) -> Result<String> {
    Ok(sha256(&serde_json::to_vec(plan).map_err(|_| {
        Failure("development plan serialization failed")
    })?))
}

pub(super) fn clean_absolute(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(Failure(
            "development credential paths must be clean absolute paths",
        ));
    }
    Ok(())
}

pub(super) fn existing_root(path: &Path) -> Result<ConfinedRoot> {
    clean_absolute(path)?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| Failure("existing private source directory is unavailable"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Failure("existing private source directory is invalid"));
    }
    ConfinedRoot::bind(path).map_err(|_| Failure("existing private source directory is unsafe"))
}

pub(super) fn existing_file(path: &Path) -> Result<(ConfinedRoot, ConfinedFile)> {
    clean_absolute(path)?;
    let root = existing_root(
        path.parent()
            .ok_or(Failure("private file parent is invalid"))?,
    )?;
    let file = root
        .open_existing_file(Path::new(
            path.file_name()
                .ok_or(Failure("private file name is invalid"))?,
        ))
        .map_err(|_| Failure("existing private file is unavailable or unsafe"))?;
    Ok((root, file))
}

pub(super) fn read_private(path: &Path, limit: u64) -> Result<(PathBuf, Zeroizing<Vec<u8>>)> {
    let (root, file) = existing_file(path)?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.file()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure("private source could not be read"))?;
    if bytes.len() as u64 > limit {
        return Err(Failure("private source exceeds its bound"));
    }
    file.revalidate(&root)
        .map_err(|_| Failure("private source identity changed"))?;
    Ok((file.path().to_owned(), bytes))
}

pub(super) fn file_digest(path: &Path) -> Result<String> {
    let (root, file) = existing_file(path)?;
    if file
        .file()
        .metadata()
        .map_err(|_| Failure("source metadata is unavailable"))?
        .len()
        > 8 * 1024 * 1024 * 1024
    {
        return Err(Failure("source journal exceeds offline planning bounds"));
    }
    let mut hash = Sha256::new();
    let mut reader = file.file();
    let mut buffer = [0_u8; 16 * 1024];
    let mut total = 0_u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| Failure("source journal could not be fingerprinted"))?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > 8 * 1024 * 1024 * 1024 {
            return Err(Failure("source journal exceeds offline planning bounds"));
        }
        hash.update(&buffer[..count]);
    }
    file.revalidate(&root)
        .map_err(|_| Failure("source journal identity changed"))?;
    Ok(hex::encode(hash.finalize()))
}

pub(super) fn write_plan(path: &Path, plan: &Plan) -> Result<String> {
    clean_absolute(path)?;
    let bytes = serde_json::to_vec_pretty(plan)
        .map_err(|_| Failure("development plan serialization failed"))?;
    if bytes.len() > 64 * 1024 {
        return Err(Failure("development plan exceeds its bound"));
    }
    let root = existing_root(
        path.parent()
            .ok_or(Failure("private plan parent is invalid"))?,
    )?;
    let leaf = Path::new(
        path.file_name()
            .ok_or(Failure("private plan name is invalid"))?,
    );
    let file = root
        .open_file(leaf)
        .map_err(|_| Failure("private plan file is unavailable"))?;
    if !file.was_created() {
        return Err(Failure("an existing development plan is never replaced"));
    }
    file.file()
        .write_all(&bytes)
        .map_err(|_| Failure("development plan could not be written"))?;
    file.file()
        .sync_all()
        .map_err(|_| Failure("development plan could not be synchronized"))?;
    file.revalidate(&root)
        .map_err(|_| Failure("development plan identity changed"))?;
    root.sync_directory()
        .map_err(|_| Failure("development plan directory could not be synchronized"))?;
    plan_digest(plan)
}

pub(super) fn read_plan(path: &Path, expected: &str) -> Result<Plan> {
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Failure(
            "an exact lowercase SHA-256 plan digest is required",
        ));
    }
    let (_, bytes) = read_private(path, 64 * 1024)?;
    let plan: Plan = serde_json::from_slice(&bytes)
        .map_err(|_| Failure("development plan is malformed or has unknown fields"))?;
    if plan.schema_version != 1 || plan.sources.len() > 64 || plan_digest(&plan)? != expected {
        return Err(Failure(
            "development plan does not match the reviewed digest",
        ));
    }
    Ok(plan)
}

const COPY_RECEIPT: &str = "offline-copy-plan.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyReceipt {
    schema_version: u16,
    plan_sha256: String,
}

pub(super) fn copy_receipt_matches(plan: &Plan) -> Result<bool> {
    let path = plan.home.join("development-credentials").join(COPY_RECEIPT);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(Failure("offline copy receipt is unavailable")),
        Ok(_) => {}
    }
    let (_, bytes) = read_private(&path, 1024)?;
    let receipt: CopyReceipt =
        serde_json::from_slice(&bytes).map_err(|_| Failure("offline copy receipt is invalid"))?;
    Ok(receipt.schema_version == 1 && receipt.plan_sha256 == plan_digest(plan)?)
}

pub(super) fn record_copy_plan(plan: &Plan) -> Result<()> {
    if copy_receipt_matches(plan)? {
        return Ok(());
    }
    let root = existing_root(&plan.home.join("development-credentials"))?;
    let file = root
        .open_file(Path::new(COPY_RECEIPT))
        .map_err(|_| Failure("offline copy receipt could not be created"))?;
    if !file.was_created() {
        return Err(Failure(
            "another reviewed copy plan already owns this target",
        ));
    }
    let bytes = serde_json::to_vec(&CopyReceipt {
        schema_version: 1,
        plan_sha256: plan_digest(plan)?,
    })
    .map_err(|_| Failure("offline copy receipt could not be encoded"))?;
    file.file()
        .write_all(&bytes)
        .map_err(|_| Failure("offline copy receipt could not be written"))?;
    file.file()
        .sync_all()
        .map_err(|_| Failure("offline copy receipt could not be synchronized"))?;
    file.revalidate(&root)
        .map_err(|_| Failure("offline copy receipt identity changed"))?;
    root.sync_directory()
        .map_err(|_| Failure("offline copy receipt directory could not be synchronized"))
}
