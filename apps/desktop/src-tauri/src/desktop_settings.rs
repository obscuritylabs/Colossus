use colossus_home::{ColossusHome, HomeError, HomeSurface, WorkspaceIdentityRef};
use colossus_sdk::{
    REMOTE_PROVIDER_TIMEOUT_MS, WorkspaceIdentity, default_managed_provider_timeout_ms,
    validate_managed_model_identifier, validate_managed_provider_base_url,
};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};
#[cfg(not(windows))]
use std::{fs::OpenOptions, io::Write as _};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::dto::CommandErrorDto;

use std::fs::File;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};

const SETTINGS_SCHEMA_VERSION: u16 = 5;
const SETTINGS_FILE: &str = "settings.json";
const THREAD_SEARCH_FILE: &str = "thread-search.redb";
const MANAGED_DIRECTORY: &str = "managed-local";
const TRUST_DIRECTORY: &str = "trust";
const MAX_CA_BUNDLE_BYTES: u64 = 4 * 1024 * 1024;
const SELF_TEST_DIRECTORY: &str = "self-test";
const SELF_TEST_RUNTIME_DIRECTORY: &str = "runtime-v2";
const SELF_TEST_WORKSPACE_DIRECTORY: &str = "workspace";
const CODEX_AUTH_DIRECTORY: &str = "codex-auth";
const MAX_SETTINGS_BYTES: u64 = 256 * 1024;
const MAX_PROVIDER_SECRET_BYTES: usize = 761;
pub(crate) const LOCAL_TERMINAL_CONSENT_VERSION: u8 = 1;
pub(crate) const MAX_EXTERNAL_TARGETS: usize = 32;
const MAX_EXTERNAL_LABEL_BYTES: usize = 80;
const MAX_CONNECTION_PATH_BYTES: usize = 2_048;
const MAX_CONNECTION_VALUE_BYTES: usize = 256;
pub(crate) const MAX_PENDING_PROVIDER_CLEANUPS: usize = 64;
pub(crate) const MAX_MANAGED_PROVIDERS: usize = 16;
pub(crate) const MAX_MANAGED_MODELS: usize = 64;
pub(crate) const MAX_WORKSPACE_PROFILES: usize = 128;
const MAX_WORKSPACE_PROFILE_NAME_BYTES: usize = 80;
const PROVIDER_KEYRING_SERVICE: &str = "com.obscuritylabs.colossus.desktop.provider";
pub(crate) const EXTERNAL_KEYRING_SERVICE: &str = "com.obscuritylabs.colossus.desktop.external";
const WORKSPACE_PARTITION_DOMAIN: &[u8] = b"colossus-desktop-managed-workspace-v1\0";
const WORKSPACE_INSTANCE_DOMAIN: &[u8] = b"colossus-desktop-managed-instance-v1\0";
#[cfg(debug_assertions)]
const DEVELOPMENT_PLAINTEXT_RUNTIME_DIRECTORY: &str = "development-plaintext";
#[cfg(debug_assertions)]
const DEVELOPMENT_PLAINTEXT_INSTANCE_DOMAIN: &[u8] =
    b"colossus-desktop-managed-development-plaintext-v1\0";
