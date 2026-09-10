//! Non-authoritative command intent and the narrow, sanitized approval disclosure.

use serde::{Deserialize, Serialize};

/// Maximum length of a model's task-specific explanation, in Unicode characters.
pub const MAX_COMMAND_JUSTIFICATION_CHARS: usize = 512;
/// Released text budget, allowing escaped/redacted representations of a 1 MiB effect.
pub const MAX_COMMAND_APPROVAL_BYTES: usize = 4 * 1024 * 1024;

/// Task intent bound to an effect, never an authorization or risk assessment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandIntent {
    /// Concise plain-language task purpose supplied before execution.
    pub justification: String,
}

impl CommandIntent {
    /// Reject missing purpose, oversized text, and display-spoofing controls.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.justification.trim().is_empty()
            || self.justification.chars().count() > MAX_COMMAND_JUSTIFICATION_CHARS
            || self
                .justification
                .chars()
                .any(unsafe_command_display_character)
        {
            return Err(
                "justification must be nonblank plain text of at most 512 characters without control or bidirectional characters",
            );
        }
        Ok(())
    }
}

/// Prepared command details released only to an approval's authorized readers.
///
/// Strings are display copies: credentials are removed and controls/backslashes
/// use Rust-style visible escapes, so literal `\\n` differs from a newline.
/// Arguments exclude the executable and preserve argument boundaries. No environment
/// values, stdin, private request hashes, or additional effect fields are released.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandApprovalContext {
    /// Agent-provided purpose, not a policy decision.
    pub justification: String,
    /// Actual prepared executable, sanitized for display.
    pub executable: String,
    /// Actual prepared arguments, individually sanitized for display.
    pub arguments: Vec<String>,
    /// Actual prepared working directory, sanitized for display.
    pub working_directory: String,
    /// At least one credential value was replaced in this display copy.
    pub redacted: bool,
}

impl CommandApprovalContext {
    /// Validate untrusted public/worker/replayed context before presenting it.
    pub fn validate(&self) -> Result<(), &'static str> {
        let fields = [
            &self.justification,
            &self.executable,
            &self.working_directory,
        ];
        if fields.iter().any(|value| value.trim().is_empty())
            // A valid 512-character intent can expand when short known secrets
            // become [REDACTED]. This is not a smaller limit on model input.
            || self.justification.len() > 8192
            || self.arguments.len() > 256
            || fields
                .into_iter()
                .chain(self.arguments.iter())
                .any(|value| value.chars().any(unsafe_command_display_character))
            || fields
                .into_iter()
                .chain(self.arguments.iter())
                .map(|value| value.len())
                .sum::<usize>()
                > MAX_COMMAND_APPROVAL_BYTES
        {
            return Err("invalid command approval context");
        }
        Ok(())
    }
}

/// Characters requiring visible escaping in command details, or rejection in intent.
pub fn unsafe_command_display_character(character: char) -> bool {
    character.is_control()
        || matches!(character, '\u{00ad}' | '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{2028}' | '\u{2029}' | '\u{fff9}'..='\u{fffb}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_is_plain_bounded_task_text() {
        for justification in ["", "  ", "line\nbreak", "\u{202e}Allow", "\u{1b}[2J"] {
            assert!(
                CommandIntent {
                    justification: justification.into()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            CommandIntent {
                justification: "é".repeat(512)
            }
            .validate()
            .is_ok()
        );
        assert!(
            CommandIntent {
                justification: "x".repeat(513)
            }
            .validate()
            .is_err()
        );
    }
}
