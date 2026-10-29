//! Centered launch branding with a bounded, upright dot-matrix yaw spin.

use super::*;
use tachyonfx::{ColorSpace, Effect, Interpolation, SimpleRng, fx, pattern::SweepPattern};

mod spin;

pub(super) const MIN_SPLASH_WIDTH: u16 = 40;
pub(super) const MIN_SPLASH_HEIGHT: u16 = 24;
const CONTENT_WIDTH: u16 = 36;
const CONTENT_HEIGHT: u16 = 22;
const INTRO_DURATION: Duration = Duration::from_millis(1_500);
const LABS_DURATION: u32 = 250;
const COLOSSUS_DELAY: u128 = 350;
const COLOSSUS_DURATION: u32 = 700;
const HINT: &str = "Press any key to continue";
const IDENTITY: &str = "OBSCURITY LABS // COLOSSUS";
const WORDMARK: [&str; 3] = [
    "┌─┐ ┌─┐ │   ┌─┐ ┌─┐ ┌─┐ │ │ ┌─┐",
    "│   │ │ │   │ │ └─┐ └─┐ │ │ └─┐",
    "└─┘ └─┘ └── └─┘ └─┘ └─┘ └─┘ └─┘",
];

/// Initial or duplicate size reports and passive pointer motion are not user skips.
pub(super) fn interrupts(input: &Event, last_size: &mut Option<(u16, u16)>) -> bool {
    match input {
        Event::Key(key) => key.kind == KeyEventKind::Press,
        Event::Mouse(mouse) => !matches!(mouse.kind, MouseEventKind::Moved | MouseEventKind::Up(_)),
        Event::Paste(_) => true,
        Event::Resize(width, height) => {
            let next = (*width, *height);
            let changed = last_size.is_some_and(|previous| previous != next);
            *last_size = Some(next);
            changed
        }
        _ => false,
    }
}

/// Process-local eligibility; the caller additionally checks fresh-session state.
pub(super) fn enabled(
    mode: ScreenMode,
    preferences: &TerminalPreferences,
    no_color: bool,
    setting: Option<&str>,
) -> bool {
    mode == ScreenMode::Alternate
        && preferences.theme != colossus_contracts::ThemeName::Mono
        && !preferences
            .custom_theme
            .as_ref()
            .is_some_and(|theme| theme.base == colossus_contracts::ThemeName::Mono)
        && !no_color
        && !setting.is_some_and(|value| {
            let value = value.trim();
            value == "0" || value.eq_ignore_ascii_case("off") || value.eq_ignore_ascii_case("false")
        })
}

pub(super) struct StartupAnimation {
    started: Option<Instant>,
    last_frame: Option<Instant>,
    labs: Effect,
    colossus: Effect,
    mark: spin::DotMark,
}

impl StartupAnimation {
    pub(super) fn new() -> Self {
        Self {
            started: None,
            last_frame: None,
            labs: fx::parallel(&[
                fx::coalesce((LABS_DURATION, Interpolation::SineOut)).with_rng(SimpleRng::new(17)),
                fade_from_background(LABS_DURATION),
            ]),
            colossus: fx::parallel(&[
                fx::coalesce((COLOSSUS_DURATION, Interpolation::QuadOut))
                    .with_pattern(SweepPattern::left_to_right(6))
                    .with_rng(SimpleRng::new(31)),
                fade_from_background(COLOSSUS_DURATION),
            ]),
            mark: spin::DotMark::new(),
        }
    }

    /// Arm on the first actual frame and expire by wall clock, even after a pause.
    pub(super) fn prepare(&mut self, now: Instant) -> bool {
        let started = *self.started.get_or_insert(now);
        now.saturating_duration_since(started) < INTRO_DURATION
    }

