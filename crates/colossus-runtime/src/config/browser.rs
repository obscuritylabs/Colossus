use crate::RuntimeError;
use colossus_contracts::BrowserLimits;
use serde::{Deserialize, Serialize};

/// Operator-controlled browser availability and resource ceilings.
/// These settings can only narrow a verified native host's capabilities.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserConfig {
    /// Allow the runtime to expose its verified browser host.
    pub enabled: bool,
    /// Load public HTTPS sites and their dependencies when destination policy permits.
    /// False restricts every request to the session's exact initial origins.
    pub public_web: bool,
    /// Maximum active browser sessions (1..=4).
    pub max_sessions: u16,
    /// Maximum aggregate tabs (1..=8).
    pub max_tabs: u16,
    /// Maximum nodes in a semantic snapshot (1..=1024).
    pub max_snapshot_nodes: u16,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        let limits = BrowserLimits::default();
        Self {
            enabled: true,
            public_web: true,
            max_sessions: limits.max_sessions,
            max_tabs: limits.max_tabs,
            max_snapshot_nodes: limits.max_snapshot_nodes,
        }
    }
}

impl BrowserConfig {
    pub(crate) fn validate(&self) -> Result<(), RuntimeError> {
        let maximum = Self::default();
        for (name, value, ceiling) in [
            ("maxSessions", self.max_sessions, maximum.max_sessions),
            ("maxTabs", self.max_tabs, maximum.max_tabs),
            (
                "maxSnapshotNodes",
                self.max_snapshot_nodes,
                maximum.max_snapshot_nodes,
            ),
        ] {
            if value == 0 || value > ceiling {
                return Err(RuntimeError::Config(format!(
                    "browser.{name} must be in 1..={ceiling}"
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn limits(&self) -> BrowserLimits {
        BrowserLimits {
            max_sessions: self.max_sessions,
            max_tabs: self.max_tabs,
            max_snapshot_nodes: self.max_snapshot_nodes,
            ..BrowserLimits::default()
        }
    }
}
