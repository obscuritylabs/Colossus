//! Bound and inspect only the dedicated default Desktop home. Never scan the OS store.

use super::{CleanupError, PROVIDER_SERVICE, RUNTIME_SERVICE};
use colossus_windows_native::{BoundPath, FileIdentity};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_ENTRIES: usize = 100_000;
const MAX_METADATA_BYTES: u64 = 1024 * 1024;

pub(super) struct CleanupPlan {
    pub keys: BTreeSet<(String, String)>,
    pub vaults: Vec<(PathBuf, String)>,
    files: super::files::CleanupFiles,
    empty_cli_directories: Vec<EmptyCliDirectory>,
}

struct EmptyCliDirectory {
    path: PathBuf,
    identity: FileIdentity,
}

impl CleanupPlan {
    pub fn inspect(home: &Path) -> Result<Self, CleanupError> {
        let mut plan = Self {
            keys: BTreeSet::new(),
            vaults: Vec::new(),
            files: super::files::CleanupFiles::default(),
            empty_cli_directories: Vec::new(),
        };
        let mut directories = vec![(home.to_owned(), 0)];
        let mut entries = 0;
        while let Some((directory, depth)) = directories.pop() {
            if depth > 32 {
                return Err(CleanupError::UnsafeData);
            }
            let binding = BoundPath::open_directory(&directory)
                .map_err(|error| CleanupError::from_native(&error))?;
            for entry in fs::read_dir(&directory).map_err(|error| CleanupError::from_io(&error))? {
                entries += 1;
                if entries > MAX_ENTRIES {
                    return Err(CleanupError::UnsafeData);
                }
                let path = entry.map_err(|error| CleanupError::from_io(&error))?.path();
                let metadata =
                    fs::symlink_metadata(&path).map_err(|error| CleanupError::from_io(&error))?;
                let relative = path
                    .strip_prefix(home)
                    .map_err(|_| CleanupError::UnsafeData)?;
                if !super::ownership::owned_path(relative, metadata.is_dir()) {
                    // Unknown folders may contain projects whose settings record
                    // no longer exists. Never infer ownership from containment.
                    return Err(CleanupError::UnsafeData);
                }
                if metadata.is_dir() {
                    let child = BoundPath::open_directory(&path)
                        .map_err(|error| CleanupError::from_native(&error))?;
                    if super::ownership::empty_cli_surface(relative) {
                        child
                            .validate_private_owner_dacl()
                            .map_err(|_| CleanupError::UnsafeData)?;
                        plan.empty_cli_directories.push(EmptyCliDirectory {
                            path: path.clone(),
                            identity: child.identity(),
                        });
                    }
                    directories.push((path, depth + 1));
                } else {
                    let file = BoundPath::open_file(&path)
                        .map_err(|error| CleanupError::from_native(&error))?;
                    plan.files.push(&path, relative, &file)?;
                    plan.inspect_file(home, &path)?;
                }
            }
            binding.revalidate().map_err(|_| CleanupError::UnsafeData)?;
        }
        plan.files.validate_links()?;
        Ok(plan)
    }

