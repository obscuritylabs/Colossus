use super::*;
use serde_json::{Value, json};

fn id(prefix: &str) -> String {
    format!("{prefix}0123456789abcdef0123456789abcdef")
}

#[test]
fn opaque_computer_ids_reject_foreign_native_and_oversized_identifiers() {
    let valid = id("cu_w_");
    assert_eq!(ComputerWindowId::parse(&valid).unwrap().as_str(), valid);
    for invalid in [
        id("cu_s_"),
        "123456".into(),
        format!("{valid}0"),
        valid.to_uppercase(),
    ] {
        assert!(ComputerWindowId::parse(&invalid).is_err());
        assert!(serde_json::from_value::<ComputerWindowId>(json!(invalid)).is_err());
    }
    assert!(serde_json::from_value::<ComputerWindowId>(json!({"hwnd": 123})).is_err());
}

#[test]
fn limits_reject_zero_overflow_unknown_fields_and_only_intersect_downward() {
    let default = ComputerLimits::default();
    let original = serde_json::to_value(default).unwrap();
    for (field, maximum) in [
        ("max_windows", 128),
        ("max_snapshot_nodes", 1024),
        ("max_screenshot_bytes", 16777216),
        ("max_screenshot_pixels", 16777216),
        ("action_timeout_ms", 10000),
        ("session_timeout_ms", 900000),
        ("observation_max_age_ms", 30000),
    ] {
        for bad in [0, maximum + 1, u64::MAX] {
            let mut altered = original.clone();
            altered[field] = json!(bad);
            assert!(
                serde_json::from_value::<ComputerLimits>(altered).is_err(),
                "{field}"
            );
        }
    }
    let mut unknown = original;
    unknown["native_handle"] = json!(42);
    assert!(serde_json::from_value::<ComputerLimits>(unknown).is_err());
    let narrow = ComputerLimits {
        max_windows: 2,
        action_timeout_ms: 100,
        ..default
    };
    assert_eq!(default.intersect(&narrow).unwrap(), narrow);
    assert!(narrow.is_within(&default));
    assert!(!default.is_within(&narrow));
}

#[test]
fn computer_input_strict_serde_rejects_native_handles_and_unbounded_coordinates() {
    for value in [
        json!({"kind":"move","x":-1,"y":0}),
        json!({"kind":"move","x":1,"y":1,"hwnd":1}),
        json!({"kind":"click","target":{"kind":"point","x":1,"y":1,"hwnd":1},"button":"left","count":1}),
        json!({"kind":"click","target":{"kind":"point","x":1,"y":1},"button":"left","count":3}),
        json!({"kind":"drag","from_x":1,"from_y":1,"to_x":2,"to_y":2,"steps":101}),
        json!({"kind":"drag","from_x":1,"from_y":1,"to_x":2,"to_y":2,"duration_ms":100}),
        json!({"kind":"scroll","x":1,"y":1,"delta_x":i32::MIN,"delta_y":0}),
        json!({"kind":"scroll","x":1,"y":1,"delta_x":0,"delta_y":0}),
        json!({"kind":"clipboard","text":"hidden"}),
    ] {
        assert!(serde_json::from_value::<ComputerInput>(value).is_err());
    }
    let input: ComputerInput = serde_json::from_value(json!({"kind":"click",
        "target":{"kind":"element","element_id":id("cu_e_")},"button":"left","count":2}))
    .unwrap();
    assert!(input.validate().is_ok());
}

#[test]
fn text_uses_utf8_byte_ceiling_and_withholds_invalid_control_sequences() {
    assert!(
        !format!(
            "{:?}",
            ComputerInput::Type {
                text: "private text".into()
            }
        )
        .contains("private text")
    );
    for text in [
        "".into(),
        "é".repeat(4097),
        "secret\0value".into(),
        "\u{1b}[2J".into(),
    ] {
        assert!(
            serde_json::from_value::<ComputerInput>(json!({"kind":"type","text":text})).is_err()
        );
    }
    for text in ["é".repeat(4096), "line one\nline two\tvalue".into()] {
        assert!(
            serde_json::from_value::<ComputerInput>(json!({"kind":"type","text":text})).is_ok()
        );
    }
}

