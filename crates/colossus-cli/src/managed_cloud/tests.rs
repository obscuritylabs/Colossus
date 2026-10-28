use super::*;
use colossus_home::ConfinedRoot;
use std::io::Write as _;

fn local() -> LocalArguments {
    LocalArguments {
        local_config: None,
        allow_approvals: false,
        enable_human_terminals: false,
        storage: StorageArguments {
            name: "fixture".into(),
            vault_key_variable: None,
        },
    }
}

fn enrollment(url: &str) -> ConnectorCommand {
    ConnectorCommand::Enroll(colossus_connector::EnrollArguments {
        url: Some(url.into()),
        enrollment_url: None,
        enrollment_ca_file: None,
        allow_loopback_http: false,
        local: local(),
    })
}

#[test]
fn default_grant_cannot_answer_approvals_or_open_terminals() {
    let mut arguments = local();
    let default = requested_scopes(&arguments);
    assert!(
        !default
            .iter()
            .any(|scope| scope == scopes::APPROVALS_RESPOND)
    );
    assert!(
        !default
            .iter()
            .any(|scope| scope == colossus_api::HUMAN_TERMINAL_SCOPE)
    );
    assert!(default.iter().any(|scope| scope == scopes::RUNS_EXECUTE));
    arguments.allow_approvals = true;
    let approval = requested_scopes(&arguments);
    assert!(
        approval
            .iter()
            .any(|scope| scope == scopes::APPROVALS_RESPOND)
    );
    assert!(
        !approval
            .iter()
            .any(|scope| scope == colossus_api::HUMAN_TERMINAL_SCOPE)
    );
    arguments.enable_human_terminals = true;
    assert!(
        requested_scopes(&arguments)
            .iter()
            .any(|scope| scope == colossus_api::HUMAN_TERMINAL_SCOPE)
    );
}

#[test]
fn managed_url_validation_precedes_native_custody() {
    assert!(validate_enrollment_url(&enrollment("https://cloud.example/api/enroll")).is_ok());
    for url in [
        "http://cloud.example/api/enroll",
        "https://user:secret@cloud.example/api/enroll",
        "https://cloud.example/api/enroll?token=secret",
        "https://cloud.example/api/enroll#secret",
        "invalid",
    ] {
        assert!(validate_enrollment_url(&enrollment(url)).is_err());
    }
}

fn temporary() -> tempfile::TempDir {
    #[cfg(windows)]
    {
        let parent = std::env::var_os("COLOSSUS_NATIVE_TEST_ROOT")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .expect("native test parent");
        let parent = PathBuf::from(parent);
        assert!(parent.is_absolute() && parent.is_dir());
        tempfile::Builder::new()
            .prefix("colossus-managed-cloud-")
            .tempdir_in(parent)
            .unwrap()
    }
    #[cfg(not(windows))]
    {
        tempfile::tempdir().unwrap()
    }
}

#[test]
fn named_metadata_binds_workspace_config_and_explicit_access() {
    let temporary = temporary();
    let root = ConfinedRoot::bind(temporary.path().join("private")).unwrap();
    let workspace = root.prepare_directory(Path::new("workspace")).unwrap();
    let identity = detect_workspace_identity(&workspace).unwrap();
    let home = root.prepare_directory(Path::new("home")).unwrap();
    let partition = root.prepare_directory(Path::new("partition")).unwrap();
    let metadata = Metadata::open(&home, &partition, "fixture").unwrap();
    // Native confined paths must use component joins rather than retaining '/' in
    // the Windows path passed to descriptor-relative creation and replacement.
    assert_eq!(
        metadata.directory.path(),
        std::fs::canonicalize(partition.join("cloud").join(metadata::digest(b"fixture"))).unwrap()
    );
    #[cfg(windows)]
    assert!(
        !metadata
            .directory
            .path()
            .as_os_str()
            .as_encoded_bytes()
            .contains(&b'/')
    );
    let binding = Binding::planned(
        &local(),
        &identity,
        &workspace.join("runtime.yaml"),
        "a".repeat(64),
        metadata.api_directory().unwrap(),
    );
    metadata.save(&binding).unwrap();
    let restored = metadata.load().unwrap().unwrap();
    restored.validate_context(&binding).unwrap();
    let mut consented = binding.clone();
    consented.grant.allow_approvals = true;
    consented.grant.enable_human_terminals = true;
    let mut bare_run = local();
    retain_consented_access(&mut bare_run, &consented).unwrap();
    assert!(bare_run.allow_approvals && bare_run.enable_human_terminals);
    assert!(retain_consented_access(&mut bare_run, &binding).is_err());
    for changed in [
        {
            let mut value = binding.clone();
            value.config_sha256 = "b".repeat(64);
            value
        },
        {
            let mut value = binding.clone();
            value.workspace_identity_sha256 = "b".repeat(64);
            value
        },
        {
            let mut value = binding.clone();
            value.grant.allow_approvals = true;
            value
        },
        {
            let mut value = binding.clone();
            value.grant.enable_human_terminals = true;
            value
        },
        {
            let mut value = binding.clone();
            value.vault_key_variable = Some("OTHER_KEY".into());
            value
        },
    ] {
        assert!(restored.validate_context(&changed).is_err());
    }
    assert!(Metadata::open(&home, &partition, "fixture").is_err());
}