const MODEL_ROLES: [&str; 7] = [
    "primary",
    "risk_evaluator",
    "context_summarizer",
    "subagent_default",
    "research_planner",
    "research_worker",
    "research_synthesizer",
];
pub(crate) const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub(crate) const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub(crate) const CODEX_BASE_URL: &str = colossus_codex_auth::CODEX_API_BASE_URL;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccessProfileSetting {
    Minimal,
    Development,
    #[default]
    AllowAll,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecutionBoundarySetting {
    #[default]
    FullAccess,
    WorkspaceIsolated,
    OfflineIsolated,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum ProviderKindSetting {
    #[serde(rename = "openai_responses", alias = "open_ai_responses")]
    Responses,
    #[serde(rename = "openai_compatible", alias = "open_ai_compatible")]
    Compatible,
    #[serde(rename = "open_ai_codex")]
    Codex,
}

pub(crate) const fn provider_base_url(kind: ProviderKindSetting) -> &'static str {
    match kind {
        ProviderKindSetting::Responses => OPENAI_BASE_URL,
        ProviderKindSetting::Compatible => OPENROUTER_BASE_URL,
        ProviderKindSetting::Codex => CODEX_BASE_URL,
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReasoningEffortSetting {
    None,
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
    Ultra,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspaceSetting {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    /// Opaque native-only identity of the directory selected by the user. This is
    /// persisted so replacing a pathname cannot inherit the prior runtime state.
    #[serde(default)]
    pub(crate) identity: Option<WorkspaceIdentity>,
    pub(crate) display_name: String,
    pub(crate) display_path: String,
}

/// Persisted folder-backed Desktop context. The renderer calls this a Space, while
/// the neutral native name keeps a future product-label change migration-free.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspaceProfile {
    pub(crate) id: String,
    pub(crate) display_name: String,
    #[serde(default)]
    pub(crate) archived: bool,
    #[serde(default)]
    pub(crate) last_opened_at_ms: u64,
    pub(crate) workspace: WorkspaceSetting,
    #[serde(default)]
    pub(crate) providers: Vec<ProviderSetting>,
    #[serde(default)]
    pub(crate) models: Vec<ModelSetting>,
    #[serde(default)]
    pub(crate) model_roles: BTreeMap<String, String>,
    pub(crate) access_profile: AccessProfileSetting,
    pub(crate) execution_boundary: ExecutionBoundarySetting,
    pub(crate) terminal_enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderSetting {
    pub(crate) profile: String,
    pub(crate) kind: ProviderKindSetting,
    pub(crate) base_url: String,
    pub(crate) credential_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) timeout_ms: Option<u64>,
}

impl ProviderSetting {
    pub(crate) fn effective_timeout_ms(&self) -> u64 {
        self.timeout_ms.unwrap_or_else(|| {
            default_managed_provider_timeout_ms(&self.base_url)
                .unwrap_or(REMOTE_PROVIDER_TIMEOUT_MS)
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModelCapabilitiesSetting {
    pub(crate) tool_calls: bool,
    pub(crate) streaming: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModelSetting {
    pub(crate) profile: String,
    pub(crate) provider_profile: String,
    pub(crate) model: String,
    pub(crate) context_window_tokens: u64,
    pub(crate) max_output_tokens: u64,
    pub(crate) capabilities: ModelCapabilitiesSetting,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<ReasoningEffortSetting>,
}

pub(crate) struct SelfTestStorage {
    pub(crate) instance_dir: PathBuf,
    pub(crate) workspace: PathBuf,
}

/// Native-only trust anchors for a saved daemon connection.
///
/// This value is persisted only in the owner-private desktop settings file. The
/// renderer receives the opaque target ID and display label, never discovery
/// paths or keyring lookup labels.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExternalTargetSetting {
    pub(crate) target_id: String,
    pub(crate) label: String,
    pub(crate) instance_id: String,
    pub(crate) certificate_sha256: String,
    pub(crate) public_api_dir: PathBuf,
    pub(crate) credential_service: String,
    pub(crate) credential_account: String,
    #[serde(default = "legacy_external_credential_binding")]
    pub(crate) requires_credential_enrollment: bool,
}

/// Native-only location and renderer-safe metadata for an imported trust bundle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CaBundleSetting {
    pub(crate) bundle_id: String,
    pub(crate) certificate_count: usize,
    pub(crate) fingerprints_sha256: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AsideSetting {
    pub(crate) space_id: String,
    pub(crate) parent_session_id: String,
    pub(crate) source_run_id: String,
    pub(crate) session_id: String,
    pub(crate) latest_run_id: String,
    pub(crate) created_at: String,
    #[serde(default)]
    pub(crate) closed: bool,
}

const fn legacy_external_credential_binding() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopSettings {
    pub(crate) schema_version: u16,
    pub(crate) managed_instance_id: String,
    /// Persisted folder-backed contexts. These are authoritative for Managed Local.
    #[serde(default)]
    pub(crate) spaces: Vec<WorkspaceProfile>,
    #[serde(default)]
    pub(crate) selected_space_id: Option<String>,
    /// Bounded linkage metadata for Space-scoped side conversations. No prompt,
    /// message, tool output, or selected text is persisted here.
    #[serde(default)]
    pub(crate) asides: Vec<AsideSetting>,
    /// Selected-Space projection retained for narrow command paths. `SettingsStore`
    /// synchronizes it into `spaces` before every write and refreshes it after load.
    pub(crate) workspace: Option<WorkspaceSetting>,
    #[serde(default)]
    pub(crate) providers: Vec<ProviderSetting>,
    #[serde(default)]
    pub(crate) models: Vec<ModelSetting>,
    #[serde(default)]
    pub(crate) model_roles: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) pending_provider_cleanup_ids: Vec<String>,
    #[serde(default)]
    pub(crate) additional_ca_bundle: Option<CaBundleSetting>,
    pub(crate) access_profile: AccessProfileSetting,
    pub(crate) execution_boundary: ExecutionBoundarySetting,
    pub(crate) terminal_enabled: bool,
    /// Versioned native confirmation for local-user shell authority. A missing or
    /// older value keeps previously persisted TUI consent from silently enabling
    /// the more powerful shell surface.
    #[serde(default)]
    pub(crate) local_terminal_consent_version: u8,
    pub(crate) selected_target_id: Option<String>,
    #[serde(default)]
    pub(crate) external_targets: Vec<ExternalTargetSetting>,
    #[serde(default)]
    pub(crate) legacy_connection_migrated: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyDesktopSettingsV1 {
    schema_version: u16,
    managed_instance_id: String,
    workspace: Option<WorkspaceSetting>,
    provider: Option<LegacyProviderSettingV1>,
    #[serde(default)]
    pending_provider_cleanup_ids: Vec<String>,
    access_profile: AccessProfileSetting,
    terminal_enabled: bool,
    selected_target_id: Option<String>,
    #[serde(default)]
    external_targets: Vec<ExternalTargetSetting>,
    #[serde(default)]
    legacy_connection_migrated: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyProviderSettingV1 {
    #[serde(rename = "kind")]
    _kind: ProviderKindSetting,
    #[serde(rename = "model")]
    _model: String,
    #[serde(rename = "baseUrl")]
    _base_url: String,
    credential_id: String,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            managed_instance_id: Uuid::now_v7().to_string(),
            spaces: Vec::new(),
            selected_space_id: None,
            asides: Vec::new(),
            workspace: None,
            providers: Vec::new(),
            models: Vec::new(),
            model_roles: BTreeMap::new(),
            pending_provider_cleanup_ids: Vec::new(),
            additional_ca_bundle: None,
            access_profile: AccessProfileSetting::AllowAll,
            execution_boundary: ExecutionBoundarySetting::FullAccess,
            terminal_enabled: false,
            local_terminal_consent_version: 0,
            selected_target_id: None,
            external_targets: Vec::new(),
            legacy_connection_migrated: false,
        }
    }
}

impl DesktopSettings {
    pub(crate) fn local_terminal_enabled(&self) -> bool {
        self.terminal_enabled
            && self.local_terminal_consent_version == LOCAL_TERMINAL_CONSENT_VERSION
    }

    pub(crate) fn managed_configured(&self) -> bool {
        self.primary_model().is_some() && self.primary_provider().is_some()
    }

    pub(crate) fn primary_model(&self) -> Option<&ModelSetting> {
        let profile = self.model_roles.get("primary")?;
        self.models.iter().find(|model| &model.profile == profile)
    }

    pub(crate) fn primary_provider(&self) -> Option<&ProviderSetting> {
        let model = self.primary_model()?;
        self.providers
            .iter()
            .find(|provider| provider.profile == model.provider_profile)
    }

    pub(crate) fn provider_credential_ids(&self) -> BTreeSet<&str> {
        let selected = self
            .providers
            .iter()
            .filter_map(|provider| provider.credential_id.as_deref())
            .collect::<BTreeSet<_>>();
        self.spaces.iter().fold(selected, |mut ids, space| {
            ids.extend(
                space
                    .providers
                    .iter()
                    .filter_map(|provider| provider.credential_id.as_deref()),
            );
            ids
        })
    }

    pub(crate) fn space(&self, space_id: &str) -> Option<&WorkspaceProfile> {
        self.spaces.iter().find(|space| space.id == space_id)
    }

    pub(crate) fn space_for_workspace_identity(
        &self,
        identity: &WorkspaceIdentity,
    ) -> Option<&WorkspaceProfile> {
        self.spaces
            .iter()
            .find(|space| space.workspace.identity.as_ref() == Some(identity))
    }

    pub(crate) fn add_space(
        &mut self,
        workspace: WorkspaceSetting,
    ) -> Result<String, CommandErrorDto> {
        self.sync_selected_space_projection()?;
        if self.spaces.len() >= MAX_WORKSPACE_PROFILES {
            return Err(CommandErrorDto::busy(
                "The Desktop Space limit has been reached. Archive an unused Space first.",
            ));
        }
        let identity = workspace.identity.as_ref().ok_or_else(workspace_error)?;
        if self.space_for_workspace_identity(identity).is_some() {
            return Err(CommandErrorDto::invalid(
                "workspace",
                "That folder already belongs to a Space.",
            ));
        }
        let id = workspace.id.clone();
        self.spaces.push(WorkspaceProfile {
            id: id.clone(),
            display_name: workspace.display_name.clone(),
            archived: false,
            last_opened_at_ms: unix_time_millis(),
            workspace,
            providers: self.providers.clone(),
            models: self.models.clone(),
            model_roles: self.model_roles.clone(),
            access_profile: self.access_profile,
            execution_boundary: self.execution_boundary,
            terminal_enabled: self.terminal_enabled,
        });
        self.activate_space(&id)?;
        Ok(id)
    }

    pub(crate) fn activate_space(&mut self, space_id: &str) -> Result<(), CommandErrorDto> {
        self.sync_selected_space_projection()?;
        let Some(index) = self.spaces.iter().position(|space| space.id == space_id) else {
            return Err(CommandErrorDto::invalid("spaceId", "The Space is unknown."));
        };
        if self.spaces[index].archived {
            return Err(CommandErrorDto::invalid(
                "spaceId",
                "Restore this Space before selecting it.",
            ));
        }
        self.spaces[index].last_opened_at_ms = unix_time_millis();
        self.selected_space_id = Some(space_id.to_owned());
        self.project_selected_space();
        self.selected_target_id = Some(space_id.to_owned());
        Ok(())
    }

    pub(crate) fn sync_selected_space_projection(&mut self) -> Result<(), CommandErrorDto> {
        let Some(space_id) = self.selected_space_id.clone() else {
            return Ok(());
        };
        let Some(space) = self.spaces.iter_mut().find(|space| space.id == space_id) else {
            return Err(storage_error());
        };
        if let Some(workspace) = &self.workspace {
            space.workspace = workspace.clone();
        }
        space.providers.clone_from(&self.providers);
        space.models.clone_from(&self.models);
        space.model_roles.clone_from(&self.model_roles);
        space.access_profile = self.access_profile;
        space.execution_boundary = self.execution_boundary;
        space.terminal_enabled = self.terminal_enabled;
        Ok(())
    }

    pub(crate) fn project_selected_space(&mut self) {
        let selected = self
            .selected_space_id
            .as_deref()
            .and_then(|space_id| self.spaces.iter().find(|space| space.id == space_id))
            .cloned();
        let Some(space) = selected else {
            // A provider may be configured before the first folder is selected.
            // Preserve that staged projection so Add Space can inherit it and so
            // legacy path-only migrations do not silently discard provider state.
            return;
        };
        self.workspace = Some(space.workspace);
        self.providers = space.providers;
        self.models = space.models;
        self.model_roles = space.model_roles;
        self.access_profile = space.access_profile;
        self.execution_boundary = space.execution_boundary;
        self.terminal_enabled = space.terminal_enabled;
    }

    fn migrate_workspace_to_space(&mut self) -> bool {
        if !self.spaces.is_empty() || self.workspace.is_none() {
            return false;
        }
        let workspace = self.workspace.clone().expect("workspace checked above");
        let id = workspace.id.clone();
        self.spaces.push(WorkspaceProfile {
            id: id.clone(),
            display_name: workspace.display_name.clone(),
            archived: false,
            last_opened_at_ms: unix_time_millis(),
            workspace,
            providers: self.providers.clone(),
            models: self.models.clone(),
            model_roles: self.model_roles.clone(),
            access_profile: self.access_profile,
            execution_boundary: self.execution_boundary,
            terminal_enabled: self.terminal_enabled,
        });
        self.selected_space_id = Some(id.clone());
        if self.selected_target_id.as_deref() == Some("managed-local") {
            self.selected_target_id = Some(id);
        }
        true
    }
}

fn unix_time_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn settings_schema_version(bytes: &[u8]) -> Result<u16, CommandErrorDto> {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| {
            value
                .get("schemaVersion")
                .and_then(serde_json::Value::as_u64)
        })
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(storage_error)
}

#[derive(Clone, Debug)]
pub(crate) struct SettingsStore {
    root: PathBuf,
    home: Option<ColossusHome>,
}

pub(crate) struct ManagedWorkspaceStorage {
    pub(crate) instance_id: String,
    pub(crate) instance_dir: PathBuf,
}

impl SettingsStore {
    pub(crate) fn open_application() -> Result<Self, CommandErrorDto> {
        let home = ColossusHome::resolve_and_ensure().map_err(home_storage_error)?;
        Self::open_home(home)
    }

    pub(crate) fn open(root: PathBuf) -> Result<Self, CommandErrorDto> {
        ensure_private_directory(&root)?;
        Ok(Self { root, home: None })
    }

    fn open_home(home: ColossusHome) -> Result<Self, CommandErrorDto> {
        let root = home.desktop_root().map_err(|_| storage_error())?;
        let mut store = Self::open(root)?;
        store.home = Some(home);
        Ok(store)
    }

    pub(crate) fn home_root(&self) -> Result<&Path, CommandErrorDto> {
        self.home
            .as_ref()
            .map(ColossusHome::root)
            .ok_or_else(storage_error)
    }

    pub(crate) fn thread_search_path(&self) -> Result<PathBuf, CommandErrorDto> {
        ensure_private_directory(&self.root)?;
        Ok(self.root.join(THREAD_SEARCH_FILE))
    }

    pub(crate) fn open_thread_search_file(&self) -> Result<File, CommandErrorDto> {
        let path = self.thread_search_path()?;
        if !path.exists() {
            write_private_file(&path, &[])?;
        }
        #[cfg(unix)]
        {
            let file = rustix::fs::open(
                &path,
                rustix::fs::OFlags::RDWR
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map(File::from)
            .map_err(|_| storage_error())?;
            let metadata = file.metadata().map_err(|_| storage_error())?;
            if !metadata.is_file()
                || metadata.uid() != rustix::process::getuid().as_raw()
                || metadata.mode() & 0o077 != 0
            {
                return Err(storage_error());
            }
            return Ok(file);
        }
        #[cfg(windows)]
        {
            let binding = colossus_windows_native::BoundPath::open_file(&path)
                .map_err(|_| storage_error())?;
            binding
                .validate_private_owner_dacl()
                .and_then(|()| binding.revalidate())
                .map_err(|_| storage_error())?;
            return binding.try_clone_file().map_err(|_| storage_error());
        }
        #[allow(unreachable_code)]
        Err(storage_error())
    }

    pub(crate) fn load(&self) -> Result<DesktopSettings, CommandErrorDto> {
        let path = self.root.join(SETTINGS_FILE);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DesktopSettings::default());
            }
            Err(_) => return Err(storage_error()),
        };
        if !metadata.file_type().is_file() || metadata.len() > MAX_SETTINGS_BYTES {
            return Err(storage_error());
        }
        #[cfg(unix)]
        if metadata.uid() != rustix::process::getuid().as_raw() || metadata.mode() & 0o077 != 0 {
            return Err(storage_error());
        }
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
        #[cfg(windows)]
        let binding =
            colossus_windows_native::BoundPath::open_file(&path).map_err(|_| storage_error())?;
        #[cfg(windows)]
        binding
            .validate_private_owner_dacl()
            .map_err(|_| storage_error())?;
        #[cfg(windows)]
        let source = binding.try_clone_file().map_err(|_| storage_error())?;
        #[cfg(not(windows))]
        let source = File::open(&path).map_err(|_| storage_error())?;
        source
            .take(MAX_SETTINGS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| storage_error())?;
        #[cfg(windows)]
        binding.revalidate().map_err(|_| storage_error())?;
        #[cfg(windows)]
        // Migration may atomically replace this file below. Release the validated read
        // handle first so Windows can detach the old destination name.
        drop(binding);
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SETTINGS_BYTES {
            return Err(storage_error());
        }
        let schema_version = settings_schema_version(&bytes)?;
        let (mut settings, mut migrated_settings) = if schema_version == 1 {
            let legacy: LegacyDesktopSettingsV1 =
                serde_json::from_slice(&bytes).map_err(|_| storage_error())?;
            (migrate_v1_settings(legacy)?, true)
        } else if matches!(schema_version, 2 | 3) {
            (migrate_legacy_settings(&bytes, schema_version)?, true)
        } else if schema_version == 4 {
            (migrate_v4_settings(&bytes)?, true)
        } else {
            (
                serde_json::from_slice(&bytes).map_err(|_| storage_error())?,
                false,
            )
        };
        migrated_settings |= settings.migrate_workspace_to_space();
        settings.project_selected_space();
        let legacy_workspace_requires_reselection =
            settings.workspace.as_ref().is_some_and(|workspace| {
                workspace
                    .identity
                    .as_ref()
                    .is_none_or(WorkspaceIdentity::is_legacy_v1)
            });
        let mut migrated_legacy_workspace = false;
        if legacy_workspace_requires_reselection {
            // Preview builds persisted only a pathname, which cannot prove that the
            // directory now at that path is the one the user selected. Never bind the
            // current occupant implicitly. Rotate old runtime state and require a fresh
            // native folder selection; provider configuration remains available only
            // after that explicit workspace authorization.
            settings.managed_instance_id = Uuid::now_v7().to_string();
            settings.workspace = None;
            if let Some(space_id) = settings.selected_space_id.take() {
                settings.spaces.retain(|space| space.id != space_id);
            }
            if settings.selected_target_id.as_deref() == Some("managed-local")
                || settings
                    .selected_target_id
                    .as_ref()
                    .is_some_and(|target_id| {
                        settings.spaces.iter().all(|space| &space.id != target_id)
                    })
            {
                settings.selected_target_id = None;
            }
            migrated_legacy_workspace = true;
        }
        // Older builds persisted caller-selected keyring labels. Normalize them in
        // native memory before validation and retain an explicit enrollment marker;
        // never copy a secret from an untrusted legacy selector.
        for target in &mut settings.external_targets {
            if canonicalize_external_credential_binding(target) {
                target.requires_credential_enrollment = true;
            }
        }
        validate_settings(&settings)?;
        if let Some(bundle) = &settings.additional_ca_bundle {
            self.ca_bundle_path(bundle)?;
        }
        if migrated_legacy_workspace || migrated_settings {
            self.save(&settings)?;
        }
        Ok(settings)
    }

    pub(crate) fn has_persisted_settings(&self) -> bool {
        self.root.join(SETTINGS_FILE).is_file()
    }

    pub(crate) fn save(&self, settings: &DesktopSettings) -> Result<(), CommandErrorDto> {
        let mut persisted = settings.clone();
        persisted.sync_selected_space_projection()?;
        validate_settings(&persisted)?;
        let bytes = serde_json::to_vec(&persisted).map_err(|_| storage_error())?;
        if bytes.len() > usize::try_from(MAX_SETTINGS_BYTES).unwrap_or(usize::MAX) {
            return Err(storage_error());
        }
        let temporary = self
            .root
            .join(format!(".{SETTINGS_FILE}.{}.tmp", Uuid::new_v4()));
        write_private_file(&temporary, &bytes)?;
        let result = (|| {
            replace_private_file(&temporary, &self.root.join(SETTINGS_FILE))?;
            sync_private_directory(&self.root)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    pub(crate) fn managed_workspace_storage(
        &self,
        instance_seed: &str,
        workspace: &Path,
        expected_identity: &WorkspaceIdentity,
    ) -> Result<ManagedWorkspaceStorage, CommandErrorDto> {
        let seed = Uuid::parse_str(instance_seed).map_err(|_| storage_error())?;
        if seed.is_nil() {
            return Err(storage_error());
        }
        let (canonical, identity) = open_workspace_identity(workspace)?;
        if canonical != workspace || &identity != expected_identity {
            return Err(workspace_error());
        }
        let canonical_workspace = canonical.to_str().ok_or_else(workspace_error)?;
        let mut partition_digest = Sha256::new();
        partition_digest.update(WORKSPACE_PARTITION_DOMAIN);
        partition_digest.update(canonical_workspace.as_bytes());
        partition_digest.update(identity.version.to_le_bytes());
        partition_digest.update(identity.sha256.as_bytes());
        let partition = partition_digest.finalize();
        let (directory, instance_partition) = if let Some(home) = &self.home {
            let identity = WorkspaceIdentityRef {
                version: identity.version,
                sha256: &identity.sha256,
            };
            let partition_id = home
                .workspace_partition_id(&canonical, identity)
                .map_err(|_| storage_error())?;
            let directory = home
                .workspace_surface_dir(&canonical, identity, HomeSurface::Desktop)
                .map_err(|_| storage_error())?;
            (directory, partition_id.into_bytes())
        } else {
            let managed_root = self.root.join(MANAGED_DIRECTORY);
            ensure_private_directory(&managed_root)?;
            let partition_id = hex::encode(&partition[..]);
            let directory = managed_root.join(&partition_id);
            ensure_private_directory(&directory)?;
            (directory, partition_id.into_bytes())
        };
        #[cfg(debug_assertions)]
        let directory = {
            let directory = directory.join(DEVELOPMENT_PLAINTEXT_RUNTIME_DIRECTORY);
            ensure_private_directory(&directory)?;
            directory
        };

        let mut instance_digest = Sha256::new();
        instance_digest.update(WORKSPACE_INSTANCE_DOMAIN);
        #[cfg(debug_assertions)]
        instance_digest.update(DEVELOPMENT_PLAINTEXT_INSTANCE_DOMAIN);
        instance_digest.update(seed.as_bytes());
        instance_digest.update(instance_partition);
        let digest = instance_digest.finalize();
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        // RFC 9562 UUIDv8 provides a stable custom namespace without pretending this
        // SHA-256 derivation is name-based UUIDv5.
        bytes[6] = (bytes[6] & 0x0f) | 0x80;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(ManagedWorkspaceStorage {
            instance_id: Uuid::from_bytes(bytes).to_string(),
            instance_dir: directory,
        })
    }

    pub(crate) fn stage_ca_bundle(
        &self,
        source_path: &Path,
    ) -> Result<CaBundleSetting, CommandErrorDto> {
        let bytes = read_ca_bundle_source(source_path)?;
        let roots = colossus_network::AdditionalRootCertificates::from_pem_bundle(&bytes)
            .map_err(|_| ca_bundle_error("The selected file is not a valid PEM CA bundle."))?;
        let bundle = CaBundleSetting {
            bundle_id: Uuid::now_v7().to_string(),
            certificate_count: roots.len(),
            fingerprints_sha256: roots.fingerprints_sha256(),
        };
        let directory = self.root.join(TRUST_DIRECTORY);
        ensure_private_directory(&directory)?;
        let destination = ca_bundle_storage_path(&directory, &bundle)?;
        write_private_file(&destination, &bytes)?;
        self.ca_bundle_path(&bundle)?;
        Ok(bundle)
    }

    pub(crate) fn ca_bundle_path(
        &self,
        bundle: &CaBundleSetting,
    ) -> Result<PathBuf, CommandErrorDto> {
        validate_ca_bundle_setting(bundle)?;
        let directory = self.root.join(TRUST_DIRECTORY);
        ensure_private_directory(&directory)?;
        let path = ca_bundle_storage_path(&directory, bundle)?;
        let bytes = read_private_file(&path, MAX_CA_BUNDLE_BYTES)?;
        let roots = colossus_network::AdditionalRootCertificates::from_pem_bundle(&bytes)
            .map_err(|_| ca_bundle_error("The imported CA bundle is no longer valid."))?;
        if roots.len() != bundle.certificate_count
            || roots.fingerprints_sha256() != bundle.fingerprints_sha256
        {
            return Err(ca_bundle_error(
                "The imported CA bundle no longer matches its saved identity.",
            ));
        }
        Ok(path)
    }

    pub(crate) fn delete_ca_bundle(&self, bundle: &CaBundleSetting) -> Result<(), CommandErrorDto> {
        let directory = self.root.join(TRUST_DIRECTORY);
        let path = ca_bundle_storage_path(&directory, bundle)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(storage_error()),
        }
    }

    pub(crate) fn self_test_storage(&self) -> Result<SelfTestStorage, CommandErrorDto> {
        let root = self.root.join(SELF_TEST_DIRECTORY);
        let instance_dir = root.join(SELF_TEST_RUNTIME_DIRECTORY);
        let workspace = root.join(SELF_TEST_WORKSPACE_DIRECTORY);
        ensure_private_directory(&root)?;
        ensure_private_directory(&instance_dir)?;
        ensure_private_directory(&workspace)?;
        Ok(SelfTestStorage {
            instance_dir,
            workspace,
        })
    }

    pub(crate) fn codex_auth_home(&self) -> Result<PathBuf, CommandErrorDto> {
        let path = self.root.join(CODEX_AUTH_DIRECTORY);
        ensure_private_directory(&path)?;
        Ok(path)
    }
}

pub(crate) fn validate_workspace(path: &Path) -> Result<WorkspaceSetting, CommandErrorDto> {
    let (canonical, identity) = open_workspace_identity(path)?;
    let display_name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(workspace_error)?
        .to_owned();
    let display_path = display_path(&canonical);
    Ok(WorkspaceSetting {
        id: Uuid::now_v7().to_string(),
        path: canonical,
        identity: Some(identity),
        display_name,
        display_path,
    })
}

pub(crate) fn revalidate_workspace(
    workspace: &WorkspaceSetting,
) -> Result<PathBuf, CommandErrorDto> {
    if !valid_opaque_id(&workspace.id) {
        return Err(workspace_error());
    }
    let (canonical, identity) = open_workspace_identity(&workspace.path)?;
    if workspace.identity.as_ref() != Some(&identity) {
        return Err(workspace_error());
    }
    Ok(canonical)
}

pub(crate) fn store_provider_secret(
    credential_id: &str,
    secret: &Zeroizing<String>,
) -> Result<(), CommandErrorDto> {
    if !valid_opaque_id(credential_id)
        || secret.is_empty()
        || secret.len() > MAX_PROVIDER_SECRET_BYTES
        || !secret
            .as_bytes()
            .iter()
            .all(|byte| (0x21..=0x7e).contains(byte))
    {
        return Err(CommandErrorDto::invalid(
            "apiKey",
            "The provider key must be bounded visible ASCII.",
        ));
    }
    keyring::Entry::new(PROVIDER_KEYRING_SERVICE, credential_id)
        .and_then(|entry| entry.set_secret(secret.as_bytes()))
        .map_err(|_| credential_error())
}

pub(crate) fn load_provider_secret(
    credential_id: &str,
) -> Result<Zeroizing<Vec<u8>>, CommandErrorDto> {
    if !valid_opaque_id(credential_id) {
        return Err(credential_error());
    }
    keyring::Entry::new(PROVIDER_KEYRING_SERVICE, credential_id)
        .and_then(|entry| entry.get_secret())
        .map(Zeroizing::new)
        .map_err(|_| credential_error())
}

pub(crate) fn delete_provider_secret(credential_id: &str) -> Result<(), CommandErrorDto> {
    if !valid_opaque_id(credential_id) {
        return Err(credential_error());
    }
    let entry = keyring::Entry::new(PROVIDER_KEYRING_SERVICE, credential_id)
        .map_err(|_| credential_error())?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(credential_error()),
    }
}

fn migrate_v1_settings(
    legacy: LegacyDesktopSettingsV1,
) -> Result<DesktopSettings, CommandErrorDto> {
    if legacy.schema_version != 1 {
        return Err(storage_error());
    }
    let mut pending = legacy.pending_provider_cleanup_ids;
    if let Some(provider) = legacy.provider {
        let LegacyProviderSettingV1 {
            _kind: _,
            _model: _,
            _base_url: _,
            credential_id,
        } = provider;
        if !pending.contains(&credential_id) {
            pending.push(credential_id);
        }
    }
    pending.sort();
    pending.dedup();
    if pending.len() > MAX_PENDING_PROVIDER_CLEANUPS {
        return Err(storage_error());
    }
    let access_profile = migrate_legacy_access_profile(legacy.access_profile);
    Ok(DesktopSettings {
        schema_version: SETTINGS_SCHEMA_VERSION,
        managed_instance_id: legacy.managed_instance_id,
        spaces: Vec::new(),
        selected_space_id: None,
        asides: Vec::new(),
        workspace: legacy.workspace,
        providers: Vec::new(),
        models: Vec::new(),
        model_roles: BTreeMap::new(),
        pending_provider_cleanup_ids: pending,
        additional_ca_bundle: None,
        access_profile,
        execution_boundary: legacy_execution_boundary(access_profile),
        terminal_enabled: legacy.terminal_enabled,
        local_terminal_consent_version: 0,
        selected_target_id: legacy
            .selected_target_id
            .filter(|target| target != "managed-local"),
        external_targets: legacy.external_targets,
        legacy_connection_migrated: legacy.legacy_connection_migrated,
    })
}

fn migrate_v4_settings(bytes: &[u8]) -> Result<DesktopSettings, CommandErrorDto> {
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| storage_error())?;
    let object = value.as_object_mut().ok_or_else(storage_error)?;
    if object
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(4)
    {
        return Err(storage_error());
    }
    object.insert(
        "schemaVersion".into(),
        serde_json::Value::from(SETTINGS_SCHEMA_VERSION),
    );
    object.insert("spaces".into(), serde_json::Value::Array(Vec::new()));
    object.insert("selectedSpaceId".into(), serde_json::Value::Null);
    serde_json::from_value(value).map_err(|_| storage_error())
}

fn migrate_legacy_settings(
    bytes: &[u8],
    expected_schema_version: u16,
) -> Result<DesktopSettings, CommandErrorDto> {
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| storage_error())?;
    let object = value.as_object_mut().ok_or_else(storage_error)?;
    if object
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(u64::from(expected_schema_version))
    {
        return Err(storage_error());
    }
    object.insert(
        "schemaVersion".into(),
        serde_json::Value::from(SETTINGS_SCHEMA_VERSION),
    );
    let legacy_access_profile = object
        .get("accessProfile")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(storage_error)?;
    let (access_profile, execution_boundary) = match legacy_access_profile {
        "minimal" => ("minimal", "offline_isolated"),
        "development" | "allow_all" => ("development", "workspace_isolated"),
        _ => return Err(storage_error()),
    };
    object.insert(
        "accessProfile".into(),
        serde_json::Value::from(access_profile),
    );
    // Schemas 1-3 always ran the managed runtime through the platform-isolating
    // backend. Preserve that boundary during migration instead of silently opting an
    // existing user into the new schema-v4 full-access default.
    object.insert(
        "executionBoundary".into(),
        serde_json::Value::from(execution_boundary),
    );
    serde_json::from_value(value).map_err(|_| storage_error())
}

const fn migrate_legacy_access_profile(profile: AccessProfileSetting) -> AccessProfileSetting {
    match profile {
        AccessProfileSetting::AllowAll => AccessProfileSetting::Development,
        AccessProfileSetting::Minimal | AccessProfileSetting::Development => profile,
    }
}

const fn legacy_execution_boundary(profile: AccessProfileSetting) -> ExecutionBoundarySetting {
    match profile {
        AccessProfileSetting::Minimal => ExecutionBoundarySetting::OfflineIsolated,
        AccessProfileSetting::Development | AccessProfileSetting::AllowAll => {
            ExecutionBoundarySetting::WorkspaceIsolated
        }
    }
}

fn validate_settings(settings: &DesktopSettings) -> Result<(), CommandErrorDto> {
    if settings.schema_version != SETTINGS_SCHEMA_VERSION
        || settings.local_terminal_consent_version > LOCAL_TERMINAL_CONSENT_VERSION
        || !Uuid::parse_str(&settings.managed_instance_id).is_ok_and(|value| !value.is_nil())
        || settings.external_targets.len() > MAX_EXTERNAL_TARGETS
        || settings.spaces.len() > MAX_WORKSPACE_PROFILES
        || settings.asides.len() > 256
        || settings.pending_provider_cleanup_ids.len() > MAX_PENDING_PROVIDER_CLEANUPS
        || settings
            .additional_ca_bundle
            .as_ref()
            .is_some_and(|bundle| validate_ca_bundle_setting(bundle).is_err())
    {
        return Err(storage_error());
    }
    validate_workspace_profiles(settings)?;
    let mut aside_sessions = HashSet::with_capacity(settings.asides.len());
    if settings.asides.iter().any(|aside| {
        !settings
            .spaces
            .iter()
            .any(|space| space.id == aside.space_id)
            || !valid_opaque_id(&aside.parent_session_id)
            || !valid_opaque_id(&aside.source_run_id)
            || !valid_opaque_id(&aside.session_id)
            || !valid_opaque_id(&aside.latest_run_id)
            || !aside_sessions.insert(aside.session_id.as_str())
            || aside.created_at.is_empty()
            || aside.created_at.len() > 64
    }) {
        return Err(storage_error());
    }
    let target_ids = validate_external_targets(settings)?;
    if settings.selected_target_id.as_deref().is_some_and(|value| {
        value != "managed-local"
            && settings.spaces.iter().all(|space| space.id != value)
            && !target_ids.contains(value)
    }) || settings.workspace.as_ref().is_some_and(invalid_workspace)
    {
        return Err(storage_error());
    }
    validate_managed_configuration(settings)?;
    let credential_ids = settings.provider_credential_ids();
    let mut cleanup_ids = HashSet::new();
    if settings
        .pending_provider_cleanup_ids
        .iter()
        .any(|credential_id| {
            !valid_opaque_id(credential_id)
                || !cleanup_ids.insert(credential_id)
                || credential_ids.contains(credential_id.as_str())
        })
    {
        return Err(storage_error());
    }
    Ok(())
}

fn validate_workspace_profiles(settings: &DesktopSettings) -> Result<(), CommandErrorDto> {
    let mut profile_ids = HashSet::with_capacity(settings.spaces.len());
    let mut workspace_identities = HashSet::with_capacity(settings.spaces.len());
    for space in &settings.spaces {
        let Some(identity) = space.workspace.identity.as_ref() else {
            return Err(storage_error());
        };
        let identity_key = (identity.version, identity.sha256.as_str());
        if !valid_opaque_id(&space.id)
            || !profile_ids.insert(space.id.as_str())
            || !workspace_identities.insert(identity_key)
            || space.display_name.trim().is_empty()
            || space.display_name.len() > MAX_WORKSPACE_PROFILE_NAME_BYTES
            || space
                .display_name
                .chars()
                .any(|character| character.is_control() || is_directional_control(character))
            || invalid_workspace(&space.workspace)
        {
            return Err(storage_error());
        }
        validate_managed_runtime_fields(&space.providers, &space.models, &space.model_roles)?;
    }
    if settings.selected_space_id.as_ref().is_some_and(|selected| {
        settings
            .spaces
            .iter()
            .find(|space| &space.id == selected)
            .is_none_or(|space| space.archived)
    }) {
        return Err(storage_error());
    }
    Ok(())
}

fn validate_external_targets(settings: &DesktopSettings) -> Result<HashSet<&str>, CommandErrorDto> {
    let mut target_ids = HashSet::with_capacity(settings.external_targets.len());
    for target in &settings.external_targets {
        let expected_account =
            external_credential_account(&target.instance_id, &target.certificate_sha256);
        if !valid_external_target_id(&target.target_id)
            || !target_ids.insert(target.target_id.as_str())
            || !valid_external_label(&target.label)
            || !uuid::Uuid::parse_str(&target.instance_id)
                .is_ok_and(|instance_id| !instance_id.is_nil())
            || target.certificate_sha256.len() != 64
            || !target
                .certificate_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !valid_private_absolute_path(&target.public_api_dir)
            || !valid_connection_value(&target.credential_service)
            || !valid_connection_value(&target.credential_account)
            || target.credential_service != EXTERNAL_KEYRING_SERVICE
            || expected_account.as_deref() != Some(target.credential_account.as_str())
        {
            return Err(storage_error());
        }
    }
    Ok(target_ids)
}

fn invalid_workspace(workspace: &WorkspaceSetting) -> bool {
    !valid_opaque_id(&workspace.id)
        || !workspace.path.is_absolute()
        || workspace
            .identity
            .as_ref()
            .is_none_or(|identity| identity.validate_current().is_err())
        || workspace.display_name.is_empty()
        || workspace.display_name.len() > 255
        || workspace.display_path.is_empty()
        || workspace.display_path.len() > 2_048
}

fn validate_managed_configuration(
    settings: &DesktopSettings,
) -> Result<BTreeSet<&str>, CommandErrorDto> {
    validate_managed_runtime_fields(&settings.providers, &settings.models, &settings.model_roles)?;
    let mut provider_profiles = BTreeSet::new();
    let mut credential_ids = BTreeSet::new();
    for provider in &settings.providers {
        if !valid_profile_name(&provider.profile)
            || !provider_profiles.insert(provider.profile.as_str())
            || provider.timeout_ms == Some(0)
            || validate_managed_provider_base_url(&provider.base_url).is_err()
            || provider
                .credential_id
                .as_deref()
                .is_some_and(|id| !valid_opaque_id(id))
        {
            return Err(storage_error());
        }
        if provider.kind == ProviderKindSetting::Codex
            && (provider.base_url != CODEX_BASE_URL || provider.credential_id.is_some())
        {
            return Err(storage_error());
        }
        if let Some(id) = provider.credential_id.as_deref() {
            credential_ids.insert(id);
        }
    }
    let mut model_profiles = BTreeSet::new();
    for model in &settings.models {
        let safety = model.context_window_tokens.div_ceil(10).max(512);
        if !valid_profile_name(&model.profile)
            || !model_profiles.insert(model.profile.as_str())
            || !provider_profiles.contains(model.provider_profile.as_str())
            || validate_managed_model_identifier(&model.model).is_err()
            || model.context_window_tokens < 1_024
            || model.max_output_tokens == 0
            || model
                .context_window_tokens
                .checked_sub(model.max_output_tokens)
                .and_then(|remaining| remaining.checked_sub(safety))
                .is_none_or(|input| input == 0)
        {
            return Err(storage_error());
        }
    }
    if settings.model_roles.iter().any(|(role, profile)| {
        !MODEL_ROLES.contains(&role.as_str()) || !model_profiles.contains(profile.as_str())
    }) {
        return Err(storage_error());
    }
    Ok(credential_ids)
}

fn validate_managed_runtime_fields(
    providers: &[ProviderSetting],
    models: &[ModelSetting],
    model_roles: &BTreeMap<String, String>,
) -> Result<(), CommandErrorDto> {
    if providers.len() > MAX_MANAGED_PROVIDERS
        || models.len() > MAX_MANAGED_MODELS
        || providers.is_empty() != models.is_empty()
        || models.is_empty() != model_roles.is_empty()
        || (!models.is_empty() && !model_roles.contains_key("primary"))
    {
        return Err(storage_error());
    }
    let mut provider_profiles = BTreeSet::new();
    for provider in providers {
        if !valid_profile_name(&provider.profile)
            || !provider_profiles.insert(provider.profile.as_str())
            || provider.timeout_ms == Some(0)
            || validate_managed_provider_base_url(&provider.base_url).is_err()
            || provider
                .credential_id
                .as_deref()
                .is_some_and(|id| !valid_opaque_id(id))
            || (provider.kind == ProviderKindSetting::Codex
                && (provider.base_url != CODEX_BASE_URL || provider.credential_id.is_some()))
        {
            return Err(storage_error());
        }
    }
    let mut model_profiles = BTreeSet::new();
    for model in models {
        let safety = model.context_window_tokens.div_ceil(10).max(512);
        if !valid_profile_name(&model.profile)
            || !model_profiles.insert(model.profile.as_str())
            || !provider_profiles.contains(model.provider_profile.as_str())
            || validate_managed_model_identifier(&model.model).is_err()
            || model.context_window_tokens < 1_024
            || model.max_output_tokens == 0
            || model
                .context_window_tokens
                .checked_sub(model.max_output_tokens)
                .and_then(|remaining| remaining.checked_sub(safety))
                .is_none_or(|input| input == 0)
        {
            return Err(storage_error());
        }
    }
    if model_roles.iter().any(|(role, profile)| {
        !MODEL_ROLES.contains(&role.as_str()) || !model_profiles.contains(profile.as_str())
    }) {
        return Err(storage_error());
    }
    Ok(())
}

fn valid_profile_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_opaque_id(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|uuid| !uuid.is_nil())
}

pub(crate) fn valid_external_target_id(value: &str) -> bool {
    value == "external-default" || valid_opaque_id(value)
}

pub(crate) fn valid_external_label(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= MAX_EXTERNAL_LABEL_BYTES
        && !value
            .chars()
            .any(|character| character.is_control() || is_directional_control(character))
}

fn is_directional_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

pub(crate) fn external_credential_account(
    instance_id: &str,
    certificate_sha256: &str,
) -> Option<String> {
    let instance_id = Uuid::parse_str(instance_id).ok()?;
    if instance_id.is_nil()
        || certificate_sha256.len() != 64
        || !certificate_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || certificate_sha256.bytes().all(|byte| byte == b'0')
    {
        return None;
    }
    Some(format!(
        "daemon-{instance_id}-{}",
        certificate_sha256.to_ascii_lowercase()
    ))
}

pub(crate) fn canonicalize_external_credential_binding(target: &mut ExternalTargetSetting) -> bool {
    let Some(account) =
        external_credential_account(&target.instance_id, &target.certificate_sha256)
    else {
        return false;
    };
    let changed = target.credential_service != EXTERNAL_KEYRING_SERVICE
        || target.credential_account != account;
    EXTERNAL_KEYRING_SERVICE.clone_into(&mut target.credential_service);
    target.credential_account = account;
    changed
}

fn valid_connection_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CONNECTION_VALUE_BYTES
        && !value.chars().any(char::is_control)
}

