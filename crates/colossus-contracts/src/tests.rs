use super::{Actor, ActorType, PolicyDecision, RunEvent, ThemeName, ThemeSpinner};

#[test]
fn application_actor_has_stable_journal_provenance() {
    let actor = Actor {
        actor_type: ActorType::Application,
        id: "app:019f7d38-649a-7580-a30f-01157b719c2a".into(),
    };
    assert_eq!(
        serde_json::to_string(&actor).expect("application actor"),
        r#"{"actor_type":"application","id":"app:019f7d38-649a-7580-a30f-01157b719c2a"}"#
    );
}

#[test]
fn policy_decision_rejects_unknown_fields() {
    let document = r#"{
            "decision_id":"d1","policy_revision":"r1","outcome":"deny",
            "reason":"no","obligations":{"sandbox_backend":"none",
            "sandbox_profile":"none","filesystem":[],"network_destinations":[],
            "allowed_environment":[],"allow_sandbox_downgrade":false,
            "timeout_ms":1,"max_output_bytes":1,"max_processes":0,
            "max_memory_bytes":1,"max_concurrency":1,"required_redactions":[],
            "require_post_effect":false,"audit_labels":{},"retention":"standard"},
            "surprise":true
        }"#;
    assert!(serde_json::from_str::<PolicyDecision>(document).is_err());
}

#[test]
fn run_event_rejects_unknown_fields() {
    let document = r#"{
            "type":"phase","phase":"preparing","turn":1,"action":null,
            "elapsed_seconds":0.1,"surprise":true
        }"#;
    assert!(serde_json::from_str::<RunEvent>(document).is_err());
}

#[test]
fn theme_names_are_stable_and_plain_migrates_to_mono() {
    for (theme, name) in [
        (ThemeName::Default, "default"),
        (ThemeName::Mono, "mono"),
        (ThemeName::HighContrast, "high_contrast"),
        (ThemeName::Carrot, "carrot"),
        (ThemeName::Hacker, "hacker"),
    ] {
        assert_eq!(theme.as_str(), name);
        assert_eq!(
            serde_json::to_string(&theme).expect("theme"),
            format!("\"{name}\"")
        );
    }
    assert_eq!(
        serde_json::from_str::<ThemeName>("\"plain\"").expect("legacy plain theme"),
        ThemeName::Mono
    );
    assert_eq!(
        serde_json::to_string(&ThemeSpinner::BouncingBar).expect("spinner"),
        "\"bouncingBar\""
    );
    assert_eq!(
        serde_json::from_str::<ThemeSpinner>("\"bouncing_bar\"").expect("legacy spinner spelling"),
        ThemeSpinner::BouncingBar
    );
}
