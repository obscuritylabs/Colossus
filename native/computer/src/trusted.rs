//! Host-only process identities; display names never establish authority.
use colossus_ports::ComputerDriverError;
use serde::{Deserialize, Serialize};

/// Exact live process identity supplied by trusted host composition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeProcessIdentity {
    pub(crate) pid: u32,
    pub(crate) birth: u64,
}

/// Private native host context. It cannot be supplied by model tool arguments.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedComputerContext {
    pub(crate) protected: Vec<NativeProcessIdentity>,
}

impl TrustedComputerContext {
    /// Protect this trusted host. Composition also protects its Desktop/controller
    /// process explicitly; do not protect Explorer or all its unrelated children.
    pub fn for_current_host() -> Result<Self, ComputerDriverError> {
        #[cfg(windows)]
        return Ok(Self {
            protected: vec![crate::platform::identity::process(std::process::id())?],
        });
        #[cfg(not(windows))]
        Err(ComputerDriverError::Unavailable)
    }
    /// Add an exact live trusted controller/approval process, including descendants.
    pub fn protect_process(self, pid: u32) -> Result<Self, ComputerDriverError> {
        if self.protected.len() >= 32 {
            return Err(ComputerDriverError::InvalidInput);
        }
        #[cfg(windows)]
        {
            let mut current = self;
            let identity = crate::platform::identity::process(pid)?;
            if !current.protected.contains(&identity) {
                current.protected.push(identity);
            }
            Ok(current)
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            Err(ComputerDriverError::Unavailable)
        }
    }
}