fn valid_private_absolute_path(path: &Path) -> bool {
    let Some(value) = path.to_str() else {
        return false;
    };
    if !path.is_absolute()
        || path.parent().is_none()
        || value.len() > MAX_CONNECTION_PATH_BYTES
        || value.chars().any(char::is_control)
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return false;
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};

        path.components().next().is_some_and(|component| {
            matches!(
                component,
                Component::Prefix(prefix)
                    if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
            )
        })
    }
    #[cfg(not(windows))]
    {
        !path
            .components()
            .any(|component| matches!(component, std::path::Component::Prefix(_)))
    }
}

fn display_path(path: &Path) -> String {
    let home = BaseDirs::new().map(|directories| directories.home_dir().to_owned());
    home.as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
        .map_or_else(
            || path.to_string_lossy().into_owned(),
            |relative| {
                if relative.as_os_str().is_empty() {
                    "~".into()
                } else {
                    format!("~/{}", relative.to_string_lossy())
                }
            },
        )
}

fn open_workspace_identity(path: &Path) -> Result<(PathBuf, WorkspaceIdentity), CommandErrorDto> {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as _;

        let canonical = fs::canonicalize(path).map_err(|_| workspace_error())?;
        let before = fs::symlink_metadata(path).map_err(|_| workspace_error())?;
        if canonical != path || !before.file_type().is_dir() || canonical.parent().is_none() {
            return Err(workspace_error());
        }
        let directory = rustix::fs::open(
            &canonical,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| workspace_error())?;
        let opened = directory.metadata().map_err(|_| workspace_error())?;
        let after = fs::symlink_metadata(&canonical).map_err(|_| workspace_error())?;
        if !opened.is_dir()
            || after.file_type().is_symlink()
            || !after.is_dir()
            || before.st_dev() != opened.st_dev()
            || before.st_ino() != opened.st_ino()
            || before.st_birthtime() != opened.st_birthtime()
            || before.st_birthtime_nsec() != opened.st_birthtime_nsec()
            || after.st_dev() != opened.st_dev()
            || after.st_ino() != opened.st_ino()
            || after.st_birthtime() != opened.st_birthtime()
            || after.st_birthtime_nsec() != opened.st_birthtime_nsec()
        {
            return Err(workspace_error());
        }
        let identity = WorkspaceIdentity::from_macos_parts(
            opened.st_dev(),
            opened.st_ino(),
            opened.st_birthtime(),
            opened.st_birthtime_nsec(),
        )
        .map_err(|_| workspace_error())?;
        Ok((canonical, identity))
    }
    #[cfg(target_os = "windows")]
    {
        let binding = colossus_windows_native::BoundPath::open_directory(path)
            .map_err(|_| workspace_error())?;
        binding.revalidate().map_err(|_| workspace_error())?;
        let kernel = binding.identity();
        let identity =
            WorkspaceIdentity::from_windows_parts(kernel.volume_serial_number, kernel.file_id)
                .map_err(|_| workspace_error())?;
        Ok((binding.canonical_path().to_owned(), identity))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = path;
        Err(workspace_error())
    }
}

