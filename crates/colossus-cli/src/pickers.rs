use super::*;

#[derive(Debug, Eq, PartialEq)]
pub(super) enum ThemePickerInput {
    Cancelled,
    Selected(String),
    Preview(String),
    Command(String),
    Invalid,
}

pub(super) fn resolve_theme_picker_name(choice: &str, names: &[String]) -> Option<String> {
    if let Ok(index) = choice.parse::<usize>() {
        return index
            .checked_sub(1)
            .and_then(|index| names.get(index))
            .cloned();
    }
    let normalized = choice.trim().to_ascii_lowercase().replace('-', "_");
    names.iter().find(|name| **name == normalized).cloned()
}

pub(super) fn parse_theme_picker_input(choice: &str, names: &[String]) -> ThemePickerInput {
    let choice = choice.trim();
    if choice.is_empty() {
        return ThemePickerInput::Cancelled;
    }
    if choice.starts_with('/') {
        return ThemePickerInput::Command(choice.into());
    }
    if let Some(preview) = choice
        .strip_prefix("p ")
        .or_else(|| choice.strip_prefix("preview "))
    {
        return resolve_theme_picker_name(preview.trim(), names)
            .map_or(ThemePickerInput::Invalid, ThemePickerInput::Preview);
    }
    resolve_theme_picker_name(choice, names)
        .map_or(ThemePickerInput::Invalid, ThemePickerInput::Selected)
}

pub(super) fn choose_theme(
    scripted_input: &mut dyn BufRead,
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
) -> Result<ThemePickerInput, Box<dyn Error>> {
    let names = themes.names();
    print_theme_library(preferences, themes)?;
    println!(
        "Enter a number or theme name to apply it, `p NUMBER` to preview it, or leave the line blank to cancel."
    );
    loop {
        let mut choice = String::new();
        if scripted_input.read_line(&mut choice)? == 0 {
            return Ok(ThemePickerInput::Cancelled);
        }
        match parse_theme_picker_input(&choice, &names) {
            ThemePickerInput::Preview(name) => {
                print_theme_preview(preferences, themes, &name)?;
                println!(
                    "Choose a number or theme name to apply it, preview another with `p NUMBER`, or leave the line blank to cancel."
                );
            }
            ThemePickerInput::Invalid => println!(
                "That is not one of the listed themes. Enter 1-{}, a theme name, `p NUMBER`, or leave the line blank to cancel.",
                names.len()
            ),
            result => return Ok(result),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum SessionPickerInput {
    Cancelled,
    Selected(String),
    Command(String),
    Invalid,
}

pub(super) fn parse_session_picker_input(
    choice: &str,
    sessions: &[SessionSummary],
) -> SessionPickerInput {
    let choice = choice.trim();
    if choice.is_empty() {
        return SessionPickerInput::Cancelled;
    }
    if choice.starts_with('/') {
        return SessionPickerInput::Command(choice.into());
    }
    if let Ok(index) = choice.parse::<usize>()
        && let Some(session) = index.checked_sub(1).and_then(|index| sessions.get(index))
    {
        return SessionPickerInput::Selected(session.id.clone());
    }
    sessions
        .iter()
        .find(|session| session.id == choice)
        .map_or(SessionPickerInput::Invalid, |session| {
            SessionPickerInput::Selected(session.id.clone())
        })
}

pub(super) fn choose_session(
    runtime: &Runtime,
    scripted_input: &mut dyn BufRead,
    limit: usize,
) -> Result<SessionPickerInput, Box<dyn Error>> {
    let mut sessions = runtime
        .list_sessions(100)?
        .into_iter()
        .filter(|session| session.message_count > 0)
        .collect::<Vec<_>>();
    sessions.truncate(limit);
    if sessions.is_empty() {
        println!("No sessions exist yet.");
        return Ok(SessionPickerInput::Cancelled);
    }
    println!("Choose a session to resume:");
    for (index, session) in sessions.iter().enumerate() {
        println!(
            "  {}. {}  {}  messages={}",
            index + 1,
            session.id,
            session.title.as_deref().unwrap_or("Untitled"),
            session.message_count
        );
    }
    println!(
        "Enter a number or exact session id (blank cancels; /command returns to the terminal)."
    );
    loop {
        let mut choice = String::new();
        if scripted_input.read_line(&mut choice)? == 0 {
            return Ok(SessionPickerInput::Cancelled);
        }
        let parsed = parse_session_picker_input(&choice, &sessions);
        if parsed != SessionPickerInput::Invalid {
            return Ok(parsed);
        }
        println!(
            "That is not one of the listed sessions. Enter 1-{}, an exact id, or leave it blank to cancel.",
            sessions.len()
        );
    }
}
