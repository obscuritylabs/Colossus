use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PresentationCommandResult {
    NotHandled,
    Handled,
    Save,
    ChooseTheme,
}

pub(super) fn terminal_completion_values(
    skill_names: &[String],
    themes: &ThemeLibrary,
) -> Vec<String> {
    let mut completion_values = TERMINAL_COMPLETIONS
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    for name in themes.names() {
        completion_values.push(format!("/theme {name}"));
        completion_values.push(format!("/theme preview {name}"));
    }
    completion_values.extend(skill_names.iter().map(|name| format!("@{name}")));
    completion_values
}

pub(super) fn doctor_profile<'a>(
    arguments: &'a str,
    family: &str,
) -> Result<Option<&'a str>, String> {
    let arguments = arguments.trim();
    if arguments == "doctor" {
        return Ok(None);
    }
    let Some(profile) = arguments.strip_prefix("doctor ") else {
        return Err(format!("/{family} expects doctor [PROFILE]"));
    };
    let profile = profile.trim();
    if profile.is_empty() || profile.split_whitespace().count() != 1 {
        return Err(format!("/{family} expects doctor [PROFILE]"));
    }
    Ok(Some(profile))
}

pub(super) fn resolve_skill_mentions(input: &str, skill_names: &[String]) -> (String, Vec<String>) {
    colossus_contracts::parse_leading_plugin_mentions(input, skill_names)
}

pub(super) fn remember_history_entry(history_entries: &mut Vec<String>, entry: &str) {
    if history_entries.last().is_some_and(|last| last == entry) {
        return;
    }
    if history_entries.len() == TERMINAL_HISTORY_CAPACITY {
        history_entries.remove(0);
    }
    history_entries.push(entry.into());
}

