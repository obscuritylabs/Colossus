use super::*;
use crate::test_support::private_tempdir;

#[test]
fn development_custody_rejects_unsafe_boundaries_before_authority_or_runtime_acquisition() {
    let directory = private_tempdir();
    let home = ConfinedRoot::bind(fs::canonicalize(directory.path()).expect("canonical home"))
        .expect("private home");
    for backend in [
        "danger_full_access",
        "external",
        "broker",
        "unknown",
        if cfg!(windows) {
            "native"
        } else {
            "windows_job"
        },
    ] {
        let mut config = RuntimeConfig::offline_template("state.redb");
        config.sandbox.backend = backend.into();
        config.sandbox.profile = "workspace-development".into();
        let error = runtime_development_protection_selected(&config, Some(&home), true, |_| {
            panic!("unsafe boundary must fail before authority/key/runtime acquisition")
        })
        .err()
        .expect("unsafe explicit custody rejected");
        assert!(error.to_string().contains("supported isolating sandbox"));
        assert_eq!(
            config.sandbox.backend, backend,
            "never silently change policy"
        );
        assert_eq!(
            fs::read_dir(home.path())
                .expect("home remains readable")
                .count(),
            0
        );
    }
}

#[test]
fn development_custody_rejects_fallback_and_reserved_tool_environment() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.sandbox.backend = if cfg!(windows) {
        "windows_job"
    } else {
        "native"
    }
    .into();
    config.sandbox.allow_broker_fallback = true;
    assert!(validate_selected_boundary(&config, true).is_err());
    config.sandbox.allow_broker_fallback = false;
    for name in [
        DEVELOPMENT_AUTHORITY_VARIABLE,
        "colossus_development_wrapping_key",
        "COLOSSUS_JOURNAL_KEY",
        "COLOSSUS_SIGNING_KEY",
        "COLOSSUS_DEV_JOURNAL_KEY",
        "colossus_dev_signing_key",
    ] {
        config.sandbox.environment = vec![name.into()];
        assert!(
            validate_selected_boundary(&config, true).is_err(),
            "reserved {name}"
        );
    }
    config.sandbox.environment = vec!["PATH".into(), "HOME".into()];
    validate_selected_boundary(&config, true).expect("supported isolation with safe environment");
}

#[test]
fn development_custody_requires_explicit_home_and_leaves_unselected_runtime_unchanged() {
    let mut config = RuntimeConfig::offline_template("state.redb");
    config.sandbox.backend = if cfg!(windows) {
        "windows_job"
    } else {
        "native"
    }
    .into();
    let error = runtime_development_protection_selected(&config, None, true, |_| panic!("no home"))
        .err()
        .expect("missing retained home rejected");
    assert!(error.to_string().contains("retained Colossus home"));
    config.sandbox.backend = "danger_full_access".into();
    runtime_development_protection_selected(&config, None, false, |_| panic!("no selector"))
        .expect("unselected runtime behavior unchanged");
    assert_eq!(config.sandbox.backend, "danger_full_access");
}
