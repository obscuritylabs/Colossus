use super::*;

/// Launch the terminal UI and retain exclusive ownership of all terminal writes.
pub async fn run_tui(
    host: Arc<dyn InteractiveHost>,
    mut options: TuiOptions,
) -> Result<(), TuiError> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(TuiError::NotInteractive);
    }
    if std::env::var_os("ZELLIJ").is_some() {
        options.screen_mode = ScreenMode::Inline;
    }
    let snapshot = host
        .bootstrap(options.bootstrap)
        .await
        .map_err(TuiError::Host)?;
    let mut state = TuiState::from_snapshot(snapshot);
    let (event_tx, mut event_rx) = mpsc::channel::<HostEvent>(256);
    let mut terminal = OwnedTerminal::new(options.screen_mode)?;

    loop {
        terminal.draw(&mut state)?;
        while let Ok(host_event) = event_rx.try_recv() {
            handle_host_event(&mut state, host_event);
        }
        if !state.is_busy()
            && !state.queue_paused
            && state.overlay.is_none()
            && let Some(line) = state.queue.pop_front()
        {
            start_line(&mut state, line, Arc::clone(&host), event_tx.clone());
        }
        if state.should_exit {
            break;
        }
        if event::poll(Duration::from_millis(33))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_key(&mut state, key, Arc::clone(&host), event_tx.clone());
                }
                Event::Paste(text) => insert_active_text(&mut state, &text),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    Ok(())
}

pub(super) fn handle_key(
    state: &mut TuiState,
    key: KeyEvent,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.cancel_focus();
        return;
    }
    if state.overlay.is_some() {
        handle_overlay_key(state, key);
        return;
    }
    match key.code {
        KeyCode::Esc => {
            state.hide_completion();
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if !state.is_busy() && state.composer.draft.is_empty() {
                state.should_exit = true;
            }
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.overlay = Some(Overlay::HistorySearch {
                query: String::new(),
            });
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.composer.cursor = 0;
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.composer.cursor = state.composer.draft.len();
        }
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            state.composer.insert(&character.to_string());
        }
        KeyCode::Backspace => state.composer.backspace(),
        KeyCode::Delete => state.composer.delete(),
        KeyCode::Left => state.composer.move_left(),
        KeyCode::Right => {
            if state.composer.cursor == state.composer.draft.len() && state.accept_completion() {
                return;
            }
            state.composer.move_right();
        }
        KeyCode::Home => state.composer.cursor = 0,
        KeyCode::End => {
            if state.composer.cursor == state.composer.draft.len() {
                state.end();
            } else {
                state.composer.cursor = state.composer.draft.len();
            }
        }
        KeyCode::Up if !state.completion_menu_candidates().is_empty() => {
            state.previous_completion();
        }
        KeyCode::Down if !state.completion_menu_candidates().is_empty() => {
            state.advance_completion();
        }
        KeyCode::Up => state.previous_history(),
        KeyCode::Down => state.next_history(),
        KeyCode::Tab => {
            state.accept_completion();
        }
        KeyCode::BackTab => state.previous_completion(),
        KeyCode::PageUp => {
            state.page_up();
            request_older_page(state, host, event_tx);
        }
        KeyCode::PageDown => state.page_down(),
        KeyCode::Enter => {
            if state.composer.completion_index.is_some() && state.accept_completion() {
                return;
            }
            let submit = !state.preferences.multiline
                || key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL);
            if submit {
                let line = state.composer.take();
                submit_line(state, line, host, event_tx);
            } else {
                state.composer.insert("\n");
            }
        }
        _ => {}
    }
}