#[test]
fn key_chords_are_closed_unique_balanced_and_cannot_escape_to_system_ui() {
    for keys in [
        vec!["Meta", "R"],
        vec!["Control"],
        vec!["A", "B"],
        vec!["A", "A"],
        vec!["Alt", "Tab"],
        vec!["Alt", "F4"],
        vec!["Control", "Shift", "Escape"],
        vec!["Control", "Alt", "Delete"],
        vec!["script.exe"],
        vec!["Return"],
    ] {
        assert!(
            serde_json::from_value::<ComputerInput>(json!({"kind":"press","keys":keys})).is_err()
        );
    }
    for keys in [
        vec!["Control", "A"],
        vec!["Shift", "ArrowLeft"],
        vec!["Enter"],
        vec!["F5"],
    ] {
        assert!(
            serde_json::from_value::<ComputerInput>(json!({"kind":"press","keys":keys})).is_ok()
        );
    }
}

#[test]
fn observation_rejects_password_values_outside_bounds_and_excessive_metadata() {
    let mut observation = ComputerObservation {
        session_id: ComputerSessionId::parse(id("cu_s_")).unwrap(),
        control_generation: 1,
        observation_id: ComputerObservationId::parse(id("cu_o_")).unwrap(),
        window_id: ComputerWindowId::parse(id("cu_w_")).unwrap(),
        width: 100,
        height: 100,
        scale_milli: 1250,
        windows: vec![],
        nodes: vec![ComputerNode {
            element_id: ComputerElementId::parse(id("cu_e_")).unwrap(),
            role: "edit".into(),
            name: "Password".into(),
            value: None,
            bounds: ComputerRect {
                x: 0,
                y: 0,
                width: 50,
                height: 20,
            },
            enabled: true,
            focused: true,
            is_password: true,
        }],
    };
    let limits = ComputerLimits::default();
    assert!(observation.validate(&limits).is_ok());
    observation.nodes[0].value = Some("private".into());
    assert!(observation.validate(&limits).is_err());
    observation.nodes[0].value = None;
    observation.nodes[0].bounds.x = 99;
    assert!(observation.validate(&limits).is_err());
    observation.nodes[0].bounds.x = 0;
    observation.nodes[0].name = "x".repeat(513);
    assert!(observation.validate(&limits).is_err());
    let mut serialized = serde_json::to_value(observation).unwrap();
    serialized["nodes"][0]["uia_pointer"] = Value::from(123);
    assert!(serde_json::from_value::<ComputerObservation>(serialized).is_err());
}

#[test]
fn negative_monitor_placement_is_valid_but_rect_and_pixel_overflow_are_rejected() {
    let mut session = ComputerSessionSummary {
        session_id: ComputerSessionId::parse(id("cu_s_")).unwrap(),
        control_generation: 1,
        window_id: ComputerWindowId::parse(id("cu_w_")).unwrap(),
        state: ComputerControlState::Active,
        reason: None,
    };
    assert!(session.validate().is_ok());
    session.reason = Some(ComputerStopReason::UserStopped);
    assert!(session.validate().is_err());
    session.state = ComputerControlState::Stopped;
    assert!(session.validate().is_ok());
    session.control_generation = 0;
    assert!(session.validate().is_err());
    assert!(
        ComputerRect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080
        }
        .validate()
        .is_ok()
    );
    assert!(
        ComputerRect {
            x: i32::MAX,
            y: 0,
            width: 1,
            height: 1
        }
        .validate()
        .is_err()
    );
    let mut descriptor = ComputerScreenshotDescriptor {
        session_id: ComputerSessionId::parse(id("cu_s_")).unwrap(),
        control_generation: 1,
        observation_id: ComputerObservationId::parse(id("cu_o_")).unwrap(),
        window_id: ComputerWindowId::parse(id("cu_w_")).unwrap(),
        width: 100,
        height: 100,
        size_bytes: 200,
        sha256: "a".repeat(64),
    };
    let limits = ComputerLimits::default();
    assert!(descriptor.validate(&limits).is_ok());
    descriptor.width = u32::MAX;
    descriptor.height = u32::MAX;
    assert!(descriptor.validate(&limits).is_err());
    descriptor.width = 100;
    descriptor.height = 100;
    descriptor.sha256 = "A".repeat(64);
    assert!(descriptor.validate(&limits).is_err());
}
