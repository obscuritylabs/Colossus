use super::*;

pub(super) fn shell_document(result: &Value) -> PresentationDocument {
    let stopped = result["stopped"].as_bool() == Some(true);
    let timed_out = result["timed_out"].as_bool() == Some(true);
    let exit_code = result["exit_code"].as_i64();
    let title = if stopped {
        "Shell command cancelled".into()
    } else if timed_out {
        "Shell command timed out".into()
    } else {
        format!(
            "Shell · exit {}",
            exit_code.map_or("unknown".into(), |code| code.to_string())
        )
    };
    let mut body = Vec::new();
    for field in ["stdout", "stderr"] {
        if let Some(text) = result[field].as_str().filter(|text| !text.is_empty()) {
            body.push(PresentationBlock::Verbatim(text.into()));
        }
    }
    if body.is_empty() {
        body.push(PresentationBlock::Text("(no output)".into()));
    }
    if result["truncated"].as_bool() == Some(true) {
        body.push(PresentationBlock::Text("Output truncated.".into()));
    }
    PresentationDocument::from_block(PresentationBlock::Card {
        title,
        tone: if stopped || timed_out || exit_code != Some(0) {
            PresentationTone::Warning
        } else {
            PresentationTone::Tool
        },
        body,
    })
}
