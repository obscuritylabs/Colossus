//! Optional, explicitly requested composer assistance; never a conversation turn.
use serde::{Deserialize, Serialize};

/// Optional authenticated composer prediction support, requiring read and execute.
pub const NEXT_PROMPT_CAPABILITY: &str = "composer.prediction.v1";

/// Request one prediction for the exact latest completed caller-owned run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredictNextPromptRequest {
    /// Completed run whose canonical conversation may be used.
    pub source_run_id: String,
}

/// Released auxiliary model output, distinct from conversation output and usage.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NextPromptPrediction {
    /// Exact completed source; editors must fence this against their current turn.
    pub source_run_id: String,
    /// Editable text, absent when unavailable, stale, already attempted, or declined.
    pub text: Option<String>,
    /// Provider-reported usage of this auxiliary request, never part of run usage.
    pub usage: Option<crate::TokenUsage>,
}

impl NextPromptPrediction {
    /// Benign absence, including on backends without prediction support.
    pub fn unavailable(source_run_id: String) -> Self {
        Self {
            source_run_id,
            text: None,
            usage: None,
        }
    }
}
