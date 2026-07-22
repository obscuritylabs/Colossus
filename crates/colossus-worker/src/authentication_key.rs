use std::{fmt, sync::Arc};
use zeroize::Zeroizing;

/// Independent authentication key for the private worker IPC protocol.
///
/// Clones share one zeroizing allocation so attached clients do not leave ordinary
/// heap copies behind. Debug output is always redacted.
#[derive(Clone)]
pub struct WorkerAuthenticationKey(Arc<Zeroizing<[u8; 32]>>);

impl WorkerAuthenticationKey {
    /// Move one exact 256-bit key into shared zeroizing memory.
    pub fn new(authentication: [u8; 32]) -> Self {
        Self::from_zeroizing(Zeroizing::new(authentication))
    }

    /// Move an already-zeroizing 256-bit key without creating an ordinary secret
    /// copy at an inherited native-channel boundary.
    pub fn from_zeroizing(authentication: Zeroizing<[u8; 32]>) -> Self {
        Self(Arc::new(authentication))
    }

    pub(super) fn expose(&self) -> &[u8; 32] {
        self.0.as_ref()
    }
}

impl fmt::Debug for WorkerAuthenticationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WorkerAuthenticationKey([REDACTED])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_authentication_debug_is_redacted() {
        let key = WorkerAuthenticationKey::new([0xa5; 32]);
        assert!(!format!("{key:?}").contains("a5"));
        assert_eq!(key.expose(), &[0xa5; 32]);
    }
}
