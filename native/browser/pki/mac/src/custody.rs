//! Immutable native browser-policy exclusion plus exact local output receipts.
mod journal;
mod package;
mod platform;

use crate::{
    MutationAuthority, MutationOperation, MutationRequest, MutationWindow, RetirementStep, Status,
};
use colossus_darwin_process::{DarwinAuditPeer, DarwinProcessIdentity};
use std::{
    os::fd::BorrowedFd,
    path::Path,
    sync::{Arc, Mutex},
};

include!(concat!(env!("OUT_DIR"), "/custody-parent-seal.rs"));

/// Concrete custody authority for the fixed, independently signed Debug package.
///
/// Construction independently checks the genuine current direct parent against
/// the compiled Core seal, fixed local package signature/inventory, immutable
/// role-policy source bindings and reviewed native five-role denial evidence.
/// It accepts no runtime trust key, policy text, path or peer-supplied code seal.
/// Browser roles can never receive the custody descriptor. Arbitrary same-user
/// native applications remain outside the documented sandbox threat boundary.
pub struct NativeMutationAuthority {
    inner: Arc<Inner>,
}
struct Inner {
    package: package::Package,
    boundary: platform::Boundary,
    parent: DarwinAuditPeer,
    generation: [u8; 32],
    journal: Mutex<journal::Journal>,
}
impl NativeMutationAuthority {
    #[cfg(feature = "macos-custody-native-acceptance")]
    /// Dedicated signed no-CEF fixture admission; never full component admission.
    pub fn admit_acceptance(
        parent: &DarwinAuditPeer,
        generation: [u8; 32],
    ) -> Result<Self, Status> {
        let identity =
            DarwinProcessIdentity::bind(platform::parent_pid()).map_err(|_| Status::Denied)?;
        let admitted = DarwinAuditPeer::enroll(&identity, EXPECTED_PARENT_CDHASH)
            .map_err(|_| Status::Denied)?;
        if generation == [0; 32] || admitted.identity() != parent.identity() {
            return Err(Status::Denied);
        }
        parent.verify().map_err(|_| Status::Denied)?;
        let package = package::Package::admit_acceptance(
            &platform::executable()?,
            admitted.identity().effective_uid(),
        )?;
        let boundary = platform::Boundary::new(package.custody_root(), &generation)?;
        parent.verify().map_err(|_| Status::Denied)?;
        package.revalidate()?;
        let journal = journal::Journal::create(
            &package,
            &boundary,
            &admitted,
            &generation,
            &EXPECTED_PARENT_CDHASH,
        )?;
        Ok(Self {
            inner: Arc::new(Inner {
                package,
                boundary,
                parent: admitted,
                generation,
                journal: Mutex::new(journal),
            }),
        })
    }
    /// Called only by dedicated TLS entry after consuming its fixed native parent
    /// channel and admitted 64-byte bootstrap. Missing denial evidence fails
    /// before namespace creation or any Security/Keychain operation.
    pub fn admit(parent: &DarwinAuditPeer, generation: [u8; 32]) -> Result<Self, Status> {
        if generation == [0; 32] {
            return Err(Status::Denied);
        }
        let identity =
            DarwinProcessIdentity::bind(platform::parent_pid()).map_err(|_| Status::Denied)?;
        let admitted = DarwinAuditPeer::enroll(&identity, EXPECTED_PARENT_CDHASH)
            .map_err(|_| Status::Denied)?;
        if admitted.identity() != parent.identity() {
            return Err(Status::Denied);
        }
        parent.verify().map_err(|_| Status::Denied)?;
        let package = package::Package::admit(
            &platform::executable()?,
            admitted.identity().effective_uid(),
        )?;
        let boundary = platform::Boundary::new(package.custody_root(), &generation)?;
        parent.verify().map_err(|_| Status::Denied)?;
        package.revalidate()?;
        let journal = journal::Journal::create(
            &package,
            &boundary,
            &admitted,
            &generation,
            &EXPECTED_PARENT_CDHASH,
        )?;
        Ok(Self {
            inner: Arc::new(Inner {
                package,
                boundary,
                parent: admitted,
                generation,
                journal: Mutex::new(journal),
            }),
        })
    }
    /// Borrow the retained exact custody parent only for local native enrollment.
    pub fn parent_fd(&self) -> BorrowedFd<'_> {
        self.inner.boundary.parent_fd()
    }
    /// Native-owned canonical parent, derived from the authenticated fixed policy.
    pub fn canonical_parent(&self) -> &Path {
        self.inner.package.custody_root()
    }
    /// Exact native broker code hash from the independently signed descriptor.
    pub fn broker_code_hash(&self) -> [u8; 20] {
        self.inner.package.broker_code_hash()
    }
    /// Optional fixed registry metadata digest authenticated by the independently
    /// compiled package signer, never a peer-nominated path/hash or key.
    pub fn registry_digest(&self) -> Option<[u8; 32]> {
        self.inner.package.registry_digest()
    }
    /// Verify the fixed TLS origin/address envelope authenticated by the local
    /// signed package. Opaque selectors cannot broaden this native authority.
    pub fn admits_tls(
        &self,
        generation: &[u8; 32],
        origin: &str,
        destinations: &[std::net::SocketAddr],
    ) -> bool {
        self.signing_admitted(generation) && self.inner.package.admits_tls(origin, destinations)
    }
}
impl MutationAuthority for NativeMutationAuthority {
    fn signing_admitted(&self, generation: &[u8; 32]) -> bool {
        *generation == self.inner.generation
            && self.inner.parent.verify().is_ok()
            && self.inner.package.revalidate().is_ok()
    }
    fn begin(&self, request: MutationRequest) -> Option<Box<dyn MutationWindow>> {
        let retirement = matches!(request.operation, MutationOperation::RetireStore);
        if request.generation != self.inner.generation
            || self.inner.package.revalidate().is_err()
            || (!retirement && self.inner.parent.verify().is_err())
            || !self.inner.boundary.begin(&request)
        {
            return None;
        }
        if !self
            .inner
            .journal
            .lock()
            .is_ok_and(|mut journal| journal.pending(&request).is_ok())
            || self.inner.package.revalidate().is_err()
            || (!retirement && self.inner.parent.verify().is_err())
        {
            self.inner.boundary.end(request.sequence);
            return None;
        }
        Some(Box::new(Window {
            inner: Arc::clone(&self.inner),
            sequence: request.sequence,
            retirement,
        }))
    }
}
struct Window {
    inner: Arc<Inner>,
    sequence: u64,
    retirement: bool,
}
impl MutationWindow for Window {
    fn exclusive(&self) -> bool {
        // Retirement of this already admitted generation remains possible after
        // native parent loss. Immutable role denies continue in every browser;
        // parent death never authorizes a fresh namespace, import or signer.
        self.inner.package.revalidate().is_ok()
            && (self.retirement || self.inner.parent.verify().is_ok())
            && self.inner.boundary.exclusive(self.sequence)
    }
    fn seal(&self, sequence: u64, fds: &[i32]) -> bool {
        sequence == self.sequence
            && self.exclusive()
            && self.inner.boundary.seal(sequence, fds)
            && self.inner.journal.lock().is_ok_and(|mut journal| {
                journal.sealed(&self.inner.boundary, sequence, fds).is_ok()
            })
            && self.exclusive()
    }
    fn checkpoint(&self, sequence: u64, step: RetirementStep) -> bool {
        sequence == self.sequence
            && self.retirement
            && self.exclusive()
            && self.inner.boundary.checkpoint(sequence, step)
            && self.inner.journal.lock().is_ok_and(|mut journal| {
                journal
                    .checkpoint(&self.inner.boundary, sequence, step)
                    .is_ok()
            })
            && self.exclusive()
    }
    fn retire_unchanged(&self, sequence: u64, fds: &[i32]) -> bool {
        // This is solely retirement of prior positive physical generations.
        // Immutable role exclusion persists after parent loss; a live parent is
        // still mandatory for new namespace/import/signing admission.
        sequence == self.sequence
            && !self.retirement
            && self.inner.package.revalidate().is_ok()
            && self.inner.boundary.retire_unchanged(sequence, fds)
            && self.inner.journal.lock().is_ok_and(|mut journal| {
                journal
                    .retire_unchanged(&self.inner.boundary, sequence, fds)
                    .is_ok()
            })
            && self.inner.package.revalidate().is_ok()
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        self.inner.boundary.end(self.sequence);
    }
}