fn validate_ca_bundle_setting(bundle: &CaBundleSetting) -> Result<(), CommandErrorDto> {
    if !valid_opaque_id(&bundle.bundle_id)
        || bundle.certificate_count == 0
        || bundle.certificate_count > 256
        || bundle.fingerprints_sha256.len() != bundle.certificate_count
        || bundle.fingerprints_sha256.iter().any(|fingerprint| {
            fingerprint.len() != 64
                || !fingerprint
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return Err(storage_error());
    }
    Ok(())
}

fn ca_bundle_storage_path(
    directory: &Path,
    bundle: &CaBundleSetting,
) -> Result<PathBuf, CommandErrorDto> {
    validate_ca_bundle_setting(bundle)?;
    Ok(directory.join(format!("{}.pem", bundle.bundle_id)))
}

fn read_ca_bundle_source(path: &Path) -> Result<Vec<u8>, CommandErrorDto> {
    if !path.is_absolute() {
        return Err(ca_bundle_error(
            "Choose a regular PEM file from the native file picker.",
        ));
    }
    #[cfg(unix)]
    {
        let before = fs::symlink_metadata(path)
            .map_err(|_| ca_bundle_error("The selected CA bundle could not be opened."))?;
        if !before.file_type().is_file() || before.len() > MAX_CA_BUNDLE_BYTES {
            return Err(ca_bundle_error(
                "The selected CA bundle must be a regular file no larger than 4 MiB.",
            ));
        }
        let mut source = rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| ca_bundle_error("The selected CA bundle could not be opened."))?;
        let opened = source
            .metadata()
            .map_err(|_| ca_bundle_error("The selected CA bundle could not be verified."))?;
        if opened.dev() != before.dev() || opened.ino() != before.ino() {
            return Err(ca_bundle_error(
                "The selected CA bundle changed while it was being opened.",
            ));
        }
        let mut bytes = Vec::with_capacity(usize::try_from(before.len()).unwrap_or(0));
        std::io::Read::by_ref(&mut source)
            .take(MAX_CA_BUNDLE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ca_bundle_error("The selected CA bundle could not be read."))?;
        if bytes.len() as u64 > MAX_CA_BUNDLE_BYTES {
            return Err(ca_bundle_error(
                "The selected CA bundle must be no larger than 4 MiB.",
            ));
        }
        return Ok(bytes);
    }
    #[cfg(windows)]
    {
        let binding = colossus_windows_native::BoundPath::open_file(path)
            .map_err(|_| ca_bundle_error("The selected CA bundle could not be opened."))?;
        let mut source = binding
            .try_clone_file()
            .map_err(|_| ca_bundle_error("The selected CA bundle could not be opened."))?;
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut source)
            .take(MAX_CA_BUNDLE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ca_bundle_error("The selected CA bundle could not be read."))?;
        binding
            .revalidate()
            .map_err(|_| ca_bundle_error("The selected CA bundle changed while it was read."))?;
        if bytes.len() as u64 > MAX_CA_BUNDLE_BYTES {
            return Err(ca_bundle_error(
                "The selected CA bundle must be no larger than 4 MiB.",
            ));
        }
        return Ok(bytes);
    }
    #[allow(unreachable_code)]
    Err(ca_bundle_error(
        "CA bundle import is unavailable on this platform.",
    ))
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), CommandErrorDto> {
    #[cfg(windows)]
    {
        colossus_windows_native::create_private_file(path, bytes).map_err(|_| storage_error())?;
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(path).map_err(|_| storage_error())?;
        if file.write_all(bytes).is_err() || file.sync_all().is_err() {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(storage_error());
        }
        drop(file);
        Ok(())
    }
}

