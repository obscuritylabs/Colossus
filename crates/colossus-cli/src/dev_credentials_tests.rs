use super::*;
use crate::{
    dev_credentials_apply,
    dev_credentials_args::{DevelopmentCredentialSources, DevelopmentCredentialsAction},
    dev_credentials_plan, dev_credentials_sources,
    public_api_admin::{OsCredentialStore, PublicApiAdminError, SecretStore},
};
use colossus_credentials::{
    DevelopmentAuthority, DevelopmentStoreScope, PlatformKeyStore, headless_credential_account,
};
use zeroize::Zeroizing;

struct PrivateRootFixture {
    _directory: PrivateTempDir,
    path: PathBuf,
}
impl PrivateRootFixture {
    fn path(&self) -> &Path {
        &self.path
    }
}
fn private_root() -> PrivateRootFixture {
    let directory = private_tempdir();
    let path = fs::canonicalize(directory.path()).unwrap();
    PrivateRootFixture {
        _directory: directory,
        path,
    }
}

fn source_args(home: &Path) -> DevelopmentCredentialSources {
    DevelopmentCredentialSources {
        home: home.to_owned(),
        desktop_vault: false,
        control_plane_vault: false,
        connector_source_home: None,
        connector_enrollment: None,
        public_api_directories: Vec::new(),
        client_configs: Vec::new(),
        journal_configs: Vec::new(),
        runtime_oauth_storage: Vec::new(),
        fresh_empty: false,
    }
}

#[test]
fn development_commands_accept_metadata_paths_but_require_explicit_reviewed_apply() {
    let parsed = Cli::try_parse_from([
        "colossus",
        "dev-credentials",
        "init",
        "--home",
        "/private/home",
        "--workspace",
        "/private/workspace",
    ])
    .unwrap();
    assert!(matches!(
        parsed.command,
        Command::DevelopmentCredentials(DevelopmentCredentialsCommand {
            command: DevelopmentCredentialsAction::Init { .. }
        })
    ));
    assert!(
        Cli::try_parse_from([
            "colossus",
            "dev-credentials",
            "rewrap",
            "--plan-file",
            "/private/plan.json",
            "--expected-plan-sha256",
            &"a".repeat(64)
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "colossus",
            "dev-credentials",
            "init",
            "--home",
            "/private/home",
            "--workspace",
            "/private/workspace",
            "--key",
            "not-an-accepted-secret-input"
        ])
        .is_err()
    );
    let source = include_str!("entrypoint.rs");
    assert!(
        source
            .find("if let Command::DevelopmentCredentials")
            .unwrap()
            < source.find("ColossusHome::resolve_and_ensure").unwrap()
    );
}

#[test]
fn fresh_activation_is_explicit_metadata_only_and_conflicting_plan_bytes_fail_closed() {
    let home = private_root();
    let workspace = private_root();
    let root = ConfinedRoot::bind(home.path()).unwrap();
    let authority =
        DevelopmentAuthority::initialize(&root, &[workspace.path().to_owned()]).unwrap();
    assert!(
        !DevelopmentAuthority::metadata(&root, &DevelopmentAuthority::path_for_home(&root), &[])
            .unwrap()
            .active
    );
    let mut args = source_args(home.path());
    args.fresh_empty = true;
    let plan = dev_credentials_sources::make_plan(&args, workspace.path()).unwrap();
    let file = DevelopmentAuthority::path_for_home(&root).join("fresh.plan.json");
    let digest = dev_credentials_plan::write_plan(&file, &plan).unwrap();
    assert_eq!(
        dev_credentials_plan::read_plan(&file, &digest).unwrap(),
        plan
    );
    assert!(dev_credentials_plan::read_plan(&file, &"b".repeat(64)).is_err());
    assert!(dev_credentials_plan::write_plan(&file, &plan).is_err());
    assert_eq!(
        dev_credentials_apply::apply(&plan, &OsCredentialStore).unwrap(),
        (0, 0)
    );
    assert!(
        DevelopmentAuthority::metadata(&root, &DevelopmentAuthority::path_for_home(&root), &[])
            .unwrap()
            .active
    );
    let _ = authority;
}

