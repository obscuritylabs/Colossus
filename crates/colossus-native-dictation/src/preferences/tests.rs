use super::*;

fn isolated() -> (DictationSettings, PathBuf) {
    let temporary = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let path = temporary.join(format!(
        "colossus-dictation-settings-{}-{}",
        std::process::id(),
        NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
    ));
    let home = ColossusHome::ensure_at(path.clone()).unwrap();
    (DictationSettings::at(home.root().to_owned()).unwrap(), path)
}

#[test]
fn settings_persist_across_independent_native_clients_without_starting_capture() {
    let (settings, path) = isolated();
    let mut preferences = settings.load().unwrap();
    assert!(!preferences.enabled);
    assert!(preferences.spoken_punctuation);
    preferences.enabled = true;
    preferences.model = ModelId::BaseEnglish;
    preferences.microphone = Some("a".repeat(64));
    preferences.spoken_punctuation = false;
    settings.save(&preferences).unwrap();
    let reopened = DictationSettings::at(path.clone()).unwrap().load().unwrap();
    assert!(reopened.enabled);
    assert_eq!(reopened.model, ModelId::BaseEnglish);
    assert_eq!(reopened.microphone, preferences.microphone);
    assert!(!reopened.spoken_punctuation);
    preferences.model = ModelId::TinyEnglish;
    settings.save(&preferences).unwrap();
    assert_eq!(settings.load().unwrap().model, ModelId::TinyEnglish);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn invalid_or_oversized_settings_fail_without_overwriting_the_last_saved_choices() {
    let (settings, path) = isolated();
    settings.save(&DictationPreferences::default()).unwrap();
    let invalid = DictationPreferences {
        schema_version: 2,
        ..DictationPreferences::default()
    };
    assert_eq!(settings.save(&invalid), Err(DictationError::Settings));
    assert_eq!(settings.load().unwrap().schema_version, 1);
    let invalid = DictationPreferences {
        model_path: Some(PathBuf::from("relative.bin")),
        ..DictationPreferences::default()
    };
    assert_eq!(settings.save(&invalid), Err(DictationError::Settings));
    let invalid = DictationPreferences {
        microphone: Some("../device".into()),
        ..DictationPreferences::default()
    };
    assert_eq!(settings.save(&invalid), Err(DictationError::Settings));
    std::fs::write(
        path.join("settings.json"),
        vec![b' '; usize::try_from(MAX_SETTINGS_BYTES).unwrap() + 1],
    )
    .unwrap();
    assert!(matches!(settings.load(), Err(DictationError::Settings)));
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
#[ignore = "requires COLOSSUS_DICTATION_TEST_MODEL pointing to the reviewed Tiny English asset"]
fn bundled_model_installs_into_the_shared_private_cache_and_reopens_offline() {
    let source = PathBuf::from(std::env::var_os("COLOSSUS_DICTATION_TEST_MODEL").unwrap());
    let (settings, path) = isolated();
    assert_eq!(settings.install(&source).unwrap(), ModelId::TinyEnglish);
    let preferences = DictationPreferences {
        enabled: true,
        ..DictationPreferences::default()
    };
    settings.save(&preferences).unwrap();
    let reopened = DictationSettings::at(path.clone()).unwrap();
    assert_eq!(
        reopened
            .selected_model(&reopened.load().unwrap())
            .unwrap()
            .id(),
        ModelId::TinyEnglish
    );
    assert_eq!(reopened.install(&source).unwrap(), ModelId::TinyEnglish);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(reopened.model_path(ModelId::TinyEnglish).unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(path).unwrap();
}