fn replace_private_file(source: &Path, destination: &Path) -> Result<(), CommandErrorDto> {
    #[cfg(windows)]
    {
        colossus_windows_native::replace_private_file(source, destination)
            .map_err(|_| storage_error())
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, destination).map_err(|_| storage_error())
    }
}

#[cfg(unix)]
fn sync_private_directory(path: &Path) -> Result<(), CommandErrorDto> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| storage_error())
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn sync_private_directory(_path: &Path) -> Result<(), CommandErrorDto> {
    // Windows does not expose the Unix directory-fsync durability primitive.
    // Preserve one fallible interface at the save call site across platforms.
    Ok(())
}

fn read_private_file(path: &Path, maximum_bytes: u64) -> Result<Vec<u8>, CommandErrorDto> {
    #[cfg(unix)]
    let mut source = {
        let metadata = fs::symlink_metadata(path).map_err(|_| storage_error())?;
        if !metadata.file_type().is_file()
            || metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.mode() & 0o077 != 0
            || metadata.len() > maximum_bytes
        {
            return Err(storage_error());
        }
        let source = rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| storage_error())?;
        let opened = source.metadata().map_err(|_| storage_error())?;
        if opened.dev() != metadata.dev() || opened.ino() != metadata.ino() {
            return Err(storage_error());
        }
        source
    };
    #[cfg(windows)]
    let (mut source, binding) = {
        let binding =
            colossus_windows_native::BoundPath::open_file(path).map_err(|_| storage_error())?;
        binding
            .validate_private_owner_dacl()
            .map_err(|_| storage_error())?;
        let source = binding.try_clone_file().map_err(|_| storage_error())?;
        (source, binding)
    };
    #[cfg(not(any(unix, windows)))]
    let mut source = return Err(storage_error());

    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut source)
        .take(maximum_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| storage_error())?;
    if bytes.len() as u64 > maximum_bytes {
        return Err(storage_error());
    }
    #[cfg(windows)]
    binding.revalidate().map_err(|_| storage_error())?;
    Ok(bytes)
}

fn ensure_private_directory(path: &Path) -> Result<(), CommandErrorDto> {
    if !path.is_absolute() || path.parent().is_none() {
        return Err(storage_error());
    }
    if !path.exists() {
        #[cfg(windows)]
        colossus_windows_native::create_private_directory(path).map_err(|_| storage_error())?;
        #[cfg(not(windows))]
        fs::create_dir_all(path).map_err(|_| storage_error())?;
        #[cfg(unix)]
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| storage_error())?;
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| storage_error())?;
    if !metadata.file_type().is_dir() {
        return Err(storage_error());
    }
    // Windows canonicalization normally rewrites `C:\...` as the equivalent
    // verbatim `\\?\C:\...` spelling. The native binding below supplies the
    // stronger object, ancestor, reparse-point, and identity checks on that platform.
    #[cfg(not(windows))]
    {
        let canonical = fs::canonicalize(path).map_err(|_| storage_error())?;
        if canonical != path {
            return Err(storage_error());
        }
    }
    #[cfg(unix)]
    if metadata.uid() != rustix::process::getuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(storage_error());
    }
    #[cfg(windows)]
    {
        let binding = colossus_windows_native::BoundPath::open_directory(path)
            .map_err(|_| storage_error())?;
        binding
            .validate_private_owner_dacl()
            .and_then(|()| binding.revalidate())
            .map_err(|_| storage_error())?;
    }
    Ok(())
}

fn storage_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "desktop_storage",
        "Colossus Desktop could not open its private application storage.",
        false,
    )
}

fn home_storage_error(error: HomeError) -> CommandErrorDto {
    let message = match error {
        HomeError::HomeDirectoryUnavailable => {
            "Colossus Desktop could not find your Windows home directory. Set COLOSSUS_HOME to an absolute private directory and restart the desktop app."
        }
        HomeError::HomeMustBeAbsolute(_) => {
            "COLOSSUS_HOME must be an absolute private directory before Colossus Desktop can start."
        }
        HomeError::UnsafePrivateDirectory(_) | HomeError::UnsafeConfinedPath(_) => {
            "The selected Colossus home is not private to your Windows account. For desktop development, use %LOCALAPPDATA%\\ColossusDevHome or set COLOSSUS_HOME to another private directory."
        }
        HomeError::Io { .. } => {
            "Colossus Desktop could not create or read its private application storage. Check that COLOSSUS_HOME points to a writable private directory."
        }
        HomeError::InvalidWorkspace(_) | HomeError::InvalidWorkspaceIdentity => {
            "Colossus Desktop could not validate its private workspace storage. Choose the workspace again after the app starts."
        }
    };
    CommandErrorDto::local_sanitized("desktop_storage", message, false)
}

fn workspace_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "workspace_invalid",
        "Choose an existing folder and reselect it if it was moved or replaced.",
        false,
    )
}

fn ca_bundle_error(message: &str) -> CommandErrorDto {
    CommandErrorDto::local_sanitized("ca_bundle_invalid", message, false)
}

