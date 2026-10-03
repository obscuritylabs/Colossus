use super::*;
use colossus_contracts::ThemeColor;

pub(super) const FOOTER_HEIGHT: u16 = 1;
const CHROME_BACKGROUND: ThemeColor = ThemeColor {
    red: 27,
    green: 30,
    blue: 34,
};

pub(super) fn render_footer(
    frame: &mut Frame<'_>,
    state: &TuiState,
    area: Rect,
    show_location: bool,
) {
    if area.height == 0 || area.width < 2 {
        return;
    }
    let row = Rect::new(area.x + 1, area.y, area.width - 2, 1);
    let width = usize::from(row.width);
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let band = chrome_band_style(&palette);
    frame.render_widget(
        Block::default().style(band),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let metadata = band;
    let status = if state.docked_decision_active()
        || matches!(state.overlay, Some(Overlay::Prompt { .. }))
    {
        "waiting".to_owned()
    } else if state.queue_paused && !state.queue.is_empty() {
        "queue paused".to_owned()
    } else if state.is_busy() {
        "working".to_owned()
    } else {
        truncate_width(&sanitize_approval_field(&state.footer.status), 12)
    };
    let approval = sanitize_approval_field(&state.footer.approval_mode);
    let mut approval_text = format!(" · approval {approval}");
    let mut badge = if state.security_posture.is_hardened() {
        String::new()
    } else {
        format!(" ⚠ Security: {} ", state.security_posture.finding_count())
    };
    if UnicodeWidthStr::width(status.as_str())
        + UnicodeWidthStr::width(approval_text.as_str())
        + UnicodeWidthStr::width(badge.as_str())
        + 2
        > width
    {
        if !badge.is_empty() {
            badge = format!(" ⚠ {} ", state.security_posture.finding_count());
        }
        approval_text = format!(" · {approval}");
    }
    let badge_width = UnicodeWidthStr::width(badge.as_str());
    let content_width = width.saturating_sub(badge_width + usize::from(!badge.is_empty()) * 2);
    let mut used =
        UnicodeWidthStr::width(status.as_str()) + UnicodeWidthStr::width(approval_text.as_str());
    let mode = format!(" {} ", state.mode.as_str());
    let mode = if used + UnicodeWidthStr::width(mode.as_str()) + 2 <= content_width {
        mode
    } else {
        String::new()
    };
    used += UnicodeWidthStr::width(mode.as_str()) + usize::from(!mode.is_empty()) * 2;
    let status_style = if matches!(status.as_str(), "waiting" | "queue paused" | "error") {
        chrome_text_style(palette.warning_style(), CHROME_BACKGROUND)
    } else {
        band
    }
    .add_modifier(Modifier::BOLD);
    let mode_gap = if mode.is_empty() { "" } else { "  " };
    let mut spans = vec![
        Span::styled(mode, chrome_chip_style(palette.user_style())),
        Span::styled(mode_gap, band),
        Span::styled(status, status_style),
        Span::styled(approval_text, metadata.add_modifier(Modifier::BOLD)),
    ];
    if !state.queue.is_empty() {
        append_footer_segment(
            &mut spans,
            &mut used,
            content_width,
            format!("{} queued", state.queue.len()),
            metadata,
        );
    }
    if let Some(plan) = &state.selected_plan {
        append_footer_segment(
            &mut spans,
            &mut used,
            content_width,
            format!(
                "plan {} r{} {}",
                short_plan_id(&plan.id),
                plan.revision,
                plan_status_label(plan.status)
            ),
            metadata,
        );
    }
    let context = state
        .footer
        .context
        .map(|(used, maximum)| format!("ctx {used}/{maximum}"));
    let reserved = context
        .as_ref()
        .map_or(0, |context| UnicodeWidthStr::width(context.as_str()) + 3);
    let route_width = content_width.saturating_sub(used + reserved + 3);
    if route_width >= 16 {
        append_footer_segment(
            &mut spans,
            &mut used,
            content_width,
            truncate_width_with_ellipsis(
                &sanitize_approval_field(&state.footer.route),
                route_width,
            ),
            metadata,
        );
    }
    if let Some(context) = context {
        append_footer_segment(&mut spans, &mut used, content_width, context, metadata);
    }
    if show_location {
        let location = if state.welcome_visible {
            welcome_workspace(&state.workspace)
        } else {
            format!(
                "session {}",
                state.session_id.chars().take(8).collect::<String>()
            )
        };
        append_footer_segment(&mut spans, &mut used, content_width, location, metadata);
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(
            row.x,
            row.y,
            u16::try_from(content_width).unwrap_or(row.width),
            1,
        ),
    );
    if !badge.is_empty() {
        let badge_width = u16::try_from(badge_width)
            .unwrap_or(row.width)
            .min(row.width);
        let badge_area = Rect::new(row.right() - badge_width, row.y, badge_width, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(
                badge,
                chrome_chip_style(palette.warning_style()),
            )),
            badge_area,
        );
    }
}

fn append_footer_segment(
    spans: &mut Vec<Span<'static>>,
    used: &mut usize,
    width: usize,
    text: String,
    style: Style,
) {
    if text.is_empty() {
        return;
    }
    let separator = if *used == 0 { "" } else { " · " };
    let required = UnicodeWidthStr::width(separator) + UnicodeWidthStr::width(text.as_str());
    if used.saturating_add(required) <= width {
        *used += required;
        spans.push(Span::styled(format!("{separator}{text}"), style));
    }
}

/// A neutral surface keeps themed accents from coloring every metadata field.
pub(super) fn chrome_band_style(palette: &TerminalPalette) -> Style {
    chrome_text_style(palette.assistant_style(), CHROME_BACKGROUND)
}

/// Keep readable theme colors; use light ink when a fixed surface needs more contrast.
pub(super) fn chrome_text_style(accent: ThemeTextStyle, background: ThemeColor) -> Style {
    let fallback = ThemeColor {
        red: 230,
        green: 237,
        blue: 243,
    };
    let foreground = accent.foreground.unwrap_or(fallback);
    let ink = relative_luminance(foreground);
    let surface = relative_luminance(background);
    let foreground = if (ink.max(surface) + 0.05) / (ink.min(surface) + 0.05) >= 4.5 {
        foreground
    } else {
        fallback
    };
    ratatui_style(accent)
        .remove_modifier(Modifier::DIM)
        .fg(Color::Rgb(
            foreground.red,
            foreground.green,
            foreground.blue,
        ))
        .bg(Color::Rgb(
            background.red,
            background.green,
            background.blue,
        ))
}

fn relative_luminance(color: ThemeColor) -> f64 {
    let linear = |channel| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red) + 0.7152 * linear(color.green) + 0.0722 * linear(color.blue)
}

pub(super) fn chrome_chip_style(accent: ThemeTextStyle) -> Style {
    let color = ratatui_style(accent).fg.unwrap_or(Color::White);
    let background = match color {
        Color::Rgb(red, green, blue) => Color::Rgb(
            soft_chip_channel(red),
            soft_chip_channel(green),
            soft_chip_channel(blue),
        ),
        _ => Color::Gray,
    };
    Style::default()
        .fg(Color::Black)
        .bg(background)
        .add_modifier(Modifier::BOLD)
}

const fn soft_chip_channel(value: u8) -> u8 {
    ((value as u16 + 510) / 3) as u8
}