    fn inspect_file(&mut self, home: &Path, path: &Path) -> Result<(), CleanupError> {
        let directory = path.parent().ok_or(CleanupError::UnsafeData)?;
        let relative = directory
            .strip_prefix(home)
            .map_err(|_| CleanupError::UnsafeData)?;
        let runtime = is_runtime_directory(relative);
        if path
            .file_name()
            .is_some_and(|name| name == "managed-config.yaml")
            && runtime
        {
            let config: ManagedConfig = serde_saphyr::from_slice(&read_metadata(path)?)
                .map_err(|_| CleanupError::UnsafeData)?;
            if let Keys::Platform {
                service,
                journal_key_id,
                signing_key_id,
            } = config.storage.keys
            {
                if service != RUNTIME_SERVICE {
                    return Err(CleanupError::UnsafeData);
                }
                let instance = journal_key_id
                    .strip_prefix("journal-")
                    .ok_or(CleanupError::UnsafeData)?;
                if Uuid::parse_str(instance)
                    .map_err(|_| CleanupError::UnsafeData)?
                    .to_string()
                    != instance
                    || signing_key_id != format!("checkpoint-{instance}")
                {
                    return Err(CleanupError::UnsafeData);
                }
                for account in [
                    format!("journal-key:{journal_key_id}"),
                    format!("journal-anchor:{journal_key_id}"),
                    format!("signing-key:{signing_key_id}"),
                ] {
                    self.keys.insert((service.clone(), account));
                }
            }
        }
        if path
            .file_name()
            .is_some_and(|name| name == "credentials-v1.redb")
        {
            let scope = if relative == Path::new("desktop") {
                "desktop-manual".to_owned()
            } else if runtime {
                let canonical =
                    fs::canonicalize(directory).map_err(|_| CleanupError::UnsafeData)?;
                let identity = serde_json::to_vec(&("colossus-runtime-oauth-vault-v1", canonical))
                    .map_err(|_| CleanupError::UnsafeData)?;
                format!("runtime-oauth-{:x}", Sha256::digest(identity))
            } else {
                return Err(CleanupError::UnsafeData);
            };
            self.vaults.push((directory.to_owned(), scope));
        }
        if path == home.join("desktop/settings.json") {
            // Only legacy provider handles recorded by this Desktop are eligible.
            // External-daemon credentials can be shared with other homes; keep them.
            let (settings, _) = crate::desktop_settings::decode_settings(&read_metadata(path)?)
                .map_err(|_| CleanupError::UnsafeData)?;
            let canonical_home = fs::canonicalize(home).map_err(|_| CleanupError::UnsafeData)?;
            for workspace in settings
                .workspace
                .iter()
                .chain(settings.spaces.iter().map(|space| &space.workspace))
            {
                if workspace.path.starts_with(home)
                    || workspace.path.starts_with(&canonical_home)
                    || fs::canonicalize(&workspace.path)
                        .is_ok_and(|path| path.starts_with(&canonical_home))
                {
                    // Even an unusually located user project is never app data.
                    return Err(CleanupError::UnsafeData);
                }
            }
            for id in settings.provider_credential_ids().into_iter().chain(
                settings
                    .pending_provider_cleanup_ids
                    .iter()
                    .map(String::as_str),
            ) {
                if Uuid::parse_str(id)
                    .map_err(|_| CleanupError::UnsafeData)?
                    .to_string()
                    != id
                {
                    return Err(CleanupError::UnsafeData);
                }
                self.keys.insert((PROVIDER_SERVICE.into(), id.into()));
            }
        }
        Ok(())
    }

    pub fn check_idle(&self) -> Result<(), super::CleanupError> {
        // Recheck emptiness before deleting keys and again before removing files.
        // CLI data created since inspection is not Desktop-owned state.
        for directory in &self.empty_cli_directories {
            let binding = BoundPath::open_directory(&directory.path)
                .map_err(|error| CleanupError::from_native(&error))?;
            if binding.identity() != directory.identity
                || fs::read_dir(&directory.path)
                    .map_err(|error| CleanupError::from_io(&error))?
                    .next()
                    .is_some()
            {
                return Err(CleanupError::UnsafeData);
            }
            binding.revalidate().map_err(|_| CleanupError::UnsafeData)?;
        }
        self.files.check_idle()
    }
}

fn read_metadata(path: &Path) -> Result<Vec<u8>, CleanupError> {
    let binding = BoundPath::open_file(path).map_err(|error| CleanupError::from_native(&error))?;
    binding
        .validate_private_owner_dacl()
        .map_err(|_| CleanupError::UnsafeData)?;
    let mut bytes = Vec::new();
    binding
        .try_clone_file()
        .map_err(|_| CleanupError::UnsafeData)?
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CleanupError::UnsafeData)?;
    if bytes.len() as u64 > MAX_METADATA_BYTES {
        return Err(CleanupError::UnsafeData);
    }
    binding.revalidate().map_err(|_| CleanupError::UnsafeData)?;
    Ok(bytes)
}

fn is_runtime_directory(relative: &Path) -> bool {
    let parts = relative
        .iter()
        .filter_map(|part| part.to_str())
        .collect::<Vec<_>>();
    let is_partition =
        |part: &str| part.len() == 64 && part.bytes().all(|byte| byte.is_ascii_hexdigit());
    match parts.as_slice() {
        [
            "desktop",
            "self-test",
            "runtime" | "runtime-v2" | "runtime-v3",
        ] => true,
        ["desktop", "managed-local", partition] | ["workspaces", partition, "desktop"] => {
            is_partition(partition)
        }
        [
            "desktop",
            "managed-local",
            partition,
            "development-plaintext",
        ]
        | ["workspaces", partition, "desktop", "development-plaintext"] => is_partition(partition),
        _ => false,
    }
}

#[derive(Deserialize)]
struct ManagedConfig {
    storage: Storage,
}
#[derive(Deserialize)]
struct Storage {
    keys: Keys,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Keys {
    None,
    Platform {
        service: String,
        journal_key_id: String,
        signing_key_id: String,
    },
}
