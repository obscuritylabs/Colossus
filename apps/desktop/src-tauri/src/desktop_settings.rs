use colossus_sdk::{
    WorkspaceIdentity, validate_managed_model_identifier, validate_managed_provider_base_url,
};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs::{self, OpenOptions},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::dto::CommandErrorDto;

#[cfg(not(windows))]
use std::fs::File;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};

const SETTINGS_SCHEMA_VERSION: u16 = 2;
const SETTINGS_FILE: &str = "desktop-settings.json";
const MANAGED_DIRECTORY: &str = "managed-local";
const TRUST_DIRECTORY: &str = "trust";
const MAX_CA_BUNDLE_BYTES: u64 = 4 * 1024 * 1024;
const SELF_TEST_DIRECTORY: &str = "offline-self-test";
const SELF_TEST_RUNTIME_DIRECTORY: &str = "runtime";
const SELF_TEST_WORKSPACE_DIRECTORY: &str = "workspace";
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
const PROVIDER_KEYRING_SERVICE: &str = "com.obscuritylabs.colossus.desktop.provider";
pub(crate) const EXTERNAL_KEYRING_SERVICE: &str = "com.obscuritylabs.colossus.desktop.external";
const WORKSPACE_PARTITION_DOMAIN: &[u8] = b"colossus-desktop-managed-workspace-v1\0";
const WORKSPACE_INSTANCE_DOMAIN: &[u8] = b"colossus-desktop-managed-instance-v1\0";
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccessProfileSetting {
    Minimal,
    #[default]
    Development,
    #[serde(rename = "allow_all")]
    LegacyAllowAll,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum ProviderKindSetting {
    #[serde(rename = "openai_responses", alias = "open_ai_responses")]
    OpenAiResponses,
    #[serde(rename = "openai_compatible", alias = "open_ai_compatible")]
    OpenAiCompatible,
}

pub(crate) const fn provider_base_url(kind: ProviderKindSetting) -> &'static str {
    match kind {
        ProviderKindSetting::OpenAiResponses => OPENAI_BASE_URL,
        ProviderKindSetting::OpenAiCompatible => OPENROUTER_BASE_URL,
    }
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderSetting {
    pub(crate) profile: String,
    pub(crate) kind: ProviderKindSetting,
    pub(crate) base_url: String,
    pub(crate) credential_id: Option<String>,
    pub(crate) timeout_ms: u64,
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

const fn legacy_external_credential_binding() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DesktopSettings {
    pub(crate) schema_version: u16,
    pub(crate) managed_instance_id: String,
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
            workspace: None,
            providers: Vec::new(),
            models: Vec::new(),
            model_roles: BTreeMap::new(),
            pending_provider_cleanup_ids: Vec::new(),
            additional_ca_bundle: None,
            access_profile: AccessProfileSetting::Development,
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
        self.providers
            .iter()
            .filter_map(|provider| provider.credential_id.as_deref())
            .collect()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SettingsStore {
    root: PathBuf,
}

pub(crate) struct ManagedWorkspaceStorage {
    pub(crate) instance_id: String,
    pub(crate) instance_dir: PathBuf,
}

impl SettingsStore {
    pub(crate) fn open(root: PathBuf) -> Result<Self, CommandErrorDto> {
        ensure_private_directory(&root)?;
        Ok(Self { root })
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
        let schema_version = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("schemaVersion")
                    .and_then(serde_json::Value::as_u64)
            })
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(storage_error)?;
        let (mut settings, migrated_provider_config) = if schema_version == 1 {
            let legacy: LegacyDesktopSettingsV1 =
                serde_json::from_slice(&bytes).map_err(|_| storage_error())?;
            (migrate_v1_settings(legacy)?, true)
        } else {
            (
                serde_json::from_slice(&bytes).map_err(|_| storage_error())?,
                false,
            )
        };
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
            if settings.selected_target_id.as_deref() == Some("managed-local") {
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
        if settings.access_profile == AccessProfileSetting::LegacyAllowAll {
            settings.access_profile = AccessProfileSetting::Development;
        }
        validate_settings(&settings)?;
        if let Some(bundle) = &settings.additional_ca_bundle {
            self.ca_bundle_path(bundle)?;
        }
        if migrated_legacy_workspace || migrated_provider_config {
            self.save(&settings)?;
        }
        Ok(settings)
    }

    pub(crate) fn has_persisted_settings(&self) -> bool {
        self.root.join(SETTINGS_FILE).is_file()
    }

    pub(crate) fn save(&self, settings: &DesktopSettings) -> Result<(), CommandErrorDto> {
        validate_settings(settings)?;
        let bytes = serde_json::to_vec(settings).map_err(|_| storage_error())?;
        if bytes.len() > usize::try_from(MAX_SETTINGS_BYTES).unwrap_or(usize::MAX) {
            return Err(storage_error());
        }
        let temporary = self
            .root
            .join(format!(".{SETTINGS_FILE}.{}.tmp", Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary).map_err(|_| storage_error())?;
        let result = (|| {
            file.write_all(&bytes).map_err(|_| storage_error())?;
            file.sync_all().map_err(|_| storage_error())?;
            drop(file);
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
        let managed_root = self.root.join(MANAGED_DIRECTORY);
        ensure_private_directory(&managed_root)?;
        let directory = managed_root.join(hex::encode(&partition[..]));
        ensure_private_directory(&directory)?;

        let mut instance_digest = Sha256::new();
        instance_digest.update(WORKSPACE_INSTANCE_DOMAIN);
        instance_digest.update(seed.as_bytes());
        instance_digest.update(partition);
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
}

pub(crate) fn application_support_root() -> Result<PathBuf, CommandErrorDto> {
    BaseDirs::new()
        .map(|directories| {
            directories
                .data_dir()
                .join("com.obscuritylabs.colossus.desktop")
        })
        .ok_or_else(storage_error)
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
    Ok(DesktopSettings {
        schema_version: SETTINGS_SCHEMA_VERSION,
        managed_instance_id: legacy.managed_instance_id,
        workspace: legacy.workspace,
        providers: Vec::new(),
        models: Vec::new(),
        model_roles: BTreeMap::new(),
        pending_provider_cleanup_ids: pending,
        additional_ca_bundle: None,
        access_profile: legacy.access_profile,
        terminal_enabled: legacy.terminal_enabled,
        local_terminal_consent_version: 0,
        selected_target_id: legacy
            .selected_target_id
            .filter(|target| target != "managed-local"),
        external_targets: legacy.external_targets,
        legacy_connection_migrated: legacy.legacy_connection_migrated,
    })
}

fn validate_settings(settings: &DesktopSettings) -> Result<(), CommandErrorDto> {
    if settings.schema_version != SETTINGS_SCHEMA_VERSION
        || settings.local_terminal_consent_version > LOCAL_TERMINAL_CONSENT_VERSION
        || !Uuid::parse_str(&settings.managed_instance_id).is_ok_and(|value| !value.is_nil())
        || settings.external_targets.len() > MAX_EXTERNAL_TARGETS
        || settings.pending_provider_cleanup_ids.len() > MAX_PENDING_PROVIDER_CLEANUPS
        || settings
            .additional_ca_bundle
            .as_ref()
            .is_some_and(|bundle| validate_ca_bundle_setting(bundle).is_err())
    {
        return Err(storage_error());
    }
    let target_ids = validate_external_targets(settings)?;
    if settings
        .selected_target_id
        .as_deref()
        .is_some_and(|value| value != "managed-local" && !target_ids.contains(value))
        || settings.workspace.as_ref().is_some_and(invalid_workspace)
        || settings.access_profile == AccessProfileSetting::LegacyAllowAll
    {
        return Err(storage_error());
    }
    let credential_ids = validate_managed_configuration(settings)?;
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
    if settings.providers.len() > MAX_MANAGED_PROVIDERS
        || settings.models.len() > MAX_MANAGED_MODELS
        || settings.providers.is_empty() != settings.models.is_empty()
        || settings.models.is_empty() != settings.model_roles.is_empty()
        || (!settings.models.is_empty() && !settings.model_roles.contains_key("primary"))
    {
        return Err(storage_error());
    }
    let mut provider_profiles = BTreeSet::new();
    let mut credential_ids = BTreeSet::new();
    for provider in &settings.providers {
        if !valid_profile_name(&provider.profile)
            || !provider_profiles.insert(provider.profile.as_str())
            || provider.timeout_ms == 0
            || validate_managed_provider_base_url(&provider.base_url).is_err()
            || provider
                .credential_id
                .as_deref()
                .is_some_and(|id| !valid_opaque_id(id))
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
    #[cfg(windows)]
    {
        let binding =
            colossus_windows_native::BoundPath::open_file(path).map_err(|_| storage_error())?;
        binding
            .validate_private_owner_dacl()
            .and_then(|()| binding.revalidate())
            .map_err(|_| storage_error())?;
    }
    Ok(())
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
    let canonical = fs::canonicalize(path).map_err(|_| storage_error())?;
    let metadata = fs::symlink_metadata(path).map_err(|_| storage_error())?;
    if canonical != path || !metadata.file_type().is_dir() {
        return Err(storage_error());
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
                timeout_ms: 120_000,
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
            }],
            model_roles: BTreeMap::from([("primary".into(), "primary".into())]),
            ..DesktopSettings::default()
        }
    }

    fn test_store() -> (tempfile::TempDir, PathBuf, SettingsStore) {
        let parent = tempfile::tempdir().expect("store parent");
        let root = fs::canonicalize(parent.path())
            .expect("canonical store parent")
            .join("store");
        let store = SettingsStore::open(root.clone()).expect("store");
        let canonical_root = fs::canonicalize(&root).expect("canonical store root");
        assert_eq!(canonical_root, root);
        (parent, canonical_root, store)
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
            serde_json::to_string(&ProviderKindSetting::OpenAiResponses).expect("serialize"),
            r#""openai_responses""#,
        );
        assert_eq!(
            serde_json::to_string(&ProviderKindSetting::OpenAiCompatible).expect("serialize"),
            r#""openai_compatible""#,
        );
        assert_eq!(
            serde_json::from_str::<ProviderKindSetting>(r#""open_ai_responses""#)
                .expect("legacy responses"),
            ProviderKindSetting::OpenAiResponses,
        );
        assert_eq!(
            serde_json::from_str::<ProviderKindSetting>(r#""open_ai_compatible""#)
                .expect("legacy compatible"),
            ProviderKindSetting::OpenAiCompatible,
        );
        assert_eq!(
            serde_json::from_str::<ProviderKindSetting>(r#""openai_responses""#)
                .expect("canonical responses"),
            ProviderKindSetting::OpenAiResponses,
        );
        assert_eq!(
            serde_json::from_str::<ProviderKindSetting>(r#""openai_compatible""#)
                .expect("canonical compatible"),
            ProviderKindSetting::OpenAiCompatible,
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
            ProviderKindSetting::OpenAiCompatible,
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
            ProviderKindSetting::OpenAiCompatible,
            OPENROUTER_BASE_URL,
            Some(Uuid::now_v7().to_string()),
        );
        store.save(&settings).expect("save");
        assert_eq!(store.load().expect("load"), settings);
        let bytes = fs::read(canonical_root.join(SETTINGS_FILE)).expect("settings");
        assert!(!String::from_utf8_lossy(&bytes).contains("provider-secret"));
    }

    #[test]
    fn settings_reject_renderer_visible_model_controls() {
        let mut settings = configured_settings(
            ProviderKindSetting::OpenAiCompatible,
            OPENROUTER_BASE_URL,
            None,
        );
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
        fs::write(
            &path,
            serde_json::to_vec(&encoded).expect("legacy settings"),
        )
        .expect("settings");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("permissions");

        let migrated = store.load().expect("migrate settings");
        assert_eq!(migrated.schema_version, 2);
        assert!(migrated.providers.is_empty());
        assert!(migrated.models.is_empty());
        assert!(migrated.model_roles.is_empty());
        assert_eq!(migrated.pending_provider_cleanup_ids, [credential_id]);
        assert_eq!(migrated.access_profile, AccessProfileSetting::Minimal);
        assert!(migrated.terminal_enabled);
        assert!(migrated.selected_target_id.is_none());
        assert!(migrated.legacy_connection_migrated);
    }

    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn managed_state_and_instance_identity_are_isolated_per_workspace() {
        let (_root_guard, _root, store) = test_store();
        let first_workspace = tempfile::tempdir().expect("first workspace");
        let second_workspace = tempfile::tempdir().expect("second workspace");
        let seed = Uuid::now_v7().to_string();
        let first_path = fs::canonicalize(first_workspace.path()).expect("first canonical");
        let second_path = fs::canonicalize(second_workspace.path()).expect("second canonical");
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
    }

    #[test]
    fn path_only_same_path_replacement_requires_reselection_and_preserves_provider() {
        let (_root_guard, root, store) = test_store();
        let workspace_parent = tempfile::tempdir().expect("workspace parent");
        let workspace_parent =
            fs::canonicalize(workspace_parent.path()).expect("canonical workspace parent");
        let workspace = workspace_parent.join("workspace");
        let moved = workspace_parent.join("workspace-moved");
        fs::create_dir(&workspace).expect("workspace");
        let old_seed = Uuid::now_v7().to_string();
        let mut legacy = configured_settings(
            ProviderKindSetting::OpenAiCompatible,
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
        fs::write(&path, serde_json::to_vec(&legacy).expect("legacy JSON"))
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
        let workspace = tempfile::tempdir().expect("workspace");
        let workspace = fs::canonicalize(workspace.path()).expect("canonical workspace");
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
        fs::write(&path, serde_json::to_vec(&legacy).expect("legacy JSON"))
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
        let workspace = tempfile::tempdir().expect("workspace");
        let workspace = fs::canonicalize(workspace.path()).expect("canonical workspace");
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
        fs::write(&path, serde_json::to_vec(&encoded).expect("preview JSON"))
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
        fs::write(&path, serde_json::to_vec(&legacy).expect("legacy JSON"))
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
        let parent = tempfile::tempdir().expect("workspace parent");
        let parent = fs::canonicalize(parent.path()).expect("canonical parent");
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
        fs::write(
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
        fs::write(&path, serde_json::to_vec(&settings).expect("settings json")).expect("write");
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
        fs::write(&path, serde_json::to_vec(&settings).expect("settings json")).expect("write");
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
