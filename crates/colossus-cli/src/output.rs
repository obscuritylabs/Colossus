use super::*;

pub(super) fn set_output_mode(mode: OutputMode) {
    let encoded = match mode {
        OutputMode::Auto => 0,
        OutputMode::Human => 1,
        OutputMode::Json => 2,
    };
    OUTPUT_MODE.store(encoded, Ordering::Relaxed);
}

pub(super) fn output_mode() -> OutputMode {
    match OUTPUT_MODE.load(Ordering::Relaxed) {
        1 => OutputMode::Human,
        2 => OutputMode::Json,
        _ => OutputMode::Auto,
    }
}

pub(super) fn set_terminal_preferences(preferences: &TerminalPreferences) {
    *TERMINAL_PREFERENCES
        .get_or_init(|| Mutex::new(TerminalPreferences::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = preferences.clone();
}

pub(super) fn terminal_preferences() -> TerminalPreferences {
    TERMINAL_PREFERENCES
        .get_or_init(|| Mutex::new(TerminalPreferences::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

pub(super) fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(columns, _)| usize::from(columns))
        .or_else(|_| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or(())
        })
        .unwrap_or(100)
        .clamp(40, 240)
}

pub(super) fn render_structured_output(
    value: &Value,
    mode: OutputMode,
    terminal: bool,
    width: usize,
    preferences: TerminalPreferences,
) -> Result<String, serde_json::Error> {
    let human = mode == OutputMode::Human || mode == OutputMode::Auto && terminal;
    if !human {
        return serde_json::to_string_pretty(value);
    }
    Ok(TerminalDocumentRenderer::new(preferences, width)
        .with_color(terminal)
        .render(&document_from_json(value, None)))
}

pub(super) fn render_run_output(
    value: &Value,
    response: &str,
    mode: OutputMode,
    terminal: bool,
    width: usize,
    preferences: TerminalPreferences,
) -> Result<String, serde_json::Error> {
    let human = mode == OutputMode::Human || mode == OutputMode::Auto && terminal;
    if !human {
        return serde_json::to_string_pretty(value);
    }
    Ok(TerminalDocumentRenderer::new(preferences, width)
        .with_color(terminal)
        .render(&PresentationDocument::from_block(
            PresentationBlock::Markdown(response.into()),
        )))
}

pub(super) fn print_json(value: &impl serde::Serialize) -> Result<(), Box<dyn Error>> {
    let value = serde_json::to_value(value)?;
    println!(
        "{}",
        render_structured_output(
            &value,
            output_mode(),
            io::stdout().is_terminal(),
            terminal_width(),
            terminal_preferences(),
        )?
    );
    Ok(())
}

pub(super) fn print_run_response(
    value: &impl serde::Serialize,
    response: &str,
) -> Result<(), Box<dyn Error>> {
    let value = serde_json::to_value(value)?;
    let terminal = io::stdout().is_terminal();
    println!(
        "{}",
        render_run_output(
            &value,
            response,
            output_mode(),
            terminal,
            terminal_width(),
            terminal_preferences(),
        )?
    );
    Ok(())
}

pub(super) fn print_theme_library(
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
) -> Result<(), Box<dyn Error>> {
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(&json!({
            "selected": preferences.theme_name(),
            "library": themes.status(),
        }));
    }

    print_terminal_document(
        &themes.status_document(preferences.theme_name()),
        preferences,
        terminal,
    );
    Ok(())
}

pub(super) fn human_output(terminal: bool) -> bool {
    output_mode() == OutputMode::Human || output_mode() == OutputMode::Auto && terminal
}

pub(super) fn print_terminal_document(
    document: &PresentationDocument,
    preferences: &TerminalPreferences,
    terminal: bool,
) {
    println!(
        "{}",
        TerminalDocumentRenderer::new(preferences.clone(), terminal_width())
            .with_color(terminal)
            .render(document)
    );
}

pub(super) fn print_theme_preview(
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    let snapshot = themes.preview(name)?;
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(&snapshot);
    }
    let preview_preferences = themes.preview_preferences(name, preferences)?;
    let document = themes.preview_document(name)?;
    print_terminal_document(&document, &preview_preferences, terminal);
    Ok(())
}

pub(super) fn print_theme_validation(
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
) -> Result<(), Box<dyn Error>> {
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(&json!({
            "valid": true,
            "library": themes.status(),
        }));
    }
    print_terminal_document(&themes.validation_document(), preferences, terminal);
    Ok(())
}

pub(super) fn print_theme_scaffold(
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
    name: &str,
) -> Result<(), Box<dyn Error>> {
    let scaffold = themes.scaffold(name)?;
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(&scaffold);
    }
    print_terminal_document(
        &ThemeLibrary::scaffold_document(&scaffold),
        preferences,
        terminal,
    );
    Ok(())
}

