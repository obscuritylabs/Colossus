//! Owner-private non-secret binding of one CLI enrollment to its local runtime.
use super::*;
use colossus_home::{ConfinedFile, ConfinedRoot, WorkspaceIdentity};
use fs4::fs_std::FileExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::io::{Read as _, Write as _};

const MAX_METADATA: u64 = 65_536;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Grant {
    pub scopes: Vec<String>,
    pub roles: Vec<String>,
    pub tools: Vec<String>,
    pub allow_approvals: bool,
    pub enable_human_terminals: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub version: u16,
    pub name: String,
    pub workspace: PathBuf,
    pub workspace_identity_version: u16,
    pub workspace_identity_sha256: String,
    pub config_path: PathBuf,
    pub config_sha256: String,
    pub public_api_directory: PathBuf,
    pub application_id: String,
    pub keyring_service: String,
    pub keyring_account: String,
    pub vault_key_variable: Option<String>,
    pub grant: Grant,
    pub stage: Stage,
    pub instance_id: Option<String>,
    pub certificate_sha256: Option<String>,
    pub credential_id: Option<String>,
    pub connection_sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stage {
    Planned,
    CredentialIntent,
    Ready,
}

pub(super) struct Metadata {
    root: ConfinedRoot,
    leaf: PathBuf,
    pub directory: ConfinedRoot,
    pub connection: PathBuf,
    pub lock: Option<ConfinedFile>,
}

pub(super) fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn bind_managed_directory(path: &Path) -> Result<ConfinedRoot, Box<dyn Error>> {
    let original = ConfinedRoot::bind(path)?;
    #[cfg(windows)]
    {
        // Private file creation uses Win32 paths. Retain a verified verbatim path
        // before adding hashed partitions and atomic staging leaves, which can
        // exceed MAX_PATH under an ordinary user profile directory.
        let retained = colossus_windows_native::BoundPath::open_directory(original.path())?;
        retained.validate_ancestor_namespace_authority()?;
        retained.validate_private_owner_dacl()?;
        retained.revalidate()?;
        let canonical = ConfinedRoot::bind(retained.canonical_path())?;
        let reopened = colossus_windows_native::BoundPath::open_directory(canonical.path())?;
        if retained.identity() != reopened.identity() {
            return Err("managed Cloud directory changed during canonical binding".into());
        }
        original.revalidate()?;
        retained.revalidate()?;
        reopened.revalidate()?;
        canonical.revalidate()?;
        Ok(canonical)
    }
    #[cfg(not(windows))]
    Ok(original)
}

impl Metadata {
    pub fn open(home: &Path, workspace_home: &Path, name: &str) -> Result<Self, Box<dyn Error>> {
        colossus_ports::CredentialKey::new("cloud-connector", name)?;
        let home = bind_managed_directory(home)?;
        let root = ConfinedRoot::bind(
            home.prepare_directory(&Path::new("cloud-connector").join("managed-clients"))?,
        )?;
        let selector = digest(name.as_bytes());
        let lock = root.open_file(Path::new(&format!("{selector}.lock")))?;
        if !lock.file().try_lock_exclusive()? {
            return Err(
                "this managed Cloud connection is busy; disconnect it before changing enrollment"
                    .into(),
            );
        }
        lock.revalidate(&root)?;
        let workspace = bind_managed_directory(workspace_home)?;
        let directory =
            ConfinedRoot::bind(workspace.prepare_directory(&Path::new("cloud").join(&selector))?)?;
        let connection = directory.path().join("connection.json");
        Ok(Self {
            root,
            leaf: PathBuf::from(format!("{selector}.json")),
            directory,
            connection,
            lock: Some(lock),
        })
    }

    pub fn api_directory(&self) -> Result<PathBuf, Box<dyn Error>> {
        Ok(self.directory.prepare_directory(Path::new("public-api"))?)
    }

    pub fn load(&self) -> Result<Option<Binding>, Box<dyn Error>> {
        let file = match self.root.open_existing_file(&self.leaf) {
            Ok(file) => file,
            Err(colossus_home::HomeError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.file().take(MAX_METADATA + 1).read_to_end(&mut bytes)?;
        file.revalidate(&self.root)?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_METADATA {
            return Err("managed Cloud metadata is incomplete; retain it for inspection".into());
        }
        let binding: Binding = serde_json::from_slice(&bytes)
            .map_err(|_| "managed Cloud metadata is invalid; restore its exact protected record")?;
        if binding.version != 1 {
            return Err("managed Cloud metadata version is unsupported".into());
        }
        Ok(Some(binding))
    }

    pub fn save(&self, binding: &Binding) -> Result<(), Box<dyn Error>> {
        write_atomic(&self.root, &self.leaf, &serde_json::to_vec(binding)?)
    }

    pub fn connection(&self, binding: &mut Binding) -> Result<(), Box<dyn Error>> {
        let instance = binding
            .instance_id
            .as_deref()
            .ok_or("managed local instance is unconfirmed")?;
        let pin = binding
            .certificate_sha256
            .as_deref()
            .ok_or("managed local TLS pin is unconfirmed")?;
        let mut config = serde_json::json!({
            "descriptor": binding.public_api_directory.join("endpoint.json"),
            "certificate": binding.public_api_directory.join("certificate.pem"),
            "instance_id": instance, "certificate_sha256": pin,
            "keyring_service": binding.keyring_service, "keyring_account": binding.keyring_account,
        });
        if let Some(variable) = &binding.vault_key_variable {
            config["headless_authority"] = serde_json::json!({
                "directory": binding.public_api_directory.join("headless-credentials"),
                "key_variable": variable,
            });
        }
        let bytes = serde_json::to_vec(&config)?;
        let expected = digest(&bytes);
        if let Some(retained) = &binding.connection_sha256 {
            if retained != &expected {
                return Err(
                    "managed connection references changed; do not replace its independent pin"
                        .into(),
                );
            }
            let file = self
                .directory
                .open_existing_file(Path::new("connection.json"))?;
            let mut actual = Vec::new();
            file.file()
                .take(MAX_METADATA + 1)
                .read_to_end(&mut actual)?;
            file.revalidate(&self.directory)?;
            if digest(&actual) != expected {
                return Err("managed connection references are missing or changed; restore the retained record".into());
            }
        } else {
            // Recover a lost write response only for the exact retained native
            // instance/pin. This never adopts a descriptor's untrusted fingerprint.
            let file = self.directory.open_file(Path::new("connection.json"))?;
            let mut actual = Vec::new();
            file.file()
                .take(MAX_METADATA + 1)
                .read_to_end(&mut actual)?;
            file.revalidate(&self.directory)?;
            if actual.is_empty() {
                drop(file);
                write_atomic(&self.directory, Path::new("connection.json"), &bytes)?;
            } else if actual != bytes {
                return Err("an unrecognized managed connection file already exists".into());
            }
            binding.connection_sha256 = Some(expected);
            self.save(binding)?;
        }
        Ok(())
    }
}

impl Binding {
    pub fn planned(
        arguments: &LocalArguments,
        identity: &WorkspaceIdentity,
        config_path: &Path,
        config_sha256: String,
        api_directory: PathBuf,
    ) -> Self {
        let name_hash = digest(arguments.storage.name.as_bytes());
        let account = digest(
            format!(
                "{}\0{}\0{}",
                arguments.storage.name,
                identity.sha256(),
                api_directory.display()
            )
            .as_bytes(),
        );
        Self {
            version: 1,
            name: arguments.storage.name.clone(),
            workspace: identity.canonical_path().to_owned(),
            workspace_identity_version: identity.version(),
            workspace_identity_sha256: identity.sha256().to_owned(),
            config_path: config_path.to_owned(),
            config_sha256,
            public_api_directory: api_directory,
            application_id: format!("app:colossus-cloud-cli-{}", &name_hash[..16]),
            keyring_service: "dev.obscuritylabs.colossus.cli.cloud".into(),
            keyring_account: account,
            vault_key_variable: arguments.storage.vault_key_variable.clone(),
            grant: Grant {
                scopes: Vec::new(),
                roles: Vec::new(),
                tools: Vec::new(),
                allow_approvals: arguments.allow_approvals,
                enable_human_terminals: arguments.enable_human_terminals,
            },
            stage: Stage::Planned,
            instance_id: None,
            certificate_sha256: None,
            credential_id: None,
            connection_sha256: None,
        }
    }

    pub fn validate_context(&self, expected: &Self) -> Result<(), Box<dyn Error>> {
        if self.name != expected.name
            || self.workspace != expected.workspace
            || self.workspace_identity_version != expected.workspace_identity_version
            || self.workspace_identity_sha256 != expected.workspace_identity_sha256
            || self.config_path != expected.config_path
            || self.config_sha256 != expected.config_sha256
            || self.public_api_directory != expected.public_api_directory
            || self.application_id != expected.application_id
            || self.keyring_service != expected.keyring_service
            || self.keyring_account != expected.keyring_account
            || self.vault_key_variable != expected.vault_key_variable
            || self.grant.allow_approvals != expected.grant.allow_approvals
            || self.grant.enable_human_terminals != expected.grant.enable_human_terminals
        {
            return Err("this enrollment name is bound to a different workspace, configuration or access choice; use its original selection or explicitly choose a new --name".into());
        }
        Ok(())
    }
}

fn write_atomic(root: &ConfinedRoot, leaf: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if bytes.len() as u64 > MAX_METADATA {
        return Err("managed Cloud metadata exceeds its bound".into());
    }
    let destination = root.open_file(leaf)?;
    let staged = root.open_file(Path::new(&format!(
        "stage-{}.json",
        uuid::Uuid::now_v7().simple()
    )))?;
    let mut stream = staged.file();
    stream.write_all(bytes)?;
    stream.sync_all()?;
    staged.revalidate(root)?;
    destination.revalidate(root)?;
    root.replace_existing_file(&staged, &destination)?;
    Ok(())
}