fn credential_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "provider_credential",
        "The provider key could not be accessed in the system keychain. If it was removed, choose Replace the stored API key in Managed Local settings and retry.",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aside_settings_persist_only_bounded_linkage_metadata() {
        let value = serde_json::to_value(AsideSetting {
            space_id: "space-1".into(),
            parent_session_id: "parent-session".into(),
            source_run_id: "source-run".into(),
            session_id: "aside-session".into(),
            latest_run_id: "aside-run".into(),
            created_at: "2026-08-15T00:00:00Z".into(),
            closed: false,
        })
        .expect("Aside setting");
        let object = value.as_object().expect("object");
        assert_eq!(object.len(), 7);
        assert!(!object.contains_key("prompt"));
        assert!(!object.contains_key("quote"));
        assert!(!object.contains_key("messages"));
        assert!(!object.contains_key("toolOutput"));
    }

    #[test]
    fn home_storage_errors_are_actionable_without_disclosing_paths() {
        let unsafe_path = PathBuf::from(r"C:\Users\private\.colossus");
        let error = home_storage_error(HomeError::UnsafePrivateDirectory(unsafe_path));
        assert_eq!(error.code, "desktop_storage");
        assert!(error.message.contains("not private"));
        assert!(error.message.contains("COLOSSUS_HOME"));
        assert!(error.message.contains("%LOCALAPPDATA%\\ColossusDevHome"));
        let serialized = serde_json::to_string(&error).expect("error serializes");
        assert!(!serialized.contains("C:\\Users\\private"));

        let relative = home_storage_error(HomeError::HomeMustBeAbsolute(PathBuf::from("relative")));
        assert!(relative.message.contains("absolute private directory"));
        assert!(!relative.message.contains("relative"));
    }

    #[test]
    fn codex_auth_home_is_app_private_storage() {
        let (_guard, root, store) = test_store();
        let auth_home = store.codex_auth_home().expect("Codex auth home");

        assert_eq!(auth_home, root.join(CODEX_AUTH_DIRECTORY));
        assert!(auth_home.is_dir());
    }

    #[test]
    fn legacy_tui_consent_cannot_silently_enable_local_shell_authority() {
        let mut settings = DesktopSettings {
            terminal_enabled: true,
            ..DesktopSettings::default()
        };
        assert!(
            !settings.local_terminal_enabled(),
            "a settings record without the versioned native warning is not shell consent"
        );

        settings.local_terminal_consent_version = LOCAL_TERMINAL_CONSENT_VERSION;
        assert!(settings.local_terminal_enabled());

        settings.local_terminal_consent_version = LOCAL_TERMINAL_CONSENT_VERSION + 1;
        assert!(
            validate_settings(&settings).is_err(),
            "a newer unknown consent contract must fail closed"
        );
    }

    #[cfg(unix)]
    fn ca_pem(name: &str) -> String {
        let mut parameters =
            rcgen::CertificateParams::new(vec![name.into()]).expect("CA parameters");
        parameters.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        parameters
            .self_signed(&rcgen::KeyPair::generate().expect("CA key"))
            .expect("CA certificate")
            .pem()
    }

    fn configured_settings(
        kind: ProviderKindSetting,
        base_url: &str,
        credential_id: Option<String>,
    ) -> DesktopSettings {
        DesktopSettings {
            providers: vec![ProviderSetting {
                profile: "primary-provider".into(),
                kind,
                base_url: base_url.into(),
                credential_id,
                timeout_ms: Some(120_000),
            }],
            models: vec![ModelSetting {
                profile: "primary".into(),
                provider_profile: "primary-provider".into(),
                model: "test-model".into(),
                context_window_tokens: 128_000,
                max_output_tokens: 16_000,
                capabilities: ModelCapabilitiesSetting {
                    tool_calls: true,
                    streaming: true,
                },
                reasoning_effort: None,
            }],
            model_roles: BTreeMap::from([("primary".into(), "primary".into())]),
            ..DesktopSettings::default()
        }
    }

    #[cfg(windows)]
    struct PrivateTestRoot {
        path: PathBuf,
    }

    #[cfg(windows)]
    impl PrivateTestRoot {
        fn in_target(prefix: &str) -> Self {
            let parent = windows_test_parent();
            let path = parent.join(format!(
                "ColossusDesktopSettingsTest-{prefix}-{}",
                Uuid::now_v7()
            ));
            colossus_windows_native::create_private_directory(&path).expect("private test root");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    #[cfg(windows)]
    impl Drop for PrivateTestRoot {
        fn drop(&mut self) {
            let expected_parent = windows_test_parent();
            assert_eq!(self.path.parent(), Some(expected_parent.as_path()));
            assert!(
                self.path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("ColossusDesktopSettingsTest-"))
            );
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(windows)]
    fn windows_test_parent() -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .expect("absolute Windows LocalAppData")
    }

    #[cfg(windows)]
    type TestStoreGuard = PrivateTestRoot;
    #[cfg(not(windows))]
    type TestStoreGuard = tempfile::TempDir;

    fn test_store() -> (TestStoreGuard, PathBuf, SettingsStore) {
        #[cfg(windows)]
        {
            let root_guard = PrivateTestRoot::in_target("store");
            let canonical_root = fs::canonicalize(root_guard.path()).expect("canonical store root");
            let store = SettingsStore::open(canonical_root.clone()).expect("store");
            return (root_guard, canonical_root, store);
        }
        #[cfg(not(windows))]
        {
            let parent = tempfile::tempdir().expect("store parent");
            let root = fs::canonicalize(parent.path())
                .expect("canonical store parent")
                .join("store");
            let store = SettingsStore::open(root.clone()).expect("store");
            let canonical_root = fs::canonicalize(&root).expect("canonical store root");
            assert_eq!(canonical_root, root);
            (parent, canonical_root, store)
        }
    }

    fn test_directory(prefix: &str) -> (TestStoreGuard, PathBuf) {
        #[cfg(windows)]
        {
            let guard = PrivateTestRoot::in_target(prefix);
            let path = fs::canonicalize(guard.path()).expect("canonical test directory");
            return (guard, path);
        }
        #[cfg(not(windows))]
        {
            let guard = tempfile::tempdir().expect("test directory");
            let path = fs::canonicalize(guard.path()).expect("canonical test directory");
            (guard, path)
        }
    }

    #[cfg(windows)]
    struct DesktopSelfTestHomeGuard {
        path: PathBuf,
    }

    #[cfg(windows)]
    impl DesktopSelfTestHomeGuard {
        fn in_local_app_data() -> Self {
            let local_app_data = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .expect("absolute Windows LocalAppData");
            let parent = fs::canonicalize(local_app_data).expect("canonical LocalAppData");
            let path = parent.join(format!("ColossusDesktopSelfTest-{}", Uuid::now_v7()));
            Self { path }
        }
    }

    #[cfg(windows)]
    impl Drop for DesktopSelfTestHomeGuard {
        fn drop(&mut self) {
            let local_app_data = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .expect("Windows LocalAppData");
            let parent = fs::canonicalize(local_app_data).expect("canonical LocalAppData");
            assert_eq!(self.path.parent(), Some(parent.as_path()));
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn application_home_uses_desktop_settings_and_workspace_surface_layout() {
        #[cfg(windows)]
        let parent_guard = PrivateTestRoot::in_target("home-parent");
        #[cfg(windows)]
        let parent = parent_guard.path().to_path_buf();
        #[cfg(not(windows))]
        let parent_guard = tempfile::tempdir().expect("home parent");
        #[cfg(not(windows))]
        let parent = fs::canonicalize(parent_guard.path()).expect("canonical home parent");
        // Represents the former Tauri Application Support root. Fresh shared-home
        // startup never receives this path and therefore must neither migrate nor
        // delete its contents.
        let legacy_application_support = parent.join("legacy-application-support");
        fs::create_dir(&legacy_application_support).expect("legacy application support");
        let legacy_marker = legacy_application_support.join("legacy-state.marker");
        fs::write(&legacy_marker, b"preserve exactly").expect("legacy marker");
        let home = ColossusHome::ensure_at(parent.join(".colossus")).expect("home");
        let store = SettingsStore::open_home(home.clone()).expect("Desktop store");
        let settings = DesktopSettings::default();
        assert_eq!(settings.access_profile, AccessProfileSetting::AllowAll);
        assert_eq!(
            settings.execution_boundary,
            ExecutionBoundarySetting::FullAccess
        );
        store.save(&settings).expect("save Desktop settings");
        assert!(home.root().join("desktop/settings.json").is_file());
        assert_eq!(
            fs::read(&legacy_marker).expect("unchanged legacy marker"),
            b"preserve exactly"
        );
        assert_eq!(
            fs::read_dir(&legacy_application_support)
                .expect("legacy directory")
                .count(),
            1,
            "fresh Desktop startup must ignore legacy application-support data"
        );

        let (_workspace_guard, workspace_path) = test_directory("workspace");
        let workspace = validate_workspace(&workspace_path).expect("workspace identity");
        let storage = store
            .managed_workspace_storage(
                &settings.managed_instance_id,
                &workspace.path,
                workspace.identity.as_ref().expect("current identity"),
            )
            .expect("managed storage");
        #[cfg(debug_assertions)]
        {
            assert_eq!(
                storage
                    .instance_dir
                    .file_name()
                    .and_then(|name| name.to_str()),
                Some(DEVELOPMENT_PLAINTEXT_RUNTIME_DIRECTORY)
            );
            assert_eq!(
                storage
                    .instance_dir
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str()),
                Some("desktop")
            );
            assert_eq!(
                storage
                    .instance_dir
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str()),
                Some("workspaces")
            );
        }
        #[cfg(not(debug_assertions))]
        {
            assert_eq!(
                storage
                    .instance_dir
                    .file_name()
                    .and_then(|name| name.to_str()),
                Some("desktop")
            );
            assert_eq!(
                storage
                    .instance_dir
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str()),
                Some("workspaces")
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn settings_store_accepts_standard_windows_path_spelling() {
        let parent = PrivateTestRoot::in_target("standard-path-parent");
        let canonical_parent = fs::canonicalize(parent.path()).expect("canonical store parent");
        assert_ne!(
            canonical_parent,
            parent.path(),
            "the fixture must exercise Windows' ordinary versus verbatim path spellings"
        );
        let root = parent.path().join("store");

        let store = SettingsStore::open(root).expect("open standard Windows storage path");
        let settings = DesktopSettings::default();
        store.save(&settings).expect("save settings");

        assert_eq!(store.load().expect("load settings"), settings);
    }

    fn test_public_api_dir(suffix: &str) -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from(format!(
                r"C:\Users\test\AppData\Local\colossus-public-api-{suffix}"
            ))
        }
        #[cfg(not(windows))]
        {
            PathBuf::from(format!("/private/tmp/colossus-public-api-{suffix}"))
        }
    }

    #[test]
    fn provider_kind_wire_values_match_renderer_and_accept_preview_aliases() {
        assert_eq!(
            serde_json::to_string(&ProviderKindSetting::Responses).expect("serialize"),
            r#""openai_responses""#,
        );
        assert_eq!(
            serde_json::to_string(&ProviderKindSetting::Compatible).expect("serialize"),
            r#""openai_compatible""#,
        );
        assert_eq!(
            serde_json::to_string(&ProviderKindSetting::Codex).expect("serialize"),
            r#""open_ai_codex""#,
        );
        assert_eq!(
            serde_json::from_str::<ProviderKindSetting>(r#""open_ai_responses""#)
                .expect("legacy responses"),
            ProviderKindSetting::Responses,
        );
        assert_eq!(
            serde_json::from_str::<ProviderKindSetting>(r#""open_ai_compatible""#)
                .expect("legacy compatible"),
            ProviderKindSetting::Compatible,
        );
        assert_eq!(
            serde_json::from_str::<ProviderKindSetting>(r#""openai_responses""#)
                .expect("canonical responses"),
            ProviderKindSetting::Responses,
        );
        assert_eq!(
            serde_json::from_str::<ProviderKindSetting>(r#""openai_compatible""#)
                .expect("canonical compatible"),
            ProviderKindSetting::Compatible,
        );
    }

    #[cfg(unix)]
    #[test]
    fn ca_bundle_is_copied_into_private_storage_and_revalidated() {
        let root = tempfile::tempdir().expect("root");
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))
            .expect("root permissions");
        let canonical_root = fs::canonicalize(root.path()).expect("canonical root");
        let source = tempfile::NamedTempFile::new().expect("source");
        fs::write(source.path(), ca_pem("private.example")).expect("write CA");
        let store = SettingsStore::open(canonical_root.clone()).expect("store");

        let bundle = store.stage_ca_bundle(source.path()).expect("import CA");
        let path = store.ca_bundle_path(&bundle).expect("validate CA");

        assert!(path.starts_with(canonical_root.join(TRUST_DIRECTORY)));
        assert_ne!(path, source.path());
        assert_eq!(bundle.certificate_count, 1);
        assert_eq!(bundle.fingerprints_sha256.len(), 1);
        assert_eq!(
            fs::symlink_metadata(&path).expect("metadata").mode() & 0o077,
            0
        );
        let settings = DesktopSettings {
            additional_ca_bundle: Some(bundle.clone()),
            ..DesktopSettings::default()
        };
        store.save(&settings).expect("save settings");
        assert_eq!(
            store.load().expect("load settings").additional_ca_bundle,
            Some(bundle.clone())
        );
        store.delete_ca_bundle(&bundle).expect("delete bundle");
        assert!(!path.exists());
    }

    #[test]
    fn preview_provider_kind_is_loaded_and_rewritten_canonically() {
        let settings = configured_settings(
            ProviderKindSetting::Compatible,
            OPENROUTER_BASE_URL,
            Some(Uuid::now_v7().to_string()),
        );
        let canonical = serde_json::to_string(&settings).expect("canonical settings");
        let preview = canonical.replace("openai_compatible", "open_ai_compatible");
        let decoded: DesktopSettings = serde_json::from_str(&preview).expect("preview settings");

        assert_eq!(decoded.providers, settings.providers);
        let rewritten = serde_json::to_string(&decoded).expect("rewritten settings");
        assert!(rewritten.contains(r#""kind":"openai_compatible""#));
        assert!(!rewritten.contains("open_ai_compatible"));
    }

    #[test]
    fn settings_round_trip_never_contains_provider_secret() {
        let (_root_guard, canonical_root, store) = test_store();
        let settings = configured_settings(
            ProviderKindSetting::Compatible,
            OPENROUTER_BASE_URL,
            Some(Uuid::now_v7().to_string()),
        );
        store.save(&settings).expect("save");
        assert_eq!(store.load().expect("load"), settings);
        let bytes = fs::read(canonical_root.join(SETTINGS_FILE)).expect("settings");
        assert!(!String::from_utf8_lossy(&bytes).contains("provider-secret"));
    }

    #[test]
    fn v4_workspace_migrates_to_the_first_folder_backed_space() {
        let (_root_guard, canonical_root, store) = test_store();
        let (_folder_guard, folder) = test_directory("workspace");
        let workspace = validate_workspace(&folder).expect("workspace identity");
        let mut legacy = configured_settings(
            ProviderKindSetting::Compatible,
            OPENROUTER_BASE_URL,
            Some(Uuid::now_v7().to_string()),
        );
        legacy.workspace = Some(workspace.clone());
        legacy.selected_target_id = Some("managed-local".into());
        let mut encoded = serde_json::to_value(legacy).expect("legacy settings");
        encoded["schemaVersion"] = serde_json::Value::from(4);
        encoded
            .as_object_mut()
            .expect("settings object")
            .remove("spaces");
        encoded
            .as_object_mut()
            .expect("settings object")
            .remove("selectedSpaceId");
        let path = canonical_root.join(SETTINGS_FILE);
        write_private_file(&path, &serde_json::to_vec(&encoded).expect("v4 settings"))
            .expect("settings");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("permissions");

        let migrated = store.load().expect("migrate v4 settings");
        assert_eq!(migrated.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(migrated.spaces.len(), 1);
        assert_eq!(migrated.spaces[0].workspace, workspace);
        assert_eq!(migrated.spaces[0].display_name, workspace.display_name);
        assert_eq!(
            migrated.selected_space_id.as_deref(),
            Some(workspace.id.as_str())
        );
        assert_eq!(migrated.selected_target_id, migrated.selected_space_id);
        assert_eq!(migrated.spaces[0].providers, migrated.providers);
        assert_eq!(store.load().expect("persisted migration"), migrated);
    }

    #[test]
    fn spaces_reject_duplicate_folder_identity_and_archive_requires_restore() {
        let (_folder_guard, folder) = test_directory("workspace");
        let workspace = validate_workspace(&folder).expect("workspace identity");
        let mut settings = DesktopSettings::default();
        let space_id = settings.add_space(workspace.clone()).expect("first Space");

        let mut duplicate = workspace;
        duplicate.id = Uuid::now_v7().to_string();
        assert!(settings.add_space(duplicate).is_err());

        settings
            .spaces
            .iter_mut()
            .find(|space| space.id == space_id)
            .expect("Space")
            .archived = true;
        assert!(settings.activate_space(&space_id).is_err());
    }

    #[test]
    fn spaces_share_credential_references_but_keep_independent_model_profiles() {
        let credential_id = Uuid::now_v7().to_string();
        let mut settings = configured_settings(
            ProviderKindSetting::Compatible,
            OPENROUTER_BASE_URL,
            Some(credential_id.clone()),
        );
        let (_first_guard, first_path) = test_directory("first-workspace");
        let (_second_guard, second_path) = test_directory("second-workspace");
        let first = validate_workspace(&first_path).expect("first identity");
        let second = validate_workspace(&second_path).expect("second identity");
        let first_id = settings.add_space(first).expect("first Space");
        let second_id = settings.add_space(second).expect("second Space");
        settings.models[0].model = "space-two-model".into();

        settings.activate_space(&first_id).expect("select first");
        assert_eq!(settings.models[0].model, "test-model");
        settings.activate_space(&second_id).expect("select second");
        assert_eq!(settings.models[0].model, "space-two-model");
        assert_eq!(
            settings.provider_credential_ids(),
            BTreeSet::from([credential_id.as_str()])
        );
    }

    #[test]
    fn codex_settings_require_the_fixed_backend_without_a_key_reference() {
        let settings = configured_settings(ProviderKindSetting::Codex, CODEX_BASE_URL, None);
        validate_settings(&settings).expect("Codex settings");

        let mut with_key = settings.clone();
        with_key.providers[0].credential_id = Some(Uuid::now_v7().to_string());
        assert!(validate_settings(&with_key).is_err());

        let mut changed_origin = settings;
        changed_origin.providers[0].base_url = "https://example.test/v1".into();
        assert!(validate_settings(&changed_origin).is_err());
    }

    #[test]
    fn settings_reject_renderer_visible_model_controls() {
        let mut settings =
            configured_settings(ProviderKindSetting::Compatible, OPENROUTER_BASE_URL, None);
        settings.models[0].model = "model\nforged-status".into();

        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn v1_provider_configuration_is_cleared_and_credential_is_queued_for_deletion() {
        let (_root_guard, canonical_root, store) = test_store();
        let credential_id = Uuid::now_v7().to_string();
        let encoded = serde_json::json!({
            "schemaVersion": 1,
            "managedInstanceId": Uuid::now_v7().to_string(),
            "workspace": null,
            "provider": {
                "kind": "openai_compatible",
                "model": "legacy-model",
                "baseUrl": OPENROUTER_BASE_URL,
                "credentialId": credential_id,
            },
            "pendingProviderCleanupIds": [],
            "accessProfile": "minimal",
            "terminalEnabled": true,
            "selectedTargetId": "managed-local",
            "externalTargets": [],
            "legacyConnectionMigrated": true,
        });
        let path = canonical_root.join(SETTINGS_FILE);
        write_private_file(
            &path,
            &serde_json::to_vec(&encoded).expect("legacy settings"),
        )
        .expect("settings");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("permissions");

        let migrated = store.load().expect("migrate settings");
        assert_eq!(migrated.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(migrated.providers.is_empty());
        assert!(migrated.models.is_empty());
        assert!(migrated.model_roles.is_empty());
        assert_eq!(migrated.pending_provider_cleanup_ids, [credential_id]);
        assert_eq!(migrated.access_profile, AccessProfileSetting::Minimal);
        assert_eq!(
            migrated.execution_boundary,
            ExecutionBoundarySetting::OfflineIsolated
        );
        assert!(migrated.terminal_enabled);
        assert!(migrated.selected_target_id.is_none());
        assert!(migrated.legacy_connection_migrated);
    }

    #[test]
    fn v1_legacy_allow_all_migrates_to_development_access() {
        let (_root_guard, canonical_root, store) = test_store();
        let encoded = serde_json::json!({
            "schemaVersion": 1,
            "managedInstanceId": Uuid::now_v7().to_string(),
            "workspace": null,
            "provider": null,
            "pendingProviderCleanupIds": [],
            "accessProfile": "allow_all",
            "terminalEnabled": false,
            "selectedTargetId": null,
            "externalTargets": [],
            "legacyConnectionMigrated": true,
        });
        let path = canonical_root.join(SETTINGS_FILE);
        write_private_file(
            &path,
            &serde_json::to_vec(&encoded).expect("legacy settings"),
        )
        .expect("settings");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("permissions");

        let migrated = store.load().expect("migrate v1 settings");
        assert_eq!(migrated.access_profile, AccessProfileSetting::Development);
        assert_eq!(
            migrated.execution_boundary,
            ExecutionBoundarySetting::WorkspaceIsolated
        );
        let rewritten: serde_json::Value =
            serde_json::from_slice(&fs::read(path).expect("rewritten settings"))
                .expect("rewritten JSON");
        assert_eq!(rewritten["accessProfile"], "development");
        assert_eq!(rewritten["executionBoundary"], "workspace_isolated");
    }

    #[test]
    fn v2_provider_timeout_is_preserved_as_an_explicit_override() {
        let (_root_guard, canonical_root, store) = test_store();
        let settings =
            configured_settings(ProviderKindSetting::Compatible, OPENROUTER_BASE_URL, None);
        let mut encoded = serde_json::to_value(settings).expect("settings");
        encoded["schemaVersion"] = serde_json::Value::from(2);
        encoded
            .as_object_mut()
            .expect("settings object")
            .remove("executionBoundary");
        let path = canonical_root.join(SETTINGS_FILE);
        write_private_file(&path, &serde_json::to_vec(&encoded).expect("v2 settings"))
            .expect("settings");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("permissions");

        let migrated = store.load().expect("migrate v2 settings");
        assert_eq!(migrated.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(migrated.providers[0].timeout_ms, Some(120_000));
        assert_eq!(migrated.access_profile, AccessProfileSetting::Development);
        assert_eq!(
            migrated.execution_boundary,
            ExecutionBoundarySetting::WorkspaceIsolated
        );
        let rewritten: serde_json::Value =
            serde_json::from_slice(&fs::read(path).expect("rewritten settings"))
                .expect("rewritten JSON");
        assert_eq!(
            rewritten["providers"][0]["timeoutMs"],
            serde_json::Value::from(120_000)
        );
        assert_eq!(rewritten["accessProfile"], "development");
        assert_eq!(rewritten["executionBoundary"], "workspace_isolated");
    }

    #[test]
    fn v3_settings_migrate_legacy_allow_all_and_preserve_workspace_isolation() {
        let (_root_guard, canonical_root, store) = test_store();
        let mut settings =
            configured_settings(ProviderKindSetting::Compatible, OPENROUTER_BASE_URL, None);
        settings.access_profile = AccessProfileSetting::AllowAll;
        let mut encoded = serde_json::to_value(settings).expect("settings");
        encoded["schemaVersion"] = serde_json::Value::from(3);
        encoded
            .as_object_mut()
            .expect("settings object")
            .remove("executionBoundary");
        let path = canonical_root.join(SETTINGS_FILE);
        write_private_file(&path, &serde_json::to_vec(&encoded).expect("v3 settings"))
            .expect("settings");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("permissions");

        let migrated = store.load().expect("migrate v3 settings");
        assert_eq!(migrated.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(
            migrated.execution_boundary,
            ExecutionBoundarySetting::WorkspaceIsolated
        );
        assert_eq!(migrated.access_profile, AccessProfileSetting::Development);
        let rewritten: serde_json::Value =
            serde_json::from_slice(&fs::read(path).expect("rewritten settings"))
                .expect("rewritten JSON");
        assert_eq!(rewritten["executionBoundary"], "workspace_isolated");
        assert_eq!(rewritten["accessProfile"], "development");
    }

    #[test]
    fn automatic_desktop_timeout_uses_the_resolved_host_default() {
        let mut settings =
            configured_settings(ProviderKindSetting::Compatible, OPENROUTER_BASE_URL, None);
        settings.providers[0].timeout_ms = None;
        assert_eq!(settings.providers[0].effective_timeout_ms(), 300_000);

        settings.providers[0].base_url = "http://127.0.0.1:11434/v1".into();
        assert_eq!(settings.providers[0].effective_timeout_ms(), 900_000);
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn managed_state_and_instance_identity_are_isolated_per_workspace() {
        let (_root_guard, _root, store) = test_store();
        let (_first_guard, first_path) = test_directory("first-workspace");
        let (_second_guard, second_path) = test_directory("second-workspace");
        let seed = Uuid::now_v7().to_string();
        let first_identity = open_workspace_identity(&first_path)
            .expect("first identity")
            .1;
        let second_identity = open_workspace_identity(&second_path)
            .expect("second identity")
            .1;

        let first = store
            .managed_workspace_storage(&seed, &first_path, &first_identity)
            .expect("first storage");
        fs::write(
            first.instance_dir.join("queued-run-marker"),
            b"workspace one",
        )
        .expect("state marker");
        let second = store
            .managed_workspace_storage(&seed, &second_path, &second_identity)
            .expect("second storage");
        let first_again = store
            .managed_workspace_storage(&seed, &first_path, &first_identity)
            .expect("stable first storage");

        assert_ne!(first.instance_dir, second.instance_dir);
        assert_ne!(first.instance_id, second.instance_id);
        assert_eq!(first.instance_dir, first_again.instance_dir);
        assert_eq!(first.instance_id, first_again.instance_id);
        assert!(!second.instance_dir.join("queued-run-marker").exists());
        #[cfg(debug_assertions)]
        assert_eq!(
            first
                .instance_dir
                .file_name()
                .and_then(|name| name.to_str()),
            Some(DEVELOPMENT_PLAINTEXT_RUNTIME_DIRECTORY)
        );
    }

    #[test]
    fn path_only_same_path_replacement_requires_reselection_and_preserves_provider() {
        let (_root_guard, root, store) = test_store();
        let (_workspace_parent_guard, workspace_parent) = test_directory("workspace-parent");
        let workspace = workspace_parent.join("workspace");
        let moved = workspace_parent.join("workspace-moved");
        fs::create_dir(&workspace).expect("workspace");
        let old_seed = Uuid::now_v7().to_string();
        let mut legacy = configured_settings(
            ProviderKindSetting::Compatible,
            OPENROUTER_BASE_URL,
            Some(Uuid::now_v7().to_string()),
        );
        legacy.managed_instance_id = old_seed.clone();
        legacy.workspace = Some(WorkspaceSetting {
            id: Uuid::now_v7().to_string(),
            path: workspace.clone(),
            identity: None,
            display_name: "workspace".into(),
            display_path: workspace.display().to_string(),
        });
        legacy.selected_target_id = Some("managed-local".into());
        let expected_providers = legacy.providers.clone();
        let expected_models = legacy.models.clone();
        let path = root.join(SETTINGS_FILE);
        write_private_file(&path, &serde_json::to_vec(&legacy).expect("legacy JSON"))
            .expect("legacy settings");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("file permissions");

        fs::rename(&workspace, &moved).expect("move originally selected workspace");
        fs::create_dir(&workspace).expect("same-path replacement");

        let migrated = store.load().expect("migrate settings");
        assert_ne!(migrated.managed_instance_id, old_seed);
        assert!(migrated.workspace.is_none());
        assert_eq!(migrated.providers, expected_providers);
        assert_eq!(migrated.models, expected_models);
        assert!(migrated.selected_target_id.is_none());
        assert_eq!(store.load().expect("persisted migration"), migrated);
    }

    #[test]
    fn v1_inode_only_workspace_identity_requires_explicit_reselection() {
        let (_root_guard, root, store) = test_store();
        let (_workspace_guard, workspace) = test_directory("workspace");
        let old_seed = Uuid::now_v7().to_string();
        let legacy = DesktopSettings {
            managed_instance_id: old_seed.clone(),
            workspace: Some(WorkspaceSetting {
                id: Uuid::now_v7().to_string(),
                path: workspace.clone(),
                identity: Some(WorkspaceIdentity::from_unix_parts(42, 84)),
                display_name: "workspace".into(),
                display_path: workspace.display().to_string(),
            }),
            selected_target_id: Some("managed-local".into()),
            ..DesktopSettings::default()
        };
        let path = root.join(SETTINGS_FILE);
        write_private_file(&path, &serde_json::to_vec(&legacy).expect("legacy JSON"))
            .expect("legacy settings");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("file permissions");

        let migrated = store.load().expect("migrate v1 settings");
        assert_ne!(migrated.managed_instance_id, old_seed);
        assert!(migrated.workspace.is_none());
        assert!(migrated.selected_target_id.is_none());
        assert_eq!(store.load().expect("persisted migration"), migrated);
    }

    #[test]
    fn missing_identity_version_migrates_without_bricking_settings() {
        let (_root_guard, root, store) = test_store();
        let (_workspace_guard, workspace) = test_directory("workspace");
        let old_seed = Uuid::now_v7().to_string();
        let legacy = DesktopSettings {
            managed_instance_id: old_seed.clone(),
            workspace: Some(WorkspaceSetting {
                id: Uuid::now_v7().to_string(),
                path: workspace.clone(),
                identity: Some(WorkspaceIdentity::from_unix_parts(42, 84)),
                display_name: "workspace".into(),
                display_path: workspace.display().to_string(),
            }),
            selected_target_id: Some("managed-local".into()),
            ..DesktopSettings::default()
        };
        let mut encoded = serde_json::to_value(&legacy).expect("legacy JSON");
        encoded["workspace"]["identity"]
            .as_object_mut()
            .expect("identity object")
            .remove("version");
        let path = root.join(SETTINGS_FILE);
        write_private_file(&path, &serde_json::to_vec(&encoded).expect("preview JSON"))
            .expect("legacy settings");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("file permissions");

        let migrated = store.load().expect("migrate missing identity version");
        assert_ne!(migrated.managed_instance_id, old_seed);
        assert!(migrated.workspace.is_none());
        assert!(migrated.selected_target_id.is_none());
        assert_eq!(store.load().expect("persisted migration"), migrated);
    }

    #[test]
    fn missing_legacy_workspace_is_cleared_and_persisted_without_bricking_settings() {
        let (_root_guard, root, store) = test_store();
        let old_seed = Uuid::now_v7().to_string();
        let missing = root.join("missing-workspace");
        let legacy = DesktopSettings {
            managed_instance_id: old_seed.clone(),
            workspace: Some(WorkspaceSetting {
                id: Uuid::now_v7().to_string(),
                path: missing.clone(),
                identity: None,
                display_name: "missing-workspace".into(),
                display_path: missing.display().to_string(),
            }),
            selected_target_id: Some("managed-local".into()),
            ..DesktopSettings::default()
        };
        let path = root.join(SETTINGS_FILE);
        write_private_file(&path, &serde_json::to_vec(&legacy).expect("legacy JSON"))
            .expect("legacy settings");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("file permissions");

        let migrated = store.load().expect("recover settings");
        assert_ne!(migrated.managed_instance_id, old_seed);
        assert!(migrated.workspace.is_none());
        assert!(migrated.selected_target_id.is_none());
        assert_eq!(store.load().expect("persisted recovery"), migrated);
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn replacement_at_same_path_gets_distinct_state_and_rejects_saved_identity() {
        let (_root_guard, _root, store) = test_store();
        let (_parent_guard, parent) = test_directory("workspace-parent");
        let workspace_path = parent.join("workspace");
        let moved_path = parent.join("workspace-moved");
        fs::create_dir(&workspace_path).expect("workspace");
        let original = validate_workspace(&workspace_path).expect("original workspace");
        let seed = Uuid::now_v7().to_string();
        let original_storage = store
            .managed_workspace_storage(
                &seed,
                &original.path,
                original.identity.as_ref().expect("original identity"),
            )
            .expect("original storage");
        fs::write(original_storage.instance_dir.join("old-state"), b"old").expect("old marker");

        fs::rename(&workspace_path, &moved_path).expect("rename original");
        fs::create_dir(&workspace_path).expect("replacement");
        assert!(revalidate_workspace(&original).is_err());

        let replacement = validate_workspace(&workspace_path).expect("replacement workspace");
        let replacement_storage = store
            .managed_workspace_storage(
                &seed,
                &replacement.path,
                replacement.identity.as_ref().expect("replacement identity"),
            )
            .expect("replacement storage");
        assert_ne!(original.identity, replacement.identity);
        assert_ne!(
            original_storage.instance_dir,
            replacement_storage.instance_dir
        );
        assert_ne!(
            original_storage.instance_id,
            replacement_storage.instance_id
        );
        assert!(!replacement_storage.instance_dir.join("old-state").exists());
    }

    #[test]
    fn offline_self_test_uses_distinct_app_private_runtime_and_workspace() {
        let (_root_guard, _root, store) = test_store();

        let storage = store.self_test_storage().expect("self-test storage");

        assert_ne!(storage.instance_dir, storage.workspace);
        assert_eq!(storage.instance_dir.parent(), storage.workspace.parent());
        assert!(storage.instance_dir.is_dir());
        assert!(storage.workspace.is_dir());
    }

    #[test]
    fn offline_self_test_uses_versioned_runtime_and_leaves_legacy_state() {
        let (_root_guard, _root, store) = test_store();
        let storage = store.self_test_storage().expect("self-test storage");
        let legacy_runtime = storage
            .instance_dir
            .parent()
            .expect("self-test root")
            .join("runtime");
        ensure_private_directory(&legacy_runtime).expect("legacy runtime");

        assert_eq!(
            storage
                .instance_dir
                .file_name()
                .and_then(|name| name.to_str()),
            Some(SELF_TEST_RUNTIME_DIRECTORY)
        );
        assert_ne!(storage.instance_dir, legacy_runtime);
        assert!(legacy_runtime.is_dir());
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "requires Windows Credential Manager and loopback networking"]
    async fn offline_self_test_launches_sidecar_and_completes_echo_run() {
        let home_guard = DesktopSelfTestHomeGuard::in_local_app_data();
        let home = ColossusHome::ensure_at(&home_guard.path).expect("home");
        let store = SettingsStore::open_home(home).expect("Desktop store");

        crate::managed_runtime::self_test(
            &crate::state::AppState::default(),
            &store,
            &DesktopSettings::default(),
        )
        .await
        .expect("offline self-test");
    }

    #[cfg(unix)]
    #[test]
    fn settings_store_rejects_group_access() {
        let root = tempfile::tempdir().expect("root");
        let canonical_root = fs::canonicalize(root.path()).expect("canonical root");
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o750)).expect("permissions");
        assert!(SettingsStore::open(canonical_root).is_err());
    }

    #[test]
    fn persisted_settings_reject_unknown_fields() {
        let (_root_guard, canonical_root, store) = test_store();
        let path = canonical_root.join(SETTINGS_FILE);
        write_private_file(
            &path,
            br#"{"schemaVersion":1,"managedInstanceId":"00000000-0000-0000-0000-000000000001","workspace":null,"provider":null,"accessProfile":"development","terminalEnabled":false,"selectedTargetId":null,"apiKey":"secret"}"#,
        )
        .expect("write");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("permissions");
        assert!(store.load().is_err());
    }

    #[test]
    fn persisted_external_targets_cannot_select_arbitrary_keychain_entries() {
        let (_root_guard, canonical_root, store) = test_store();
        let instance_id = Uuid::now_v7().to_string();
        let certificate_sha256 = "a".repeat(64);
        let settings = DesktopSettings {
            external_targets: vec![ExternalTargetSetting {
                target_id: Uuid::now_v7().to_string(),
                label: "Imported daemon".into(),
                instance_id: instance_id.clone(),
                certificate_sha256: certificate_sha256.clone(),
                public_api_dir: test_public_api_dir("imported"),
                credential_service: "com.example.unrelated".into(),
                credential_account: "private-mail-password".into(),
                requires_credential_enrollment: false,
            }],
            ..DesktopSettings::default()
        };
        let path = canonical_root.join(SETTINGS_FILE);
        write_private_file(
            &path,
            &serde_json::to_vec(&settings).expect("settings json"),
        )
        .expect("write");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("permissions");

        let loaded = store.load().expect("load normalized settings");
        let target = loaded.external_targets.first().expect("external target");
        assert_eq!(target.credential_service, EXTERNAL_KEYRING_SERVICE);
        assert_eq!(
            target.credential_account,
            external_credential_account(&instance_id, &certificate_sha256)
                .expect("bound credential account")
        );
        assert!(target.requires_credential_enrollment);
    }

    #[test]
    fn external_labels_reject_directional_spoofing() {
        assert!(valid_external_label("Production daemon"));
        assert!(!valid_external_label("Production\u{202e}nimda"));
        assert!(!valid_external_label("Production\u{2066}daemon"));
    }

    #[cfg(windows)]
    #[test]
    fn private_connection_paths_reject_remote_windows_namespaces() {
        assert!(valid_private_absolute_path(Path::new(
            r"C:\Users\test\AppData\Local\colossus-api"
        )));
        assert!(!valid_private_absolute_path(Path::new(
            r"\\server\share\colossus-api"
        )));
        assert!(!valid_private_absolute_path(Path::new(
            r"\\?\UNC\server\share\colossus-api"
        )));
    }

    #[test]
    fn persisted_external_targets_reject_directional_spoofing() {
        let (_root_guard, canonical_root, store) = test_store();
        let instance_id = Uuid::now_v7().to_string();
        let certificate_sha256 = "a".repeat(64);
        let settings = DesktopSettings {
            external_targets: vec![ExternalTargetSetting {
                target_id: Uuid::now_v7().to_string(),
                label: "Production\u{202e}nimda".into(),
                instance_id: instance_id.clone(),
                certificate_sha256: certificate_sha256.clone(),
                public_api_dir: test_public_api_dir("spoofed"),
                credential_service: EXTERNAL_KEYRING_SERVICE.into(),
                credential_account: external_credential_account(&instance_id, &certificate_sha256)
                    .expect("bound credential account"),
                requires_credential_enrollment: false,
            }],
            ..DesktopSettings::default()
        };
        let path = canonical_root.join(SETTINGS_FILE);
        write_private_file(
            &path,
            &serde_json::to_vec(&settings).expect("settings json"),
        )
        .expect("write");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("permissions");

        assert!(store.load().is_err());
    }

    #[test]
    fn bounded_external_target_set_fits_private_settings_storage() {
        let (_root_guard, _canonical_root, store) = test_store();
        let targets = (0..MAX_EXTERNAL_TARGETS)
            .map(|index| {
                let instance_id = Uuid::now_v7().to_string();
                let certificate_sha256 = "a".repeat(64);
                let credential_account =
                    external_credential_account(&instance_id, &certificate_sha256)
                        .expect("credential binding");
                ExternalTargetSetting {
                    target_id: Uuid::now_v7().to_string(),
                    label: format!("External daemon {index}"),
                    instance_id,
                    certificate_sha256,
                    public_api_dir: test_public_api_dir(&index.to_string()),
                    credential_service: EXTERNAL_KEYRING_SERVICE.into(),
                    credential_account,
                    requires_credential_enrollment: false,
                }
            })
            .collect::<Vec<_>>();
        let settings = DesktopSettings {
            selected_target_id: targets.first().map(|target| target.target_id.clone()),
            external_targets: targets,
            ..DesktopSettings::default()
        };

        store.save(&settings).expect("save bounded target set");
        assert_eq!(store.load().expect("load bounded target set"), settings);
    }
}
