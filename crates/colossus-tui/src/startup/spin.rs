//! Orthographic yaw of the original 48-by-40 Braille dot matrix.

use super::*;

const WIDTH: usize = 24;
const HEIGHT: usize = 10;
const DOT_WIDTH: f64 = 48.0;
const SPIN_MILLIS: u32 = 1_050;
const SOURCE: &str = include_str!("../../assets/obscurity-mark.braille");
const DOTS: [(u8, usize, usize); 8] = [
    (1, 0, 0),
    (2, 0, 1),
    (4, 0, 2),
    (8, 1, 0),
    (16, 1, 1),
    (32, 1, 2),
    (64, 0, 3),
    (128, 1, 3),
];

pub(super) struct DotMark {
    front: [u8; WIDTH * HEIGHT],
    dots: Vec<(u16, u16)>,
}

impl DotMark {
    pub(super) fn new() -> Self {
        let mut front = [0; WIDTH * HEIGHT];
        let mut dots = Vec::new();
        for (row, line) in SOURCE.lines().take(HEIGHT).enumerate() {
            for (column, character) in line.chars().take(WIDTH).enumerate() {
                let mask = u32::from(character)
                    .checked_sub(0x2800)
                    .filter(|mask| *mask <= 255)
                    .unwrap_or(0) as u8;
                front[row * WIDTH + column] = mask;
                for (bit, dx, dy) in DOTS {
                    if mask & bit != 0 {
                        dots.push(((column * 2 + dx) as u16, (row * 4 + dy) as u16));
                    }
                }
            }
        }
        Self { front, dots }
    }

    fn projected(&self, elapsed: u128) -> [u8; WIDTH * HEIGHT] {
        if elapsed == 0 || elapsed >= u128::from(SPIN_MILLIS) {
            return self.front;
        }
        let elapsed = u32::try_from(elapsed).unwrap_or_default();
        let angle = std::f64::consts::TAU * f64::from(elapsed) / f64::from(SPIN_MILLIS);
        let cosine = angle.cos();
        let center = (DOT_WIDTH - 1.0) / 2.0;
        let mut projected = [0; WIDTH * HEIGHT];
        for &(x, y) in &self.dots {
            // Y never rotates: the crest stays upright while its face turns
            // through edge-on and reverse views, then settles on the exact source.
            let x = (center + (f64::from(x) - center) * cosine)
                .round()
                .clamp(0.0, DOT_WIDTH - 1.0) as usize;
            let y = usize::from(y);
            let bit = DOTS
                .iter()
                .find(|(_, dx, dy)| *dx == x % 2 && *dy == y % 4)
                .map_or(0, |(bit, _, _)| *bit);
            projected[(y / 4) * WIDTH + x / 2] |= bit;
        }
        projected
    }

    pub(super) fn render(&self, buffer: &mut Buffer, area: Rect, elapsed: u128, style: Style) {
        let projected = self.projected(elapsed);
        for row in 0..usize::from(area.height).min(HEIGHT) {
            for column in 0..usize::from(area.width).min(WIDTH) {
                let mask = projected[row * WIDTH + column];
                let character = if mask == 0 {
                    ' '
                } else {
                    char::from_u32(0x2800 + u32::from(mask)).unwrap_or(' ')
                };
                if let Some(cell) =
                    buffer.cell_mut(Position::new(area.x + column as u16, area.y + row as u16))
                {
                    cell.set_char(character).set_style(style);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dot_bounds(image: &[u8; WIDTH * HEIGHT]) -> (usize, usize, Vec<usize>) {
        let mut xs = Vec::new();
        let mut ys = BTreeSet::new();
        for (index, mask) in image.iter().enumerate() {
            for (bit, dx, dy) in DOTS {
                if mask & bit != 0 {
                    xs.push((index % WIDTH) * 2 + dx);
                    ys.insert((index / WIDTH) * 4 + dy);
                }
            }
        }
        (
            *xs.iter().min().unwrap(),
            *xs.iter().max().unwrap(),
            ys.into_iter().collect(),
        )
    }

    #[test]
    fn dot_matrix_rotates_through_narrow_edges_without_tilting_and_settles_exactly() {
        let mark = DotMark::new();
        assert!(mark.dots.len() > 500);
        let front = dot_bounds(&mark.front);
        let edge = dot_bounds(&mark.projected(263));
        assert!(front.1 - front.0 > 40);
        assert!(edge.1 - edge.0 <= 2);
        assert_eq!(
            front.2, edge.2,
            "yaw preserves every occupied vertical dot row"
        );
        assert_ne!(
            mark.projected(525),
            mark.front,
            "the reverse face is a real mirrored projection"
        );
        let frames = (0..14)
            .map(|frame| mark.projected(frame * 75))
            .collect::<BTreeSet<_>>();
        assert!(frames.len() >= 8);
        assert_eq!(mark.projected(u128::from(SPIN_MILLIS)), mark.front);
        assert_eq!(mark.projected(30_000), mark.front);
    }
}
