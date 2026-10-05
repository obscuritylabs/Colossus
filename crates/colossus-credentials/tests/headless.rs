//! Headless key rotation/restart acceptance runs in isolated processes. Keeping
//! subprocess creation outside vault unit tests avoids inherited flock lifetimes.
use colossus_contracts::CredentialError;
use colossus_credentials::{EnvironmentKeyStore, PlatformKeyStore};
use colossus_home::ColossusHome;

#[test]
fn sealed_authority_survives_restart_and_rejects_changed_key() {
    let directory = tempfile::tempdir().unwrap();
    let home =
        ColossusHome::ensure_at(directory.path().canonicalize().unwrap().join("private")).unwrap();
    for (mode, wrapping_key) in [
        ("write", "a".repeat(64)),
        ("read", "a".repeat(64)),
        ("wrong-key", "b".repeat(64)),
    ] {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "headless_child", "--ignored"])
            .env("COLOSSUS_HEADLESS_TEST_ROOT", home.root())
            .env("COLOSSUS_HEADLESS_TEST_MODE", mode)
            .env("COLOSSUS_HEADLESS_ACCEPTANCE_KEY", wrapping_key)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    for entry in std::fs::read_dir(home.root().join("headless-keys")).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(
            !bytes
                .windows(b"public-api-bearer".len())
                .any(|window| window == b"public-api-bearer")
        );
    }
}

#[test]
#[ignore = "isolated process helper invoked by sealed authority acceptance"]
fn headless_child() {
    let path = std::env::var_os("COLOSSUS_HEADLESS_TEST_ROOT").unwrap();
    let home = ColossusHome::ensure_at(path).unwrap();
    let store = EnvironmentKeyStore::new(
        home.confined_root().clone(),
        "COLOSSUS_HEADLESS_ACCEPTANCE_KEY".into(),
    )
    .unwrap();
    match std::env::var("COLOSSUS_HEADLESS_TEST_MODE")
        .unwrap()
        .as_str()
    {
        "write" => {
            assert!(store.read("authority").unwrap().is_none());
            assert_eq!(std::fs::read_dir(home.root()).unwrap().count(), 0);
            store.write("authority", b"public-api-bearer").unwrap();
        }
        "read" => assert_eq!(
            store.read("authority").unwrap().unwrap().as_slice(),
            b"public-api-bearer"
        ),
        "wrong-key" => assert_eq!(
            store.read("authority").unwrap_err(),
            CredentialError::Corrupt
        ),
        _ => panic!("unknown mode"),
    }
}
