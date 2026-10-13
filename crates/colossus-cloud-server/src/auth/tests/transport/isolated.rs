//! Keep real native installation inventory inside the transport fixture's home.
use colossus_home::ColossusHome;
use std::{path::Path, time::Duration};

pub(super) fn directory() -> tempfile::TempDir {
    // Protected native state requires an owner-controlled ancestor namespace.
    // Windows' shared temporary directory does not establish that authority.
    #[cfg(windows)]
    let directory = tempfile::Builder::new()
        .prefix("colossus-cloud-transport-")
        .tempdir_in(std::env::var_os("USERPROFILE").expect("test user profile"))
        .unwrap();
    #[cfg(not(windows))]
    let directory = tempfile::tempdir().unwrap();
    directory
}

/// Return true in the parent after the isolated child completed the acceptance.
pub(super) async fn run(test: &str, ignored: bool) -> bool {
    const ISOLATED: &str = "COLOSSUS_CLOUD_TRANSPORT_TEST_ISOLATED";
    if std::env::var(ISOLATED).as_deref() == Ok(test) {
        return false;
    }
    let directory = directory();
    let root = directory.path().canonicalize().unwrap();
    let home = ColossusHome::ensure_at(root.join("installation")).unwrap();
    // Enrollment creates a real stable opaque host ID under COLOSSUS_HOME.
    // A child owns that environment before threads start; the parent and other
    // concurrent tests never adopt the fixture's installation home.
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", test, "--test-threads=1"])
        .env(ISOLATED, test)
        .env("COLOSSUS_HOME", home.root())
        .kill_on_drop(true);
    if ignored {
        command.arg("--ignored");
    }
    let output = tokio::time::timeout(Duration::from_secs(120), command.output())
        .await
        .expect("isolated transport acceptance deadline")
        .unwrap();
    assert!(
        output.status.success(),
        "isolated transport acceptance failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // A zero-test child must not count as acceptance. This also proves that the
    // real production inventory used the fixture home rather than ambient state.
    home.confined_root()
        .open_existing_file(Path::new("cloud-connector/host-id"))
        .expect("isolated native host identity was persisted");
    true
}
