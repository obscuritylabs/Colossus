use crate::ffi;
use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
};

/// Positive native operation for which the supervisor excludes foreign writers.
#[derive(Clone, Copy, Debug)]
pub enum MutationOperation {
    CreateStore,
    ImportIdentity,
    RetireStore,
    CreateNamespace,
}
/// Borrowed native descriptors and exact generation. Implementations never close
/// these descriptors or grant their namespace to Root/browser processes.
pub struct MutationRequest {
    pub generation: [u8; 32],
    pub parent_fd: i32,
    pub directory_fd: i32,
    pub sequence: u64,
    pub operation: MutationOperation,
}
/// An independent native supervisor's actual writer-exclusion mechanism. A
/// permission mode, advisory lock or caller assertion cannot satisfy this trait.
pub trait MutationAuthority: Send + Sync {
    /// Admit and hold one immutable creator-only namespace mutation window.
    fn begin(&self, request: MutationRequest) -> Option<Box<dyn MutationWindow>>;
    /// Independently revalidate the retained native parent, immutable package
    /// and exact enrollment generation before and after each signing operation.
    /// Parent loss denies fresh signing even when retirement remains admitted.
    fn signing_admitted(&self, generation: &[u8; 32]) -> bool;
}
/// Retain exclusion until drop and issue an exact positive output receipt.
pub trait MutationWindow: Send + Sync {
    /// Revalidate independent exclusion throughout the native operation.
    fn exclusive(&self) -> bool;
    /// Seal the exact borrowed output descriptors only after native success.
    fn seal(&self, sequence: u64, output_fds: &[i32]) -> bool;
    /// Persist a positively verified retirement step before publishing progress.
    fn checkpoint(&self, sequence: u64, step: RetirementStep) -> bool;
    /// Quarantine unknown import contents only when the SAME previously sealed
    /// physical descriptors and exact namespace remain independently bound.
    /// This admits retirement only; it never adopts replacements or readiness.
    fn retire_unchanged(&self, sequence: u64, previous_fds: &[i32]) -> bool;
}
/// Ordered physical retirement receipts; no signing or import authority.
#[derive(Clone, Copy, Debug)]
pub enum RetirementStep {
    NamespaceRenamed,
    StoreRemoved,
    AuxiliaryRemoved,
    DirectoryRemoved,
}
pub(crate) struct Context {
    authority: Arc<dyn MutationAuthority>,
    pub(crate) signing: crate::signing::SigningAdmission,
    window: Mutex<Option<(u64, Box<dyn MutationWindow>)>>,
}
impl Context {
    pub fn new(authority: Arc<dyn MutationAuthority>, generation: [u8; 32]) -> Box<Self> {
        Box::new(Self {
            signing: crate::signing::SigningAdmission::new(Arc::clone(&authority), generation),
            authority,
            window: Mutex::new(None),
        })
    }
    pub fn callbacks(&mut self) -> ffi::Callbacks {
        ffi::Callbacks {
            context: self as *mut Self as *mut c_void,
            begin,
            exclusive,
            seal,
            end,
            signing_admitted,
            checkpoint,
            retire_unchanged,
        }
    }
}
extern "C" fn retire_unchanged(
    raw: *mut c_void,
    sequence: u64,
    fds: *const i32,
    count: usize,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        if fds.is_null() || count != 2 {
            return false;
        }
        // SAFETY: the locally linked failed-import owner borrows exactly its
        // retained pre-import store/lock descriptor pair for this callback.
        let fds = unsafe { std::slice::from_raw_parts(fds, count) };
        context(raw)
            .and_then(|context| context.window.lock().ok())
            .is_some_and(|held| {
                held.as_ref().is_some_and(|(seq, window)| {
                    *seq == sequence && window.retire_unchanged(sequence, fds)
                })
            })
    }))
    .unwrap_or(false)
}
extern "C" fn checkpoint(raw: *mut c_void, sequence: u64, step: u32) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        let step = match step {
            0 => RetirementStep::NamespaceRenamed,
            1 => RetirementStep::StoreRemoved,
            2 => RetirementStep::AuxiliaryRemoved,
            3 => RetirementStep::DirectoryRemoved,
            _ => return false,
        };
        context(raw)
            .and_then(|context| context.window.lock().ok())
            .is_some_and(|held| {
                held.as_ref().is_some_and(|(seq, window)| {
                    *seq == sequence && window.exclusive() && window.checkpoint(sequence, step)
                })
            })
    }))
    .unwrap_or(false)
}
extern "C" fn signing_admitted(raw: *mut c_void, generation: *const u8) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        if generation.is_null() {
            return false;
        }
        // SAFETY: the locally linked owner borrows its live fixed32-byte
        // generation synchronously and retains no pointer after this callback.
        let generation: &[u8; 32] = unsafe { &*generation.cast::<[u8; 32]>() };
        context(raw).is_some_and(|context| context.signing.admit(generation))
    }))
    .unwrap_or(false)
}
fn context(raw: *mut c_void) -> Option<&'static Context> {
    // SAFETY: only the locally linked bridge calls these callbacks. NativeIdentity
    // owns this stable Box until after the bridge and every window are destroyed.
    unsafe { (raw as *const Context).as_ref() }
}
extern "C" fn begin(
    raw: *mut c_void,
    generation: *const u8,
    parent_fd: i32,
    directory_fd: i32,
    sequence: u64,
    operation: u32,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(context) = context(raw) else {
            return false;
        };
        if generation.is_null()
            || sequence == 0
            || parent_fd < 0
            || (operation == 3 && directory_fd != -1)
            || (operation != 3 && directory_fd < 0)
        {
            return false;
        }
        let operation = match operation {
            0 => MutationOperation::CreateStore,
            1 => MutationOperation::ImportIdentity,
            2 => MutationOperation::RetireStore,
            3 => MutationOperation::CreateNamespace,
            _ => return false,
        };
        // SAFETY: native Begin supplies its live fixed-size generation array.
        let generation = unsafe { std::slice::from_raw_parts(generation, 32) }
            .try_into()
            .unwrap_or([0; 32]);
        let Ok(mut held) = context.window.lock() else {
            return false;
        };
        if held.is_some() {
            return false;
        }
        let Some(window) = context.authority.begin(MutationRequest {
            generation,
            parent_fd,
            directory_fd,
            sequence,
            operation,
        }) else {
            return false;
        };
        if !window.exclusive() {
            return false;
        }
        *held = Some((sequence, window));
        true
    }))
    .unwrap_or(false)
}
extern "C" fn exclusive(raw: *mut c_void, sequence: u64) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        context(raw)
            .and_then(|context| context.window.lock().ok())
            .is_some_and(|held| {
                held.as_ref()
                    .is_some_and(|(seq, window)| *seq == sequence && window.exclusive())
            })
    }))
    .unwrap_or(false)
}
extern "C" fn seal(raw: *mut c_void, sequence: u64, fds: *const i32, count: usize) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        if fds.is_null() || !(1..=2).contains(&count) {
            return false;
        }
        // SAFETY: native preparation passes one namespace descriptor or exactly
        // two store/lock descriptors, all live for this synchronous callback.
        let fds = unsafe { std::slice::from_raw_parts(fds, count) };
        context(raw)
            .and_then(|context| context.window.lock().ok())
            .is_some_and(|held| {
                held.as_ref().is_some_and(|(seq, window)| {
                    *seq == sequence && window.exclusive() && window.seal(sequence, fds)
                })
            })
    }))
    .unwrap_or(false)
}
extern "C" fn end(raw: *mut c_void, sequence: u64) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(context) = context(raw)
            && let Ok(mut held) = context.window.lock()
            && held.as_ref().is_some_and(|(seq, _)| *seq == sequence)
        {
            held.take();
        }
    }));
}