    /// Render the complete splash inside `area`; return whether its one-shot is active.
    /// Calls after expiry render the exact settled front face without restarting it.
    pub(super) fn render(
        &mut self,
        buffer: &mut Buffer,
        area: Rect,
        preferences: &TerminalPreferences,
        now: Instant,
    ) -> bool {
        let active = self.prepare(now);
        let started = self.started.unwrap_or(now);
        let previous = self.last_frame.unwrap_or(started);
        let now = now.max(previous);
        self.last_frame = Some(now);
        let elapsed = now.saturating_duration_since(started).as_millis();
        let prior = previous.saturating_duration_since(started).as_millis();
        let area = area.intersection(buffer.area);
        Clear.render(area, buffer);
        let palette = TerminalPalette::for_preferences(preferences);
        let primary = ratatui_style(palette.user_style()).add_modifier(Modifier::BOLD);
        let wordmark = ratatui_style(palette.assistant_style()).add_modifier(Modifier::BOLD);
        let meta = ratatui_style(palette.meta_style());
        if area.is_empty() {
            return false;
        }
        if area.width < MIN_SPLASH_WIDTH || area.height < MIN_SPLASH_HEIGHT {
            draw_centered(buffer, area, area.y + area.height / 2, "COLOSSUS", primary);
            draw_centered(buffer, area, area.bottom() - 1, HINT, meta);
            return false;
        }

        let content = content_area(area);
        let labs = labs_area(content);
        let colossus = colossus_area(content);
        self.mark
            .render(buffer, Rect::new(labs.x, labs.y, 24, 10), elapsed, primary);
        draw_centered(buffer, area, content.y + 11, "OBSCURITY LABS", meta);
        for (row, text) in WORDMARK.iter().enumerate() {
            draw_centered(buffer, area, content.y + 13 + row as u16, text, wordmark);
        }
        draw_centered(buffer, area, content.y + 17, "COLOSSUS", primary);
        draw_centered(buffer, area, content.y + 18, "AI agent runtime", meta);
        draw_centered(buffer, area, content.y + 21, HINT, meta);
        draw_centered(buffer, area, area.bottom() - 1, IDENTITY, meta);
        if !active {
            return false;
        }
        if !self.labs.done() {
            self.labs.set_area(labs);
            self.labs
                .process(effect_delta(prior, elapsed, 0, LABS_DURATION), buffer, labs);
        }
        if elapsed <= COLOSSUS_DELAY {
            for position in colossus.positions() {
                if let Some(cell) = buffer.cell_mut(position) {
                    cell.set_char(' ');
                }
            }
        } else if !self.colossus.done() {
            self.colossus.set_area(colossus);
            self.colossus.process(
                effect_delta(prior, elapsed, COLOSSUS_DELAY, COLOSSUS_DURATION),
                buffer,
                colossus,
            );
        }
        true
    }
}

fn content_area(area: Rect) -> Rect {
    Rect::new(
        area.x + (area.width - CONTENT_WIDTH) / 2,
        area.y + (area.height - CONTENT_HEIGHT) / 2,
        CONTENT_WIDTH,
        CONTENT_HEIGHT,
    )
}

fn labs_area(content: Rect) -> Rect {
    Rect::new(content.x + 6, content.y, 24, 12)
}

fn colossus_area(content: Rect) -> Rect {
    Rect::new(content.x, content.y + 13, CONTENT_WIDTH, 6)
}

fn draw_centered(buffer: &mut Buffer, area: Rect, y: u16, text: &str, style: Style) {
    if y < area.bottom() {
        Paragraph::new(truncate_width(text, usize::from(area.width)))
            .alignment(Alignment::Center)
            .style(style)
            .render(Rect::new(area.x, y, area.width, 1), buffer);
    }
}

fn effect_delta(prior: u128, elapsed: u128, delay: u128, duration: u32) -> tachyonfx::Duration {
    let duration = u128::from(duration);
    let prior = prior.saturating_sub(delay).min(duration);
    let elapsed = elapsed.saturating_sub(delay).min(duration);
    tachyonfx::Duration::from_millis(
        u32::try_from(elapsed.saturating_sub(prior)).unwrap_or_default(),
    )
}

fn fade_from_background(duration: u32) -> Effect {
    fx::effect_fn(
        (),
        (duration, Interpolation::SineOut),
        |_, context, cells| {
            let alpha = context.alpha();
            cells.for_each_cell(|_, cell| {
                // Native Reset backgrounds have no known RGB value, especially on
                // light terminals. Preserve palette ink and use density reveal there.
                if cell.bg != Color::Reset {
                    cell.fg = ColorSpace::Rgb.lerp(&cell.bg, &cell.fg, alpha);
                }
            });
        },
    )
}

#[cfg(test)]
mod tests;
