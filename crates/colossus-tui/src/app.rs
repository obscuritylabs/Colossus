use super::*;
use crate::contract::DEFAULT_GOAL_ITERATIONS;

/// Launch the terminal UI and retain exclusive ownership of all terminal writes.
pub async fn run_tui(host: Arc<dyn InteractiveHost>, options: TuiOptions) -> Result<(), TuiError> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(TuiError::NotInteractive);
    }
    let snapshot = host
        .bootstrap(options.bootstrap)
        .await
        .map_err(TuiError::Host)?;
    let mut state = TuiState::from_snapshot(snapshot);
    if options.screen_mode == ScreenMode::Inline {
        preload_native_history(&mut state, Arc::clone(&host)).await;
    }
    let (event_tx, mut event_rx) = mpsc::channel::<HostEvent>(256);
    let mut terminal = OwnedTerminal::new(options.screen_mode)?;

    loop {
        terminal.draw(&mut state)?;
        while let Ok(host_event) = event_rx.try_recv() {
            handle_host_event(&mut state, host_event);
        }
        continue_native_history_preload(
            &mut state,
            Arc::clone(&host),
            event_tx.clone(),
            options.screen_mode,
        );
        if !state.is_busy() && !state.queue_paused && state.overlay.is_none() {
            if let Some(request) = state.pending_plan_execution.take() {
                start_plan_execution(&mut state, request, Arc::clone(&host), event_tx.clone());
            } else if let Some(line) = state.queue.pop_front() {
                start_line(&mut state, line, Arc::clone(&host), event_tx.clone());
            }
        }
        if state.should_exit {
            break;
        }
        if event::poll(Duration::from_millis(33))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_key(
                        &mut state,
                        key,
                        Arc::clone(&host),
                        event_tx.clone(),
                        options.screen_mode,
                    );
                }
                Event::Mouse(mouse) => {
                    if handle_mouse(&mut state, mouse) {
                        request_older_page(&mut state, Arc::clone(&host), event_tx.clone());
                    }
                }
                Event::Paste(text) => insert_active_text(&mut state, &text),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    Ok(())
}

fn continue_native_history_preload(
    state: &mut TuiState,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
    screen_mode: ScreenMode,
) {
    if screen_mode != ScreenMode::Inline || !state.has_more || state.loading_older {
        return;
    }
    if state.older_page_failed {
        state.has_more = false;
        state.before_sequence = None;
        return;
    }
    if state.native_history_pages_loaded >= MAX_NATIVE_HISTORY_PAGES {
        state.append_entry(native_history_limit_entry());
        state.has_more = false;
        state.before_sequence = None;
        return;
    }
    if state.before_sequence.is_none() {
        state.append_entry(error_entry(
            "Older transcript history advertised no continuation cursor; native scrollback starts at the oldest safely loaded page.",
        ));
        state.has_more = false;
        return;
    }
    state.native_history_pages_loaded += 1;
    request_older_page(state, host, event_tx);
}

async fn preload_native_history(state: &mut TuiState, host: Arc<dyn InteractiveHost>) {
    for _ in 0..MAX_NATIVE_HISTORY_PAGES {
        if !state.has_more {
            return;
        }
        let Some(before_sequence) = state.before_sequence else {
            state.append_entry(error_entry(
                "Older transcript history advertised no continuation cursor; native scrollback starts at the oldest safely loaded page.",
            ));
            state.has_more = false;
            return;
        };
        let page = match host
            .older_messages(&state.session_id, before_sequence)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                state.append_entry(error_entry(&format!(
                    "Older transcript history could not be restored into native scrollback: {error}"
                )));
                state.has_more = false;
                return;
            }
        };
        if page.has_more
            && (page.messages.is_empty() || page.before_sequence == Some(before_sequence))
        {
            state.append_entry(error_entry(
                "Older transcript history did not advance its continuation cursor; loading stopped safely.",
            ));
            state.has_more = false;
            return;
        }
        state.prepend_page(page);
    }
    if state.has_more {
        state.append_entry(native_history_limit_entry());
        state.has_more = false;
        state.before_sequence = None;
    }
}

fn native_history_limit_entry() -> TranscriptEntry {
    error_entry(&format!(
        "Native scrollback restored at most the newest {MAX_NATIVE_HISTORY_MESSAGES} transcript messages. Use session inspection for earlier durable history."
    ))
}

