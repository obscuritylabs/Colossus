//! Bounded, case-sensitive selectors for tool catalogs, never action authority.

/// A full-name selector with `*` as its only wildcard (zero or more characters).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolNamePattern(String);

/// Invalid tool selector syntax or size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolNamePatternError;

impl std::fmt::Display for ToolNamePatternError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tool selectors must use 1..=128 ASCII letters, digits, dot, underscore, hyphen, or single * wildcards; regex and consecutive stars are unsupported")
    }
}

impl std::error::Error for ToolNamePatternError {}

impl ToolNamePattern {
    /// Parse a bounded selector without regex, escaping, or implicit case folding.
    ///
    /// # Errors
    /// Rejects empty, oversized, non-ASCII, unsupported, or consecutive-star syntax.
    pub fn parse(value: &str) -> Result<Self, ToolNamePatternError> {
        if value.is_empty()
            || value.len() > 128
            || value.contains("**")
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'*')
            })
        {
            return Err(ToolNamePatternError);
        }
        Ok(Self(value.into()))
    }

    /// Original validated selector for diagnostics and configuration round trips.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Match the entire name, with constant memory and no recursive backtracking.
    /// Work is bounded by name length times the selector's maximum 128 bytes.
    #[must_use]
    pub fn matches(&self, name: &str) -> bool {
        let pattern = self.0.as_bytes();
        let name = name.as_bytes();
        let (mut p, mut n) = (0, 0);
        let mut star = None;
        let mut retry = 0;
        while n < name.len() {
            if p < pattern.len() && pattern[p] == b'*' {
                star = Some(p);
                p += 1;
                retry = n;
            } else if p < pattern.len() && pattern[p] == name[n] {
                p += 1;
                n += 1;
            } else if let Some(last_star) = star {
                retry += 1;
                n = retry;
                p = last_star + 1;
            } else {
                return false;
            }
        }
        if p < pattern.len() && pattern[p] == b'*' {
            p += 1;
        }
        p == pattern.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_match_whole_case_sensitive_names() {
        for (pattern, name, expected) in [
            ("get_*", "get_user", true),
            ("get_*", "get_", true),
            ("get_*", "Get_user", false),
            ("get_*", "forget_user", false),
            ("*_search", "splunk_search", true),
            ("*_search", "splunk_search_all", false),
            ("filesystem.*", "filesystem.read", true),
            ("filesystem.*", "filesystemXread", false),
            ("get_*_item*", "get_user_item_2", true),
            ("get_*_item", "get_item", false),
            ("a*ab", "aaab", true),
            ("a*b*c", "abbbc", true),
            ("a*b*c", "abbbd", false),
            ("*", "new_tool", true),
            ("echo", "echo", true),
            ("echo", "echo_extra", false),
        ] {
            assert_eq!(
                ToolNamePattern::parse(pattern).unwrap().matches(name),
                expected,
                "{pattern} against {name}"
            );
        }
    }

    #[test]
    fn unsupported_syntax_and_unbounded_patterns_fail_closed() {
        for invalid in [
            "",
            "get_?",
            "get_[ab]",
            "^get_.*$",
            "get_*|set_*",
            " get_*",
            "get_*\n",
            "gét_*",
            "**",
            "get_**",
            "get_\\*",
        ] {
            assert!(ToolNamePattern::parse(invalid).is_err(), "{invalid}");
        }
        assert!(ToolNamePattern::parse(&"a".repeat(128)).is_ok());
        assert!(ToolNamePattern::parse(&"a".repeat(129)).is_err());
        let adversarial = ToolNamePattern::parse(&format!("*{}b", "a".repeat(126))).unwrap();
        assert!(!adversarial.matches(&"a".repeat(16_384)));
    }
}
