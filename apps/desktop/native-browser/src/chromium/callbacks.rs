//! Bounded callback copies and generation fencing. No page receives host objects.

use std::{
    collections::HashMap,
    ffi::{c_char, c_void},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::sync::oneshot;

use crate::{BrowserError, BrowserEvent, EventSink, NavigationPolicy, PageState};

use super::ffi;

const MAX_STATE_BYTES: usize = 32 * 1024;
type QuitHandler = Arc<dyn Fn() + Send + Sync>;
static QUIT_HANDLER: OnceLock<Mutex<Option<QuitHandler>>> = OnceLock::new();
static QUIT_PENDING: AtomicBool = AtomicBool::new(false);

pub(super) fn bind_quit_handler(
    handler: impl Fn() + Send + Sync + 'static,
) -> Result<(), BrowserError> {
    let handler: QuitHandler = Arc::new(handler);
    {
        let mut registered = QUIT_HANDLER
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| BrowserError::Unavailable)?;
        if registered.is_some() {
            return Err(BrowserError::Unavailable);
        }
        *registered = Some(handler.clone());
    }
    if QUIT_PENDING.swap(false, Ordering::AcqRel) {
        handler();
    }
    Ok(())
}

pub(super) fn release_quit_handler() {
    if let Some(handler) = QUIT_HANDLER.get()
        && let Ok(mut handler) = handler.lock()
    {
        *handler = None;
    }
}

