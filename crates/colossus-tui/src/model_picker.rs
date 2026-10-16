use super::*;

fn matching_choices(request: &InteractivePrompt, query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    request
        .choices
        .iter()
        .enumerate()
        .filter_map(|(index, choice)| choice.to_lowercase().contains(&query).then_some(index))
        .collect()
}

pub(super) fn handle_model_picker_key(state: &mut TuiState, key: KeyEvent) {
    let Some(Overlay::Prompt {
        request,
        input,
        selected,
        ..
    }) = state.overlay.as_mut()
    else {
        return;
    };
    if key.code == KeyCode::Esc {
        if input.is_empty() {
            state.cancel_overlay();
        } else {
            input.clear();
        }
        return;
    }
    if key.code == KeyCode::Enter {
        let answer = selected
            .and_then(|index| request.choices.get(index))
            .cloned();
        if let Some(answer) = answer
            && let Some(Overlay::Prompt { request, .. }) = state.overlay.take()
        {
            let _ = request.response.send(PromptResponse::Answer(answer));
        }
        return;
    }
    let indices = matching_choices(request, input);
    let position = selected
        .and_then(|index| indices.iter().position(|value| *value == index))
        .unwrap_or(0);
    match key.code {
        KeyCode::Up | KeyCode::BackTab => {
            *selected = indices
                .get(
                    position
                        .checked_sub(1)
                        .unwrap_or(indices.len().saturating_sub(1)),
                )
                .copied()
        }
        KeyCode::Down | KeyCode::Tab => {
            *selected = indices
                .get(if position + 1 < indices.len() {
                    position + 1
                } else {
                    0
                })
                .copied()
        }
        KeyCode::Home => *selected = indices.first().copied(),
        KeyCode::End => *selected = indices.last().copied(),
        KeyCode::Backspace => {
            input.pop();
            *selected = matching_choices(request, input).first().copied();
        }
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            if input.len() < 128 && (character != '/' || !input.is_empty()) {
                input.push(character);
            }
            *selected = matching_choices(request, input).first().copied();
        }
        _ => {}
    }
}

pub(super) fn render_model_picker(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    let Some(Overlay::Prompt {
        request,
        input,
        selected,
        ..
    }) = state.overlay.as_ref()
    else {
        return;
    };
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let indices = matching_choices(request, input);
    let areas = render_browser_shell(
        frame,
        &palette,
        area,
        60,
        BrowserHeader {
            title: &request.title,
            count: format!("{} choices", indices.len()),
            search_hint: "Type to search",
            query: input,
            search_active: true,
        },
    );
    let focus = selected
        .and_then(|index| indices.iter().position(|value| *value == index))
        .unwrap_or(0);
    let height = usize::from(areas.list.height).max(1);
    let start = focus
        .saturating_sub(height / 2)
        .min(indices.len().saturating_sub(height));
    let rows = indices
        .iter()
        .skip(start)
        .take(height)
        .map(|index| {
            let is_selected = selected == &Some(*index);
            Line::from(Span::styled(
                format!(
                    "{} {}",
                    if is_selected { "›" } else { " " },
                    sanitize_approval_field(&request.choices[*index])
                ),
                ratatui_style(if is_selected {
                    palette.user_style()
                } else {
                    palette.meta_style()
                })
                .add_modifier(if is_selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(if rows.is_empty() {
            vec![Line::from("No matching choices")]
        } else {
            rows
        }),
        areas.list,
    );
    if let Some(preview) = areas.preview {
        let text = selected
            .and_then(|index| request.choices.get(index))
            .map_or("Choose a model or reasoning level.", String::as_str);
        frame.render_widget(Paragraph::new(format!("{}\n\nApplies to the next turn in this conversation. Esc keeps your current choice.", sanitize_approval_field(text))).wrap(Wrap {trim: false}), preview);
    }
    render_browser_controls(
        frame,
        &palette,
        areas.controls,
        &[
            ("↑/↓", "Select"),
            ("Enter", "Apply"),
            ("Esc", "Back"),
            ("Type", "Search"),
        ],
    );
}
