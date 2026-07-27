/// Restrict `value` to the inclusive range from `minimum` to `maximum`.
///
/// # Panics
///
/// Panics when `minimum` is greater than `maximum`.
pub fn clamp(value: i32, minimum: i32, maximum: i32) -> i32 {
    assert!(minimum <= maximum, "minimum must not exceed maximum");
    value.max(minimum).min(maximum.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::clamp;

    #[test]
    fn leaves_values_inside_the_range_unchanged() {
        assert_eq!(clamp(5, 0, 10), 5);
    }

    #[test]
    fn clamps_values_outside_the_range() {
        assert_eq!(clamp(-2, 0, 10), 0);
        assert_eq!(clamp(12, 0, 10), 10);
    }
}
