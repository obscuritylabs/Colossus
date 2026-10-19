//! Checked physical-pixel transformations. Caller placement is never authority.
use colossus_contracts::ComputerRect;

pub(crate) fn point(rect: ComputerRect, x: i32, y: i32) -> Option<(i32, i32)> {
    if x < 0 || y < 0 || x as u32 >= rect.width || y as u32 >= rect.height {
        return None;
    }
    Some((rect.x.checked_add(x)?, rect.y.checked_add(y)?))
}

pub(crate) fn normalize(rect: ComputerRect, x: i32, y: i32) -> Option<(i32, i32)> {
    let x = i64::from(x) - i64::from(rect.x);
    let y = i64::from(y) - i64::from(rect.y);
    if rect.width < 2
        || rect.height < 2
        || x < 0
        || y < 0
        || x >= i64::from(rect.width)
        || y >= i64::from(rect.height)
    {
        return None;
    }
    Some((
        (x * 65535 / i64::from(rect.width - 1)) as i32,
        (y * 65535 / i64::from(rect.height - 1)) as i32,
    ))
}

pub(crate) fn bounded(value: &str, limit: usize) -> String {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].chars().filter(|c| !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn negative_monitor_origin_and_edges_use_virtual_desktop_coordinates() {
        let desktop = ComputerRect {
            x: -1920,
            y: -100,
            width: 3840,
            height: 1180,
        };
        assert_eq!(normalize(desktop, -1920, -100), Some((0, 0)));
        assert_eq!(normalize(desktop, 1919, 1079), Some((65535, 65535)));
        assert!(normalize(desktop, 1920, 0).is_none());
        assert!(normalize(desktop, -1921, 0).is_none());
        assert_eq!(
            point(
                ComputerRect {
                    x: -100,
                    y: 50,
                    width: 200,
                    height: 100
                },
                0,
                0
            ),
            Some((-100, 50))
        );
        assert!(
            point(
                ComputerRect {
                    x: i32::MAX,
                    y: 0,
                    width: 200,
                    height: 10
                },
                1,
                0
            )
            .is_none()
        );
    }
    #[test]
    fn bounded_metadata_preserves_utf8_and_removes_controls() {
        assert_eq!(bounded("a😀中\n", 6), "a😀");
        assert_eq!(bounded("a\0b\nc", 20), "abc");
    }
}
