use crate::{PresentationBlock, PresentationDocument, PresentationTone};
use colossus_contracts::RunTokenUsage;

/// Present provider-reported usage for one direct run, including partial runs.
/// Cached input and reasoning counts are subsets of input/output, never extras.
pub fn run_token_usage_document(report: &RunTokenUsage) -> PresentationDocument {
    let Some(usage) = report.usage.as_ref() else {
        return PresentationDocument::from_block(PresentationBlock::Card {
            title: "Run usage".into(),
            tone: PresentationTone::Neutral,
            body: vec![PresentationBlock::Text(
                if report.reports == 0 {
                    "Token usage was not reported for this run."
                } else {
                    "Reported token usage is unavailable for this run."
                }
                .into(),
            )],
        });
    };
    let mut details = Vec::new();
    if let Some(cached) = usage.cached_input_tokens {
        details.push(format!("cached input {}", grouped(cached)));
    }
    if let Some(reasoning) = usage.reasoning_tokens {
        details.push(format!("reasoning output {}", grouped(reasoning)));
    }
    let mut body = vec![PresentationBlock::Text(format!(
        "Input {} · Output {}",
        grouped(usage.input_tokens),
        grouped(usage.output_tokens)
    ))];
    if !details.is_empty() {
        body.push(PresentationBlock::Text(format!(
            "Included: {}",
            details.join(" · ")
        )));
    }
    PresentationDocument::from_block(PresentationBlock::Card {
        title: format!("Run usage · {} tokens", grouped(usage.total_tokens)),
        tone: PresentationTone::Neutral,
        body,
    })
}

fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(digit);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TerminalDocumentRenderer, TerminalPreferences};
    use colossus_contracts::ProviderUsage;

    fn report(usage: Option<ProviderUsage>) -> RunTokenUsage {
        RunTokenUsage {
            run_id: "run-a".into(),
            session_id: "session-a".into(),
            through_version: 4,
            reports: u64::from(usage.is_some()),
            usage,
        }
    }

    #[test]
    fn run_usage_distinguishes_unreported_zero_and_optional_subsets() {
        let render = |value| {
            TerminalDocumentRenderer::new(TerminalPreferences::default(), 100)
                .render(&run_token_usage_document(&report(value)))
        };
        let unknown = render(None);
        assert!(unknown.contains("Run usage") && unknown.contains("not reported"));
        assert!(!unknown.contains("0 tokens"));
        let zero = render(Some(ProviderUsage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cached_input_tokens: Some(0),
            reasoning_tokens: None,
        }));
        assert!(zero.contains("0 tokens") && zero.contains("cached input 0"));
        assert!(!zero.contains("reasoning output"));
        let counted = render(Some(ProviderUsage {
            input_tokens: 1_000,
            output_tokens: 250,
            total_tokens: 1_250,
            cached_input_tokens: Some(600),
            reasoning_tokens: Some(100),
        }));
        assert!(counted.contains("1,250 tokens"));
        assert!(counted.contains("Input 1,000") && counted.contains("Output 250"));
        assert!(counted.contains("Included: cached input 600 · reasoning output 100"));
        assert!(!counted.contains("1,950"));
    }

    #[test]
    fn unavailable_reported_usage_is_not_described_as_unreported_or_zero() {
        let unavailable = RunTokenUsage {
            reports: 2,
            ..report(None)
        };
        let rendered = TerminalDocumentRenderer::new(TerminalPreferences::default(), 100)
            .render(&run_token_usage_document(&unavailable));
        assert!(rendered.contains("Reported token usage is unavailable"));
        assert!(!rendered.contains("not reported") && !rendered.contains("0 tokens"));
    }
}
