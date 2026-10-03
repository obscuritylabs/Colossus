use super::*;

#[test]
fn legacy_boolean_modes_preserve_explicit_choices_and_missing_images() {
    let value: ModelFeatureSettings = serde_json::from_value(serde_json::json!({
        "toolCalls": true, "streaming": false
    }))
    .expect("legacy settings");
    assert_eq!(value.tool_calls, ModelFeatureMode::On);
    assert_eq!(value.streaming, ModelFeatureMode::Off);
    assert_eq!(value.image_inputs, ModelFeatureMode::Off);
    assert_eq!(value.server_compaction, ModelFeatureMode::Auto);
    assert_eq!(
        serde_json::to_value(value).expect("migrated settings")["toolCalls"],
        "on"
    );
}

#[test]
fn auto_uses_unknown_and_positive_evidence_and_respects_explicit_negative() {
    for declared in [None, Some(true), Some(false)] {
        assert!(!ModelFeatureMode::Off.eligible(declared, false));
        assert!(ModelFeatureMode::On.eligible(declared, true));
        assert_eq!(
            ModelFeatureMode::Auto.eligible(declared, false),
            declared != Some(false)
        );
        assert!(!ModelFeatureMode::Auto.eligible(declared, true));
    }
    for value in [
        serde_json::json!("sometimes"),
        serde_json::json!(1),
        serde_json::json!(null),
    ] {
        assert!(serde_json::from_value::<ModelFeatureMode>(value).is_err());
    }
}

#[test]
fn new_partial_profiles_default_to_auto_and_reject_invalid_fields() {
    for value in [
        serde_json::json!({}),
        serde_json::json!({"toolCalls":"auto"}),
    ] {
        let settings: ModelFeatureSettings = serde_json::from_value(value).unwrap();
        assert_eq!(settings, ModelFeatureSettings::default());
    }
    for value in [
        serde_json::json!({"streaming":null}),
        serde_json::json!({"imageInputs":"sometimes"}),
        serde_json::json!({"unknown":true}),
        serde_json::json!({"declared":{"streaming":"auto"}}),
    ] {
        assert!(serde_json::from_value::<ModelFeatureSettings>(value).is_err());
    }
}
