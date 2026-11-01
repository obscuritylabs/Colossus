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
    assert_eq!(value.document_inputs, ModelFeatureMode::Off);
    assert_eq!(value.server_compaction, ModelFeatureMode::Auto);
    assert_eq!(
        serde_json::to_value(value).expect("migrated settings")["toolCalls"],
        "on"
    );
}

#[test]
fn disabled_documents_preserve_legacy_serialization_and_explicit_modes_round_trip() {
    let legacy = serde_json::json!({
        "toolCalls": "auto", "streaming": "auto", "imageInputs": "auto",
        "serverCompaction": "auto",
        "declared": {"toolCalls": null, "streaming": null, "imageInputs": null, "serverCompaction": null}
    });
    let settings: ModelFeatureSettings = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(settings.document_inputs, ModelFeatureMode::Off);
    assert_eq!(serde_json::to_value(settings).unwrap(), legacy);
    let legacy_capabilities =
        serde_json::json!({"toolCalls": true, "streaming": true, "imageInputs": false});
    let capabilities: ModelCapabilities =
        serde_json::from_value(legacy_capabilities.clone()).unwrap();
    assert!(!capabilities.document_inputs);
    assert_eq!(
        serde_json::to_value(capabilities).unwrap(),
        legacy_capabilities
    );
    for mode in [ModelFeatureMode::On, ModelFeatureMode::Auto] {
        let settings = ModelFeatureSettings {
            document_inputs: mode,
            ..settings
        };
        let json = serde_json::to_value(settings).unwrap();
        assert!(json.get("documentInputs").is_some());
        assert_eq!(
            serde_json::from_value::<ModelFeatureSettings>(json).unwrap(),
            settings
        );
    }
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
