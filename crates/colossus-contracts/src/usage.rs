//! Reported provider token accounting reconstructed from canonical run events.

use crate::ProviderUsage;
use serde::{Deserialize, Serialize};

/// Reported token counts from one exact run stream through a durable version.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunTokenUsage {
    /// Exact canonical run identifier.
    pub run_id: String,
    /// Canonical session containing the run.
    pub session_id: String,
    /// Last run-stream version included in this summary.
    pub through_version: u64,
    /// Number of provider accounting reports included in the summary.
    pub reports: u64,
    /// Combined reported counts; absent when no usage was reported.
    pub usage: Option<ProviderUsage>,
}

impl ProviderUsage {
    /// Combine reports without recomputing provider totals or counting details twice.
    ///
    /// Optional details remain known only when both reports supplied them. Returns
    /// None if the sum of any required or known optional counter overflows.
    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        Some(Self {
            input_tokens: self.input_tokens.checked_add(other.input_tokens)?,
            output_tokens: self.output_tokens.checked_add(other.output_tokens)?,
            total_tokens: self.total_tokens.checked_add(other.total_tokens)?,
            cached_input_tokens: match (self.cached_input_tokens, other.cached_input_tokens) {
                (Some(left), Some(right)) => Some(left.checked_add(right)?),
                _ => None,
            },
            reasoning_tokens: match (self.reasoning_tokens, other.reasoning_tokens) {
                (Some(left), Some(right)) => Some(left.checked_add(right)?),
                _ => None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn known_zero() -> ProviderUsage {
        ProviderUsage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            cached_input_tokens: Some(0),
            reasoning_tokens: Some(0),
        }
    }

    #[test]
    fn checked_add_preserves_explicit_zero_and_missing_details() {
        let zero = known_zero();
        assert_eq!(zero.checked_add(&zero), Some(known_zero()));

        let unknown = ProviderUsage {
            cached_input_tokens: None,
            reasoning_tokens: None,
            ..known_zero()
        };
        assert_eq!(zero.checked_add(&unknown), Some(unknown.clone()));
        assert_eq!(unknown.checked_add(&zero), Some(unknown));

        let missing_cache = ProviderUsage {
            cached_input_tokens: None,
            ..known_zero()
        };
        assert_eq!(zero.checked_add(&missing_cache), Some(missing_cache));
    }

    #[test]
    fn checked_add_preserves_provider_totals_independently_of_token_details() {
        let left = ProviderUsage {
            input_tokens: 2,
            output_tokens: 3,
            total_tokens: 8,
            cached_input_tokens: Some(1),
            reasoning_tokens: Some(2),
        };
        let right = ProviderUsage {
            input_tokens: 4,
            output_tokens: 1,
            total_tokens: 9,
            cached_input_tokens: Some(2),
            reasoning_tokens: Some(1),
        };
        assert_eq!(
            left.checked_add(&right),
            Some(ProviderUsage {
                input_tokens: 6,
                output_tokens: 4,
                total_tokens: 17,
                cached_input_tokens: Some(3),
                reasoning_tokens: Some(3),
            })
        );
    }

    #[test]
    fn checked_add_rejects_each_required_counter_overflow() {
        let increment = ProviderUsage {
            input_tokens: 1,
            output_tokens: 1,
            total_tokens: 2,
            ..known_zero()
        };
        for left in [
            ProviderUsage {
                input_tokens: u64::MAX,
                ..known_zero()
            },
            ProviderUsage {
                output_tokens: u64::MAX,
                ..known_zero()
            },
            ProviderUsage {
                total_tokens: u64::MAX,
                ..known_zero()
            },
        ] {
            assert!(left.checked_add(&increment).is_none());
        }
    }

    #[test]
    fn checked_add_rejects_each_known_optional_counter_overflow() {
        let increment = ProviderUsage {
            cached_input_tokens: Some(1),
            reasoning_tokens: Some(1),
            ..known_zero()
        };
        for left in [
            ProviderUsage {
                cached_input_tokens: Some(u64::MAX),
                ..known_zero()
            },
            ProviderUsage {
                reasoning_tokens: Some(u64::MAX),
                ..known_zero()
            },
        ] {
            assert!(left.checked_add(&increment).is_none());
        }
    }

    #[test]
    fn run_token_usage_preserves_absence_and_rejects_unknown_fields() {
        let absent = RunTokenUsage {
            run_id: "run-one".into(),
            session_id: "session-one".into(),
            through_version: 1,
            reports: 0,
            usage: None,
        };
        let reported_zero = RunTokenUsage {
            through_version: 2,
            reports: 1,
            usage: Some(known_zero()),
            ..absent.clone()
        };
        for summary in [absent, reported_zero] {
            let encoded = serde_json::to_value(&summary).expect("serialize usage");
            assert_eq!(
                serde_json::from_value::<RunTokenUsage>(encoded.clone())
                    .expect("deserialize usage"),
                summary
            );
            let mut unexpected = encoded;
            unexpected["unrecognized"] = json!(true);
            assert!(serde_json::from_value::<RunTokenUsage>(unexpected).is_err());
        }
    }
}
