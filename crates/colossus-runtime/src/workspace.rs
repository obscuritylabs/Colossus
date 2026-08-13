use super::*;

/// Explicit host context used when composing a runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeOpenOptions {
    /// Canonical repository workspace used by tools and repository identity.
    pub workspace: PathBuf,
    /// Explicit Colossus home selected and validated by the trusted host.
    ///
    /// Embedded SDK hosts that do not opt into the shared user home leave this unset.
    /// Runtime composition never resolves ambient home-directory environment state.
    pub colossus_home: Option<PathBuf>,
    /// Retained authority for descriptor-relative reads beneath `colossus_home`.
    pub(super) colossus_home_root: Option<ConfinedRoot>,
    /// Whether user-facing runs automatically load home and workspace AGENTS.md files.
    ///
    /// This is disabled only by trusted native composition for a dedicated internal
    /// diagnostic probe; public run requests cannot change it.
    pub(super) automatic_agent_instructions: bool,
    /// Whether configured network destinations may activate general model-visible fetch tools.
    /// Provider, authentication, search, MCP, and integration adapters remain independently
    /// configured when this is false.
    pub(super) model_network_tools: bool,
    pub(super) expected_workspace_identity: Option<WorkspaceIdentityToken>,
}

impl RuntimeOpenOptions {
    /// Resolve one existing workspace directory without changing process state.
    pub fn for_workspace(workspace: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        Self {
            workspace: workspace.as_ref().to_owned(),
            colossus_home: None,
            colossus_home_root: None,
            automatic_agent_instructions: true,
            model_network_tools: true,
            expected_workspace_identity: None,
        }
        .canonicalized()
    }

    /// Revalidate and canonicalize the selected workspace without discarding host context.
    pub fn canonicalized(mut self) -> Result<Self, RuntimeError> {
        self.workspace = fs::canonicalize(&self.workspace)?;
        if !self.workspace.is_dir() {
            return Err(RuntimeError::Config(format!(
                "workspace is not a directory: {}",
                self.workspace.display()
            )));
        }
        Ok(self)
    }

    /// Suppress automatic AGENTS.md loading for one trusted internal diagnostic runtime.
    ///
    /// Explicit probe and immutable runtime-mode instructions remain active. This option
    /// belongs to native bootstrap composition and is not a per-run authority control.
    #[must_use]
    pub fn without_automatic_agent_instructions_for_diagnostics(mut self) -> Self {
        self.automatic_agent_instructions = false;
        self
    }

    /// Prevent configured provider/authentication origins from activating general model fetch tools.
    #[must_use]
    pub fn without_model_network_tools(mut self) -> Self {
        self.model_network_tools = false;
        self
    }

    /// Whether trusted composition allows general model-visible network tools.
    pub const fn model_network_tools_enabled(&self) -> bool {
        self.model_network_tools
    }

    /// Attach the absolute Colossus home resolved by a trusted interface adapter.
    pub fn with_colossus_home(mut self, home: impl Into<PathBuf>) -> Result<Self, RuntimeError> {
        let home = home.into();
        if !home.is_absolute() {
            return Err(RuntimeError::Config(
                "the explicit Colossus home must be an absolute path".into(),
            ));
        }
        let root = ConfinedRoot::bind(&home).map_err(|error| {
            RuntimeError::Config(format!("the explicit Colossus home is unsafe: {error}"))
        })?;
        self.colossus_home = Some(root.path().to_owned());
        self.colossus_home_root = Some(root);
        Ok(self)
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
        assert!(selected.colossus_home.is_none());
        assert!(selected.colossus_home_root.is_none());
        assert!(selected.automatic_agent_instructions);
    }

    #[test]
    fn diagnostic_suppression_is_explicit_and_survives_canonicalization() {
        let workspace = tempfile::tempdir().expect("workspace");
        let selected = RuntimeOpenOptions::for_workspace(workspace.path())
            .expect("workspace options")
            .without_automatic_agent_instructions_for_diagnostics()
            .canonicalized()
            .expect("canonicalized options");
        assert!(!selected.automatic_agent_instructions);
    }

    #[test]
    fn workspace_options_accept_an_explicit_non_ambient_colossus_home() {
        let workspace = tempfile::tempdir().expect("workspace");
        let home = tempfile::tempdir().expect("home");
        let home_path = home.path().canonicalize().expect("canonical home");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            fs::set_permissions(&home_path, fs::Permissions::from_mode(0o700))
                .expect("private home permissions");
        }
        let selected = RuntimeOpenOptions::for_workspace(workspace.path())
            .expect("workspace options")
            .with_colossus_home(&home_path)
            .expect("absolute home")
            .canonicalized()
            .expect("canonicalized options");
        assert_eq!(selected.colossus_home.as_deref(), Some(home_path.as_path()));
        assert_eq!(
            selected.colossus_home_root.as_ref().map(ConfinedRoot::path),
            Some(home_path.as_path())
        );
        assert!(
            RuntimeOpenOptions::for_workspace(workspace.path())
                .expect("workspace options")
                .with_colossus_home("relative-home")
                .is_err()
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