#[derive(Default)]
struct ReadOnlySecrets {
    values: BTreeMap<(String, String), Vec<u8>>,
}
impl SecretStore for ReadOnlySecrets {
    fn read(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, PublicApiAdminError> {
        Ok(self
            .values
            .get(&(service.into(), account.into()))
            .cloned()
            .map(Zeroizing::new))
    }
    fn write(&self, _: &str, _: &str, _: &[u8]) -> Result<(), PublicApiAdminError> {
        panic!("source writes are forbidden")
    }
    fn delete(&self, _: &str, _: &str) -> Result<(), PublicApiAdminError> {
        panic!("source deletion is forbidden")
    }
}

#[test]
fn offline_public_api_rewrap_preserves_original_seeds_and_refuses_missing_conflicting_material() {
    let home = private_root();
    let workspace = private_root();
    let root = ConfinedRoot::bind(home.path()).unwrap();
    let authority =
        DevelopmentAuthority::initialize(&root, &[workspace.path().to_owned()]).unwrap();
    let api_dir = root.prepare_directory(Path::new("api")).unwrap();
    let api = ConfinedRoot::bind(&api_dir).unwrap();
    api.open_file(Path::new(".public-api.lock")).unwrap();
    api.open_file(Path::new("certificate.pem"))
        .unwrap()
        .file()
        .write_all(b"synthetic public certificate")
        .unwrap();
    let mut args = source_args(home.path());
    args.public_api_directories.push(api_dir.clone());
    let plan = dev_credentials_sources::make_plan(&args, workspace.path()).unwrap();
    let service = public_api_admin::namespace_service(&api_dir);
    let mut secrets = ReadOnlySecrets::default();
    for (index, account) in [
        public_api_admin::AUTHENTICATION_ROOT_ACCOUNT,
        public_api_admin::TLS_SEED_ACCOUNT,
        public_api_admin::INSTANCE_SEED_ACCOUNT,
    ]
    .iter()
    .enumerate()
    {
        secrets.values.insert(
            (service.clone(), account.to_string()),
            vec![(index + 1) as u8; 32],
        );
    }
    let saved = secrets.values.clone();
    let absent = ReadOnlySecrets::default();
    assert!(dev_credentials_apply::apply(&plan, &absent).is_err());
    assert!(
        !DevelopmentAuthority::metadata(&root, &DevelopmentAuthority::path_for_home(&root), &[])
            .unwrap()
            .active
    );
    assert_eq!(
        dev_credentials_apply::apply(&plan, &secrets).unwrap(),
        (3, 0)
    );
    assert_eq!(secrets.values, saved);
    assert_eq!(
        dev_credentials_apply::apply(&plan, &secrets).unwrap(),
        (0, 3)
    );
    let target = authority.store(DevelopmentStoreScope::PublicApi).unwrap();
    for ((service, account), original) in &saved {
        assert_eq!(
            target
                .read(&headless_credential_account(service, account))
                .unwrap()
                .unwrap()
                .as_slice(),
            original.as_slice()
        );
    }
    secrets.values.insert(
        (
            service,
            public_api_admin::AUTHENTICATION_ROOT_ACCOUNT.into(),
        ),
        vec![99; 32],
    );
    assert!(dev_credentials_apply::apply(&plan, &secrets).is_err());
    assert_eq!(
        target
            .read(&headless_credential_account(
                &public_api_admin::namespace_service(&api_dir),
                public_api_admin::AUTHENTICATION_ROOT_ACCOUNT
            ))
            .unwrap()
            .unwrap()
            .as_slice(),
        &[1; 32]
    );
    // Normal worker shutdown removes publication leaves, not its three original
    // OS seed accounts. Offline planning must not restart that worker.
    secrets.values = saved;
    fs::remove_file(api_dir.join("certificate.pem")).unwrap();
    let stopped = dev_credentials_sources::make_plan(&args, workspace.path()).unwrap();
    assert!(matches!(
        &stopped.sources[0],
        dev_credentials_plan::Source::PublicApi {
            certificate_file_sha256: None,
            ..
        }
    ));
    assert_eq!(
        dev_credentials_apply::apply(&stopped, &secrets).unwrap(),
        (0, 3)
    );
    let client = root.open_file(Path::new("client.json")).unwrap();
    let client_bytes = serde_json::to_vec(&json!({
        "descriptor": api_dir.join("endpoint.json"),
        "certificate": api_dir.join("certificate.pem"),
        "instance_id": "00000000-0000-0000-0000-000000000002",
        "certificate_sha256": "f".repeat(64),
        "keyring_service": "synthetic-bound-client",
        "keyring_account": "original-application"
    }))
    .unwrap();
    client.file().write_all(&client_bytes).unwrap();
    let bearer = format!(
        "cls_v1.00000000-0000-0000-0000-000000000003.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([9_u8; 32])
    );
    secrets.values.insert(
        (
            "synthetic-bound-client".into(),
            "original-application".into(),
        ),
        bearer.as_bytes().to_vec(),
    );
    let retained = secrets.values.clone();
    args.client_configs.push(client.path().to_owned());
    let stopped = dev_credentials_sources::make_plan(&args, workspace.path()).unwrap();
    assert_eq!(
        dev_credentials_apply::apply(&stopped, &secrets).unwrap(),
        (1, 3)
    );
    assert_eq!(secrets.values, retained);
    assert_eq!(fs::read(client.path()).unwrap(), client_bytes);
    assert_eq!(
        target
            .read(&headless_credential_account(
                "synthetic-bound-client",
                "original-application"
            ))
            .unwrap()
            .unwrap()
            .as_slice(),
        bearer.as_bytes()
    );
}

#[test]
fn invalid_existing_bearers_are_never_replaced_or_exposed() {
    assert!(dev_credentials_apply::validate_test_bearer(b"not-a-Colossus-bearer").is_err());
    let valid = format!(
        "cls_v1.00000000-0000-0000-0000-000000000001.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7_u8; 32])
    );
    assert!(dev_credentials_apply::validate_test_bearer(valid.as_bytes()).is_ok());
}

#[test]
fn journal_fingerprint_rewinds_retained_handle_without_releasing_offline_lease() {
    use crate::dev_credentials_lease::OfflineFileLease;
    use fs4::fs_std::FileExt;

    let home = private_root();
    let root = ConfinedRoot::bind(home.path()).unwrap();
    let file = root.open_file(Path::new("journal.bin")).unwrap();
    let bytes = vec![7_u8; 32 * 1024 + 3];
    file.file().write_all(&bytes).unwrap();
    file.file().sync_all().unwrap();
    let contender = fs::File::open(file.path()).unwrap();
    let lease = OfflineFileLease::acquire(file, "lease failed", "lease busy").unwrap();

    for _ in 0..2 {
        assert_eq!(
            dev_credentials_plan::confined_file_digest(&root, lease.file()).unwrap(),
            dev_credentials_plan::sha256(&bytes)
        );
        assert!(!FileExt::try_lock_exclusive(&contender).unwrap());
    }
    drop(lease);
    assert!(FileExt::try_lock_exclusive(&contender).unwrap());
    FileExt::unlock(&contender).unwrap();
}

#[test]
fn plaintext_journal_plan_requires_frozen_canonical_metadata_and_retains_source_bytes() {
    use colossus_journal_redb::{DisabledCheckpointSigner, PlaintextKeyProvider, RedbEventJournal};
    let home = private_root();
    let workspace = private_root();
    let root = ConfinedRoot::bind(home.path()).unwrap();
    DevelopmentAuthority::initialize(&root, &[workspace.path().to_owned()]).unwrap();
    let journal_path = root
        .open_file(Path::new("state.redb"))
        .unwrap()
        .path()
        .to_owned();
    let journal = RedbEventJournal::open(
        &journal_path,
        Arc::new(PlaintextKeyProvider),
        Arc::new(DisabledCheckpointSigner),
    )
    .unwrap();
    let config = root.open_file(Path::new("config.yaml")).unwrap();
    let config_text = format!(
        "schemaVersion: 3\nstorage:\n  path: {}\n",
        serde_json::to_string(&journal_path).unwrap()
    );
    config.file().write_all(config_text.as_bytes()).unwrap();
    let mut args = source_args(home.path());
    args.journal_configs.push(config.path().to_owned());
    assert!(dev_credentials_sources::make_plan(&args, workspace.path()).is_err());
    drop(journal);
    let original = fs::read(&journal_path).unwrap();
    let planned = dev_credentials_sources::make_plan(&args, workspace.path()).unwrap();
    assert!(matches!(
        planned.sources[0],
        dev_credentials_plan::Source::PlaintextJournal { .. }
    ));
    assert_eq!(fs::read(&journal_path).unwrap(), original);
    assert_eq!(
        dev_credentials_apply::apply(&planned, &ReadOnlySecrets::default()).unwrap(),
        (0, 0)
    );
    assert_eq!(fs::read(&journal_path).unwrap(), original);
    assert_eq!(fs::read(config.path()).unwrap(), config_text.as_bytes());
}

type KeyReadHook = Arc<dyn Fn(&str) + Send + Sync>;
#[derive(Default)]
struct MemoryVaultKeys(
    std::sync::Mutex<BTreeMap<String, Vec<u8>>>,
    std::sync::Mutex<Option<KeyReadHook>>,
);
impl PlatformKeyStore for MemoryVaultKeys {
    fn read(
        &self,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, colossus_contracts::CredentialError> {
        let hook = self.1.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(account);
        }
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(account)
            .cloned()
            .map(Zeroizing::new))
    }
    fn write(
        &self,
        account: &str,
        bytes: &[u8],
    ) -> Result<(), colossus_contracts::CredentialError> {
        self.0
            .lock()
            .unwrap()
            .insert(account.into(), bytes.to_vec());
        Ok(())
    }
}