pub(super) fn handle_overlay_key(state: &mut TuiState, key: KeyEvent) {
    if key.code == KeyCode::Esc {
        state.cancel_focus();
        return;
    }
    let Some(overlay) = state.overlay.as_mut() else {
        return;
    };
    match overlay {
        Overlay::Prompt {
            request,
            input,
            selected,
        } => match key.code {
            KeyCode::Enter => {
                let overlay = state.overlay.take();
                if let Some(Overlay::Prompt {
                    request,
                    input,
                    selected,
                }) = overlay
                {
                    let answer = input.trim();
                    let response = if answer.is_empty() {
                        selected
                            .and_then(|index| request.choices.get(index))
                            .cloned()
                            .map(PromptResponse::Answer)
                            .unwrap_or(PromptResponse::Cancelled)
                    } else if let Ok(index) = answer.parse::<usize>() {
                        request
                            .choices
                            .get(index.saturating_sub(1))
                            .cloned()
                            .map(PromptResponse::Answer)
                            .unwrap_or(PromptResponse::Cancelled)
                    } else if request.allow_free_form
                        || request.choices.iter().any(|choice| choice == answer)
                    {
                        PromptResponse::Answer(answer.to_owned())
                    } else {
                        PromptResponse::Cancelled
                    };
                    let _ = request.response.send(response);
                }
            }
            KeyCode::Up | KeyCode::BackTab if !request.choices.is_empty() => {
                let current = selected.unwrap_or(0);
                *selected = Some(if current == 0 {
                    request.choices.len() - 1
                } else {
                    current - 1
                });
            }
            KeyCode::Down | KeyCode::Tab if !request.choices.is_empty() => {
                *selected =
                    Some(selected.map_or(0, |current| (current + 1) % request.choices.len()));
            }
            KeyCode::Home if !request.choices.is_empty() => {
                *selected = Some(0);
            }
            KeyCode::End if !request.choices.is_empty() => {
                *selected = Some(request.choices.len() - 1);
            }
            KeyCode::PageUp if !request.choices.is_empty() => {
                *selected = Some(selected.unwrap_or(0).saturating_sub(5));
            }
            KeyCode::PageDown if !request.choices.is_empty() => {
                *selected = Some(
                    selected
                        .unwrap_or(0)
                        .saturating_add(5)
                        .min(request.choices.len() - 1),
                );
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                input.push(character);
            }
            _ => {}
        },
        Overlay::HistorySearch { query } => match key.code {
            KeyCode::Enter => {
                let selected = state
                    .history
                    .iter()
                    .rev()
                    .find(|entry| entry.contains(query.as_str()))
                    .cloned();
                state.overlay = None;
                if let Some(selected) = selected {
                    state.composer.set(selected);
                }
            }
            KeyCode::Backspace => {
                query.pop();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                query.push(character);
            }
            _ => {}
        },
        Overlay::QueuePaused => match key.code {
            KeyCode::Char('r' | 'R') | KeyCode::Enter => {
                state.queue_paused = false;
                state.overlay = None;
            }
            KeyCode::Char('c' | 'C') => {
                state.queue.clear();
                state.queue_paused = false;
                state.overlay = None;
            }
            _ => {}
        },
    }
}

pub(super) fn request_older_page(
    state: &mut TuiState,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    if state.loading_older || !state.has_more {
        return;
    }
    let Some(before_sequence) = state.before_sequence else {
        return;
    };
    state.loading_older = true;
    let session_id = state.session_id.clone();
    tokio::spawn(async move {
        let result = host.older_messages(&session_id, before_sequence).await;
        let _ = event_tx.send(HostEvent::OlderPage(result)).await;
    });
}

pub(super) fn insert_active_text(state: &mut TuiState, text: &str) {
    let text = sanitize_input(text);
    if let Some(overlay) = state.overlay.as_mut() {
        match overlay {
            Overlay::Prompt { input, .. } | Overlay::HistorySearch { query: input } => {
                input.push_str(&text);
            }
            Overlay::QueuePaused => {}
        }
    } else {
        state.composer.insert(&text);
    }
}

