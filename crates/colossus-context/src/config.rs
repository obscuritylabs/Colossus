use super::*;

/// Strict context-window and compaction settings.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContextConfig {
    /// Create snapshots automatically when the threshold is crossed.
    pub auto_compaction: bool,
    /// Fallback model context window used for deterministic budgeting.
    pub context_window_tokens: u64,
    /// Integer percentage at which automatic compaction begins.
    pub compact_at_percent: u8,
    /// Integer percentage targeted after compaction.
    pub target_percent: u8,
    /// Number of newest canonical messages never summarized automatically.
    pub preserve_recent_messages: usize,
    /// Prefer a policy-bound context-summarizer model before deterministic fallback.
    pub model_assisted: bool,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            auto_compaction: true,
            context_window_tokens: 32_768,
            compact_at_percent: 70,
            target_percent: 45,
            preserve_recent_messages: 8,
            model_assisted: true,
        }
    }
}

impl ContextConfig {
    /// Validate safety-relevant budget relationships.
    pub fn validate(&self) -> Result<(), ContextError> {
        if self.context_window_tokens < 1_024
            || self.target_percent == 0
            || self.compact_at_percent >= 100
            || self.target_percent >= self.compact_at_percent
            || self.preserve_recent_messages > 1_024
        {
            return Err(ContextError::Configuration(
                "contextWindowTokens must be >=1024, targetPercent must be below compactAtPercent, percentages must be in 1..100, and preserveRecentMessages must be <=1024"
                    .into(),
            ));
        }
        Ok(())
    }

    pub(super) fn threshold_tokens(&self) -> u64 {
        self.context_window_tokens * u64::from(self.compact_at_percent) / 100
    }

    pub(super) fn target_tokens(&self) -> u64 {
        self.context_window_tokens * u64::from(self.target_percent) / 100
    }
}
