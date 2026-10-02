use super::*;
use colossus_tui::{
    InteractiveApprovalMode, InteractiveCommand, LocalCommand, ResearchCommand, RuntimeCommand,
    parse_interactive_command,
};

#[test]
fn every_terminal_catalog_entry_preserves_its_command_route_and_arguments() {
    let local_commands = BTreeMap::from([
        ("/help", InteractiveCommand::Local(LocalCommand::Help)),
        (
            "/tui prefs",
            InteractiveCommand::Local(LocalCommand::Preferences),
        ),
        (
            "/tui save",
            InteractiveCommand::Local(LocalCommand::SavePreferences),
        ),
        (
            "/tui reset",
            InteractiveCommand::Local(LocalCommand::ResetPreferences),
        ),
        (
            "/provider diagnostics on",
            InteractiveCommand::Local(LocalCommand::ProviderDiagnostics(true)),
        ),
        (
            "/provider diagnostics off",
            InteractiveCommand::Local(LocalCommand::ProviderDiagnostics(false)),
        ),
        (
            "/permissions",
            InteractiveCommand::Runtime(RuntimeCommand::Permissions(None)),
        ),
        (
            "/permissions deny",
            InteractiveCommand::Runtime(RuntimeCommand::Permissions(Some(
                InteractiveApprovalMode::Deny,
            ))),
        ),
        (
            "/permissions ask",
            InteractiveCommand::Runtime(RuntimeCommand::Permissions(Some(
                InteractiveApprovalMode::Ask,
            ))),
        ),
        (
            "/permissions risk-auto",
            InteractiveCommand::Runtime(RuntimeCommand::Permissions(Some(
                InteractiveApprovalMode::RiskAuto,
            ))),
        ),
        (
            "/permissions full-access",
            InteractiveCommand::Runtime(RuntimeCommand::Permissions(Some(
                InteractiveApprovalMode::FullAccess,
            ))),
        ),
        (
            "/research",
            InteractiveCommand::Research(ResearchCommand::Toggle),
        ),
        (
            "/research list",
            InteractiveCommand::Research(ResearchCommand::List),
        ),
        ("/exit", InteractiveCommand::Local(LocalCommand::Exit)),
    ]);
    let completions = terminal_completion_values(&[], &ThemeLibrary::default());
    for command in completions {
        for input in [command.clone(), format!(" \t{command}\r\n")] {
            let parsed = parse_interactive_command(&input);
            if let Some(expected) = local_commands.get(command.as_str()) {
                assert_eq!(&parsed, expected, "{input:?}");
            } else {
                let InteractiveCommand::Runtime(RuntimeCommand::Known { name, arguments }) = parsed
                else {
                    panic!("catalog command lost its host route: {input:?}: {parsed:?}");
                };
                assert!(!name.is_empty() && !name.chars().any(char::is_whitespace));
                assert_eq!(format!("/{name} {arguments}").trim_end(), command);
            }
        }
    }
}

#[test]
fn terminal_display_switches_apply_every_advertised_value() {
    let themes = ThemeLibrary::default();
    let mut preferences = TerminalPreferences::default();
    for (command, expected) in [
        ("/stream on", StreamDisplayMode::On),
        ("/stream raw", StreamDisplayMode::Raw),
        ("/stream off", StreamDisplayMode::Off),
        ("/stream on", StreamDisplayMode::On),
    ] {
        apply_switch(command, &mut preferences, &themes);
        assert_eq!(preferences.stream_mode, expected, "{command}");
    }
    for (command, expected) in [
        ("/events compact", EventDisplayMode::Compact),
        ("/events verbose", EventDisplayMode::Verbose),
        ("/events off", EventDisplayMode::Off),
        ("/trace", EventDisplayMode::Compact),
        ("/trace", EventDisplayMode::Off),
    ] {
        apply_switch(command, &mut preferences, &themes);
        assert_eq!(preferences.events_mode, expected, "{command}");
    }
    for (command, expected) in [
        ("/reasoning on", true),
        ("/reasoning off", false),
        ("/reasoning on", true),
    ] {
        apply_switch(command, &mut preferences, &themes);
        assert_eq!(preferences.show_reasoning, expected, "{command}");
    }
    for (command, expected) in [
        ("/multiline on", true),
        ("/multiline off", false),
        ("/multiline toggle", true),
        ("/multiline toggle", false),
    ] {
        apply_switch(command, &mut preferences, &themes);
        assert_eq!(preferences.multiline, expected, "{command}");
    }
    for (command, expected) in [
        ("/transcript compact", TranscriptDensity::Compact),
        ("/transcript comfortable", TranscriptDensity::Comfortable),
    ] {
        apply_switch(command, &mut preferences, &themes);
        assert_eq!(preferences.transcript_density, expected, "{command}");
    }
    for theme in themes.names() {
        apply_switch(&format!("/theme {theme}"), &mut preferences, &themes);
        assert_eq!(preferences.theme_name(), theme);
    }
    apply_switch("/theme reset", &mut preferences, &themes);
    assert_eq!(preferences.theme_name(), "default");
    apply_switch("/tui reset", &mut preferences, &themes);
    assert_eq!(preferences, TerminalPreferences::default());
    apply_switch("/tui save", &mut preferences, &themes);
}

fn apply_switch(command: &str, preferences: &mut TerminalPreferences, themes: &ThemeLibrary) {
    assert_eq!(
        handle_presentation_command(command, preferences, themes).expect("display command"),
        PresentationCommandResult::Save,
        "{command}"
    );
}