pub(super) fn submit_line(
    state: &mut TuiState,
    line: String,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    let line = line.trim().to_owned();
    if line.is_empty() {
        return;
    }
    state.remember_history(&line);
    let history_host = Arc::clone(&host);
    let history_tx = event_tx.clone();
    let history_line = line.clone();
    tokio::spawn(async move {
        if let Err(error) = history_host.append_history(history_line).await {
            let _ = history_tx.send(HostEvent::HistoryWarning(error)).await;
        }
    });

    if state.is_busy() {
        if matches!(
            parse_interactive_command(&line),
            InteractiveCommand::Local(LocalCommand::Help | LocalCommand::Preferences)
        ) {
            start_line(state, line, host, event_tx);
            return;
        }
        if state.queue.len() < MAX_QUEUED_TURNS {
            state.queue.push_back(line);
        } else {
            state.append_entry(error_entry("The future-turn queue is full (8 entries)."));
        }
        return;
    }
    start_line(state, line, host, event_tx);
}

pub(super) fn start_line(
    state: &mut TuiState,
    line: String,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    match parse_interactive_command(&line) {
        InteractiveCommand::Empty => {}
        InteractiveCommand::Local(command) => handle_local_command(state, command, host, event_tx),
        InteractiveCommand::Runtime(command) => {
            state.append_entry(user_entry(&line, TranscriptKind::Command));
            state.operation = Some(OperationKind::Command);
            state.started_at = Some(Instant::now());
            state.activity = Some(format!("running /{}", runtime_command_name(&command)));
            let session_id = state.session_id.clone();
            let sticky_skills = state.sticky_skills.clone();
            let task_tx = event_tx.clone();
            tokio::spawn(async move {
                let result = host
                    .execute_command(command, &session_id, &sticky_skills, task_tx.clone())
                    .await
                    .map(OperationResult::Command);
                let _ = task_tx
                    .send(HostEvent::OperationFinished(Box::new(result)))
                    .await;
            });
        }
        InteractiveCommand::Turn(prompt) => {
            state.append_entry(user_entry(&prompt, TranscriptKind::User));
            state.operation = Some(OperationKind::Run);
            state.started_at = Some(Instant::now());
            state.activity = Some("waiting for model".into());
            let control = RunControl::default();
            state.control = Some(control.clone());
            let request = state.run_request(prompt);
            let task_tx = event_tx.clone();
            tokio::spawn(async move {
                let result = host
                    .run_turn(request, task_tx.clone(), control)
                    .await
                    .map(OperationResult::Run);
                let _ = task_tx
                    .send(HostEvent::OperationFinished(Box::new(result)))
                    .await;
            });
        }
    }
}

pub(super) fn handle_local_command(
    state: &mut TuiState,
    command: LocalCommand,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    match command {
        LocalCommand::Exit => state.should_exit = true,
        LocalCommand::Help => state.append_entry(TranscriptEntry {
            sequence: None,
            kind: TranscriptKind::Command,
            document: help_document(),
            temporary: false,
        }),
        LocalCommand::Preferences => state.append_entry(TranscriptEntry {
            sequence: None,
            kind: TranscriptKind::Command,
            document: preferences_document(&state.preferences),
            temporary: false,
        }),
        LocalCommand::ProviderDiagnostics(enabled) => {
            state.provider_response_diagnostics = enabled;
            state.append_entry(TranscriptEntry {
                sequence: None,
                kind: TranscriptKind::Command,
                document: provider_diagnostics_document(enabled),
                temporary: false,
            });
        }
        LocalCommand::SavePreferences | LocalCommand::ResetPreferences => {
            let preferences = if command == LocalCommand::ResetPreferences {
                TerminalPreferences::default()
            } else {
                state.preferences.clone()
            };
            state.operation = Some(OperationKind::Command);
            state.activity = Some("saving terminal preferences".into());
            let task_tx = event_tx.clone();
            tokio::spawn(async move {
                let result = host.save_preferences(preferences).await.map(|preferences| {
                    OperationResult::Command(HostCommandResult {
                        document: preferences_document(&preferences),
                        session: None,
                        preferences: Some(preferences),
                        completions: None,
                        sticky_skills: None,
                        footer: None,
                        clear_transcript: false,
                    })
                });
                let _ = task_tx
                    .send(HostEvent::OperationFinished(Box::new(result)))
                    .await;
            });
        }
    }
}