fn public_bytes(raw: *const u8, length: usize, maximum: usize) -> Option<&'static [u8]> {
    if raw.is_null() || length == 0 || length > maximum {
        return None;
    }
    // SAFETY: locally linked validators borrow the exact native public input
    // range synchronously. They retain no references after returning.
    Some(unsafe { std::slice::from_raw_parts(raw, length) })
}
#[unsafe(no_mangle)]
extern "C" fn colossus_mac_validate_ca(raw: *const u8, length: usize) -> bool {
    public_bytes(raw, length, 1024 * 1024)
        .is_some_and(|bytes| colossus_native_browser_pki::ca_der(bytes).is_ok())
}
#[unsafe(no_mangle)]
extern "C" fn colossus_mac_validate_identity(raw: *const u8, length: usize) -> bool {
    public_bytes(raw, length, 65536).is_some_and(|bytes| {
        matches!(
            colossus_native_browser_pki::identity_validity(bytes),
            Ok(Some(_))
        )
    })
}
#[unsafe(no_mangle)]
extern "C" fn colossus_mac_validate_origin(raw: *const u8, length: usize) -> bool {
    public_bytes(raw, length, 4096)
        .is_some_and(|bytes| std::str::from_utf8(bytes).is_ok_and(crate::owner::valid_origin))
}
