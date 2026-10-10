use super::*;

fn job() -> DarwinKeeperJob {
    DarwinKeeperJob::new(
        "com.obscuritylabs.colossus.browser-keeper.123".into(),
        "com.obscuritylabs.colossus.browser-keeper-service.123".into(),
        "/Applications/Colossus.app/Contents/MacOS/colossus-browser-keeper".into(),
        "/private/tmp/colossus-keeper-123/stdout".into(),
        "/private/tmp/colossus-keeper-123/stderr".into(),
        [7; 16],
        [9; 32],
    )
    .unwrap()
}

#[test]
fn launch_description_is_fresh_independent_bounded_and_fail_closed() {
    let job = job();
    let plist = job.property_list().unwrap();

    for required in [
        "<key>RunAtLoad</key><true/>",
        "<key>KeepAlive</key><false/>",
        "<key>SessionCreate</key><true/>",
        "<key>AbandonProcessGroup</key><false/>",
        "<key>Umask</key><string>0077</string>",
        "--allocation-nonce",
        "07070707070707070707070707070707",
        "--policy-sha256",
        "0909090909090909090909090909090909090909090909090909090909090909",
    ] {
        assert!(plist.contains(required), "missing {required}");
    }
    assert_eq!(plist.matches("<key>MachServices</key>").count(), 1);
    assert!(!plist.contains("EnvironmentVariables"));
    assert!(!plist.contains("WaitForDebugger"));
}

#[test]
fn launch_description_rejects_ambiguous_or_unbounded_inputs() {
    let base = job();
    for (label, service) in [
        ("same", "same"),
        ("bad/name", "good.name"),
        ("good.name", "bad<name"),
        (&"x".repeat(256), "good.name"),
    ] {
        assert!(
            DarwinKeeperJob::new(
                label.into(),
                service.into(),
                base.executable.clone(),
                base.standard_out.clone(),
                base.standard_error.clone(),
                [1; 16],
                [2; 32],
            )
            .is_err()
        );
    }
    assert!(
        DarwinKeeperJob::new(
            "good.label".into(),
            "good.service".into(),
            "relative/keeper".into(),
            "/private/tmp/out".into(),
            "/private/tmp/err".into(),
            [0; 16],
            [0; 32],
        )
        .is_err()
    );
    for invalid_path in [
        "/private/tmp/keeper//stdout".to_string(),
        "/private/tmp/keeper/./stdout".to_string(),
        "/private/tmp/keeper/../stdout".to_string(),
        "/private/tmp/keeper/stdout\n<key>KeepAlive</key>".to_string(),
        format!("/{}", "x".repeat(MAX_PATH_BYTES)),
    ] {
        assert!(
            DarwinKeeperJob::new(
                "good.label".into(),
                "good.service".into(),
                base.executable.clone(),
                invalid_path.into(),
                base.standard_error.clone(),
                [1; 16],
                [2; 32],
            )
            .is_err()
        );
    }
}

#[test]
fn policy_comparison_binds_both_nonce_and_digest() {
    assert!(matches_policy([7; 16], [9; 32], [7; 16], [9; 32]));
    assert!(!matches_policy([7; 16], [9; 32], [8; 16], [9; 32]));
    assert!(!matches_policy([7; 16], [9; 32], [7; 16], [8; 32]));
}
