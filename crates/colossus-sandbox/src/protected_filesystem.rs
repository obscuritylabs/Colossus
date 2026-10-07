use super::*;
use colossus_home::{ConfinedFile, ConfinedRoot};

// Bound metadata validation before an effect; exhausting the bound fails closed.
const MAX_PROTECTED_ENTRIES: usize = 100_000;

/// Native-owned deny roots added independently of caller policy or grants.
/// Retained confinement rejects replaced roots, symbolic links and hard links.
#[derive(Clone, Default)]
pub struct ProtectedFilesystem {
    roots: Vec<ConfinedRoot>,
}

impl ProtectedFilesystem {
    /// Bind already validated private roots. This reads metadata, never contents,
    /// and neither creates paths nor grants access to them.
    pub fn new(roots: Vec<ConfinedRoot>) -> Result<Self, ExecutionError> {
        let protection = Self { roots };
        protection.snapshot()?;
        Ok(protection)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    pub(super) fn snapshot(&self) -> Result<ProtectedFilesystemSnapshot, ExecutionError> {
        let mut files = Vec::new();
        let mut count = 0_usize;
        for root in &self.roots {
            root.revalidate().map_err(protected_failure)?;
            let mut directories = vec![root.path().to_owned()];
            while let Some(directory) = directories.pop() {
                for entry in fs::read_dir(directory).map_err(protected_failure)? {
                    let path = entry.map_err(protected_failure)?.path();
                    count = count.saturating_add(1);
                    if count > MAX_PROTECTED_ENTRIES {
                        return Err(protected_failure("entry bound"));
                    }
                    let metadata = fs::symlink_metadata(&path).map_err(protected_failure)?;
                    if metadata.file_type().is_symlink() {
                        return Err(protected_failure("symbolic link"));
                    }
                    if metadata.is_dir() {
                        root.revalidate_directory(&path)
                            .map_err(protected_failure)?;
                        directories.push(path);
                    } else if metadata.is_file() {
                        let relative = root.relative(&path).map_err(protected_failure)?;
                        let file = root
                            .open_existing_file(relative)
                            .map_err(protected_failure)?;
                        file.revalidate(root).map_err(protected_failure)?;
                        files.push((root.clone(), file));
                    } else {
                        return Err(protected_failure("nonregular entry"));
                    }
                }
            }
            root.revalidate().map_err(protected_failure)?;
        }
        Ok(ProtectedFilesystemSnapshot {
            roots: self.roots.clone(),
            files,
        })
    }

    pub(super) fn restrict_process(
        &self,
        obligations: &mut PolicyObligations,
    ) -> Result<(), ExecutionError> {
        if self.is_empty() {
            return Ok(());
        }
        if !isolating_backend(&obligations.sandbox_backend)
            || obligations.resource_authority == ResourceAuthority::Ambient
            || obligations.allow_sandbox_downgrade
        {
            return Err(adapter_failure(
                "development credential custody requires an isolating sandbox",
            ));
        }
        self.snapshot()?.revalidate()?;
        for root in &self.roots {
            let path = root
                .path()
                .to_str()
                .ok_or_else(|| protected_failure("non-UTF-8 root"))?;
            if !obligations
                .protected_filesystem
                .iter()
                .any(|existing| existing == path)
            {
                obligations.protected_filesystem.push(path.to_owned());
            }
        }
        Ok(())
    }
}

pub(super) struct ProtectedFilesystemSnapshot {
    roots: Vec<ConfinedRoot>,
    files: Vec<(ConfinedRoot, ConfinedFile)>,
}

impl ProtectedFilesystemSnapshot {
    pub(super) fn paths(&self) -> Vec<PathBuf> {
        self.roots
            .iter()
            .map(|root| root.path().to_owned())
            .collect()
    }

    pub(super) fn check_path(&self, path: &Path) -> Result<(), ExecutionError> {
        if self.roots.iter().any(|root| path.starts_with(root.path())) {
            return Err(adapter_failure(
                "native credential authority is unavailable to filesystem effects",
            ));
        }
        Ok(())
    }

    pub(super) fn discoverable(&self, path: &Path) -> bool {
        self.roots.is_empty()
            || fs::canonicalize(path).is_ok_and(|path| self.check_path(&path).is_ok())
    }

    pub(super) fn check_file(&self, file: &fs::File) -> Result<(), ExecutionError> {
        for (_, protected) in &self.files {
            if protected.shares_identity(file).map_err(protected_failure)? {
                return Err(adapter_failure(
                    "native credential authority is unavailable to filesystem effects",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn revalidate(&self) -> Result<(), ExecutionError> {
        for root in &self.roots {
            root.revalidate().map_err(protected_failure)?;
        }
        for (root, file) in &self.files {
            file.revalidate(root).map_err(protected_failure)?;
        }
        Ok(())
    }
}

pub(super) fn isolating_backend(backend: &str) -> bool {
    match backend {
        "native" => cfg!(any(target_os = "linux", target_os = "macos")),
        "windows_job" => cfg!(windows),
        "oci" => cfg!(unix),
        _ => false,
    }
}

fn protected_failure(_: impl std::fmt::Display) -> ExecutionError {
    adapter_failure("native credential authority confinement is invalid")
}

#[cfg(test)]
mod tests;
