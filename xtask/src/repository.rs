use std::{
    env,
    path::{Path, PathBuf},
};

use crate::command::Task;

#[derive(Debug)]
pub(super) struct Repository {
    root: PathBuf,
}

impl Repository {
    pub(super) fn discover() -> Result<Self, String> {
        let current = env::current_dir()
            .map_err(|error| format!("could not read the current directory: {error}"))?;
        if let Some(root) = find_root(&current) {
            return Ok(Self { root });
        }

        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest
            .parent()
            .filter(|candidate| is_root(candidate))
            .ok_or_else(|| "could not locate the Colossus repository root".to_owned())?;
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    pub(super) fn path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.join(relative)
    }

    pub(super) fn task(&self, program: impl Into<std::ffi::OsString>) -> Task {
        Task::new(&self.root, program)
    }
}

fn find_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|candidate| is_root(candidate))
        .map(Path::to_path_buf)
}

fn is_root(path: &Path) -> bool {
    path.join(".git").exists()
        && path.join("Cargo.toml").is_file()
        && path.join("scripts/ci/classify-changes.sh").is_file()
}

#[cfg(test)]
mod tests {
    use super::find_root;

    #[test]
    fn finds_the_repository_from_the_xtask_source_tree() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let root = find_root(&source).expect("repository root");
        assert_eq!(
            root.file_name().and_then(|name| name.to_str()),
            Some("Colossus")
        );
    }
}
