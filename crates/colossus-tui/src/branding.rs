use super::*;

const OBSCURITY_MARK: &str = include_str!("../assets/obscurity-mark.braille");
const MARK_WIDTH: u16 = 24;
const MARK_HEIGHT: u16 = 10;

/// The encoded brand mark uses ordinary terminal cells and only fills blank space.
pub(super) fn render_welcome_mark(frame: &mut Frame<'_>, state: &TuiState, area: Rect) {
    if !state.welcome_visible || area.width < 96 || area.height < 18 {
        return;
    }
    let palette = TerminalPalette::for_preferences(&state.preferences);
    let mut style = Style::default().add_modifier(Modifier::DIM);
    if let Some(color) = palette.meta_style().foreground {
        let neutral = (u16::from(color.red) + u16::from(color.green) + u16::from(color.blue)) / 3;
        // Keep a hint of the theme color at one third of its brightness.
        let muted = |channel| ((u16::from(channel) + neutral * 3) / 12) as u8;
        style = style.fg(Color::Rgb(
            muted(color.red),
            muted(color.green),
            muted(color.blue),
        ));
    }
    let x = area.right().saturating_sub(MARK_WIDTH + 2);
    let y = area.y;
    let buffer = frame.buffer_mut();
    // Skip the whole mark if welcome or startup text occupies it.
    let mark_area = Rect::new(x, y, MARK_WIDTH, MARK_HEIGHT);
    if mark_area.positions().any(|position| {
        buffer
            .cell(position)
            .is_some_and(|cell| !cell.symbol().trim().is_empty())
    }) {
        return;
    }
    for (row, line) in OBSCURITY_MARK.lines().enumerate() {
        for (column, character) in line.chars().enumerate() {
            if let Some(cell) = buffer.cell_mut(Position::new(
                x + u16::try_from(column).unwrap_or_default(),
                y + u16::try_from(row).unwrap_or_default(),
            )) {
                cell.set_char(character).set_style(style);
            }
        }
    }
}
