use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProviderRecoveryAction {
    Retry,
    Dismiss,
}

pub(super) struct ProviderRecovery {
    session_id: String,
    ready: bool,
    focused: Option<ProviderRecoveryAction>,
    retry_area: Rect,
    dismiss_area: Rect,
}

impl ProviderRecovery {
    pub(super) fn clear_hit_areas(&mut self) {
        self.retry_area = Rect::default();
        self.dismiss_area = Rect::default();
    }
}

pub(super) enum ProviderRecoveryKey {
    Ignored,
    Handled,
    Action(ProviderRecoveryAction),
}

/// Offer ordinary conversation input only after a categorical, known provider failure.
/// Generic failures without an HTTP response can also represent policy or binding errors.
pub(super) fn observe_provider_failure(state: &mut TuiState, envelope: &RunEventEnvelope) {
    state.provider_recovery = match &envelope.event {
        RunEvent::Error {
            code, http_status, ..
        } if state.operation == Some(OperationKind::Run)
            && envelope.session_id == state.session_id
            && (matches!(
                code.as_str(),
                "provider.context_limit_exceeded"
                    | "provider.request_too_large"
                    | "provider.output_limit_exceeded"
                    | "provider.rate_limited"
                    | "provider.quota_exceeded"
                    | "provider.content_filtered"
                    | "provider.unavailable"
                    | "provider.temporarily_unavailable"
                    | "provider.invalid_tool_arguments"
            ) || (code == "provider.failed" && http_status.is_some())) =>
        {
            Some(ProviderRecovery {
                session_id: envelope.session_id.clone(),
                ready: false,
                focused: None,
                retry_area: Rect::default(),
                dismiss_area: Rect::default(),
            })
        }
        _ => None,
    };
}

pub(super) fn finish_provider_failure(state: &mut TuiState, failed_run: bool) {
    if failed_run {
        if let Some(recovery) = &mut state.provider_recovery {
            recovery.ready = true;
            // A provider error proves this input reached the run. Its images are
            // already canonical; a Continue submission must not attach them twice.
            state.pending_images.clear();
        }
    } else {
        state.provider_recovery = None;
    }
}

pub(super) fn provider_recovery_visible(state: &TuiState) -> bool {
    !state.is_busy()
        && state.overlay.is_none()
        && state
            .provider_recovery
            .as_ref()
            .is_some_and(|recovery| recovery.ready && recovery.session_id == state.session_id)
}

pub(super) fn provider_recovery_focused(state: &TuiState) -> bool {
    provider_recovery_visible(state)
        && state
            .provider_recovery
            .as_ref()
            .is_some_and(|recovery| recovery.focused.is_some())
}

pub(super) fn provider_recovery_height(state: &TuiState) -> u16 {
    if provider_recovery_visible(state) {
        3
    } else {
        0
    }
}

pub(super) fn provider_recovery_key(state: &mut TuiState, key: KeyEvent) -> ProviderRecoveryKey {
    if !provider_recovery_visible(state) {
        return ProviderRecoveryKey::Ignored;
    }
    let has_completion = !state.completion_menu_candidates().is_empty();
    let recovery = state.provider_recovery.as_mut().expect("visible recovery");
    match key.code {
        KeyCode::Tab if recovery.focused.is_some() || !has_completion => {
            recovery.focused = match recovery.focused {
                None => Some(ProviderRecoveryAction::Retry),
                Some(ProviderRecoveryAction::Retry) => Some(ProviderRecoveryAction::Dismiss),
                Some(ProviderRecoveryAction::Dismiss) => None,
            };
        }
        KeyCode::BackTab if recovery.focused.is_some() || !has_completion => {
            recovery.focused = match recovery.focused {
                None => Some(ProviderRecoveryAction::Dismiss),
                Some(ProviderRecoveryAction::Dismiss) => Some(ProviderRecoveryAction::Retry),
                Some(ProviderRecoveryAction::Retry) => None,
            };
        }
        KeyCode::Left | KeyCode::Right if recovery.focused.is_some() => {
            recovery.focused = match recovery.focused {
                Some(ProviderRecoveryAction::Retry) => Some(ProviderRecoveryAction::Dismiss),
                _ => Some(ProviderRecoveryAction::Retry),
            };
        }
        KeyCode::Enter if recovery.focused.is_some() => {
            return ProviderRecoveryKey::Action(recovery.focused.expect("focused action"));
        }
        KeyCode::Esc if recovery.focused.is_some() => recovery.focused = None,
        _ => {
            recovery.focused = None;
            return ProviderRecoveryKey::Ignored;
        }
    }
    ProviderRecoveryKey::Handled
}

pub(super) fn provider_recovery_mouse(
    state: &TuiState,
    mouse: MouseEvent,
) -> Option<ProviderRecoveryAction> {
    if !provider_recovery_visible(state)
        || state.selecting_text
        || mouse.kind != MouseEventKind::Down(crossterm::event::MouseButton::Left)
    {
        return None;
    }
    let recovery = state.provider_recovery.as_ref()?;
    let position = Position::new(mouse.column, mouse.row);
    if recovery.retry_area.contains(position) {
        Some(ProviderRecoveryAction::Retry)
    } else if recovery.dismiss_area.contains(position) {
        Some(ProviderRecoveryAction::Dismiss)
    } else {
        None
    }
}

pub(super) fn render_provider_recovery(frame: &mut Frame<'_>, state: &mut TuiState, area: Rect) {
    let Some(recovery) = state.provider_recovery.as_mut() else {
        return;
    };
    recovery.clear_hit_areas();
    if area.height < 3 {
        return;
    }
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let metadata = ratatui_style(palette.meta_style());
    let button = ratatui_style(palette.user_style()).add_modifier(Modifier::BOLD);
    let action_style = |action| {
        if recovery.focused == Some(action) {
            button.add_modifier(Modifier::REVERSED)
        } else {
            button
        }
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                " Provider request failed.",
                ratatui_style(palette.warning_style()),
            )),
            Line::from(Span::styled(
                " Retry continues this conversation.",
                metadata,
            )),
            Line::from(vec![
                Span::raw(" "),
                Span::styled("[ Retry ]", action_style(ProviderRecoveryAction::Retry)),
                Span::raw("  "),
                Span::styled("[ Dismiss ]", action_style(ProviderRecoveryAction::Dismiss)),
                Span::styled("  Tab to choose", metadata),
            ]),
        ]),
        area,
    );
    recovery.retry_area = Rect::new(area.x + 1, area.y + 2, 9, 1);
    recovery.dismiss_area = Rect::new(area.x + 12, area.y + 2, 11, 1);
}
