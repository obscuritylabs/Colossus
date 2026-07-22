use super::*;

/// Explicit host context used when composing a runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeOpenOptions {
    /// Canonical repository workspace used by tools and repository identity.
    pub workspace: PathBuf,
    pub(super) expected_workspace_identity: Option<WorkspaceIdentityToken>,
}

impl RuntimeOpenOptions {
    /// Resolve one existing workspace directory without changing process state.
    pub fn for_workspace(workspace: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let workspace = fs::canonicalize(workspace)?;
        if !workspace.is_dir() {
            return Err(RuntimeError::Config(format!(
                "workspace is not a directory: {}",
                workspace.display()
            )));
        }
        Ok(Self {
            workspace,
            expected_workspace_identity: None,
        })
    }

    /// Require runtime lease acquisition to retain the exact directory identity
    /// captured by a trusted host before private bootstrap.
    #[must_use]
    pub fn with_expected_workspace_identity(mut self, identity: WorkspaceIdentityToken) -> Self {
        self.expected_workspace_identity = Some(identity);
        self
    }

    pub(super) fn current() -> Result<Self, RuntimeError> {
        Self::for_workspace(std::env::current_dir()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_options_canonicalize_existing_directories() {
        let root = tempfile::tempdir().expect("workspace");
        fs::create_dir(root.path().join("nested")).expect("nested");
        let selected = RuntimeOpenOptions::for_workspace(root.path().join("nested").join(".."))
            .expect("canonical workspace");
        assert_eq!(
            selected.workspace,
            fs::canonicalize(root.path()).expect("canonical root")
        );
    }

    #[test]
    fn workspace_options_reject_files_and_missing_paths() {
        let root = tempfile::tempdir().expect("workspace");
        let file = root.path().join("file.txt");
        fs::write(&file, "not a workspace").expect("file");
        assert!(RuntimeOpenOptions::for_workspace(&file).is_err());
        assert!(RuntimeOpenOptions::for_workspace(root.path().join("missing")).is_err());
    }
}
