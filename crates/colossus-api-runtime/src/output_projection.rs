//! Keep separate model turns readable in the released application output feed.

use colossus_contracts::{ProviderEvent, RunEvent, RunPhase};

#[derive(Default)]
pub(super) struct OutputProjection {
    has_output: bool,
    next_turn: bool,
    spacing: LineBreaks,
}

#[derive(Clone, Copy, Default)]
struct LineBreaks {
    count: u8,
    carriage_return: bool,
}

impl LineBreaks {
    fn observe(&mut self, bytes: &[u8]) {
        for byte in bytes {
            match byte {
                b'\r' => {
                    self.count = self.count.saturating_add(1).min(2);
                    self.carriage_return = true;
                }
                b'\n' => {
                    if !self.carriage_return {
                        self.count = self.count.saturating_add(1).min(2);
                    }
                    self.carriage_return = false;
                }
                _ => *self = Self::default(),
            }
        }
    }
}

impl OutputProjection {
    pub(super) fn apply(&mut self, event: &mut RunEvent) {
        if matches!(
            event,
            RunEvent::Phase {
                phase: RunPhase::WaitingForModel,
                ..
            }
        ) {
            self.next_turn = self.has_output;
        }
        if let RunEvent::Provider {
            event: ProviderEvent::ModelDelta { text },
        } = event
        {
            if text.is_empty() {
                return;
            }
            let leading = text
                .bytes()
                .take_while(|c| matches!(c, b'\r' | b'\n'))
                .count();
            // A line break may itself span several provider deltas. Wait for
            // visible text before deciding how much paragraph padding is missing.
            if self.next_turn && leading < text.len() {
                let separator = ["", "\n", "\n\n"]
                    .into_iter()
                    .find(|separator| {
                        let mut spacing = self.spacing;
                        spacing.observe(separator.as_bytes());
                        spacing.observe(&text.as_bytes()[..leading]);
                        spacing.count >= 2
                    })
                    .unwrap_or("\n\n");
                text.insert_str(0, separator);
            }
            self.spacing.observe(text.as_bytes());
            self.has_output = true;
            if leading < text.len() {
                self.next_turn = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting() -> RunEvent {
        RunEvent::Phase {
            phase: RunPhase::WaitingForModel,
            turn: Some(1),
            action: None,
            elapsed_seconds: 0.0,
        }
    }

    fn delta(projection: &mut OutputProjection, text: &str) -> String {
        let mut event = RunEvent::Provider {
            event: ProviderEvent::ModelDelta { text: text.into() },
        };
        projection.apply(&mut event);
        match event {
            RunEvent::Provider {
                event: ProviderEvent::ModelDelta { text },
            } => text,
            _ => unreachable!(),
        }
    }

    #[test]
    fn fragmented_text_is_exact_with_paragraphs_between_separate_model_calls() {
        let mut projection = OutputProjection::default();
        projection.apply(&mut waiting());
        let mut output = delta(&mut projection, "I'm doing");
        output += &delta(&mut projection, " a thing.");
        projection.apply(&mut waiting());
        output += &delta(&mut projection, "Now another thing.");
        output += &delta(&mut projection, " Still the same turn.");
        assert_eq!(
            output,
            "I'm doing a thing.\n\nNow another thing. Still the same turn."
        );
    }

    #[test]
    fn empty_tool_only_turns_add_no_output_and_existing_spacing_is_preserved() {
        let mut projection = OutputProjection::default();
        projection.apply(&mut waiting());
        assert_eq!(delta(&mut projection, ""), "");
        projection.apply(&mut waiting());
        assert_eq!(delta(&mut projection, "first\n\n"), "first\n\n");
        projection.apply(&mut waiting());
        assert_eq!(delta(&mut projection, ""), "");
        projection.apply(&mut waiting());
        assert_eq!(delta(&mut projection, "next"), "next");
        projection.apply(&mut waiting());
        assert_eq!(delta(&mut projection, "\n\nlast"), "\n\nlast");
    }

    #[test]
    fn paragraph_boundaries_are_invariant_to_split_lf_and_crlf_chunks() {
        for (previous, chunks, expected) in [
            ("old", vec!["\n", "\nnext"], "old\n\nnext"),
            ("old", vec!["\r", "\n", "\r", "\nnext"], "old\r\n\r\nnext"),
            ("old\r", vec!["next"], "old\r\n\nnext"),
            ("old\r\n", vec!["\r", "\nnext"], "old\r\n\r\nnext"),
        ] {
            let mut projection = OutputProjection::default();
            projection.apply(&mut waiting());
            let mut output = delta(&mut projection, previous);
            projection.apply(&mut waiting());
            for chunk in chunks {
                output += &delta(&mut projection, chunk);
            }
            assert_eq!(output, expected);
        }
    }
}
