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

#[tokio::test]
async fn native_hide_can_acquire_preferences_during_confirmation_and_stales_save() {
    let state = ExtrasState::default();
    state.preferences.lock().unwrap().pet_enabled = true;
    let requested = ExtrasPreferences {
        appshots_enabled: true,
        pet_enabled: true,
        ..ExtrasPreferences::default()
    };
    let result = review_preferences(&state, &requested, 0, || async {
        // The same lock is used by native Hide and movement callbacks. This
        // must remain available for the entire confirmation wait.
        let mut native = state
            .preferences
            .try_lock()
            .expect("modal held preferences");
        native.pet_enabled = false;
        native.pet_x = Some(42.);
        native.pet_y = Some(84.);
        state.revision.fetch_add(1, Ordering::AcqRel);
        Ok(true)
    })
    .await;
    assert!(result.is_err());
    let native = state.preferences.lock().unwrap();
    assert!(!native.pet_enabled);
    assert!(!native.appshots_enabled);
    assert_eq!((native.pet_x, native.pet_y), (Some(42.), Some(84.)));
    assert_eq!(state.revision.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn cancelled_confirmation_leaves_preferences_and_revision_unchanged() {
    let state = ExtrasState::default();
    let requested = ExtrasPreferences {
        appshots_enabled: true,
        ..ExtrasPreferences::default()
    };
    assert!(
        review_preferences(&state, &requested, 0, || async {
            assert!(state.preferences.try_lock().is_ok());
            Ok(false)
        })
        .await
        .is_err()
    );
    assert!(!state.preferences.lock().unwrap().appshots_enabled);
    assert_eq!(state.revision.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn unchanged_capture_consent_does_not_open_another_confirmation() {
    let state = ExtrasState::default();
    state.preferences.lock().unwrap().appshots_enabled = true;
    let requested = ExtrasPreferences {
        appshots_enabled: true,
        pet_enabled: true,
        ..ExtrasPreferences::default()
    };
    review_preferences(&state, &requested, 0, || {
        std::future::ready(Err(error(
            "unchanged capture consent must not prompt again",
        )))
    })
    .await
    .unwrap();
}

#[test]
fn confirmation_slot_remains_exclusive_until_its_owned_guard_drops() {
    let state = ExtrasState::default();
    let guard = ConfirmationGuard::claim(&state).unwrap();
    assert!(ConfirmationGuard::claim(&state).is_err());
    assert!(state.preferences.try_lock().is_ok());
    drop(guard);
    assert!(ConfirmationGuard::claim(&state).is_ok());
}

#[test]
fn committed_companion_failure_keeps_saved_preferences_and_reports_recovery() {
    let state = ExtrasState::default();
    let preferences = ExtrasPreferences {
        pet_enabled: true,
        ..ExtrasPreferences::default()
    };
    *state.preferences.lock().unwrap() = preferences.clone();
    state.revision.store(1, Ordering::Release);
    let context = committed_context(
        &state,
        1,
        preferences,
        Err(error("untrusted diagnostic must not escape")),
    )
    .unwrap();
    assert!(context.preferences.pet_enabled);
    assert_eq!(context.revision, 1);
    let status = context.shortcut_error.as_deref().unwrap();
    assert!(status.contains("Settings were saved"));
    assert!(status.contains("Toggle Show mini hacker"));
    assert!(!status.contains("untrusted diagnostic"));
    assert_eq!(
        state.shortcut_error.lock().unwrap().as_deref(),
        Some(status)
    );
    assert!(state.preferences.lock().unwrap().pet_enabled);
    assert_eq!(state.revision.load(Ordering::Acquire), 1);
    let recovered =
        committed_context(&state, 1, state.preferences.lock().unwrap().clone(), Ok(())).unwrap();
    assert!(recovered.shortcut_error.is_none());
    assert!(state.shortcut_error.lock().unwrap().is_none());
}

#[test]
fn companion_window_authority_rejects_other_application_windows() {
    assert!(pet::require_pet_window_label("developer-pet").is_ok());
    for untrusted in [
        "",
        "main",
        "terminal",
        "client-identity-review",
        "developer-pet-other",
    ] {
        assert!(pet::require_pet_window_label(untrusted).is_err());
    }
}