pub(super) fn print_theme_applied(
    preferences: &TerminalPreferences,
    themes: &ThemeLibrary,
) -> Result<(), Box<dyn Error>> {
    let terminal = io::stdout().is_terminal();
    if !human_output(terminal) {
        return print_json(preferences);
    }
    print_terminal_document(
        &themes.selection_document(preferences.theme_name()),
        preferences,
        terminal,
    );
    Ok(())
}

pub(super) fn write_stderr_document(document: &PresentationDocument) -> io::Result<()> {
    let terminal = io::stderr().is_terminal();
    let rendered = TerminalDocumentRenderer::new(terminal_preferences(), terminal_width())
        .with_color(terminal)
        .render(document);
    eprintln!("{rendered}");
    io::stderr().flush()
}

pub(super) fn print_terminal_help(preferences: &TerminalPreferences) {
    let document = terminal_help_document(preferences);
    println!(
        "{}",
        TerminalDocumentRenderer::new(preferences.clone(), terminal_width())
            .with_color(io::stdout().is_terminal())
            .render(&document)
    );
}

fn terminal_help_document(preferences: &TerminalPreferences) -> PresentationDocument {
    let mut table = PresentationTable::new(
        ["Area", "Commands", "What it does"],
        "No interactive terminal commands are available.",
    );
    for row in [
        [
            "Conversation",
            "/resume · /sessions · /session show|new|resume",
            "Resume or manage durable conversations",
        ],
        [
            "Work",
            "/work · /tasks · /decisions · /plans · /goals · /goal resume GOAL_ID · /agents",
            "Inspect and drive durable work",
        ],
        [
            "Plan workflow",
            "/plan [on|off|status|new|list] · /plan use PLAN_ID · /plan show [PLAN_ID] · /plan approve|discard · /plan execute [direct|goal [ITERATIONS]]",
            "Create, refine, approve, discard, or execute a selected plan",
        ],
        [
            "Memory & context",
            "/memories · /memory search · /context status|list|compact|restore",
            "Recall canonical memory and manage context",
        ],
        [
            "Agent resources",
            "/tools · /skills · /skill use|active|clear|show|resources|read",
            "Discover tools and activate skills",
        ],
        [
            "Research",
            "/research · /research list · /mcp servers|tools|call",
            "Run research and inspect MCP capabilities",
        ],
        [
            "Extensions",
            "/packs list|show|verify|install|enable|disable|call · /collections verify|install · /registry pull|push · /integrations",
            "Manage trusted extension surfaces",
        ],
        [
            "Runtime",
            "/workflow list|status|schedule · /telemetry · /audit verify · /projection status",
            "Inspect durable runs and runtime health",
        ],
        [
            "Provider diagnostics",
            "/models doctor [PROFILE] · /provider doctor [PROFILE]",
            "Run a bounded probe and show released provider error details",
        ],
        [
            "Appearance",
            "/theme · /stream · /events · /reasoning · /transcript · /multiline",
            "Tune the terminal experience",
        ],
        ["Exit", "/exit · Ctrl-D", "Leave the terminal safely"],
    ] {
        table.push_row(row);
    }
    PresentationDocument {
        blocks: vec![
            PresentationBlock::Markdown(
                "# Colossus Terminal\n\nType a normal message to talk to the configured primary model. Press **Tab** to complete commands and `@skill` names."
                    .into(),
            ),
            PresentationBlock::KeyValue(vec![
                ("Theme".into(), preferences.theme_name().into()),
                ("Stream".into(), preferences.stream_mode.as_str().into()),
                ("Events".into(), preferences.events_mode.as_str().into()),
                (
                    "Reasoning summaries".into(),
                    if preferences.show_reasoning { "on" } else { "off" }.into(),
                ),
                (
                    "Transcript".into(),
                    preferences.transcript_density.as_str().into(),
                ),
                (
                    "Multiline".into(),
                    if preferences.multiline { "on" } else { "off" }.into(),
                ),
            ]),
            PresentationBlock::Table(table),
        ],
    }
}

pub(super) fn parse_toggle(value: &str) -> Option<bool> {
    match value {
        "on" | "true" => Some(true),
        "off" | "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_terminal_help_documents_the_complete_plan_workflow() {
        let preferences = TerminalPreferences::default();
        let rendered = TerminalDocumentRenderer::new(preferences.clone(), 240)
            .with_color(false)
            .render(&terminal_help_document(&preferences));

        for command in [
            "/plan [on|off|status|new|list]",
            "/plan use PLAN_ID",
            "/plan show [PLAN_ID]",
            "/plan approve|discard",
            "/plan execute [direct|goal [ITERATIONS]]",
            "/goal resume GOAL_ID",
        ] {
            assert!(rendered.contains(command), "missing {command}: {rendered}");
        }
    }
}