fn request_quit() {
    let handler = QUIT_HANDLER
        .get()
        .and_then(|handler| handler.lock().ok())
        .and_then(|handler| handler.clone());
    if let Some(handler) = handler {
        handler();
    } else {
        QUIT_PENDING.store(true, Ordering::Release);
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageUpdate {
    url: Option<String>,
    title: Option<String>,
    can_go_back: Option<bool>,
    can_go_forward: Option<bool>,
    loading: Option<bool>,
}
pub(super) struct Entry {
    pub generation: u64,
    pub policy: NavigationPolicy,
    pub sink: EventSink,
    pub page: PageState,
    pub created: Option<oneshot::Sender<()>>,
    pub closed: Option<oneshot::Sender<()>>,
    pub inspection: Option<oneshot::Sender<PageState>>,
    pub abandoned: bool,
    pub identity_epoch: u64,
    pub identity_request: Option<crate::pki::IdentityRequest>,
    #[cfg(feature = "native-test-driver")]
    pub acceptance: Option<oneshot::Sender<Result<super::AcceptanceProbe, crate::BrowserError>>>,
}

pub(super) fn entries() -> &'static Mutex<HashMap<u64, Entry>> {
    static ENTRIES: OnceLock<Mutex<HashMap<u64, Entry>>> = OnceLock::new();
    ENTRIES.get_or_init(Mutex::default)
}

/// Consuming this exact review is the dispatch point shared with revocation.
pub(super) fn consume_identity(
    entry: &mut Entry,
    generation: u64,
    review: &crate::pki::IdentityRequest,
    fingerprint: Option<&str>,
) -> Result<i32, BrowserError> {
    if entry.generation != generation || entry.abandoned || entry.closed.is_some() {
        return Err(BrowserError::Closed);
    }
    let pending = entry
        .identity_request
        .as_ref()
        .filter(|pending| {
            pending.id == review.id
                && pending.epoch == review.epoch
                && pending.epoch == entry.identity_epoch
                && pending.origin == review.origin
                && !pending.is_expired()
        })
        .ok_or(BrowserError::Closed)?;
    let index = fingerprint.map_or(Ok(-1), |fingerprint| {
        pending
            .candidate_index(fingerprint)
            .ok_or(BrowserError::Unavailable)
    })?;
    entry.identity_request = None;
    Ok(index)
}

pub(super) fn callbacks() -> ffi::Callbacks {
    ffi::Callbacks {
        owner: std::ptr::null_mut(),
        event,
        allow_url,
        select_identity,
        schedule_pump: None,
    }
}

fn cancel_identity(entry: &mut Entry, payload: *const u8, length: usize) {
    if payload.is_null() || length != std::mem::size_of::<u64>() {
        return;
    }
    let mut request = [0; 8];
    // SAFETY: Native ABI passes the exact bounded borrowed request ID.
    request.copy_from_slice(unsafe { std::slice::from_raw_parts(payload, length) });
    if entry
        .identity_request
        .as_ref()
        .is_some_and(|pending| pending.id == u64::from_ne_bytes(request))
    {
        entry.identity_request = None;
    }
}

#[cfg(feature = "native-test-driver")]
fn complete_acceptance(entry: &mut Entry, success: bool, evidence: Option<&[u8]>) {
    if let Some(sender) = entry.acceptance.take() {
        let result = evidence
            .filter(|_| success)
            .ok_or(BrowserError::Unavailable)
            .and_then(|evidence| {
                serde_json::from_slice(evidence).map_err(|_| BrowserError::Unavailable)
            });
        let _ = sender.send(result);
    }
}

unsafe extern "C" fn event(
    _: *mut c_void,
    tab: u64,
    generation: u64,
    event: u32,
    _: i32,
    success: i32,
    payload: *const u8,
    length: usize,
) {
    // Native callers own the payload during this callback. All parsing/copies
    // are bounded, and panics never unwind into Chromium.
    let _ = std::panic::catch_unwind(|| {
        // Native application Quit is independent of any tab. Pages cannot
        // generate this event, and the trusted host binds its exit callback.
        if event == 17 && tab == 0 && generation == 0 {
            eprintln!("native AppKit Quit callback received");
            request_quit();
            return;
        }
        let Ok(mut entries) = entries().lock() else {
            return;
        };
        let Some(entry) = entries
            .get_mut(&tab)
            .filter(|entry| entry.generation == generation)
        else {
            return;
        };
        let mut emit = None;
        match event {
            1 => {
                if let Some(sender) = entry.created.take() {
                    let _ = sender.send(());
                }
            }
            2 => {
                entry.page.loading = success != 0;
                emit = Some(BrowserEvent::Loading(success != 0));
            }
            3 if !payload.is_null() && length <= MAX_STATE_BYTES => {
                // SAFETY: Shim guarantees a valid borrowed buffer of `length` bytes.
                let bytes = unsafe { std::slice::from_raw_parts(payload, length) };
                if let Ok(page) = serde_json::from_slice::<PageUpdate>(bytes) {
                    if page.url.as_ref().is_some_and(|url| url.len() > 8_192)
                        || page.title.as_ref().is_some_and(|title| title.len() > 4_096)
                    {
                        return;
                    }
                    if let Some(url) = page.url {
                        entry.page.url = if entry.policy.allows(&url) {
                            url
                        } else {
                            String::new()
                        };
                    }
                    if let Some(title) = page.title {
                        entry.page.title = title;
                    }
                    if let Some(value) = page.can_go_back {
                        entry.page.can_go_back = value;
                    }
                    if let Some(value) = page.can_go_forward {
                        entry.page.can_go_forward = value;
                    }
                    if let Some(value) = page.loading {
                        entry.page.loading = value;
                    }
                    if let Some(sender) = entry.inspection.take() {
                        let _ = sender.send(entry.page.clone());
                    }
                }
            }
            4 => emit = Some(BrowserEvent::Failed),
            5 | 13 | 15 => emit = Some(BrowserEvent::Crashed),
            6 => emit = Some(BrowserEvent::Blocked),
            7 => emit = Some(BrowserEvent::Download),
            8 if !payload.is_null() && length <= 8_192 => {
                // SAFETY: Shim guarantees callback buffer lifetime and length.
                if let Ok(url) =
                    std::str::from_utf8(unsafe { std::slice::from_raw_parts(payload, length) })
                {
                    emit = Some(BrowserEvent::Popup(url.to_owned()));
                }
            }
            9 => emit = Some(BrowserEvent::TlsFailed),
            10 => {
                if let Some(sender) = entry.closed.take() {
                    let _ = sender.send(());
                }
                // OnBeforeClose is terminal even if the original waiter timed
                // out. Removal lets a later native-close reconciliation succeed.
                entries.remove(&tab);
                return;
            }
            14 => emit = Some(BrowserEvent::AuthenticationRequired),
            18 => cancel_identity(entry, payload, length),
            #[cfg(feature = "native-test-driver")]
            16 => {
                let evidence = if !payload.is_null() && length <= MAX_STATE_BYTES {
                    // SAFETY: The shim supplies the bounded borrowed evidence buffer.
                    Some(unsafe { std::slice::from_raw_parts(payload, length) })
                } else {
                    None
                };
                complete_acceptance(entry, success != 0, evidence);
            }
            _ => {}
        }
        let sink = entry.sink.clone();
        drop(entries);
        if let Some(event) = emit {
            sink(event);
        }
    });
}

unsafe extern "C" fn allow_url(
    _: *mut c_void,
    tab: u64,
    generation: u64,
    url: *const c_char,
    length: usize,
    _: i32,
) -> i32 {
    std::panic::catch_unwind(|| {
        if url.is_null() || length > 8_192 {
            return 0;
        }
        // SAFETY: Shim supplies a valid, bounded borrowed URL buffer.
        let Ok(url) =
            std::str::from_utf8(unsafe { std::slice::from_raw_parts(url.cast(), length) })
        else {
            return 0;
        };
        // Clone the authority reference under one tiny critical section. URL
        // parsing/policy work never holds the lifetime-event registry lock.
        let policy = {
            let Ok(entries) = entries().lock() else {
                return 0;
            };
            entries
                .get(&tab)
                .filter(|entry| entry.generation == generation && !entry.abandoned)
                .map(|entry| entry.policy.clone())
        };
        i32::from(policy.is_some_and(|policy| policy.allows(url)))
    })
    .unwrap_or(0)
}

unsafe extern "C" fn select_identity(
    _: *mut c_void,
    tab: u64,
    generation: u64,
    request_id: u64,
    origin: *const c_char,
    origin_length: usize,
    candidates: *const ffi::Certificate,
    candidate_count: usize,
) -> i32 {
    std::panic::catch_unwind(|| {
        use crate::pki::{
            IdentityRequest,
            selection::{MAX_CANDIDATES, MAX_CERTIFICATE_BYTES},
        };
        if origin.is_null()
            || origin_length > 8192
            || candidates.is_null()
            || candidate_count == 0
            || candidate_count > MAX_CANDIDATES
        {
            return -1;
        }
        // SAFETY: CEF owns these bounded borrowed origin/certificate buffers for
        // the callback lifetime; only validated public metadata is retained.
        let Ok(origin) = std::str::from_utf8(unsafe {
            std::slice::from_raw_parts(origin.cast(), origin_length)
        }) else {
            return -1;
        };
        let (policy, epoch) = {
            let Ok(entries) = entries().lock() else {
                return -1;
            };
            let Some(entry) = entries.get(&tab).filter(|entry| {
                entry.generation == generation && !entry.abandoned && entry.closed.is_none()
            }) else {
                return -1;
            };
            (entry.policy.clone(), entry.identity_epoch)
        };
        if !policy.allows(origin) {
            return -1;
        }
        let mut certificates = Vec::with_capacity(candidate_count);
        // SAFETY: Native shim supplies candidate_count valid certificate entries.
        for candidate in unsafe { std::slice::from_raw_parts(candidates, candidate_count) } {
            if candidate.der.is_null()
                || candidate.der_len == 0
                || candidate.der_len > MAX_CERTIFICATE_BYTES
            {
                return -1;
            }
            // SAFETY: The validated bounded public DER remains native-owned.
            certificates
                .push(unsafe { std::slice::from_raw_parts(candidate.der, candidate.der_len) });
        }
        let Ok(review) = IdentityRequest::new(request_id, epoch, origin, &certificates) else {
            return -1;
        };
        let Ok(mut entries) = entries().lock() else {
            return -1;
        };
        let Some(entry) = entries.get_mut(&tab).filter(|entry| {
            entry.generation == generation
                && !entry.abandoned
                && entry.closed.is_none()
                && entry.identity_epoch == epoch
        }) else {
            return -1;
        };
        entry.identity_request = Some(review);
        // Defer the native callback. No first-candidate or OS chooser fallback.
        -2
    })
    .unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_entry() -> (Entry, crate::pki::IdentityRequest, String) {
        let mut parameters = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        parameters.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
        parameters.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
        let certificate = parameters
            .self_signed(&rcgen::KeyPair::generate().unwrap())
            .unwrap();
        let review = crate::pki::IdentityRequest::new(
            41,
            9,
            "https://example.com",
            &[certificate.der().as_ref()],
        )
        .unwrap();
        let fingerprint = review.candidates[0].fingerprint_sha256.clone();
        (
            Entry {
                generation: 17,
                policy: NavigationPolicy::default(),
                sink: std::sync::Arc::new(|_| {}),
                page: PageState::default(),
                created: None,
                closed: None,
                inspection: None,
                abandoned: false,
                identity_epoch: 9,
                identity_request: Some(review.clone()),
                #[cfg(feature = "native-test-driver")]
                acceptance: None,
            },
            review,
            fingerprint,
        )
    }

    #[test]
    fn identity_review_is_single_use_and_bound_to_generation_epoch_origin_and_fingerprint() {
        let (mut entry, review, fingerprint) = review_entry();
        assert_eq!(
            consume_identity(&mut entry, 18, &review, Some(&fingerprint)),
            Err(BrowserError::Closed)
        );
        assert_eq!(
            consume_identity(&mut entry, 17, &review, Some(&"a".repeat(64))),
            Err(BrowserError::Unavailable)
        );
        let mut different_origin = review.clone();
        different_origin.origin = "https://other.example.com".into();
        assert_eq!(
            consume_identity(&mut entry, 17, &different_origin, Some(&fingerprint)),
            Err(BrowserError::Closed)
        );
        let mut replaced = review.clone();
        replaced.id += 1;
        assert_eq!(
            consume_identity(&mut entry, 17, &replaced, Some(&fingerprint)),
            Err(BrowserError::Closed)
        );
        entry.identity_epoch += 1;
        assert_eq!(
            consume_identity(&mut entry, 17, &review, Some(&fingerprint)),
            Err(BrowserError::Closed)
        );
        entry.identity_epoch -= 1;
        assert_eq!(
            consume_identity(&mut entry, 17, &review, Some(&fingerprint)),
            Ok(0)
        );
        assert!(entry.identity_request.is_none());
        assert_eq!(
            consume_identity(&mut entry, 17, &review, Some(&fingerprint)),
            Err(BrowserError::Closed)
        );
    }

    #[test]
    fn closing_or_abandoned_tabs_cannot_finish_a_dialog_and_dismissal_selects_none() {
        let (mut entry, review, fingerprint) = review_entry();
        entry.abandoned = true;
        assert_eq!(
            consume_identity(&mut entry, 17, &review, Some(&fingerprint)),
            Err(BrowserError::Closed)
        );
        entry.abandoned = false;
        let (send, _) = oneshot::channel();
        entry.closed = Some(send);
        assert_eq!(
            consume_identity(&mut entry, 17, &review, Some(&fingerprint)),
            Err(BrowserError::Closed)
        );
        entry.closed = None;
        assert_eq!(consume_identity(&mut entry, 17, &review, None), Ok(-1));
        assert!(entry.identity_request.is_none());
    }

    #[test]
    fn delayed_close_reconciles_after_waiter_cancellation_and_fences_generation() {
        let tab = u64::MAX;
        let (sender, receiver) = oneshot::channel();
        drop(receiver);
        entries().lock().unwrap().insert(
            tab,
            Entry {
                generation: 17,
                policy: NavigationPolicy::default(),
                sink: std::sync::Arc::new(|_| {}),
                page: PageState::default(),
                created: None,
                closed: Some(sender),
                inspection: None,
                identity_epoch: 0,
                identity_request: None,
                abandoned: false,
                #[cfg(feature = "native-test-driver")]
                acceptance: None,
            },
        );
        // SAFETY: Callback takes no payload for CLOSED and pointer is null.
        unsafe {
            event(std::ptr::null_mut(), tab, 16, 10, 0, 1, std::ptr::null(), 0);
        }
        assert!(entries().lock().unwrap().contains_key(&tab));
        unsafe {
            event(std::ptr::null_mut(), tab, 17, 10, 0, 1, std::ptr::null(), 0);
        }
        assert!(!entries().lock().unwrap().contains_key(&tab));
    }
}
