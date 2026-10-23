//! Cell-based selection over a frozen copy of the rendered terminal view.

use super::*;

const MAX_COPY_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy)]
struct Range {
    anchor: Position,
    head: Position,
    area: Rect,
    dragging: bool,
}

#[derive(Default)]
pub(super) struct TextSelection {
    frame: Option<Buffer>,
    range: Option<Range>,
    pub(super) regions: Vec<Rect>,
    pending_copy: Option<String>,
}

impl TextSelection {
    pub(super) fn active(&self) -> bool {
        self.range.is_some()
    }

    pub(super) fn clear(&mut self) {
        self.range = None;
    }

    pub(super) fn remember(&mut self, buffer: &Buffer) {
        self.frame = Some(buffer.clone());
    }

    pub(super) fn handle_mouse(&mut self, mouse: MouseEvent) -> bool {
        let point = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(buffer) = self
                    .frame
                    .as_ref()
                    .filter(|buffer| buffer.area.contains(point))
                else {
                    return false;
                };
                let area = self
                    .regions
                    .iter()
                    .copied()
                    .find(|area| area.contains(point))
                    .unwrap_or(buffer.area);
                let point = cell_start(buffer, point, area);
                self.range = Some(Range {
                    anchor: point,
                    head: point,
                    area,
                    dragging: true,
                });
                true
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                let Some(range) = self.range.as_mut().filter(|range| range.dragging) else {
                    return false;
                };
                let Some(buffer) = self.frame.as_ref() else {
                    return false;
                };
                range.head = cell_start(buffer, point, range.area);
                if matches!(mouse.kind, MouseEventKind::Up(_)) {
                    range.dragging = false;
                    if range.anchor == range.head {
                        self.clear();
                    }
                }
                true
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                self.clear();
                false
            }
            _ => false,
        }
    }

    /// Redraw from the exact original cells, so streaming cannot move the selection.
    pub(super) fn render(&mut self, target: &mut Buffer) -> bool {
        let Some(range) = self.range else {
            return false;
        };
        let Some(buffer) = self.frame.as_ref() else {
            return false;
        };
        if target.area != buffer.area {
            self.clear();
            return false;
        }
        target.clone_from(buffer);
        let (start, end) = ordered(range);
        for y in start.y..=end.y {
            let (left, right) = row_bounds(buffer, range.area, start, end, y);
            for x in left..right {
                if let Some(cell) = target.cell_mut((x, y)) {
                    cell.set_style(Style::default().add_modifier(Modifier::REVERSED));
                }
            }
        }
        true
    }

    pub(super) fn request_copy(&mut self) {
        self.pending_copy = self.text();
    }

    pub(super) fn take_copy(&mut self) -> Option<String> {
        self.pending_copy.take()
    }

    fn text(&self) -> Option<String> {
        let range = self.range?;
        let buffer = self.frame.as_ref()?;
        let (start, end) = ordered(range);
        let mut text = String::new();
        for y in start.y..=end.y {
            let (mut x, right) = row_bounds(buffer, range.area, start, end, y);
            let mut row = String::new();
            while x < right {
                let cell = buffer.cell((x, y))?;
                let symbol = cell.symbol();
                if text.len() + row.len() + symbol.len() > MAX_COPY_BYTES {
                    text.push_str(row.trim_end());
                    return Some(text);
                }
                row.push_str(symbol);
                x = x.saturating_add(
                    u16::try_from(UnicodeWidthStr::width(symbol))
                        .unwrap_or(1)
                        .max(1),
                );
            }
            text.push_str(row.trim_end());
            if y < end.y && text.len() < MAX_COPY_BYTES {
                text.push('\n');
            }
        }
        Some(text)
    }
}

fn ordered(range: Range) -> (Position, Position) {
    if (range.anchor.y, range.anchor.x) <= (range.head.y, range.head.x) {
        (range.anchor, range.head)
    } else {
        (range.head, range.anchor)
    }
}

fn cell_start(buffer: &Buffer, point: Position, area: Rect) -> Position {
    let y = point.y.clamp(area.top(), area.bottom().saturating_sub(1));
    let column = point.x.clamp(area.left(), area.right().saturating_sub(1));
    let mut x = area.left();
    while x < column {
        let width = buffer
            .cell((x, y))
            .map_or(1, |cell| UnicodeWidthStr::width(cell.symbol()).max(1));
        let next = x.saturating_add(u16::try_from(width).unwrap_or(1));
        if next > column {
            break;
        }
        x = next;
    }
    Position::new(x, y)
}

fn row_bounds(buffer: &Buffer, area: Rect, start: Position, end: Position, y: u16) -> (u16, u16) {
    let left = if y == start.y { start.x } else { area.left() };
    let right = if y == end.y {
        let width = buffer
            .cell(end)
            .map_or(1, |cell| UnicodeWidthStr::width(cell.symbol()).max(1));
        end.x
            .saturating_add(u16::try_from(width).unwrap_or(1))
            .min(area.right())
    } else {
        area.right()
    };
    (left, right)
}

#[cfg(test)]
mod tests;
