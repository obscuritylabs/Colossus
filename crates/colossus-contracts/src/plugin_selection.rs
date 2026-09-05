//! Shared, non-authoritative parsing for conversation and per-message skill selections.

/// Match upstream Agent Plugin and Agent Skill name rules for one qualified identifier.
#[must_use]
pub fn valid_plugin_skill_id(id: &str) -> bool {
    let Some((plugin, skill)) = id.split_once('/') else {
        return false;
    };
    let segment = |name: &str, dots: bool| {
        !name.is_empty()
            && name.len() <= 64
            && name.as_bytes()[0].is_ascii_alphanumeric()
            && name.as_bytes()[name.len() - 1].is_ascii_alphanumeric()
            && !name.contains("--")
            && !name.contains("..")
            && name.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || byte == b'-'
                    || (dots && byte == b'.')
            })
    };
    segment(plugin, true) && segment(skill, false)
}

/// Remove only recognized leading `@plugin/skill` tokens. Unknown mentions and ordinary
/// text remain verbatim. The runtime must revalidate every resulting selection at execution.
#[must_use]
pub fn parse_leading_plugin_mentions(input: &str, available: &[String]) -> (String, Vec<String>) {
    let mut explicit = Vec::new();
    let mut prompt = input.trim_start();
    while let Some(token) = prompt.split_whitespace().next() {
        let Some(id) = token.strip_prefix('@') else {
            break;
        };
        if !id.contains('/') || !available.iter().any(|candidate| candidate == id) {
            break;
        }
        if !explicit.iter().any(|candidate| candidate == id) {
            explicit.push(id.to_owned());
        }
        prompt = prompt[token.len()..].trim_start();
    }
    if explicit.is_empty() {
        (input.into(), explicit)
    } else {
        (prompt.into(), explicit)
    }
}

/// Merge per-message and conversation selections without widening or duplicating them.
#[must_use]
pub fn merge_plugin_selections(message: &[String], conversation: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for id in message.iter().chain(conversation) {
        if !result.contains(id) {
            result.push(id.clone());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_known_leading_qualified_mentions_are_consumed() {
        let available = vec!["colossus/plugin-authoring".into(), "colossus/coding".into()];
        assert_eq!(
            parse_leading_plugin_mentions("  ordinary @colossus/coding text", &available).0,
            "  ordinary @colossus/coding text"
        );
        assert_eq!(
            parse_leading_plugin_mentions("  @unknown/name text", &available).0,
            "  @unknown/name text"
        );
        assert_eq!(
            parse_leading_plugin_mentions(
                "@colossus/plugin-authoring @colossus/plugin-authoring @someone text",
                &available
            ),
            (
                "@someone text".into(),
                vec!["colossus/plugin-authoring".into()]
            )
        );
        assert_eq!(
            merge_plugin_selections(&available, &["colossus/coding".into()]),
            available
        );
    }

    #[test]
    fn qualified_identifiers_follow_both_upstream_name_grammars() {
        for id in [
            "colossus/plugin-authoring",
            "dev.example.plugin/test-1",
            "x/1",
        ] {
            assert!(valid_plugin_skill_id(id), "{id}");
        }
        for id in [
            "coding", "x/", "/x", "UPPER/x", "x/UPPER", "a_b/x", "a--b/x", "a..b/x", "a/-b",
            "a/b-", "a/b--c", "a/b/c", "a/b.c", "é/x",
        ] {
            assert!(!valid_plugin_skill_id(id), "{id}");
        }
        assert!(!valid_plugin_skill_id(&format!("{}/x", "a".repeat(65))));
    }
}
