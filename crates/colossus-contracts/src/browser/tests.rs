use super::*;
use serde_json::json;

#[test]
fn destinations_canonicalize_exact_origins_and_reject_credentials() {
    let url = BrowserUrl::parse("https://EXAMPLE.test:443/a?token=private#section").unwrap();
    assert_eq!(
        url.origin(),
        BrowserOrigin::parse("https://example.test/").unwrap()
    );
    assert!(!format!("{url:?}").contains("private"));
    for value in [
        "file:///tmp/key",
        "javascript:alert(1)",
        "https://user:password@example.test",
        "https://example.test/\n",
    ] {
        assert!(BrowserUrl::parse(value).is_err());
    }
    for value in [
        "https://example.test/path",
        "https://example.test?token=x",
        "https://example.test#x",
        "https://*.example.test",
    ] {
        assert!(BrowserOrigin::parse(value).is_err(), "accepted {value}");
    }
}

#[test]
fn closed_browser_contracts_reject_raw_protocol_and_invalid_handles() {
    assert!(serde_json::from_value::<BrowserAction>(json!({"kind":"navigate", "url":"https://example.test", "cdp_method":"Runtime.evaluate"})).is_err());
    assert!(
        serde_json::from_value::<BrowserAction>(
            json!({"kind":"evaluate", "script":"return document.cookie"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<BrowserAction>(json!({"kind":"back", "cdp_target":"native"}))
            .is_err()
    );
    assert!(serde_json::from_value::<BrowserOpenOptions>(json!({"mode":"headless", "allowed_origins":["https://example.test"], "profile_path":"/tmp/personal"})).is_err());
    assert!(BrowserSessionId::parse("ws://localhost:9222/devtools/browser/target").is_err());
    assert!(BrowserSessionId::parse("bt_01234567890123456789012345678901").is_err());
    assert!(
        serde_json::from_str::<BrowserSessionId>("\"bs_ABCDEF01234567890123456789012345\"")
            .is_err()
    );
}

#[test]
fn persistent_profile_choice_is_opaque_and_temporary_by_default() {
    let open: BrowserOpenOptions = serde_json::from_value(json!({
        "mode":"headless", "allowed_origins":["https://example.test"]
    }))
    .unwrap();
    assert_eq!(open.profile, BrowserProfileSelection::Temporary);
    assert_eq!(
        serde_json::from_value::<BrowserProfileSelection>(json!({"kind":"temporary"})).unwrap(),
        BrowserProfileSelection::Temporary
    );
    assert!(matches!(
        serde_json::from_value::<BrowserProfileSelection>(
            json!({"kind":"workspace","id":"bp_01234567890123456789012345678901"})
        ),
        Ok(BrowserProfileSelection::Workspace { .. })
    ));
    for profile in [
        json!({"kind":"workspace","id":"/tmp/personal-chrome"}),
        json!({"kind":"workspace","id":"bp_01234567890123456789012345678901","path":"/tmp/cache"}),
        json!({"kind":"temporary","cache_path":"/tmp/cache"}),
    ] {
        assert!(serde_json::from_value::<BrowserProfileSelection>(profile).is_err());
    }
}
