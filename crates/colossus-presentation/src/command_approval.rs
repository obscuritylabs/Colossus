use colossus_contracts::CommandApprovalContext;

use crate::{PresentationBlock, PresentationDocument, PresentationError, PresentationTone};

/// Render the same frozen prepared-command disclosure across terminal transports.
/// Full details retain every accepted argument; previews explicitly announce omission.
pub fn command_approval_document(
    context: &CommandApprovalContext,
    policy: Option<&str>,
    risk: Option<&str>,
    full: bool,
) -> Result<PresentationDocument, PresentationError> {
    context
        .validate()
        .map_err(|message| PresentationError::Invalid(message.into()))?;
    let argv = std::iter::once(&context.executable)
        .chain(&context.arguments)
        .collect::<Vec<_>>();
    let command = serde_json::to_string(&argv)
        .map_err(|error| PresentationError::Invalid(error.to_string()))?;
    let preview = |text: &str, limit: usize, suffix: &str| {
        let prefix = text.chars().take(limit).collect::<String>();
        if prefix.len() == text.len() {
            prefix
        } else {
            format!("{prefix} … {suffix}")
        }
    };
    let mut details = vec![
        (
            "Reason — agent-provided".into(),
            context.justification.clone(),
        ),
        (
            "Command".into(),
            preview(&command, 240, "see full command details"),
        ),
        (
            "Working directory".into(),
            context.working_directory.clone(),
        ),
    ];
    if context.redacted {
        details.push((
            "Disclosure".into(),
            "Credential-bearing text is redacted; execution input is unchanged.".into(),
        ));
    }
    if let Some(policy) = policy {
        details.push(("Policy".into(), policy.into()));
    }
    if let Some(risk) = risk {
        details.push(("Risk review".into(), risk.into()));
    }
    let argv_details = serde_json::to_string_pretty(&argv)
        .map_err(|error| PresentationError::Invalid(error.to_string()))?;
    let cwd_details = serde_json::to_string(&context.working_directory)
        .map_err(|error| PresentationError::Invalid(error.to_string()))?;
    let full_command = format!(
        "Working directory (JSON display string):\n{cwd_details}\n\nPrepared argument vector (executable first):\n{argv_details}"
    );
    Ok(PresentationDocument::from_block(PresentationBlock::Card {
        title: "Approval required".into(),
        tone: PresentationTone::Warning,
        body: vec![
            PresentationBlock::KeyValue(details),
            PresentationBlock::Text(
                "Prepared argument vector (executable first; JSON display strings):".into(),
            ),
            // Source-code blocks intentionally clip lines. Approval details must wrap
            // every released character, including long single-argument commands.
            PresentationBlock::Verbatim(if full {
                full_command
            } else {
                preview(
                    &full_command,
                    1200,
                    "preview only; type details for the full command",
                )
            }),
        ],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_preserve_tail_and_argument_boundaries() {
        let context = CommandApprovalContext {
            justification: "Check dependency versions.".into(),
            executable: "/bin/sh".into(),
            arguments: vec!["-c".into(), format!("{} END", "x".repeat(70000))],
            working_directory: "/work/two  spaces".into(),
            redacted: false,
        };
        let full = format!(
            "{:?}",
            command_approval_document(&context, Some("Explicit approval required"), None, true)
                .unwrap()
        );
        assert!(full.contains("END"));
        assert!(full.contains("/work/two  spaces"));
        let preview = format!(
            "{:?}",
            command_approval_document(&context, None, None, false).unwrap()
        );
        assert!(!preview.contains("END"));
        assert!(preview.contains("preview only"));
        assert!(preview.contains("Reason — agent-provided"));
        for width in [24, 80, 200] {
            let document = command_approval_document(&context, None, None, true).unwrap();
            let rendered =
                crate::TerminalDocumentRenderer::new(crate::TerminalPreferences::default(), width)
                    .render(&document);
            assert!(rendered.contains("END"), "tail clipped at width {width}");
        }
    }
}