#[test]
fn cli_retains_named_source_lease_while_later_vault_sources_are_sealed() {
    use colossus_contracts::{CredentialError, VaultRecord};
    use colossus_credentials::PlatformCredentialVault;
    use colossus_ports::CredentialVault as _;
    use std::sync::atomic::{AtomicBool, Ordering};
    let source_home = private_root();
    let target_home = private_root();
    let workspace = private_root();
    let source_root = ConfinedRoot::bind(source_home.path()).unwrap();
    let source_directory = source_root
        .prepare_directory(Path::new("cloud-connector"))
        .unwrap();
    let source_root = ConfinedRoot::bind(&source_directory).unwrap();
    let keys = Arc::new(MemoryVaultKeys::default());
    let selected = colossus_connector::EnrollmentStore::credential_key("cloud-e2e-cli").unwrap();
    let source = PlatformCredentialVault::with_key_store(
        source_root.clone(),
        "cloud-connector",
        keys.clone(),
    )
    .unwrap();
    source
        .write(
            &selected,
            &VaultRecord::new(b"original-selected-record".to_vec()).unwrap(),
        )
        .unwrap();
    drop(source);
    let target_root = ConfinedRoot::bind(target_home.path()).unwrap();
    DevelopmentAuthority::initialize(&target_root, &[workspace.path().to_owned()]).unwrap();
    let desktop = target_root.prepare_directory(Path::new("desktop")).unwrap();
    let desktop_root = ConfinedRoot::bind(&desktop).unwrap();
    let vault = PlatformCredentialVault::with_key_store(
        desktop_root.clone(),
        "desktop-manual",
        keys.clone(),
    )
    .unwrap();
    vault
        .write(
            &selected,
            &VaultRecord::new(b"second-isolated-vault-record".to_vec()).unwrap(),
        )
        .unwrap();
    drop(vault);
    let metadata = PlatformCredentialVault::key_metadata(&desktop_root, "desktop-manual")
        .unwrap()
        .unwrap();
    let observed = Arc::new(AtomicBool::new(false));
    let check = Arc::clone(&observed);
    let weak_keys = Arc::downgrade(&keys);
    *keys.1.lock().unwrap() = Some(Arc::new(move |account| {
        if account == metadata.account {
            let source = PlatformCredentialVault::with_key_store(
                source_root.clone(),
                "cloud-connector",
                weak_keys.upgrade().unwrap(),
            )
            .unwrap();
            assert!(matches!(
                source.write(
                    &selected,
                    &VaultRecord::new(b"must-not-replace-source".to_vec()).unwrap()
                ),
                Err(CredentialError::Busy)
            ));
            check.store(true, Ordering::SeqCst);
        }
    }));
    let mut args = source_args(target_home.path());
    args.connector_source_home = Some(source_home.path().to_owned());
    args.connector_enrollment = Some("cloud-e2e-cli".into());
    args.desktop_vault = true;
    let plan = dev_credentials_sources::make_plan(&args, workspace.path()).unwrap();
    assert_eq!(
        dev_credentials_apply::apply_with_vault_keys(&plan, &ReadOnlySecrets::default(), keys)
            .unwrap(),
        (2, 0)
    );
    assert!(observed.load(Ordering::SeqCst));
}

