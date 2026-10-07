//! Operator-owned native Secret Service acceptance, isolated from user collections.
#![cfg(target_os = "linux")]
use colossus_contracts::VaultRecord;
use colossus_credentials::PlatformCredentialVault;
use colossus_home::ColossusHome;
use colossus_ports::{CredentialKey, CredentialVault};

#[test]
#[ignore = "requires an isolated, unlocked Secret Service session; Linux Desktop builder runs this"]
fn native_secret_service_survives_process_restart() {
    assert!(std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some());
    let temporary = tempfile::tempdir().unwrap();
    let home =
        ColossusHome::ensure_at(temporary.path().canonicalize().unwrap().join("private")).unwrap();
    for operation in ["write", "read", "delete"] {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "native_secret_service_child", "--ignored"])
            .env("COLOSSUS_NATIVE_ACCEPTANCE_ROOT", home.root())
            .env("COLOSSUS_NATIVE_ACCEPTANCE_OPERATION", operation)
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
    }
    for entry in std::fs::read_dir(home.root()).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = std::fs::read(path).unwrap();
            assert!(
                !bytes
                    .windows(b"synthetic-native-secret".len())
                    .any(|part| part == b"synthetic-native-secret")
            );
        }
    }
}

#[test]
#[ignore = "isolated native credential acceptance subprocess"]
fn native_secret_service_child() {
    let home =
        ColossusHome::ensure_at(std::env::var_os("COLOSSUS_NATIVE_ACCEPTANCE_ROOT").unwrap())
            .unwrap();
    let vault =
        PlatformCredentialVault::new(home.confined_root().clone(), "native-linux-acceptance")
            .unwrap();
    let key = CredentialKey::new("acceptance", "native-restart").unwrap();
    match std::env::var("COLOSSUS_NATIVE_ACCEPTANCE_OPERATION")
        .unwrap()
        .as_str()
    {
        "write" => vault
            .write(
                &key,
                &VaultRecord::new(b"synthetic-native-secret".to_vec()).unwrap(),
            )
            .unwrap(),
        "read" => assert_eq!(
            vault.read(&key).unwrap().unwrap().expose(),
            b"synthetic-native-secret"
        ),
        "delete" => {
            vault.delete(&key).unwrap();
            vault.delete_key_for_uninstall().unwrap();
        }
        _ => panic!("invalid acceptance operation"),
    }
}
