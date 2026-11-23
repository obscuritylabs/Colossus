use super::*;
use serde_json::{Value, json};
use std::fs;

#[test]
fn worker_errors_and_results_contribute_only_bounded_categories() {
    let mut observation = WorkerObservation::Unobserved;
    observation.record(&Ok::<_, WorkerError>(json!({
        "workspace": "/private/support-path-sentinel",
        "authentication": "support-secret-sentinel"
    })));
    let report = SupportReport::new(HostFacts::Worker(observation))
        .encode()
        .unwrap();
    let actual: Value = serde_json::from_slice(&report).unwrap();
    assert_eq!(
        actual,
        json!({
            "schema_version": 1,
            "colossus_version": env!("CARGO_PKG_VERSION"),
            "platform": { "os": std::env::consts::OS, "architecture": std::env::consts::ARCH },
            "runtime_mode": "worker",
            "worker": { "status": "last_observed_available" }
        })
    );

    observation.record(&Err::<(), _>(WorkerError::Unavailable(
        "/private/support-path-sentinel: support-secret-sentinel".into(),
    )));
    observation.record(&Err::<(), _>(WorkerError::Cancelled));
    let report = SupportReport::new(HostFacts::Worker(observation))
        .encode()
        .unwrap();
    assert!(report.len() <= MAX_REPORT_BYTES);
    let text = std::str::from_utf8(&report).unwrap();
    assert!(!text.contains("support-path-sentinel"));
    assert!(!text.contains("support-secret-sentinel"));
    let actual: Value = serde_json::from_slice(&report).unwrap();
    assert_eq!(
        actual["worker"],
        json!({ "status": "unavailable", "reason": "connection_unavailable" })
    );

    observation.record(&Err::<(), _>(WorkerError::Remote(
        "support-secret-sentinel".into(),
    )));
    let report = SupportReport::new(HostFacts::Worker(observation))
        .encode()
        .unwrap();
    let actual: Value = serde_json::from_slice(&report).unwrap();
    assert_eq!(
        actual["worker"],
        json!({ "status": "last_observed_available" })
    );
    assert!(
        !std::str::from_utf8(&report)
            .unwrap()
            .contains("support-secret-sentinel")
    );
}

#[test]
fn save_preserves_existing_file_until_explicit_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("report with spaces.json");
    fs::write(&output, b"original").unwrap();
    let control = RunControl::default();
    assert!(matches!(
        save(HostFacts::Embedded, &output, false, &control),
        Err(SaveError::Exists)
    ));
    assert_eq!(fs::read(&output).unwrap(), b"original");
    save(HostFacts::Embedded, &output, true, &control).unwrap();
    let report: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(report["runtime_mode"], "embedded");
    assert_eq!(report["worker"], json!({ "status": "not_applicable" }));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn cancellation_and_destination_collisions_drop_staging_without_clobbering() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("report.json");
    let control = RunControl::default();
    let destination = Destination::open(&output).unwrap();
    let mut staged = destination.stage().unwrap();
    staged.file_mut().write_all(b"staged bytes").unwrap();
    fs::write(&output, b"racing file").unwrap();
    assert!(matches!(
        staged.publish(false, &control),
        Err(SaveError::Exists)
    ));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);

    let staged = destination.stage().unwrap();
    control.cancel();
    assert!(matches!(
        staged.publish(true, &control),
        Err(SaveError::Cancelled)
    ));
    assert_eq!(fs::read(&output).unwrap(), b"racing file");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn destination_does_not_normalize_directory_syntax_to_a_file() {
    for path in ["report/", "report/.", "report/..", "."] {
        assert!(matches!(
            destination::output_name(Path::new(path)),
            Err(SaveError::Destination)
        ));
    }
}

#[cfg(unix)]
#[test]
fn moved_destination_cleans_up_in_its_original_directory() {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("parent");
    let moved = directory.path().join("moved");
    fs::create_dir(&parent).unwrap();
    let destination = Destination::open(&parent.join("report.json")).unwrap();
    let staged = destination.stage().unwrap();
    fs::rename(&parent, &moved).unwrap();
    fs::create_dir(&parent).unwrap();
    assert!(matches!(
        staged.publish(false, &RunControl::default()),
        Err(SaveError::Destination)
    ));
    assert!(fs::read_dir(&parent).unwrap().next().is_none());
    assert!(fs::read_dir(&moved).unwrap().next().is_none());
}

#[cfg(windows)]
#[test]
fn destination_directory_stays_in_place_until_publication_ends() {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("parent");
    let moved = directory.path().join("moved");
    fs::create_dir(&parent).unwrap();
    let destination = Destination::open(&parent.join("report.json")).unwrap();
    assert!(fs::rename(&parent, &moved).is_err());
    drop(destination);
    fs::rename(&parent, &moved).unwrap();
}

#[test]
fn a_missing_parent_or_directory_destination_does_not_create_output() {
    let directory = tempfile::tempdir().unwrap();
    let missing_parent = directory.path().join("missing");
    let control = RunControl::default();
    assert!(matches!(
        save(
            HostFacts::Unresolved,
            &missing_parent.join("report.json"),
            false,
            &control
        ),
        Err(SaveError::Destination)
    ));
    assert!(matches!(
        save(HostFacts::Unresolved, directory.path(), true, &control),
        Err(SaveError::Destination)
    ));
    assert!(fs::read_dir(directory.path()).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn replacing_a_symlink_is_rejected_without_touching_its_target() {
    let directory = tempfile::tempdir().unwrap();
    let existing = directory.path().join("existing");
    let link = directory.path().join("report.json");
    fs::write(&existing, b"original").unwrap();
    std::os::unix::fs::symlink(&existing, &link).unwrap();
    assert!(matches!(
        save(HostFacts::Unresolved, &link, true, &RunControl::default()),
        Err(SaveError::Destination)
    ));
    assert_eq!(fs::read_link(&link).unwrap(), existing);
    assert_eq!(fs::read(&existing).unwrap(), b"original");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[cfg(windows)]
#[test]
fn windows_network_device_stream_and_drive_relative_paths_are_rejected() {
    for path in [
        r"\\example\share\report.json",
        r"\\?\UNC\example\share\report.json",
        r"\\.\pipe\report",
        r"C:report.json",
        r"C:\report.json:extra",
    ] {
        assert!(!destination::local_path_syntax(Path::new(path)), "{path}");
    }
    assert!(destination::local_path_syntax(Path::new(
        r"C:\folder\report.json"
    )));
    assert!(destination::local_path_syntax(Path::new(
        r"folder\report.json"
    )));
    assert!(destination::output_name(Path::new(r"C:\report.json.")).is_err());
    assert!(destination::output_name(Path::new(r"C:\report.json ")).is_err());
}