pub(super) fn handle_presentation_command(
    line: &str,
    preferences: &mut TerminalPreferences,
    themes: &ThemeLibrary,
) -> Result<PresentationCommandResult, Box<dyn Error>> {
    let mut changed = false;
    match line {
        "/tui" | "/tui prefs" => print_json(preferences)?,
        "/tui save" => changed = true,
        "/tui reset" => {
            *preferences = TerminalPreferences::default();
            changed = true;
        }
        "/theme" if human_output(io::stdout().is_terminal()) => {
            return Ok(PresentationCommandResult::ChooseTheme);
        }
        "/theme" | "/theme list" => print_theme_library(preferences, themes)?,
        "/theme reset" => {
            preferences.select_builtin_theme(ThemeName::Default);
            changed = true;
        }
        "/theme preview" => print_theme_library(preferences, themes)?,
        command if command.starts_with("/theme preview ") => {
            match print_theme_preview(
                preferences,
                themes,
                command.trim_start_matches("/theme preview ").trim(),
            ) {
                Ok(()) => {}
                Err(error) => println!("recoverable: {error}"),
            }
        }
        "/theme validate" => print_theme_validation(preferences, themes)?,
        "/theme scaffold" => {
            println!("recoverable: usage: /theme scaffold NAME");
        }
        command if command.starts_with("/theme scaffold ") => {
            match print_theme_scaffold(
                preferences,
                themes,
                command.trim_start_matches("/theme scaffold ").trim(),
            ) {
                Ok(()) => {}
                Err(error) => println!("recoverable: {error}"),
            }
        }
        command if command.starts_with("/theme save ") => {
            println!(
                "note: `/theme save NAME` is deprecated; `/theme NAME` applies and saves immediately."
            );
            match themes.select(
                command.trim_start_matches("/theme save ").trim(),
                preferences,
            ) {
                Ok(()) => changed = true,
                Err(error) => println!("recoverable: {error}"),
            }
        }
        command if command.starts_with("/theme ") => {
            match themes.select(command.trim_start_matches("/theme ").trim(), preferences) {
                Ok(()) => changed = true,
                Err(error) => println!("recoverable: {error}"),
            }
        }
        "/events" => println!("events={}", preferences.events_mode.as_str()),
        "/events compact" => {
            preferences.events_mode = EventDisplayMode::Compact;
            changed = true;
        }
        "/events verbose" => {
            preferences.events_mode = EventDisplayMode::Verbose;
            changed = true;
        }
        "/events off" => {
            preferences.events_mode = EventDisplayMode::Off;
            changed = true;
        }
        "/transcript" => println!("transcript={}", preferences.transcript_density.as_str()),
        "/transcript comfortable" => {
            preferences.transcript_density = TranscriptDensity::Comfortable;
            changed = true;
        }
        "/transcript compact" => {
            preferences.transcript_density = TranscriptDensity::Compact;
            changed = true;
        }
        "/stream" => println!("stream={}", preferences.stream_mode.as_str()),
        "/stream on" => {
            preferences.stream_mode = StreamDisplayMode::On;
            changed = true;
        }
        "/stream raw" => {
            preferences.stream_mode = StreamDisplayMode::Raw;
            changed = true;
        }
        "/stream off" => {
            preferences.stream_mode = StreamDisplayMode::Off;
            changed = true;
        }
        "/reasoning" => println!(
            "reasoning={}",
            if preferences.show_reasoning {
                "on"
            } else {
                "off"
            }
        ),
        command if command.starts_with("/reasoning ") => {
            if let Some(value) = parse_toggle(command.trim_start_matches("/reasoning ")) {
                preferences.show_reasoning = value;
                changed = true;
            } else {
                println!("recoverable: /reasoning expects on or off");
            }
        }
        "/multiline" => println!(
            "multiline={}",
            if preferences.multiline { "on" } else { "off" }
        ),
        command if command.starts_with("/multiline ") => {
            let value = command.trim_start_matches("/multiline ");
            if value == "toggle" {
                preferences.multiline = !preferences.multiline;
                changed = true;
            } else if let Some(value) = parse_toggle(value) {
                preferences.multiline = value;
                changed = true;
            } else {
                println!("recoverable: /multiline expects on, off, or toggle");
            }
        }
        "/trace" => {
            preferences.events_mode = if preferences.events_mode == EventDisplayMode::Off {
                EventDisplayMode::Compact
            } else {
                EventDisplayMode::Off
            };
            changed = true;
        }
        command
            if command.starts_with("/tui ")
                || command.starts_with("/events ")
                || command.starts_with("/transcript ")
                || command.starts_with("/stream ") =>
        {
            println!("recoverable: invalid presentation command; use /help");
        }
        _ => return Ok(PresentationCommandResult::NotHandled),
    }
    if changed {
        Ok(PresentationCommandResult::Save)
    } else {
        Ok(PresentationCommandResult::Handled)
    }
}

pub(super) fn cli_error(message: impl Into<String>) -> std::io::Error {
    std::io::Error::other(message.into())
}

pub(super) fn parse_environment(
    entries: Vec<String>,
) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let mut environment = BTreeMap::new();
    for entry in entries {
        let (name, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("environment entry must be KEY=VALUE: {entry}"))?;
        if name.is_empty() || environment.insert(name.into(), value.into()).is_some() {
            return Err(format!("environment name is empty or duplicated: {name}").into());
        }
    }
    Ok(environment)
}

pub(super) fn parse_headers(
    entries: Vec<String>,
) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let mut headers = BTreeMap::new();
    for entry in entries {
        let (name, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("header entry must be NAME=VALUE: {entry}"))?;
        if name.is_empty() || headers.insert(name.into(), value.into()).is_some() {
            return Err(format!("header name is empty or duplicated: {name}").into());
        }
    }
    Ok(headers)
}

pub(super) const MAX_WEBHOOK_HTTP_HEADER_BYTES: usize = 64 * 1024;
pub(super) const MAX_WEBHOOK_HTTP_BODY_BYTES: usize = 1024 * 1024;