#[test]
fn generated_connection_retains_independent_pin_and_rejects_tampering() {
    let temporary = temporary();
    let root = ConfinedRoot::bind(temporary.path().join("private")).unwrap();
    let workspace = root.prepare_directory(Path::new("workspace")).unwrap();
    let identity = detect_workspace_identity(&workspace).unwrap();
    let home = root.prepare_directory(Path::new("home")).unwrap();
    let partition = root.prepare_directory(Path::new("partition")).unwrap();
    let metadata = Metadata::open(&home, &partition, "fixture").unwrap();
    #[cfg(windows)]
    assert!(
        !metadata
            .api_directory()
            .unwrap()
            .as_os_str()
            .as_encoded_bytes()
            .contains(&b'/')
    );
    let mut binding = Binding::planned(
        &local(),
        &identity,
        &workspace.join("runtime.yaml"),
        "a".repeat(64),
        metadata.api_directory().unwrap(),
    );
    binding.instance_id = Some(uuid::Uuid::now_v7().to_string());
    binding.certificate_sha256 = Some("c".repeat(64));
    binding.credential_id = Some(uuid::Uuid::now_v7().to_string());
    binding.stage = Stage::Ready;
    metadata.save(&binding).unwrap();
    metadata.connection(&mut binding).unwrap();
    let published_hash = binding.connection_sha256.take();
    metadata.save(&binding).unwrap();
    // The connection committed, but publishing its checksum response was lost.
    // The same retained instance/pin may recover only these exact reference bytes.
    metadata.connection(&mut binding).unwrap();
    assert_eq!(binding.connection_sha256, published_hash);
    metadata.connection(&mut binding).unwrap();
    let bytes = std::fs::read(&metadata.connection).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains(&"c".repeat(64)));
    assert!(!text.contains("bearer") && !text.contains("invitation") && !text.contains("token"));
    let file = metadata
        .directory
        .open_file(Path::new("connection.json"))
        .unwrap();
    file.file().set_len(0).unwrap();
    let mut stream = file.file();
    stream.write_all(b"{}").unwrap();
    stream.sync_all().unwrap();
    assert!(metadata.connection(&mut binding).is_err());
}

#[cfg(windows)]
#[test]
fn ordinary_windows_profile_path_supports_long_atomic_connection_leaves() {
    use std::{os::windows::ffi::OsStrExt as _, path::Component};
    let temporary = temporary();
    let canonical = std::fs::canonicalize(temporary.path()).unwrap();
    let mut components = canonical.components();
    let drive = match components.next().unwrap() {
        Component::Prefix(prefix) => match prefix.kind() {
            std::path::Prefix::VerbatimDisk(drive) => drive,
            _ => panic!("native fixture must reside on an owned local disk"),
        },
        _ => panic!("native fixture must have an absolute prefix"),
    };
    assert_eq!(components.next(), Some(Component::RootDir));
    let mut ordinary = PathBuf::from(format!("{}:\\", char::from(drive)));
    for component in components {
        assert!(matches!(component, Component::Normal(_)));
        ordinary.push(component);
    }
    assert!(!ordinary.as_os_str().to_string_lossy().starts_with(r"\\?\"));
    let root = ConfinedRoot::bind(ordinary.join("private")).unwrap();
    let home = root.prepare_directory(Path::new("home")).unwrap();
    let workspace = root.prepare_directory(Path::new("workspace")).unwrap();
    let partition = root
        .prepare_directory(
            &Path::new("workspaces")
                .join("a".repeat(64))
                .join("p".repeat(32))
                .join("cli"),
        )
        .unwrap();
    let identity = detect_workspace_identity(&workspace).unwrap();
    let metadata = Metadata::open(&home, &partition, "long-profile").unwrap();
    assert!(
        metadata
            .directory
            .path()
            .join("stage-01a18333d15470a1bb492dec464909dc.json")
            .as_os_str()
            .encode_wide()
            .count()
            > 260
    );
    let mut binding = Binding::planned(
        &local(),
        &identity,
        &workspace.join("runtime.yaml"),
        "a".repeat(64),
        metadata.api_directory().unwrap(),
    );
    binding.instance_id = Some(uuid::Uuid::now_v7().to_string());
    binding.certificate_sha256 = Some("c".repeat(64));
    binding.credential_id = Some(uuid::Uuid::now_v7().to_string());
    binding.stage = Stage::Ready;
    metadata.save(&binding).unwrap();
    metadata.connection(&mut binding).unwrap();
    let connection = std::fs::read(&metadata.connection).unwrap();
    metadata.save(&binding).unwrap();
    drop(metadata);
    let retained = Metadata::open(&home, &partition, "long-profile").unwrap();
    let mut restored = retained.load().unwrap().unwrap();
    retained.connection(&mut restored).unwrap();
    assert_eq!(std::fs::read(&retained.connection).unwrap(), connection);
}
