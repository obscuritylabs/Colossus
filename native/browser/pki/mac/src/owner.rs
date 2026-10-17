use crate::{
    callbacks::{Context, MutationAuthority},
    ffi,
};
use std::{
    ffi::c_void,
    fmt,
    os::fd::{AsRawFd as _, BorrowedFd},
    path::Path,
    ptr::NonNull,
    sync::Arc,
    time::Instant,
};
use zeroize::Zeroize as _;

/// Closed native failures; never contains paths, passwords or certificate bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Ready,
    Denied,
    Unavailable,
    OutcomeUnknown,
}
impl Status {
    fn native(value: u32) -> Self {
        match value {
            0 => Self::Ready,
            1 => Self::Denied,
            2 => Self::Unavailable,
            _ => Self::OutcomeUnknown,
        }
    }
}
/// Supported fixed TLS1.3 SHA256 client CertificateVerify scheme.
#[derive(Clone, Copy, Debug)]
pub enum Scheme {
    EcdsaP256Sha256,
    RsaPssSha256,
}
impl Scheme {
    fn native(self) -> u32 {
        match self {
            Self::EcdsaP256Sha256 => 0x0403,
            Self::RsaPssSha256 => 0x0804,
        }
    }
}
/// Authenticated private-native enrollment only. No field is a browser/model path.
pub struct Enrollment<'a> {
    pub parent_fd: BorrowedFd<'a>,
    pub canonical_parent: &'a Path,
    pub origin: &'a str,
    pub code_hash: [u8; 20],
    pub generation: [u8; 32],
    pub ca_der: &'a [Vec<u8>],
    pub reviewed_leaf: &'a [u8],
    pub scheme: Scheme,
    pub encrypted_pkcs12: &'a [u8],
    pub password: &'a mut [u8],
}
/// A failed preparation retains a real partial owner's cleanup obligation.
pub struct PreparationFailure {
    pub status: Status,
    pub owner: Option<NativeIdentity>,
}
/// Fixed Debug phase/check and OSStatus only; no identity or private contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub phase: u32,
    pub check: u32,
    pub os_status: i32,
}
impl fmt::Debug for PreparationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparationFailure")
            .field("status", &self.status)
            .field("cleanup_obligation", &self.owner.is_some())
            .finish()
    }
}

