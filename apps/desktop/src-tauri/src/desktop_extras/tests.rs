use super::*;
#[test]
fn defaults_are_quiet_and_capture_requires_explicit_enablement() {
    let value = ExtrasPreferences::default();
    assert!(!value.appshots_enabled);
    assert!(!value.pet_enabled);
    assert_eq!(value.appshot_shortcut, "Control+Alt+Shift+S");
    assert!(value.validate().is_ok());
}
#[test]
fn shortcut_and_native_position_inputs_are_bounded() {
    for invalid in [
        "S",
        "Alt+S",
        "Control+Alt+S+S",
        "Control+Alt+<script>",
        "Control+Alt+Shift+Enter",
    ] {
        let value = ExtrasPreferences {
            appshot_shortcut: invalid.into(),
            ..ExtrasPreferences::default()
        };
        assert!(value.validate().is_err());
    }
    let mut value = ExtrasPreferences {
        pet_x: Some(f64::NAN),
        ..ExtrasPreferences::default()
    };
    assert!(value.validate().is_err());
    value.pet_x = None;
    value.pet_size = "arbitrary".into();
    assert!(value.validate().is_err());
}

#[test]
fn expired_capture_bytes_are_removed_while_unexpired_previews_remain() {
    let now = Instant::now();
    let capture = |age| PendingCapture {
        image: vec![1, 2, 3],
        text: Some("fixture".into()),
        title: "fixture".into(),
        width: 1,
        height: 1,
        created: now
            .checked_sub(std::time::Duration::from_secs(age))
            .unwrap(),
    };
    let mut pending = BTreeMap::from([
        ("expired".into(), capture(600)),
        ("current".into(), capture(599)),
    ]);
    assert_eq!(capture::expire_pending(&mut pending, now), vec!["expired"]);
    assert_eq!(pending.len(), 1);
    assert!(pending.contains_key("current"));
    assert_eq!(
        capture::expire_pending(&mut pending, now + std::time::Duration::from_secs(1)),
        vec!["current"]
    );
    assert!(pending.is_empty());
}

#[test]
fn discard_invalidates_only_its_exact_inflight_capture() {
    let state = ExtrasState::default();
    *state.active_capture.lock().unwrap() = Some("current".into());
    capture::cancel_inflight(&state, "older").unwrap();
    assert_eq!(state.capture_generation.load(Ordering::Acquire), 0);
    assert_eq!(
        state.active_capture.lock().unwrap().as_deref(),
        Some("current")
    );
    capture::cancel_inflight(&state, "current").unwrap();
    assert_eq!(state.capture_generation.load(Ordering::Acquire), 1);
    assert!(state.active_capture.lock().unwrap().is_none());
}

#[cfg(windows)]
#[test]
fn default_capture_shortcut_uses_the_native_plugin_grammar() {
    let shortcut = ExtrasPreferences::default().appshot_shortcut;
    assert!(
        shortcut
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .is_ok()
    );
    assert!(
        "Control+Alt+H"
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .is_ok()
    );
}
