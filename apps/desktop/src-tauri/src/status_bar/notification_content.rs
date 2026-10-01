use serde::Deserialize;

#[cfg(any(test, target_os = "macos", windows))]
use super::BackgroundNotificationKind;
use crate::dto::CommandErrorDto;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BackgroundNotificationContent {
    thread_title: String,
    output_preview: String,
}

impl BackgroundNotificationContent {
    pub(super) fn validate(&self) -> Result<(), CommandErrorDto> {
        if self.thread_title.trim().is_empty()
            || !valid_text(&self.thread_title, 96)
            || !valid_text(&self.output_preview, 240)
        {
            return Err(CommandErrorDto::invalid(
                "content",
                "Notification titles and previews must be bounded plain text.",
            ));
        }
        Ok(())
    }

    #[cfg(any(test, target_os = "macos", windows))]
    pub(super) fn presentation(&self, kind: BackgroundNotificationKind) -> (String, &str) {
        let (status, fallback) = match kind {
            BackgroundNotificationKind::NeedsAttention => {
                ("Needs input", "Open Colossus to review the waiting work.")
            }
            BackgroundNotificationKind::WorkCompleted => {
                ("Work finished", "Open Colossus to see the result.")
            }
            BackgroundNotificationKind::WorkFailed => {
                ("Work stopped", "Open Colossus to review the result.")
            }
        };
        let body = if self.output_preview.trim().is_empty() {
            fallback
        } else {
            self.output_preview.as_str()
        };
        (format!("{status} · {}", self.thread_title), body)
    }
}

fn valid_text(value: &str, maximum: usize) -> bool {
    value.chars().take(maximum + 1).count() <= maximum
        && !value.chars().any(|ch| {
            ch.is_control()
                || matches!(ch,
                    '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}' | '\u{feff}'
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_identify_the_thread_and_bodies_preview_the_response() {
        let content = BackgroundNotificationContent {
            thread_title: "Review deployment 🚀".into(),
            output_preview: "Updated the deployment and checked the configuration.".into(),
        };
        assert!(content.validate().is_ok());
        for (kind, prefix) in [
            (BackgroundNotificationKind::WorkCompleted, "Work finished"),
            (BackgroundNotificationKind::NeedsAttention, "Needs input"),
            (BackgroundNotificationKind::WorkFailed, "Work stopped"),
        ] {
            let (title, body) = content.presentation(kind);
            assert_eq!(title, format!("{prefix} · Review deployment 🚀"));
            assert_eq!(body, content.output_preview);
        }
    }

    #[test]
    fn empty_previews_keep_actionable_status_text() {
        let content = BackgroundNotificationContent {
            thread_title: "Review deployment".into(),
            output_preview: String::new(),
        };
        assert!(content.validate().is_ok());
        assert_eq!(
            content
                .presentation(BackgroundNotificationKind::WorkCompleted)
                .1,
            "Open Colossus to see the result."
        );
        assert_eq!(
            content
                .presentation(BackgroundNotificationKind::NeedsAttention)
                .1,
            "Open Colossus to review the waiting work."
        );
    }

    #[test]
    fn native_boundary_rejects_unbounded_or_control_text() {
        for (title, preview) in [
            (" ".into(), String::new()),
            ("x".repeat(97), String::new()),
            ("Title".into(), "x".repeat(241)),
            ("Bad\nTitle".into(), String::new()),
            ("Title".into(), "Hide\u{202e} text".into()),
        ] {
            assert!(
                BackgroundNotificationContent {
                    thread_title: title,
                    output_preview: preview
                }
                .validate()
                .is_err()
            );
        }
    }
}
