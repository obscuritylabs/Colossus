use super::*;

/// Shared reconstruction and validation checks for presentation repository adapters.
pub fn assert_presentation_repository_conformance(repository: &dyn PresentationRepository) {
    assert_eq!(
        repository.load().expect("default presentation profile"),
        TerminalPreferences::default()
    );
    let expected = TerminalPreferences {
        theme: ThemeName::HighContrast,
        multiline: true,
        stream_mode: StreamDisplayMode::Off,
        events_mode: EventDisplayMode::Verbose,
        show_reasoning: false,
        transcript_density: TranscriptDensity::Compact,
        ..TerminalPreferences::default()
    };
    let saved = repository
        .save(
            expected.clone(),
            Actor {
                actor_type: ActorType::User,
                id: "conformance-user".into(),
            },
        )
        .expect("save presentation profile");
    assert_eq!(saved, expected);
    assert_eq!(repository.load().expect("reconstructed profile"), expected);
    assert!(
        repository
            .list_history(10)
            .expect("empty history")
            .is_empty()
    );
    assert_eq!(
        repository
            .append_history(
                "first prompt".into(),
                Actor {
                    actor_type: ActorType::User,
                    id: "conformance-user".into(),
                },
            )
            .expect("append history"),
        "first prompt"
    );
    repository
        .append_history(
            "first prompt".into(),
            Actor {
                actor_type: ActorType::User,
                id: "conformance-user".into(),
            },
        )
        .expect("deduplicate history");
    repository
        .append_history(
            "second prompt".into(),
            Actor {
                actor_type: ActorType::User,
                id: "conformance-user".into(),
            },
        )
        .expect("append second history");
    assert_eq!(
        repository.list_history(1).expect("bounded history"),
        vec!["second prompt"]
    );
    assert_eq!(
        repository.list_history(10).expect("history"),
        vec!["first prompt", "second prompt"]
    );
    assert!(repository.list_history(0).is_err());
    assert!(
        repository
            .append_history(
                " ".into(),
                Actor {
                    actor_type: ActorType::User,
                    id: "conformance-user".into(),
                },
            )
            .is_err()
    );
    let invalid = TerminalPreferences {
        schema_version: u16::MAX,
        ..TerminalPreferences::default()
    };
    assert!(
        repository
            .save(
                invalid,
                Actor {
                    actor_type: ActorType::User,
                    id: "conformance-user".into(),
                },
            )
            .is_err(),
        "unknown presentation schema must fail closed"
    );
}
