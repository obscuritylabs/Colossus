use std::{
    env,
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::{command::Task, timing::Timings};

#[derive(Debug)]
pub(super) struct Repository {
    root: PathBuf,
    timings: Rc<Timings>,
}

impl Repository {
    pub(super) fn discover() -> Result<Self, String> {
        let current = env::current_dir()
            .map_err(|error| format!("could not read the current directory: {error}"))?;
        if let Some(root) = find_root(&current) {
            return Ok(Self {
                root,
                timings: Rc::new(Timings::new()),
            });
        }

        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest
            .parent()
            .filter(|candidate| is_root(candidate))
            .ok_or_else(|| "could not locate the Colossus repository root".to_owned())?;
        Ok(Self {
            root: root.to_path_buf(),
            timings: Rc::new(Timings::new()),
        })
    }

    pub(super) fn path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.join(relative)
    }

    pub(super) fn task(&self, program: impl Into<std::ffi::OsString>) -> Task {
        Task::new(&self.root, program).with_timings(Rc::clone(&self.timings))
    }

    pub(super) fn report_timings(&self) {
        self.timings.report();
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
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let source = manifest.join("src");
        let root = find_root(&source).expect("repository root");
        assert_eq!(root, manifest.parent().expect("xtask repository parent"));
    }
}
