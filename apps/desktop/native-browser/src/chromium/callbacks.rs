//! Bounded callback copies and generation fencing. No page receives host objects.

use std::{
    collections::HashMap,
    ffi::{c_char, c_void},
    sync::{Mutex, OnceLock},
};

use tokio::sync::oneshot;

use crate::{BrowserEvent, EventSink, NavigationPolicy, PageState};

use super::ffi;

const MAX_STATE_BYTES: usize = 32 * 1024;
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
}

pub(super) fn entries() -> &'static Mutex<HashMap<u64, Entry>> {
    static ENTRIES: OnceLock<Mutex<HashMap<u64, Entry>>> = OnceLock::new();
    ENTRIES.get_or_init(Mutex::default)
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
    _: u64,
    _: u64,
    _: *const c_char,
    _: usize,
    _: *const ffi::Certificate,
    _: usize,
) -> i32 {
    // Native origin-bound selection must be proven before enabling mTLS. Never
    // fall through to an unrestricted Chromium or platform identity chooser.
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

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
                abandoned: false,
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
