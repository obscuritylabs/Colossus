use super::*;
use std::fs;

fn home() -> (tempfile::TempDir, ColossusHome) {
    #[cfg(windows)]
    let directory = tempfile::tempdir_in(
        std::env::var_os("LOCALAPPDATA").expect("private Windows test parent"),
    )
    .expect("temporary profile parent");
    #[cfg(not(windows))]
    let directory = tempfile::tempdir().expect("temporary profile parent");
    let parent = fs::canonicalize(directory.path()).expect("canonical test parent");
    let home = ColossusHome::ensure_at(parent.join("home")).expect("private test home");
    (directory, home)
}

fn catalog(revision: u64, endpoint: &str) -> Catalog {
    Catalog {
        revision,
        profiles: vec![ControlPlaneProfile {
            id: "acceptance-profile".into(),
            label: "Control Plane".into(),
            endpoint: endpoint.into(),
        }],
        default_profile: Some("acceptance-profile".into()),
    }
}

fn rejected_before_home(catalog: Catalog) {
    assert!(
        access_with_home(Some(catalog), || panic!("invalid metadata resolved a home")).is_err()
    );
}

#[test]
fn profiles_allow_https_and_strict_loopback_endpoints() {
    let (_guard, home) = home();
    let mut revision = 0;
    for endpoint in [
        "https://control-plane.example/api",
        "http://127.0.0.1:8090",
        "http://localhost:8090",
        "http://[::1]:8090",
    ] {
        let saved = access_with_home(Some(catalog(revision, endpoint)), || Ok(home.clone()))
            .expect("valid connection metadata");
        assert_eq!(saved.profiles[0].endpoint, endpoint);
        revision = saved.revision;
    }
}

#[test]
fn profiles_reject_credential_urls_remote_http_and_invalid_names() {
    for endpoint in [
        "http://control-plane.example",
        "http://127.0.0.1.example",
        "https://user@control-plane.example",
        "https://user:password@control-plane.example",
        "https://control-plane.example?token=synthetic",
        "https://control-plane.example#fragment",
        "file:///private/tmp/profiles",
    ] {
        rejected_before_home(catalog(0, endpoint));
    }
    for id in ["", "../profile", "profile.name"] {
        let mut invalid = catalog(0, "https://control-plane.example");
        invalid.profiles[0].id = id.into();
        rejected_before_home(invalid);
    }
    for label in [" ", "Control\nPlane"] {
        let mut invalid = catalog(0, "https://control-plane.example");
        invalid.profiles[0].label = label.into();
        rejected_before_home(invalid);
    }
    let mut duplicate = catalog(0, "https://control-plane.example");
    duplicate.profiles.push(duplicate.profiles[0].clone());
    rejected_before_home(duplicate.clone());
    duplicate.profiles[1].id = "another-profile".into();
    duplicate.profiles[1].label = " control plane ".into();
    rejected_before_home(duplicate);
    let mut unknown_default = catalog(0, "https://control-plane.example");
    unknown_default.default_profile = Some("missing-profile".into());
    rejected_before_home(unknown_default);
}

#[test]
fn profiles_revision_cas_is_durable_across_reopen() {
    let (_guard, home) = home();
    let initial = access_with_home(None, || Ok(home.clone())).expect("empty catalog");
    assert_eq!(initial.revision, 0);
    assert!(initial.profiles.is_empty());
    let saved = access_with_home(Some(catalog(0, "https://first.example")), || {
        Ok(home.clone())
    })
    .expect("first save");
    assert_eq!(saved.revision, 1);
    assert!(
        access_with_home(Some(catalog(0, "https://stale.example")), || {
            Ok(home.clone())
        })
        .is_err()
    );
    let reopened = ColossusHome::ensure_at(home.root()).expect("reopened private home");
    let unchanged = access_with_home(None, || Ok(reopened.clone())).expect("retained catalog");
    assert_eq!(unchanged.revision, 1);
    assert_eq!(unchanged.profiles[0].endpoint, "https://first.example");
    let updated = access_with_home(Some(catalog(1, "https://second.example")), || {
        Ok(reopened.clone())
    })
    .expect("next revision save");
    assert_eq!(updated.revision, 2);
    let retained = access_with_home(None, || Ok(reopened)).expect("updated retained catalog");
    assert_eq!(retained.revision, 2);
    assert_eq!(retained.profiles[0].endpoint, "https://second.example");
    assert_eq!(
        retained.default_profile.as_deref(),
        Some("acceptance-profile")
    );
}

#[test]
fn unavailable_enrollment_metadata_does_not_reinterpret_a_committed_bookmark_save() {
    let (_guard, home) = home();
    let saved = access_with_home(Some(catalog(0, "https://saved.example")), || Ok(home))
        .expect("bookmark committed");
    let response =
        serde_json::to_value(snapshot(saved, Err(failure()))).expect("public command response");
    assert_eq!(response["revision"], 1);
    assert_eq!(response["profiles"][0]["endpoint"], "https://saved.example");
    assert_eq!(response["connectionStatusUnavailable"], true);
    assert_eq!(response["connections"], serde_json::json!([]));
    assert!(response.get("connection_status_unavailable").is_none());
}

#[test]
fn profiles_concurrent_writers_share_one_revision() {
    let (_guard, home) = home();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let writers = ["https://first.example", "https://second.example"].map(|endpoint| {
            let home = home.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                access_with_home(Some(catalog(0, endpoint)), || Ok(home)).is_ok()
            })
        });
        writers.map(|writer| writer.join().expect("profile writer"))
    });
    assert_eq!(results.into_iter().filter(|saved| *saved).count(), 1);
    let retained = access_with_home(None, || Ok(home)).expect("one durable winning catalog");
    assert_eq!(retained.revision, 1);
    assert!(matches!(
        retained.profiles[0].endpoint.as_str(),
        "https://first.example" | "https://second.example"
    ));
}

#[cfg(unix)]
#[test]
fn profiles_reject_symlinked_directory_and_catalog() {
    let (_guard, home) = home();
    let outside = home.root().parent().expect("test parent").join("outside");
    fs::create_dir(&outside).expect("outside test directory");
    let profiles = home.root().join("desktop-control-plane");
    std::os::unix::fs::symlink(&outside, &profiles).expect("synthetic directory link");
    assert!(access_with_home(None, || Ok(home.clone())).is_err());
    assert_eq!(fs::read_dir(&outside).expect("untouched target").count(), 0);
    fs::remove_file(&profiles).expect("remove synthetic directory link");
    access_with_home(None, || Ok(home.clone())).expect("create confined catalog");
    let outside_file = outside.join("retained.json");
    fs::write(&outside_file, b"retain exactly").expect("outside marker");
    fs::remove_file(profiles.join("profiles.json")).expect("remove empty test catalog");
    std::os::unix::fs::symlink(&outside_file, profiles.join("profiles.json"))
        .expect("synthetic catalog link");
    assert!(
        access_with_home(Some(catalog(0, "https://control-plane.example")), || {
            Ok(home.clone())
        })
        .is_err()
    );
    assert!(access_with_home(None, || Ok(home)).is_err());
    assert_eq!(
        fs::read(outside_file).expect("untouched marker"),
        b"retain exactly"
    );
}
