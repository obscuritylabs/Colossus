//! Only the fixed native log verbosity selector crosses an env-cleared launch.

pub(crate) fn debug_enabled() -> bool {
    debug_value(std::env::var("COLOSSUS_LOG_LEVEL").ok().as_deref())
}

fn debug_value(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.eq_ignore_ascii_case("debug"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn log_verbosity_cannot_enable_dependency_wire_tracing() {
        assert!(super::debug_value(Some("DEBUG")));
        for value in [
            None,
            Some("info"),
            Some("trace"),
            Some("reqwest=trace"),
            Some("debug\nTOKEN=secret"),
        ] {
            assert!(!super::debug_value(value));
        }
    }
}