/// Apply one captured mouse event and report whether an older transcript page is needed.
pub(super) fn handle_mouse(state: &mut TuiState, mouse: MouseEvent) -> bool {
    if state.overlay.is_some() {
        return false;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            state.scroll_up_lines(MOUSE_SCROLL_LINES);
            state.at_transcript_top()
        }
        MouseEventKind::ScrollDown => {
            state.scroll_down_lines(MOUSE_SCROLL_LINES);
            false
        }
        _ => false,
    }
}

fn handle_key(
    state: &mut TuiState,
    key: KeyEvent,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
    screen_mode: ScreenMode,
) {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.interrupt_or_exit();
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
        KeyCode::PageUp if screen_mode == ScreenMode::Alternate => {
            state.page_up();
            request_older_page(state, host, event_tx);
        }
        KeyCode::PageDown if screen_mode == ScreenMode::Alternate => state.page_down(),
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
        Overlay::PlanExecutionChoice { plan, selected } => match key.code {
            KeyCode::Enter => {
                let plan = plan.clone();
                let strategy = match *selected {
                    0 => Some(PlanExecutionStrategy::Direct),
                    1 => Some(PlanExecutionStrategy::Goal {
                        max_iterations: DEFAULT_GOAL_ITERATIONS,
                    }),
                    _ => None,
                };
                state.overlay = None;
                if let Some(strategy) = strategy {
                    state.pending_plan_execution = Some(InteractivePlanExecutionRequest {
                        session_id: state.session_id.clone(),
                        plan_id: plan.id,
                        revision: plan.revision,
                        strategy,
                    });
                }
            }
            KeyCode::Up | KeyCode::BackTab => {
                *selected = if *selected == 0 { 2 } else { *selected - 1 };
            }
            KeyCode::Down | KeyCode::Tab => {
                *selected = (*selected + 1) % 3;
            }
            KeyCode::Home => *selected = 0,
            KeyCode::End => *selected = 2,
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
    state.older_page_failed = false;
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
            Overlay::PlanExecutionChoice { .. } | Overlay::QueuePaused => {}
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
            start_host_command(state, command, host, event_tx);
        }
        InteractiveCommand::Plan(command) => {
            handle_plan_command(state, command, host, event_tx);
        }
        InteractiveCommand::Invalid(message) => state.append_entry(error_entry(&message)),
        InteractiveCommand::Turn(prompt) => {
            let request = match state.run_request(prompt.clone()) {
                Ok(request) => request,
                Err(error) => {
                    state.append_entry(error_entry(&error));
                    return;
                }
            };
            state.append_entry(user_entry(&prompt, TranscriptKind::User));
            state.operation = Some(OperationKind::Run);
            state.started_at = Some(Instant::now());
            state.activity = Some("waiting for model".into());
            let control = RunControl::default();
            state.control = Some(control.clone());
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

fn start_host_command(
    state: &mut TuiState,
    command: RuntimeCommand,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    state.operation = Some(OperationKind::Command);
    state.started_at = Some(Instant::now());
    state.activity = Some(format!("running /{}", runtime_command_name(&command)));
    let session_id = state.session_id.clone();
    let sticky_skills = state.sticky_skills.clone();
    let control = RunControl::default();
    state.control = Some(control.clone());
    let task_tx = event_tx.clone();
    tokio::spawn(async move {
        let result = host
            .execute_command(
                command,
                &session_id,
                &sticky_skills,
                task_tx.clone(),
                control,
            )
            .await
            .map(OperationResult::Command);
        let _ = task_tx
            .send(HostEvent::OperationFinished(Box::new(result)))
            .await;
    });
}

fn handle_plan_command(
    state: &mut TuiState,
    command: PlanCommand,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    match command {
        PlanCommand::Toggle => {
            state.mode = match state.mode {
                InteractiveMode::Execute => InteractiveMode::Plan,
                InteractiveMode::Plan => InteractiveMode::Execute,
            };
            append_plan_status(state);
        }
        PlanCommand::On => {
            state.mode = InteractiveMode::Plan;
            append_plan_status(state);
        }
        PlanCommand::Off => {
            state.mode = InteractiveMode::Execute;
            append_plan_status(state);
        }
        PlanCommand::Status => append_plan_status(state),
        PlanCommand::New => {
            state.mode = InteractiveMode::Plan;
            state.selected_plan = None;
            append_plan_status(state);
        }
        PlanCommand::List => start_host_command(
            state,
            RuntimeCommand::Plan(PlanHostCommand::List),
            host,
            event_tx,
        ),
        PlanCommand::Use { plan_id } => start_host_command(
            state,
            RuntimeCommand::Plan(PlanHostCommand::Use { plan_id }),
            host,
            event_tx,
        ),
        PlanCommand::Show { plan_id } => {
            let plan_id =
                plan_id.or_else(|| state.selected_plan.as_ref().map(|plan| plan.id.clone()));
            let Some(plan_id) = plan_id else {
                state.append_entry(error_entry(
                    "No plan is selected. Use /plan show PLAN_ID or /plan use PLAN_ID.",
                ));
                return;
            };
            start_host_command(
                state,
                RuntimeCommand::Plan(PlanHostCommand::Show { plan_id }),
                host,
                event_tx,
            );
        }
        PlanCommand::Approve => {
            let Some(plan) = selected_draft(state, "approve") else {
                return;
            };
            start_host_command(
                state,
                RuntimeCommand::Plan(PlanHostCommand::Approve {
                    plan_id: plan.id,
                    revision: plan.revision,
                }),
                host,
                event_tx,
            );
        }
        PlanCommand::Discard => {
            let Some(plan) = selected_actionable_plan(state, "discard") else {
                return;
            };
            start_host_command(
                state,
                RuntimeCommand::Plan(PlanHostCommand::Discard {
                    plan_id: plan.id,
                    revision: plan.revision,
                }),
                host,
                event_tx,
            );
        }
        PlanCommand::Execute { strategy } => {
            let Some(plan) = selected_approved_plan(state) else {
                return;
            };
            if let Some(strategy) = strategy {
                start_plan_execution(
                    state,
                    InteractivePlanExecutionRequest {
                        session_id: state.session_id.clone(),
                        plan_id: plan.id,
                        revision: plan.revision,
                        strategy,
                    },
                    host,
                    event_tx,
                );
            } else {
                state.overlay = Some(Overlay::PlanExecutionChoice { plan, selected: 0 });
            }
        }
    }
}

fn selected_draft(state: &mut TuiState, action: &str) -> Option<PlanRecord> {
    let Some(plan) = state.selected_plan.clone() else {
        state.append_entry(error_entry(&format!(
            "No plan is selected. Use /plan use PLAN_ID before /plan {action}."
        )));
        return None;
    };
    if plan.status != PlanStatus::Draft {
        state.append_entry(error_entry(&format!(
            "Plan {} is {} and cannot be used for /plan {action}.",
            short_plan_id(&plan.id),
            plan_status_label(plan.status)
        )));
        return None;
    }
    Some(plan)
}

fn selected_actionable_plan(state: &mut TuiState, action: &str) -> Option<PlanRecord> {
    let Some(plan) = state.selected_plan.clone() else {
        state.append_entry(error_entry(&format!(
            "No plan is selected. Use /plan use PLAN_ID before /plan {action}."
        )));
        return None;
    };
    if !matches!(plan.status, PlanStatus::Draft | PlanStatus::Approved) {
        state.append_entry(error_entry(&format!(
            "Plan {} is {} and cannot be used for /plan {action}.",
            short_plan_id(&plan.id),
            plan_status_label(plan.status)
        )));
        return None;
    }
    Some(plan)
}

fn selected_approved_plan(state: &mut TuiState) -> Option<PlanRecord> {
    let Some(plan) = state.selected_plan.clone() else {
        state.append_entry(error_entry(
            "No plan is selected. Use /plan use PLAN_ID before /plan execute.",
        ));
        return None;
    };
    if plan.status != PlanStatus::Approved {
        state.append_entry(error_entry(&format!(
            "Plan {} is {}. Approve it before execution.",
            short_plan_id(&plan.id),
            plan_status_label(plan.status)
        )));
        return None;
    }
    Some(plan)
}

fn start_plan_execution(
    state: &mut TuiState,
    request: InteractivePlanExecutionRequest,
    host: Arc<dyn InteractiveHost>,
    event_tx: mpsc::Sender<HostEvent>,
) {
    state.operation = Some(OperationKind::Run);
    state.started_at = Some(Instant::now());
    state.activity = Some(match request.strategy {
        PlanExecutionStrategy::Direct => "executing approved plan".into(),
        PlanExecutionStrategy::Goal { max_iterations } => {
            format!("running approved plan in Goal Mode ({max_iterations} iterations)")
        }
    });
    let control = RunControl::default();
    state.control = Some(control.clone());
    let task_tx = event_tx.clone();
    tokio::spawn(async move {
        let result = host
            .run_plan_execution(request, task_tx.clone(), control)
            .await
            .map(OperationResult::PlanExecution);
        let _ = task_tx
            .send(HostEvent::OperationFinished(Box::new(result)))
            .await;
    });
}

fn append_plan_status(state: &mut TuiState) {
    state.append_entry(TranscriptEntry {
        sequence: None,
        kind: TranscriptKind::Command,
        document: plan_status_document(state.mode, state.selected_plan.as_ref()),
        temporary: false,
    });
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
                        plan_selection: PlanSelectionUpdate::Unchanged,
                        continue_queue: true,
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
                Ok(page) => {
                    state.older_page_failed = false;
                    state.prepend_page(page);
                }
                Err(error) => {
                    state.older_page_failed = true;
                    state.append_entry(error_entry(&format!(
                        "Older transcript messages could not be loaded: {error}"
                    )));
                }
            }
        }
        HostEvent::OperationFinished(result) => {
            if matches!(state.overlay, Some(Overlay::Prompt { .. }))
                && let Some(Overlay::Prompt { request, .. }) = state.overlay.take()
            {
                let _ = request.response.send(PromptResponse::Cancelled);
            }
            let result = *result;
            state.operation = None;
            state.control = None;
            state.activity = None;
            state.started_at = None;
            let successful = match result {
                Ok(OperationResult::Command(result)) => apply_command_result(state, result),
                Ok(OperationResult::Run(HostRunResult {
                    outcome: AgentRunOutcome::Completed { result },
                    footer,
                    plan_selection,
                })) => {
                    let selection_valid = apply_plan_selection(state, plan_selection);
                    finalize_assistant(state, &result.output);
                    state.footer = footer;
                    selection_valid
                }
                Ok(OperationResult::Run(HostRunResult {
                    outcome: AgentRunOutcome::Cancelled { .. },
                    footer,
                    plan_selection,
                })) => {
                    apply_plan_selection(state, plan_selection);
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
                Ok(OperationResult::PlanExecution(result)) => {
                    let session_matches = result.plan.session_id == state.session_id;
                    if !session_matches {
                        state.append_entry(error_entry(
                            "The host returned consumed-plan evidence for a different session.",
                        ));
                    }
                    let selection_valid = apply_plan_selection(state, result.plan_selection);
                    state.footer = result.footer;
                    if !result.document.is_empty() {
                        state.append_entry(TranscriptEntry {
                            sequence: None,
                            kind: TranscriptKind::Command,
                            document: result.document,
                            temporary: false,
                        });
                    }
                    match result.outcome {
                        HostPlanExecutionOutcome::CancelledBeforeStart => {
                            state.append_entry(TranscriptEntry {
                                sequence: None,
                                kind: TranscriptKind::Command,
                                document: PresentationDocument::from_block(
                                    PresentationBlock::Card {
                                        title: "Plan execution cancelled".into(),
                                        tone: PresentationTone::Warning,
                                        body: vec![PresentationBlock::Text(
                                            "The plan was not consumed. Plan mode and the current selection remain active.".into(),
                                        )],
                                    },
                                ),
                                temporary: false,
                            });
                            false
                        }
                        HostPlanExecutionOutcome::FailedBeforeConsumption(error) => {
                            state.append_entry(error_entry(&format!(
                                "Plan execution failed before consumption: {error}"
                            )));
                            false
                        }
                        HostPlanExecutionOutcome::Completed => {
                            state.mode = InteractiveMode::Execute;
                            state.selected_plan = None;
                            session_matches && selection_valid
                        }
                        HostPlanExecutionOutcome::CancelledAfterConsumption => {
                            state.mode = InteractiveMode::Execute;
                            state.selected_plan = None;
                            state.append_entry(TranscriptEntry {
                                sequence: None,
                                kind: TranscriptKind::Command,
                                document: PresentationDocument::from_block(
                                    PresentationBlock::Card {
                                        title: "Plan execution cancelled".into(),
                                        tone: PresentationTone::Warning,
                                        body: vec![PresentationBlock::Text(
                                            "The plan was already consumed. Inspect /plans before starting new work.".into(),
                                        )],
                                    },
                                ),
                                temporary: false,
                            });
                            false
                        }
                        HostPlanExecutionOutcome::FailedAfterConsumption(error) => {
                            state.mode = InteractiveMode::Execute;
                            state.selected_plan = None;
                            state.append_entry(error_entry(&format!(
                                "Plan execution failed after consumption: {error}"
                            )));
                            false
                        }
                        HostPlanExecutionOutcome::ConsumedOutcomeUnknown(error) => {
                            state.mode = InteractiveMode::Execute;
                            state.selected_plan = None;
                            state.append_entry(error_entry(&format!(
                                "The plan was consumed, but its execution outcome is unknown: {error}. Inspect /plans and linked run or Goal evidence before retrying."
                            )));
                            false
                        }
                        HostPlanExecutionOutcome::OutcomeUnknown(error) => {
                            state.mode = InteractiveMode::Execute;
                            state.selected_plan = None;
                            state.append_entry(error_entry(&format!(
                                "Plan execution outcome is unknown: {error}. Inspect /plans before retrying."
                            )));
                            false
                        }
                    }
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
            finalize_intermediate_assistant_output(state);
            state.activity = Some(format!("running {}", call.name));
            state
                .active_calls
                .insert(call.call_id.clone(), call.clone());
            return;
        }
        RunEvent::PlanWritten { plan } => {
            apply_plan_selection(state, PlanSelectionUpdate::Set(Box::new(plan.clone())));
        }
        _ => {}
    }

    let (kind, call) = match &event {
        RunEvent::ToolCompleted { result, .. } => {
            finalize_intermediate_assistant_output(state);
            (
                TranscriptKind::Tool,
                state.active_calls.remove(&result.call_id),
            )
        }
        RunEvent::ToolCancelled { call, .. } => {
            finalize_intermediate_assistant_output(state);
            state.active_calls.remove(&call.call_id);
            (TranscriptKind::Tool, None)
        }
        RunEvent::Error { .. } => (TranscriptKind::Error, None),
        RunEvent::PlanWritten { .. } => (TranscriptKind::Command, None),
        RunEvent::Provider {
            event: ProviderEvent::ReasoningSummary { .. },
        } => (TranscriptKind::Assistant, None),
        RunEvent::Provider {
            event: ProviderEvent::Usage { .. },
        } => (TranscriptKind::Command, None),
        RunEvent::Provider { .. } => return,
        RunEvent::Phase { .. } | RunEvent::ToolStarted { .. } => return,
    };
    let source = TranscriptRenderSource::RunEvent {
        event: Box::new(event),
        call,
    };
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

pub(super) fn apply_command_result(state: &mut TuiState, result: HostCommandResult) -> bool {
    let continue_queue = result.continue_queue;
    if result.clear_transcript {
        state.transcript.clear();
        state.transcript_sources.clear();
        state.transcript_epoch = state.transcript_epoch.wrapping_add(1);
        state.native_history_pages_loaded = 0;
        state.older_page_failed = false;
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
        state.selected_plan = None;
        let (transcript, transcript_sources) =
            transcript_from_messages(page.messages, &state.preferences);
        state.transcript = transcript;
        state.transcript_sources = transcript_sources;
        state.transcript_epoch = state.transcript_epoch.wrapping_add(1);
        state.native_history_pages_loaded = 0;
        state.older_page_failed = false;
        state.before_sequence = page.before_sequence;
        state.has_more = page.has_more;
        state.end();
    }
    if let Some(preferences) = result.preferences {
        state.set_preferences(preferences);
    }
    if let Some(completions) = result.completions {
        state.set_completions(completions);
    }
    if let Some(sticky_skills) = result.sticky_skills {
        state.sticky_skills = sticky_skills;
    }
    if let Some(footer) = result.footer {
        state.footer = footer;
    }
    apply_plan_selection(state, result.plan_selection) && continue_queue
}

fn apply_plan_selection(state: &mut TuiState, update: PlanSelectionUpdate) -> bool {
    if let Err(error) = state.apply_plan_selection(update) {
        state.append_entry(error_entry(&error));
        false
    } else {
        true
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

fn finalize_intermediate_assistant_output(state: &mut TuiState) {
    let old_line_count = if state.scroll_from_bottom > 0 {
        transcript_lines(state, state.transcript_width).len()
    } else {
        0
    };
    let render_as_markdown =
        state.preferences.stream_mode != colossus_contracts::StreamDisplayMode::Raw;
    let mut changed = false;
    for entry in &mut state.transcript {
        if !entry.temporary || entry.kind != TranscriptKind::Assistant {
            continue;
        }
        if render_as_markdown && entry.document.blocks.len() == 1 {
            let markdown = match &entry.document.blocks[0] {
                PresentationBlock::Text(text) => Some(text.clone()),
                _ => None,
            };
            if let Some(markdown) = markdown {
                entry.document.blocks[0] = PresentationBlock::Markdown(markdown);
            }
        }
        entry.temporary = false;
        changed = true;
    }
    if changed && state.scroll_from_bottom > 0 {
        state.preserve_scroll_after_line_change(old_line_count);
    }
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