pub(super) fn handle_host_event(state: &mut TuiState, event: HostEvent) {
    match event {
        HostEvent::Run(envelope) => handle_run_event(state, envelope),
        HostEvent::Notice(document) => {
            state.append_entry(TranscriptEntry {
                sequence: None,
                kind: TranscriptKind::Command,
                document,
                temporary: false,
            });
        }
        HostEvent::Prompt(request) => {
            if state.overlay.is_some() {
                let _ = request.response.send(PromptResponse::Cancelled);
            } else {
                let selected = request
                    .initial_choice
                    .filter(|index| *index < request.choices.len());
                state.overlay = Some(Overlay::Prompt {
                    request,
                    input: String::new(),
                    selected,
                });
            }
        }
        HostEvent::HistoryWarning(error) => {
            state.append_entry(error_entry(&format!("History was not persisted: {error}")))
        }
        HostEvent::OlderPage(result) => {
            state.loading_older = false;
            match result {
                Ok(page) => state.prepend_page(page),
                Err(error) => state.append_entry(error_entry(&format!(
                    "Older transcript messages could not be loaded: {error}"
                ))),
            }
        }
        HostEvent::OperationFinished(result) => {
            let result = *result;
            state.operation = None;
            state.control = None;
            state.activity = None;
            state.started_at = None;
            let successful = match result {
                Ok(OperationResult::Command(result)) => {
                    apply_command_result(state, result);
                    true
                }
                Ok(OperationResult::Run(HostRunResult {
                    outcome: AgentRunOutcome::Completed { result },
                    footer,
                })) => {
                    finalize_assistant(state, &result.output);
                    state.footer = footer;
                    true
                }
                Ok(OperationResult::Run(HostRunResult {
                    outcome: AgentRunOutcome::Cancelled { .. },
                    footer,
                })) => {
                    state.footer = footer;
                    state.append_entry(TranscriptEntry {
                        sequence: None,
                        kind: TranscriptKind::Command,
                        document: PresentationDocument::from_block(PresentationBlock::Card {
                            title: "Run cancelled".into(),
                            tone: PresentationTone::Warning,
                            body: vec![PresentationBlock::Text(
                                "No new effect will start. Any active effect settled first.".into(),
                            )],
                        }),
                        temporary: false,
                    });
                    false
                }
                Err(error) => {
                    state.footer.status = "error".into();
                    state.append_entry(error_entry(&format!("Operation failed: {error}")));
                    false
                }
            };
            if successful && !state.queue_paused {
                // The event loop starts this on its next key/tick iteration through a
                // synthetic queue drain in `draw`; this avoids re-entrant host spawning.
            } else if !state.queue.is_empty() {
                state.queue_paused = true;
                state.overlay = Some(Overlay::QueuePaused);
            }
        }
    }
}

pub(super) fn handle_run_event(state: &mut TuiState, envelope: RunEventEnvelope) {
    let event = envelope.event;
    match &event {
        RunEvent::Provider {
            event: ProviderEvent::ModelDelta { text },
        } if state.preferences.stream_mode != colossus_contracts::StreamDisplayMode::Off => {
            update_streaming_assistant(state, text);
            return;
        }
        RunEvent::Provider {
            event: ProviderEvent::FinalOutput { text },
        } => {
            finalize_assistant(state, text);
            return;
        }
        RunEvent::Phase {
            phase,
            action,
            elapsed_seconds,
            ..
        } => {
            state.activity = Some(action.clone().unwrap_or_else(|| {
                format!(
                    "{} ({elapsed_seconds:.1}s)",
                    format!("{phase:?}").to_lowercase()
                )
            }));
            return;
        }
        RunEvent::ToolStarted { call, .. } => {
            state.activity = Some(format!("running {}", call.name));
            state
                .active_calls
                .insert(call.call_id.clone(), call.clone());
            return;
        }
        _ => {}
    }

    let (kind, call) = match &event {
        RunEvent::ToolCompleted { result, .. } => (
            TranscriptKind::Tool,
            state.active_calls.remove(&result.call_id),
        ),
        RunEvent::ToolCancelled { call, .. } => {
            state.active_calls.remove(&call.call_id);
            (TranscriptKind::Tool, None)
        }
        RunEvent::Error { .. } => (TranscriptKind::Error, None),
        RunEvent::Provider {
            event: ProviderEvent::ReasoningSummary { .. },
        } => (TranscriptKind::Assistant, None),
        RunEvent::Provider {
            event: ProviderEvent::Usage { .. },
        } => (TranscriptKind::Command, None),
        RunEvent::Provider { .. } => return,
        RunEvent::Phase { .. } | RunEvent::ToolStarted { .. } => return,
    };
    let source = TranscriptRenderSource::RunEvent { event, call };
    if let Some(document) = source.render(&state.preferences) {
        state.append_entry_with_source(
            TranscriptEntry {
                sequence: None,
                kind,
                document,
                temporary: false,
            },
            Some(source),
        );
    }
}

