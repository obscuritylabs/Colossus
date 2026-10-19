use crate::RuntimeError;
use colossus_contracts::ComputerLimits;
use serde::{Deserialize, Serialize};

/// Operator-controlled normal-user Windows computer use. Disabled by default.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ComputerConfig {
    /// Expose only an independently verified interactive native host; never grants an effect.
    pub enabled: bool,
    /// Maximum top-level window choices, 1 through 128.
    pub max_windows: u16,
    /// Maximum native semantic nodes, 1 through 1024.
    pub max_snapshot_nodes: u16,
    /// Maximum private screenshot bytes, at most 16 MiB.
    pub max_screenshot_bytes: u32,
    /// Maximum captured physical pixels, at most 16 million.
    pub max_screenshot_pixels: u32,
    /// Absolute native action deadline, at most ten seconds.
    pub action_timeout_ms: u32,
    /// Nonrenewable writer lifetime, at most fifteen minutes.
    pub session_timeout_ms: u32,
    /// Maximum input observation age, at most thirty seconds.
    pub observation_max_age_ms: u32,
}

impl Default for ComputerConfig {
    fn default() -> Self {
        let limits = ComputerLimits::default();
        Self {
            enabled: false,
            max_windows: limits.max_windows,
            max_snapshot_nodes: limits.max_snapshot_nodes,
            max_screenshot_bytes: limits.max_screenshot_bytes,
            max_screenshot_pixels: limits.max_screenshot_pixels,
            action_timeout_ms: limits.action_timeout_ms,
            session_timeout_ms: limits.session_timeout_ms,
            observation_max_age_ms: limits.observation_max_age_ms,
        }
    }
}

impl ComputerConfig {
    pub(crate) fn validate(&self) -> Result<(), RuntimeError> {
        self.limits().validate().map_err(|_| {
            RuntimeError::Config(
                "computer resource bounds exceed the native protocol ceilings".into(),
            )
        })
    }

    pub(crate) fn limits(&self) -> ComputerLimits {
        ComputerLimits {
            max_windows: self.max_windows,
            max_snapshot_nodes: self.max_snapshot_nodes,
            max_screenshot_bytes: self.max_screenshot_bytes,
            max_screenshot_pixels: self.max_screenshot_pixels,
            action_timeout_ms: self.action_timeout_ms,
            session_timeout_ms: self.session_timeout_ms,
            observation_max_age_ms: self.observation_max_age_ms,
        }
    }
}