#[test]
fn exact_named_enrollment_copy_never_exports_global_master_or_sibling_records() {
    use colossus_contracts::VaultRecord;
    use colossus_credentials::PlatformCredentialVault;
    use colossus_ports::{CredentialKey, CredentialVault as _};
    let source_home = private_root();
    let target_home = private_root();
    let workspace = private_root();
    let source_root = ConfinedRoot::bind(source_home.path()).unwrap();
    let source_directory = source_root
        .prepare_directory(Path::new("cloud-connector"))
        .unwrap();
    let keys = Arc::new(MemoryVaultKeys::default());
    let source_vault = PlatformCredentialVault::with_key_store(
        ConfinedRoot::bind(&source_directory).unwrap(),
        "cloud-connector",
        keys.clone(),
    )
    .unwrap();
    let selected = colossus_connector::EnrollmentStore::credential_key("cloud-e2e-cli").unwrap();
    let sibling = CredentialKey::new("cloud-connector", "unrelated-enrollment").unwrap();
    source_vault
        .write(
            &selected,
            &VaultRecord::new(
                b"synthetic-selected-enrollment-with-original-node-grant-and-TLS".to_vec(),
            )
            .unwrap(),
        )
        .unwrap();
    source_vault
        .write(
            &sibling,
            &VaultRecord::new(b"unrelated-record-is-not-copied".to_vec()).unwrap(),
        )
        .unwrap();
    drop(source_vault);
    let original_keys = keys.0.lock().unwrap().clone();
    let target_root = ConfinedRoot::bind(target_home.path()).unwrap();
    let authority =
        DevelopmentAuthority::initialize(&target_root, &[workspace.path().to_owned()]).unwrap();
    let mut args = source_args(target_home.path());
    args.connector_source_home = Some(source_home.path().to_owned());
    args.connector_enrollment = Some("cloud-e2e-cli".into());
    let plan = dev_credentials_sources::make_plan(&args, workspace.path()).unwrap();
    assert!(matches!(
        plan.sources[0],
        dev_credentials_plan::Source::ConnectorEnrollment { .. }
    ));
    let source_metadata = PlatformCredentialVault::key_metadata(
        &ConfinedRoot::bind(&source_directory).unwrap(),
        "cloud-connector",
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        dev_credentials_apply::apply_with_vault_keys(
            &plan,
            &ReadOnlySecrets::default(),
            keys.clone()
        )
        .unwrap(),
        (1, 0)
    );
    assert_eq!(keys.0.lock().unwrap().clone(), original_keys);
    let target_keys = Arc::new(
        authority
            .store(DevelopmentStoreScope::ControlPlaneVault)
            .unwrap(),
    );
    assert!(
        target_keys
            .read(&source_metadata.account)
            .unwrap()
            .is_none()
    );
    let target_directory = target_home.path().join("cloud-connector");
    let target_vault = PlatformCredentialVault::with_key_store(
        ConfinedRoot::bind(&target_directory).unwrap(),
        "cloud-connector",
        target_keys,
    )
    .unwrap();
    assert_eq!(
        target_vault.read(&selected).unwrap().unwrap().expose(),
        b"synthetic-selected-enrollment-with-original-node-grant-and-TLS"
    );
    assert!(target_vault.read(&sibling).unwrap().is_none());
    drop(target_vault);
    let target_metadata = PlatformCredentialVault::key_metadata(
        &ConfinedRoot::bind(&target_directory).unwrap(),
        "cloud-connector",
    )
    .unwrap()
    .unwrap();
    assert_ne!(source_metadata.vault_id, target_metadata.vault_id);
    assert_eq!(
        dev_credentials_apply::apply_with_vault_keys(
            &plan,
            &ReadOnlySecrets::default(),
            keys.clone()
        )
        .unwrap(),
        (0, 1)
    );
    let conflicting = PlatformCredentialVault::with_key_store(
        ConfinedRoot::bind(&target_directory).unwrap(),
        "cloud-connector",
        Arc::new(
            authority
                .store(DevelopmentStoreScope::ControlPlaneVault)
                .unwrap(),
        ),
    )
    .unwrap();
    conflicting
        .write(
            &selected,
            &VaultRecord::new(b"different-existing-target-record".to_vec()).unwrap(),
        )
        .unwrap();
    drop(conflicting);
    assert!(
        dev_credentials_apply::apply_with_vault_keys(
            &plan,
            &ReadOnlySecrets::default(),
            keys.clone()
        )
        .is_err()
    );
    let conflicting = PlatformCredentialVault::with_key_store(
        ConfinedRoot::bind(&target_directory).unwrap(),
        "cloud-connector",
        Arc::new(
            authority
                .store(DevelopmentStoreScope::ControlPlaneVault)
                .unwrap(),
        ),
    )
    .unwrap();
    assert_eq!(
        conflicting.read(&selected).unwrap().unwrap().expose(),
        b"different-existing-target-record"
    );
    drop(conflicting);
    let retained = PlatformCredentialVault::with_key_store(
        ConfinedRoot::bind(&source_directory).unwrap(),
        "cloud-connector",
        keys.clone(),
    )
    .unwrap();
    assert_eq!(
        retained.read(&selected).unwrap().unwrap().expose(),
        b"synthetic-selected-enrollment-with-original-node-grant-and-TLS"
    );
    assert_eq!(
        retained.read(&sibling).unwrap().unwrap().expose(),
        b"unrelated-record-is-not-copied"
    );
    drop(retained);
    assert_eq!(keys.0.lock().unwrap().clone(), original_keys);
    args.home = source_home.path().to_owned();
    DevelopmentAuthority::initialize(&source_root, &[workspace.path().to_owned()]).unwrap();
    assert!(
        dev_credentials_sources::make_plan(&args, workspace.path())
            .unwrap_err()
            .to_string()
            .contains("non-overlapping")
    );
    let nested = source_root
        .prepare_directory(Path::new("nested-home"))
        .unwrap();
    let nested_root = ConfinedRoot::bind(&nested).unwrap();
    DevelopmentAuthority::initialize(&nested_root, &[workspace.path().to_owned()]).unwrap();
    args.home = nested;
    assert!(
        dev_credentials_sources::make_plan(&args, workspace.path())
            .unwrap_err()
            .to_string()
            .contains("non-overlapping")
    );
}
