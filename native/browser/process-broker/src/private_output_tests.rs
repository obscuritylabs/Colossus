//! Plain file/pipe fixtures only; no executable, browser, rights or OS store.
use super::*;
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, DirBuilder},
    os::unix::fs::{DirBuilderExt as _, PermissionsExt as _, symlink},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    main: PathBuf,
    namespace: PathBuf,
    diagnostic: PathBuf,
    policy: [u8; 32],
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "colossus-private-output-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        DirBuilder::new().mode(0o700).create(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let app = root.join("Fixture.app");
        fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        let main = app.join("Contents/MacOS/colossus-native-browser-host");
        let namespace = root.join("broker");
        DirBuilder::new().mode(0o700).create(&namespace).unwrap();
        let diagnostic = namespace.join("process-diagnostics");
        DirBuilder::new().mode(0o700).create(&diagnostic).unwrap();
        let bytes = format!("COLOSSUS_MAC_NETWORK_ENVELOPE_V1\n1\n12345\n{}\n{}\nfixture\n{}/profile\n{}\n/home/fixture\n",root.display(),app.display(),root.display(),namespace.display()).into_bytes();
        let resource = app.join("Contents/Resources/colossus-network-envelope.policy");
        fs::write(&resource, &bytes).unwrap();
        fs::set_permissions(resource, fs::Permissions::from_mode(0o400)).unwrap();
        Self {
            root,
            main,
            namespace,
            diagnostic,
            policy: Sha256::digest(bytes).into(),
        }
    }
    fn open(&self) -> Diagnostic {
        Diagnostic::open(&self.main, self.policy, self.diagnostic.to_str().unwrap()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn receipt() -> ProcessReceipt {
    ProcessReceipt {
        nonce: [2; 16],
        main_status: Some(6),
        helpers_reaped: 0,
        unknown_creations: 1,
        keeper_empty: false,
        output_drained: false,
        channels_closed: false,
    }
}
#[test]
fn private_startup_failure_receipt_preserves_unknown_facts_and_never_overwrites_evidence() {
    let fixture = Fixture::new();
    let diagnostic = fixture.open();
    diagnostic.persist(None, &receipt()).unwrap();
    let path = fixture.diagnostic.join("creator-cleanup.json");
    let before = fs::read(&path).unwrap();
    assert_eq!(
        serde_json::from_slice::<ProcessReceipt>(&before).unwrap(),
        receipt()
    );
    for name in ["main.stdout.log", "main.stderr.log", "creator-cleanup.json"] {
        let metadata = fixture.diagnostic.join(name).symlink_metadata().unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(metadata.nlink(), 1);
    }
    assert!(diagnostic.persist(None, &receipt()).is_err());
    assert_eq!(fs::read(path).unwrap(), before);
}
#[test]
fn compiled_directory_must_be_exactly_under_the_digest_bound_policy_namespace() {
    let fixture = Fixture::new();
    let other = fixture.root.join("process-diagnostics");
    DirBuilder::new().mode(0o700).create(&other).unwrap();
    assert!(Diagnostic::open(&fixture.main, fixture.policy, other.to_str().unwrap()).is_err());
    assert!(
        Diagnostic::open(&fixture.main, [1; 32], fixture.diagnostic.to_str().unwrap()).is_err()
    );
    fs::remove_dir(&fixture.diagnostic).unwrap();
    symlink(&other, &fixture.diagnostic).unwrap();
    assert!(
        Diagnostic::open(
            &fixture.main,
            fixture.policy,
            fixture.diagnostic.to_str().unwrap()
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(other).unwrap().count(), 0);
}
#[test]
fn replaced_or_public_directory_cannot_receive_diagnostic_bytes() {
    let fixture = Fixture::new();
    let diagnostic = fixture.open();
    let prior = fixture.namespace.join("retained-diagnostics");
    fs::rename(&fixture.diagnostic, &prior).unwrap();
    DirBuilder::new()
        .mode(0o700)
        .create(&fixture.diagnostic)
        .unwrap();
    assert!(diagnostic.persist(None, &receipt()).is_err());
    assert_eq!(fs::read_dir(&fixture.diagnostic).unwrap().count(), 0);
    assert_eq!(fs::read_dir(prior).unwrap().count(), 0);
    fs::set_permissions(&fixture.diagnostic, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        Diagnostic::open(
            &fixture.main,
            fixture.policy,
            fixture.diagnostic.to_str().unwrap()
        )
        .is_err()
    );
}
