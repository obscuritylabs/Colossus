//! Fixed, bounded workspace discovery. Directory presence grants no authority.

use super::*;
use colossus_contracts::PluginOrigin;
use colossus_home::{WorkspaceIdentity, detect_workspace_identity};

mod discovery;
pub use discovery::{discover_workspace_plugins, discover_workspace_plugins_with_icon_budget};

#[cfg(test)]
mod tests;

/// Maximum local sources inspected in one discovery.
pub const MAX_WORKSPACE_PLUGINS: usize = 128;
/// Aggregate source bytes admitted by automatic workspace discovery.
pub const MAX_WORKSPACE_PLUGIN_BYTES: u64 = 256 * 1024 * 1024;

/// Object and path identity of an explicitly accepted mutable source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePluginSource {
    /// Normalized path beneath the selected workspace.
    pub path: String,
    /// Portable manifest name; a name change requires a new acceptance.
    pub name: String,
    /// Platform directory identity derivation version.
    pub identity_version: u16,
    /// Opaque directory identity, preventing acceptance after root replacement.
    pub identity_sha256: String,
}

/// A validated directory and its captured deterministic package bytes.
pub struct WorkspacePluginCandidate {
    /// Exact source identity presented for acceptance.
    pub source: WorkspacePluginSource,
    /// Discovery metadata. Instructions remain unavailable until accepted.
    pub record: AgentPluginRecord,
    /// Immutable bytes to publish after source authorization.
    pub artifact: BuiltPluginArtifact,
    /// Conservative extracted-content, blob and filesystem-overhead cache cost.
    pub cache_bytes: u64,
    identity: WorkspaceIdentity,
}

impl WorkspacePluginCandidate {
    /// Prove that the source root has not been replaced since capture.
    pub fn revalidate(&self) -> Result<(), StoreError> {
        self.identity.revalidate().map_err(adapter)
    }
}

/// Rejected source metadata, without untrusted file contents.
pub struct WorkspacePluginIssue {
    /// Workspace-relative source path.
    pub path: String,
    /// Bounded diagnostic suitable for authorized inventory.
    pub detail: String,
}

/// Independent discovery outcomes for valid and rejected workspace sources.
#[derive(Default)]
pub struct WorkspacePluginDiscovery {
    /// Valid candidates in deterministic source-path order.
    pub candidates: Vec<WorkspacePluginCandidate>,
    /// Rejected sources; these do not disable unrelated candidates.
    pub issues: Vec<WorkspacePluginIssue>,
}

/// Bind a normalized directory beneath a workspace, rejecting linked ancestry.
pub fn workspace_plugin_root(workspace: &Path, relative: &Path) -> Result<PathBuf, StoreError> {
    let workspace = ReadRoot::bind(workspace)?;
    posix_path(relative)?;
    if relative.as_os_str().is_empty() {
        return Err(adapter("workspace plugin source path is empty"));
    }
    let mut path = workspace.path().to_owned();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(adapter(
                "workspace plugin source must stay within the workspace",
            ));
        };
        path.push(name);
        let _binding = ReadRoot::bind(&path)?;
    }
    workspace.revalidate()?;
    Ok(path)
}

/// Capture one source after validating its object identity and byte bounds.
pub fn capture_workspace_plugin(
    workspace: &Path,
    relative: &Path,
) -> Result<WorkspacePluginCandidate, StoreError> {
    let mut remaining = MAX_WORKSPACE_PLUGIN_BYTES;
    capture_with_budget(
        workspace,
        relative,
        &mut remaining,
        &mut crate::icons::IconBudget::default(),
    )
}

fn capture_with_budget(
    workspace: &Path,
    relative: &Path,
    remaining: &mut u64,
    icons: &mut crate::icons::IconBudget,
) -> Result<WorkspacePluginCandidate, StoreError> {
    let root = workspace_plugin_root(workspace, relative)?;
    let identity = detect_workspace_identity(&root).map_err(adapter)?;
    let mut files = Vec::new();
    collect_regular_files(&root, &root, 0, &mut files)?;
    let reader = ReadRoot::bind(&root)?;
    let mut owned = Vec::new();
    for relative in files {
        let file = reader.open_file(&relative, (*remaining).min(MAX_FILE_BYTES))?;
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt as _;
            file.metadata().map_err(adapter)?.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        let mut bytes = Vec::new();
        file.take(remaining.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(adapter)?;
        *remaining = remaining
            .checked_sub(bytes.len() as u64)
            .ok_or_else(|| adapter("workspace plugin discovery exceeds 256 MiB"))?;
        owned.push((posix_path(&relative)?, bytes, executable));
    }
    reader.revalidate()?;
    let files = owned
        .iter()
        .map(|(path, bytes, executable)| PluginFile {
            path,
            bytes,
            executable: *executable,
        })
        .collect::<Vec<_>>();
    let artifact = build_plugin_artifact_from_files(&files)?;
    let cache_bytes = owned
        .iter()
        .fold(64 * 1024_u64, |total, (path, bytes, _)| {
            total
                .saturating_add(bytes.len() as u64)
                .saturating_add(4096_u64.saturating_mul(path.split('/').count() as u64))
        })
        // A filesystem without hard links copies blobs into the retained layout.
        .saturating_add(2_u64.saturating_mul(artifact.layer.len() as u64))
        .saturating_add(2_u64.saturating_mul(artifact.manifest.len() as u64))
        .saturating_add(2_u64.saturating_mul(artifact.config.len() as u64));
    let temporary = tempfile::tempdir().map_err(adapter)?;
    let captured = temporary.path().join("plugin");
    extract_plugin_artifact(&artifact, &captured)?;
    let mut record = load_plugin_with_icon_budget(&captured, icons)?;
    if record.installation.manifest.name == "colossus" {
        return Err(adapter(
            "colossus is reserved for the executable-bundled plugin",
        ));
    }
    let config: colossus_contracts::AgentPluginOciConfig =
        serde_json::from_slice(&artifact.config).map_err(adapter)?;
    if config.name != record.installation.manifest.name {
        return Err(adapter("workspace plugin changed during discovery"));
    }
    identity.revalidate().map_err(adapter)?;
    workspace_plugin_root(workspace, relative)?;
    let source = WorkspacePluginSource {
        path: posix_path(relative)?,
        name: config.name,
        identity_version: identity.version(),
        identity_sha256: identity.sha256().into(),
    };
    record.installation.origin = PluginOrigin::Workspace;
    record.installation.root = root.display().to_string();
    record.installation.digest = artifact.manifest_digest.clone();
    record.installation.source = source.path.clone();
    record.installation.trust.method = "workspace-directory".into();
    Ok(WorkspacePluginCandidate {
        source,
        record,
        artifact,
        cache_bytes,
        identity,
    })
}