pub(super) fn apply_command_result(state: &mut TuiState, result: HostCommandResult) {
    if result.clear_transcript {
        state.transcript.clear();
        state.transcript_sources.clear();
        state.end();
    }
    if !result.document.is_empty() {
        state.append_entry(TranscriptEntry {
            sequence: None,
            kind: TranscriptKind::Command,
            document: result.document,
            temporary: false,
        });
    }
    if let Some((session_id, page)) = result.session {
        state.session_id = session_id;
        let (transcript, transcript_sources) =
            transcript_from_messages(page.messages, &state.preferences);
        state.transcript = transcript;
        state.transcript_sources = transcript_sources;
        state.before_sequence = page.before_sequence;
        state.has_more = page.has_more;
        state.end();
    }
    if let Some(preferences) = result.preferences {
        state.set_preferences(preferences);
    }
    if let Some(completions) = result.completions {
        state.completions = completions;
    }
    if let Some(sticky_skills) = result.sticky_skills {
        state.sticky_skills = sticky_skills;
    }
    if let Some(footer) = result.footer {
        state.footer = footer;
    }
}

pub(super) fn update_streaming_assistant(state: &mut TuiState, delta: &str) {
    let old_line_count = if state.scroll_from_bottom > 0 {
        transcript_lines(state, state.transcript_width).len()
    } else {
        0
    };
    if let Some(entry) = state.transcript.last_mut()
        && entry.temporary
        && entry.kind == TranscriptKind::Assistant
        && let Some(PresentationBlock::Text(text)) = entry.document.blocks.first_mut()
    {
        text.push_str(delta);
        if state.scroll_from_bottom > 0 {
            state.preserve_scroll_after_line_change(old_line_count);
        }
        return;
    }
    state.append_entry(TranscriptEntry {
        sequence: None,
        kind: TranscriptKind::Assistant,
        document: PresentationDocument::from_block(PresentationBlock::Text(delta.into())),
        temporary: true,
    });
}

pub(super) fn finalize_assistant(state: &mut TuiState, output: &str) {
    let old_line_count = if state.scroll_from_bottom > 0 {
        transcript_lines(state, state.transcript_width).len()
    } else {
        0
    };
    if let Some(entry) = state.transcript.last_mut()
        && entry.temporary
        && entry.kind == TranscriptKind::Assistant
    {
        entry.document =
            if state.preferences.stream_mode == colossus_contracts::StreamDisplayMode::Raw {
                PresentationDocument::from_block(PresentationBlock::Text(output.into()))
            } else {
                PresentationDocument::from_block(PresentationBlock::Markdown(output.into()))
            };
        entry.temporary = false;
        if state.scroll_from_bottom > 0 {
            state.preserve_scroll_after_line_change(old_line_count);
        }
        return;
    }
    let completed = PresentationDocument::from_block(PresentationBlock::Markdown(output.into()));
    if state.transcript.last().is_some_and(|entry| {
        !entry.temporary && entry.kind == TranscriptKind::Assistant && entry.document == completed
    }) {
        return;
    }
    if !output.is_empty() {
        state.append_entry(TranscriptEntry {
            sequence: None,
            kind: TranscriptKind::Assistant,
            document: completed,
            temporary: false,
        });
    }
}
