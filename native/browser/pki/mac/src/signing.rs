//! Retained native signing admission shared by Rust and its locally linked ABI.
use crate::{MutationAuthority, Status};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct SigningAdmission {
    authority: Arc<dyn MutationAuthority>,
    generation: [u8; 32],
    revoked: AtomicBool,
}
impl SigningAdmission {
    pub fn new(authority: Arc<dyn MutationAuthority>, generation: [u8; 32]) -> Self {
        Self {
            authority,
            generation,
            revoked: AtomicBool::new(false),
        }
    }
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }
    pub fn admit(&self, generation: &[u8; 32]) -> bool {
        if *generation != self.generation || *generation == [0; 32] {
            return false;
        }
        if self.revoked.load(Ordering::Acquire) {
            return false;
        }
        if !self.authority.signing_admitted(generation) {
            self.revoke();
            return false;
        }
        !self.revoked.load(Ordering::Acquire)
    }
    /// No output escapes after parent/package loss during the native operation.
    pub fn run<T>(
        &self,
        generation: &[u8; 32],
        operation: impl FnOnce() -> Result<T, Status>,
    ) -> Result<T, Status> {
        if !self.admit(generation) {
            return Err(Status::Denied);
        }
        let result = operation();
        if !self.admit(generation) {
            return Err(Status::Denied);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MutationRequest, MutationWindow};
    use std::sync::atomic::AtomicUsize;

    struct MemoryAuthority {
        parent_alive: AtomicBool,
        package_current: AtomicBool,
        generation: [u8; 32],
    }
    impl MutationAuthority for MemoryAuthority {
        fn begin(&self, _: MutationRequest) -> Option<Box<dyn MutationWindow>> {
            None
        }
        fn signing_admitted(&self, generation: &[u8; 32]) -> bool {
            *generation == self.generation
                && self.parent_alive.load(Ordering::Acquire)
                && self.package_current.load(Ordering::Acquire)
        }
    }
    fn fixture() -> (Arc<MemoryAuthority>, SigningAdmission) {
        let authority = Arc::new(MemoryAuthority {
            parent_alive: AtomicBool::new(true),
            package_current: AtomicBool::new(true),
            generation: [7; 32],
        });
        let admission = SigningAdmission::new(authority.clone(), [7; 32]);
        (authority, admission)
    }
    #[test]
    fn parent_loss_before_handshake_or_signature_never_calls_native_operation() {
        let (authority, admission) = fixture();
        authority.parent_alive.store(false, Ordering::Release);
        let native_calls = AtomicUsize::new(0);
        assert_eq!(
            admission.run(&[7; 32], || {
                native_calls.fetch_add(1, Ordering::Relaxed);
                Ok(1)
            }),
            Err(Status::Denied)
        );
        assert_eq!(native_calls.load(Ordering::Relaxed), 0);
        authority.parent_alive.store(true, Ordering::Release);
        assert!(!admission.admit(&[7; 32]));
    }
    #[test]
    fn loss_during_native_signature_discards_output_and_irreversibly_revokes() {
        let (authority, admission) = fixture();
        assert_eq!(
            admission.run(&[7; 32], || {
                authority.parent_alive.store(false, Ordering::Release);
                Ok(vec![1, 2, 3])
            }),
            Err(Status::Denied)
        );
        authority.parent_alive.store(true, Ordering::Release);
        assert!(!admission.admit(&[7; 32]));
    }
    #[test]
    fn changed_package_and_explicit_revocation_deny_fresh_signing() {
        let (authority, admission) = fixture();
        assert!(!admission.admit(&[8; 32]));
        assert_eq!(admission.run(&[7; 32], || Ok(42)), Ok(42));
        authority.package_current.store(false, Ordering::Release);
        assert!(!admission.admit(&[7; 32]));
        let (_, admission) = fixture();
        admission.revoke();
        assert!(!admission.admit(&[7; 32]));
    }
}
