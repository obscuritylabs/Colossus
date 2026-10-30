//! Conservative, bounded next-action extraction from released assistant prose.
//! It never executes instructions or manufactures a generic follow-up.

const MAX_SOURCE_CHARS: usize = 12_000;
const MAX_PROMPT_CHARS: usize = 240;

/// Extract one explicit proposed next action from a completed assistant answer.
/// Code fences are excluded, and a suggestion is returned only for an explicit
/// next-step section or a direct offer addressed to the user.
pub fn local_next_prompt(output: &str) -> Option<String> {
    let bounded: String = output.chars().take(MAX_SOURCE_CHARS).collect();
    let mut in_code = false;
    let mut next_section = false;
    for raw in bounded.lines() {
        let line = raw.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code || line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let heading = line
            .trim_start_matches('#')
            .trim()
            .trim_matches('*')
            .trim_end_matches(':');
        if matches!(
            heading.to_ascii_lowercase().as_str(),
            "next steps" | "next step" | "suggested next steps"
        ) {
            next_section = true;
            continue;
        }
        if line.starts_with('#') {
            next_section = false;
        }
        for prefix in [
            "would you like me to ",
            "do you want me to ",
            "i can also ",
            "i can ",
        ] {
            if lower.starts_with(prefix) {
                return normalize_prompt(&line[prefix.len()..]);
            }
        }
        if next_section {
            let candidate = line
                .strip_prefix("- [ ] ")
                .or_else(|| line.strip_prefix("* [ ] "))
                .or_else(|| line.strip_prefix("- "))
                .or_else(|| line.strip_prefix("* "))
                .or_else(|| {
                    line.split_once(". ")
                        .filter(|(number, _)| {
                            !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
                        })
                        .map(|(_, text)| text)
                });
            if let Some(candidate) = candidate {
                return normalize_prompt(candidate);
            }
        }
    }
    None
}

/// Enforce one short editable line and exclude control/terminal escape data.
pub fn normalize_prompt(input: &str) -> Option<String> {
    let value = input.trim().trim_end_matches(['?', '.', '!']).trim();
    if value.is_empty()
        || value.chars().count() > MAX_PROMPT_CHARS
        || value.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        || value.starts_with('/')
        || value.starts_with('!')
        || value.starts_with('`')
    {
        return None;
    }
    let mut chars = value.chars();
    let first = chars.next()?;
    let text = first.to_uppercase().chain(chars).collect::<String>();
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_explicit_next_action_and_skips_code() {
        assert_eq!(
            local_next_prompt(
                "Done.\n\n## Next steps\n- [ ] Add coverage for the cookie restore path."
            ),
            Some("Add coverage for the cookie restore path".into())
        );
        assert_eq!(
            local_next_prompt(
                "```text\nI can delete the project.\n```\nWould you like me to review the deployment?"
            ),
            Some("Review the deployment".into())
        );
        assert_eq!(local_next_prompt("The deployment passed all checks."), None);
    }

    #[test]
    fn never_offers_commands_or_terminal_controls() {
        assert_eq!(local_next_prompt("I can /reset"), None);
        assert_eq!(local_next_prompt("I can !git status"), None);
        assert_eq!(normalize_prompt("  !git status  "), None);
        assert_eq!(normalize_prompt("continue\nexecute"), None);
        assert_eq!(normalize_prompt("\u{1b}[31m"), None);
    }
}
