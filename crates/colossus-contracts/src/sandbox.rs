use super::*;

/// Explicit process-execution modes that do not provide a Colossus isolation boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxBoundaryMode {
    /// The embedding platform is asserted to provide the isolation boundary.
    External,
    /// No Colossus or external isolation boundary is asserted.
    DangerFullAccess,
}

impl SandboxBoundaryMode {
    /// Stable sandbox backend name used by configuration and policy obligations.
    pub const fn as_backend(self) -> &'static str {
        match self {
            Self::External => "external",
            Self::DangerFullAccess => "danger_full_access",
        }
    }

    /// Parse one direct-execution backend name.
    pub fn from_backend(backend: &str) -> Option<Self> {
        match backend {
            "external" => Some(Self::External),
            "danger_full_access" => Some(Self::DangerFullAccess),
            _ => None,
        }
    }
}