struct Inner {
    raw: NonNull<c_void>,
    context: Box<Context>,
    generation: [u8; 32],
}
// SAFETY: the opaque native object serializes every operation under its own
// mutex; its stable callback context contains only Send + Sync trait objects.
unsafe impl Send for Inner {}
// SAFETY: see above; no key or mutable native memory is exposed to callers.
unsafe impl Sync for Inner {}
impl Inner {
    fn revoke(&self) {
        self.context.signing.revoke();
        // SAFETY: live Arc-owned native object with serialized revocation.
        unsafe { ffi::colossus_mac_identity_revoke(self.raw.as_ptr()) };
    }
    fn signing_operation<T>(
        &self,
        generation: &[u8; 32],
        operation: impl FnOnce() -> Result<T, Status>,
    ) -> Result<T, Status> {
        let result = self.context.signing.run(generation, operation);
        if !self.context.signing.admit(&self.generation) {
            self.revoke();
            return Err(Status::Denied);
        }
        result
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        // SAFETY: this Arc is the final native owner and all handshake Arcs are gone.
        // Dropping releases refs only; unknown physical state is deliberately kept.
        unsafe { ffi::colossus_mac_identity_drop(self.raw.as_ptr()) };
        self.generation.zeroize();
    }
}
/// Private in-process owner. Its key and native references never leave C++.
pub struct NativeIdentity {
    inner: Arc<Inner>,
    leaf: Vec<u8>,
    scheme: Scheme,
}
impl NativeIdentity {
    /// Prepare only inside independent admitted writer exclusion. The password
    /// is erased on every result. Failures may retain a partial physical owner.
    pub fn prepare(
        enrollment: Enrollment<'_>,
        authority: Arc<dyn MutationAuthority>,
    ) -> Result<Self, PreparationFailure> {
        struct Wipe<'a>(&'a mut [u8]);
        impl Drop for Wipe<'_> {
            fn drop(&mut self) {
                self.0.zeroize();
            }
        }
        let password = Wipe(enrollment.password);
        let denied = || PreparationFailure {
            status: Status::Denied,
            owner: None,
        };
        let parent = enrollment.canonical_parent.to_str().ok_or_else(denied)?;
        if !valid_origin(enrollment.origin)
            || parent.len() > 4096
            || parent.as_bytes().contains(&0)
            || enrollment.code_hash.iter().all(|byte| *byte == 0)
            || enrollment.generation.iter().all(|byte| *byte == 0)
            || enrollment.ca_der.len() > 8
            || enrollment.ca_der.iter().map(Vec::len).sum::<usize>() > 1024 * 1024
            || enrollment.encrypted_pkcs12.is_empty()
            || enrollment.encrypted_pkcs12.len() > 4 * 1024 * 1024
            || password.0.is_empty()
            || password.0.len() > 128
            || password.0.iter().any(|byte| !(32..=126).contains(byte))
            || !matches!(
                colossus_native_browser_pki::identity_validity(enrollment.reviewed_leaf),
                Ok(Some(_))
            )
            || enrollment
                .ca_der
                .iter()
                .any(|root| colossus_native_browser_pki::ca_der(root).is_err())
        {
            return Err(denied());
        }
        let mut context = Context::new(authority, enrollment.generation);
        let ca: Vec<_> = enrollment
            .ca_der
            .iter()
            .map(|bytes| ffi::Bytes::new(bytes))
            .collect();
        let input = ffi::Input {
            parent_fd: enrollment.parent_fd.as_raw_fd(),
            parent_path: ffi::Bytes::new(parent.as_bytes()),
            origin: ffi::Bytes::new(enrollment.origin.as_bytes()),
            reviewed_leaf: ffi::Bytes::new(enrollment.reviewed_leaf),
            encrypted_pkcs12: ffi::Bytes::new(enrollment.encrypted_pkcs12),
            ca: ca.as_ptr(),
            ca_count: ca.len(),
            code_hash: enrollment.code_hash.as_ptr(),
            generation: enrollment.generation.as_ptr(),
            password: password.0.as_mut_ptr(),
            password_size: password.0.len(),
            mutations: context.callbacks(),
        };
        let mut raw = std::ptr::null_mut();
        // SAFETY: every buffer lives for this synchronous call; native Prepare
        // duplicates the parent fd and retains only copied public data/context.
        let status =
            Status::native(unsafe { ffi::colossus_mac_identity_prepare(&input, &mut raw) });
        let owner = NonNull::new(raw).map(|raw| Self {
            inner: Arc::new(Inner {
                raw,
                context,
                generation: enrollment.generation,
            }),
            leaf: enrollment.reviewed_leaf.to_vec(),
            scheme: enrollment.scheme,
        });
        match (status, owner) {
            (Status::Ready, Some(owner)) => Ok(owner),
            (status, owner) => Err(PreparationFailure { status, owner }),
        }
    }
    /// Reviewed public leaf and deliberately supported signature algorithm only.
    pub fn public_leaf(&self) -> (&[u8], Scheme) {
        (&self.leaf, self.scheme)
    }
    /// Inspect the last fixed native validation category without native effects.
    pub fn diagnostic(&self) -> Option<Diagnostic> {
        let mut result = Diagnostic {
            phase: 0,
            check: 0,
            os_status: 0,
        };
        // SAFETY: the retained native owner writes three exact scalar outputs
        // under its mutex and exposes no pointer, path, certificate or key bytes.
        let copied = unsafe {
            ffi::colossus_mac_identity_diagnostic(
                self.inner.raw.as_ptr(),
                &mut result.phase,
                &mut result.check,
                &mut result.os_status,
            )
        };
        (copied && result.phase <= 6 && result.check <= 25).then_some(result)
    }
    /// Begin one local handshake after exact decrypted request-origin admission.
    pub fn begin(
        &self,
        generation: &[u8; 32],
        origin: &str,
        leaf: &[u8; 32],
        deadline: Instant,
    ) -> Result<NativeHandshake, Status> {
        let milliseconds = deadline
            .saturating_duration_since(Instant::now())
            .as_millis();
        if milliseconds == 0 || milliseconds > 30000 {
            return Err(Status::Denied);
        }
        let mut sequence = 0;
        self.inner.signing_operation(generation, || {
            // SAFETY: live Arc-owned native context and exact fixed-size buffers.
            let status = Status::native(unsafe {
                ffi::colossus_mac_identity_begin(
                    self.inner.raw.as_ptr(),
                    generation.as_ptr(),
                    ffi::Bytes::new(origin.as_bytes()),
                    leaf.as_ptr(),
                    milliseconds as u64,
                    &mut sequence,
                )
            });
            if status != Status::Ready || sequence == 0 {
                return Err(status);
            }
            Ok(())
        })?;
        Ok(NativeHandshake {
            inner: Arc::clone(&self.inner),
            sequence: Some(sequence),
            generation: *generation,
        })
    }
    /// Revoke and drain local signing. Physical deletion requires explicit finish.
    pub fn revoke(&self) {
        self.inner.revoke();
    }
    /// Verify exact native item and filesystem removal; unknown state is retained.
    pub fn finish(&self) -> Result<(), Status> {
        self.revoke();
        // SAFETY: live native object; native Finish retains resumable state.
        let status =
            Status::native(unsafe { ffi::colossus_mac_identity_finish(self.inner.raw.as_ptr()) });
        if status == Status::Ready {
            Ok(())
        } else {
            Err(status)
        }
    }
}
impl Drop for NativeIdentity {
    fn drop(&mut self) {
        self.revoke();
    }
}
impl fmt::Debug for NativeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeIdentity").finish_non_exhaustive()
    }
}
/// A noncloneable, one-use native TLS transcript token. Drop cancels it.
pub struct NativeHandshake {
    inner: Arc<Inner>,
    sequence: Option<u64>,
    generation: [u8; 32],
}
impl NativeHandshake {
    /// The only private-key operation: fixed RFC8446 client CertificateVerify.
    pub fn certificate_verify(
        &mut self,
        scheme: Scheme,
        transcript: &[u8; 32],
    ) -> Result<Vec<u8>, Status> {
        let sequence = self.sequence.take().ok_or(Status::Denied)?;
        let mut signature = zeroize::Zeroizing::new([0_u8; 512]);
        let mut length = signature.len();
        self.inner.signing_operation(&self.generation, || {
            // SAFETY: token retains native context and all exact-size input/output
            // buffers remain live synchronously. No native key reference is returned.
            let status = Status::native(unsafe {
                ffi::colossus_mac_identity_verify(
                    self.inner.raw.as_ptr(),
                    sequence,
                    self.generation.as_ptr(),
                    scheme.native(),
                    transcript.as_ptr(),
                    signature.as_mut_ptr(),
                    &mut length,
                )
            });
            if status != Status::Ready || length == 0 || length > signature.len() {
                return Err(status);
            }
            Ok(signature[..length].to_vec())
        })
    }
}
impl Drop for NativeHandshake {
    fn drop(&mut self) {
        if let Some(sequence) = self.sequence.take() {
            // SAFETY: handshake Arc retains native owner until cancellation finishes.
            unsafe {
                ffi::colossus_mac_identity_cancel(
                    self.inner.raw.as_ptr(),
                    sequence,
                    self.generation.as_ptr(),
                )
            };
        }
        self.generation.zeroize();
    }
}
pub(crate) fn valid_origin(origin: &str) -> bool {
    url::Url::parse(origin).is_ok_and(|url| {
        origin.len() <= 4096
            && url.scheme() == "https"
            && url.origin().ascii_serialization() == origin
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && url.username().is_empty()
            && url.password().is_none()
    })
}
