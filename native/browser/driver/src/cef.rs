//! Main-thread CEF ownership and a closed set of native DevTools translations.
use std::{
    collections::{HashMap, HashSet},
    ffi::{CString, c_char, c_void},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use colossus_contracts::{
    BrowserAction, BrowserElementId, BrowserElementRef, BrowserKey, BrowserObservation,
    BrowserOrigin, BrowserSessionId, BrowserSnapshotId, BrowserTabId, BrowserTabSummary,
    BrowserWaitCondition,
};
use colossus_ports::{
    BrowserDriverCommand, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
};
use serde_json::{Value, json};

use crate::{ffi, identity, semantic};

mod capture;
mod presentation;
mod transfer;

#[derive(Default)]
struct State {
    created: HashSet<u64>,
    generations: HashMap<u64, u64>,
    closed: HashSet<u64>,
    loading: HashSet<u64>,
    failed: HashSet<u64>,
    metadata: HashMap<u64, (Option<BrowserOrigin>, String, u64)>,
    completed: HashMap<u64, u64>,
    addresses: HashMap<u64, String>,
    history: HashMap<u64, (bool, bool)>,
    expected: Option<i32>,
    result: Option<Result<Value, BrowserDriverError>>,
}
pub struct Callbacks {
    state: Mutex<State>,
    origins: Mutex<Vec<BrowserOrigin>>,
    identity: Mutex<identity::Policy>,
    identity_revoked: AtomicBool,
    identity_cancellation: Mutex<Option<Arc<AtomicBool>>>,
}
impl Default for Callbacks {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            origins: Mutex::new(Vec::new()),
            identity: Mutex::new(identity::Policy::default()),
            identity_revoked: AtomicBool::new(false),
            identity_cancellation: Mutex::new(None),
        }
    }
}
impl Callbacks {
    pub fn configure_identity(
        &self,
        policy: identity::Policy,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(), BrowserDriverError> {
        *self
            .identity
            .lock()
            .map_err(|_| BrowserDriverError::Failed)? = policy;
        *self
            .identity_cancellation
            .lock()
            .map_err(|_| BrowserDriverError::Failed)? = Some(cancelled);
        Ok(())
    }
    fn identity_cancelled(&self) -> bool {
        self.identity_cancellation.lock().map_or(true, |cancelled| {
            cancelled
                .as_ref()
                .is_none_or(|cancelled| cancelled.load(Ordering::Acquire))
        })
    }
    pub fn ffi(&self) -> ffi::Callbacks {
        ffi::Callbacks {
            owner: std::ptr::from_ref(self).cast_mut().cast(),
            event,
            allow_url,
            select_identity: Some(select_identity),
            schedule_pump: None,
        }
    }
}

unsafe extern "C" fn event(
    owner: *mut c_void,
    tab: u64,
    generation: u64,
    kind: u32,
    command: i32,
    success: i32,
    bytes: *const u8,
    len: usize,
) {
    // SAFETY: bootstrap retains the boxed callbacks until CEF shutdown completes.
    let Some(callbacks) = (unsafe { owner.cast::<Callbacks>().as_ref() }) else {
        return;
    };
    let Ok(mut state) = callbacks.state.lock() else {
        return;
    };
    if len > 16 * 1024 * 1024 || (len > 0 && bytes.is_null()) {
        state.failed.insert(tab);
        return;
    }
    let payload = if len == 0 {
        &[]
    } else {
        // SAFETY: C ABI borrows valid bytes for exactly this callback, copied/parsed here.
        unsafe { std::slice::from_raw_parts(bytes, len) }
    };
    match kind {
        1 => {
            state.created.insert(tab);
            state.generations.insert(tab, generation);
        }
        2 => {
            if success != 0 {
                if state.loading.insert(tab) {
                    let metadata = state.metadata.entry(tab).or_default();
                    metadata.2 = metadata.2.saturating_add(1);
                }
            } else {
                state.loading.remove(&tab);
                *state.completed.entry(tab).or_default() += 1;
            }
        }
        3 if len <= 16 * 1024 => {
            if let Ok(value) = serde_json::from_slice::<Value>(payload) {
                let metadata = state.metadata.entry(tab).or_default();
                if let Some(url) = value.get("url").and_then(Value::as_str) {
                    let origin = colossus_contracts::BrowserUrl::parse(url)
                        .ok()
                        .map(|url| url.origin());
                    if origin != metadata.0 {
                        metadata.2 = metadata.2.saturating_add(1);
                    }
                    metadata.0 = origin;
                    if url.len() <= 4096 {
                        state.addresses.insert(tab, url.to_owned());
                    }
                }
                let metadata = state.metadata.entry(tab).or_default();
                if let Some(title) = value.get("title").and_then(Value::as_str) {
                    metadata.1 = semantic::bounded(title, 1024);
                }
                if let (Some(back), Some(forward)) = (
                    value.get("canGoBack").and_then(Value::as_bool),
                    value.get("canGoForward").and_then(Value::as_bool),
                ) {
                    state.history.insert(tab, (back, forward));
                }
            }
        }
        4 | 5 | 9 | 13 | 15 => {
            state.failed.insert(tab);
        }
        10 => {
            state.closed.insert(tab);
            state.created.remove(&tab);
            state.generations.remove(&tab);
            state.loading.remove(&tab);
        }
        11 if state.expected == Some(command) => {
            state.result = Some(if success != 0 {
                serde_json::from_slice(payload).map_err(|_| BrowserDriverError::Failed)
            } else {
                Err(BrowserDriverError::Failed)
            });
        }
        #[cfg(all(target_os = "linux", feature = "native-custody-test"))]
        19 if state.generations.get(&tab) == Some(&generation) && payload.len() == 1 => {
            if payload == [1] && success == 1 {
                eprintln!("COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_DENIED");
            } else {
                eprintln!("COLOSSUS_NATIVE_NSS_OPEN_TEST_V1_FAILED");
            }
        }
        _ => {}
    }
}
unsafe extern "C" fn allow_url(
    owner: *mut c_void,
    _tab: u64,
    _generation: u64,
    bytes: *const c_char,
    len: usize,
    _navigation: i32,
) -> i32 {
    if bytes.is_null() || len > 4096 {
        return 0;
    }
    // SAFETY: native CEF owns owner until shutdown and borrows the URL for this callback.
    let Some(callbacks) = (unsafe { owner.cast::<Callbacks>().as_ref() }) else {
        return 0;
    };
    // SAFETY: non-null bounded C ABI URL bytes remain borrowed during this callback.
    let Ok(url) =
        std::str::from_utf8(unsafe { std::slice::from_raw_parts(bytes.cast::<u8>(), len) })
    else {
        return 0;
    };
    let Ok(url) = colossus_contracts::BrowserUrl::parse(url) else {
        return 0;
    };
    i32::from(
        callbacks
            .origins
            .lock()
            .is_ok_and(|origins| origins.contains(&url.origin())),
    )
}

unsafe extern "C" fn select_identity(
    owner: *mut c_void,
    tab: u64,
    generation: u64,
    request_id: u64,
    origin: *const c_char,
    origin_len: usize,
    certificates: *const ffi::Certificate,
    count: usize,
) -> i32 {
    if owner.is_null()
        || origin.is_null()
        || origin_len > 4096
        || certificates.is_null()
        || count == 0
        || count > 64
        || request_id == 0
    {
        return -1;
    }
    // SAFETY: CEF holds the boxed native callback owner through shutdown.
    let callbacks = unsafe { &*owner.cast::<Callbacks>() };
    if callbacks.identity_revoked.load(Ordering::Acquire)
        || callbacks.identity_cancelled()
        || !callbacks.state.lock().is_ok_and(|state| {
            state.generations.get(&tab) == Some(&generation)
                && !state.failed.contains(&tab)
                && !state.closed.contains(&tab)
        })
    {
        return -1;
    }
    // SAFETY: the native ABI borrows bounded origin bytes for this callback.
    let Ok(origin) =
        std::str::from_utf8(unsafe { std::slice::from_raw_parts(origin.cast::<u8>(), origin_len) })
    else {
        return -1;
    };
    let Ok(origin) = BrowserOrigin::parse(origin) else {
        return -1;
    };
    if !callbacks
        .origins
        .lock()
        .is_ok_and(|allowed| allowed.contains(&origin))
    {
        return -1;
    }
    // SAFETY: native CEF borrows exactly count public certificates for the callback.
    let certificates = unsafe { std::slice::from_raw_parts(certificates, count) };
    let mut public = Vec::with_capacity(count);
    for certificate in certificates {
        if certificate.der.is_null() || certificate.der_len == 0 || certificate.der_len > 65_536 {
            return -1;
        }
        // SAFETY: validated native public DER pointer/length, never retained after callback.
        public.push(unsafe { std::slice::from_raw_parts(certificate.der, certificate.der_len) });
    }
    let selected = callbacks
        .identity
        .lock()
        .ok()
        .and_then(|policy| policy.select(&origin, &public))
        .unwrap_or(-1);
    if callbacks.identity_cancelled() {
        -1
    } else {
        selected
    }
}

struct Tab {
    native: u64,
    summary: BrowserTabSummary,
    revision: u64,
    native_document: u64,
    snapshot: Option<BrowserSnapshotId>,
    elements: HashMap<BrowserElementId, i32>,
}
pub struct Host {
    callbacks: Box<Callbacks>,
    cancelled: Arc<AtomicBool>,
    session: Option<BrowserSessionId>,
    tabs: HashMap<BrowserTabId, Tab>,
    next_tab: u64,
    next_command: i32,
    initialized: bool,
    expected_revision: Option<(u64, u64, u64)>,
    presentation: presentation::State,
    screenshot: Option<capture::Transfer>,
    transfer: transfer::State,
}
impl Host {
    pub fn new(callbacks: Box<Callbacks>, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            callbacks,
            cancelled,
            session: None,
            tabs: HashMap::new(),
            next_tab: 1,
            next_command: 1,
            initialized: true,
            expected_revision: None,
            presentation: presentation::State::default(),
            screenshot: None,
            transfer: transfer::State::default(),
        }
    }
    fn pump(&self) -> Result<(), BrowserDriverError> {
        #[cfg(target_os = "macos")]
        // SAFETY: the dedicated Host is owned only by CEF's bootstrap main
        // thread. This entry refuses a Desktop/Tauri application delegate.
        if unsafe { ffi::colossus_cef_standalone_platform_pump() } != 0 {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        // SAFETY: Host is owned and called only on the main thread that bootstrapped CEF.
        if unsafe { ffi::colossus_cef_pump() } == 0 {
            Ok(())
        } else {
            Err(BrowserDriverError::OutcomeUnknown)
        }
    }
    fn state<T>(&self, read: impl FnOnce(&State) -> T) -> Result<T, BrowserDriverError> {
        self.callbacks
            .state
            .lock()
            .map(|state| read(&state))
            .map_err(|_| BrowserDriverError::OutcomeUnknown)
    }
    fn interrupted(&self, control: &BrowserDriverControl) -> bool {
        self.cancelled.load(Ordering::Acquire) || control.is_cancelled()
    }
    fn until(
        &self,
        control: Option<&BrowserDriverControl>,
        timeout: Duration,
        mut ready: impl FnMut(&State) -> bool,
    ) -> Result<(), BrowserDriverError> {
        let deadline = Instant::now() + timeout;
        loop {
            self.pump()?;
            if self.state(&mut ready)? {
                return Ok(());
            }
            if control.is_some_and(|control| self.interrupted(control))
                || Instant::now() >= deadline
            {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn method(
        &mut self,
        tab: u64,
        method: &str,
        params: Value,
        control: &BrowserDriverControl,
    ) -> Result<Value, BrowserDriverError> {
        if self.interrupted(control) {
            return Err(BrowserDriverError::Cancelled);
        }
        self.check_document_fence()?;
        let command = self.next_command;
        self.next_command = self
            .next_command
            .checked_add(1)
            .filter(|command| *command < i32::MAX)
            .ok_or(BrowserDriverError::LimitExceeded)?;
        {
            let mut state = self
                .callbacks
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            state.expected = Some(command);
            state.result = None;
        }
        let params = serde_json::to_vec(&params).map_err(|_| BrowserDriverError::Failed)?;
        let method = CString::new(method).map_err(|_| BrowserDriverError::Failed)?;
        // SAFETY: exact private native tab and fixed method; JSON bytes remain alive for the call.
        let status = unsafe {
            ffi::colossus_cef_devtools(
                tab,
                1,
                command,
                method.as_ptr(),
                params.as_ptr(),
                params.len(),
            )
        };
        status_result(status)?;
        self.until(Some(control), Duration::from_secs(30), |state| {
            state.result.is_some() || state.failed.contains(&tab)
        })?;
        let mut state = self
            .callbacks
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        state.expected = None;
        if state.failed.contains(&tab) {
            state.result = None;
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        state.result.take().ok_or(BrowserDriverError::Failed)?
    }
    fn metadata(
        &self,
        tab: u64,
    ) -> Result<(Option<BrowserOrigin>, String, u64), BrowserDriverError> {
        self.state(|state| state.metadata.get(&tab).cloned().unwrap_or_default())
    }
    fn native_document(&self, tab: u64) -> Result<u64, BrowserDriverError> {
        let mut document = 0;
        // SAFETY: exact owned tab and bounded output storage on the CEF UI thread.
        status_result(unsafe { ffi::colossus_cef_presentation_document(tab, 1, &mut document) })?;
        if document == 0 {
            return Err(BrowserDriverError::Stale);
        }
        Ok(document)
    }
    fn check_document_fence(&self) -> Result<(), BrowserDriverError> {
        if let Some((native, revision, document)) = self.expected_revision
            && (self
                .metadata(native)
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
                .2
                != revision
                || self
                    .native_document(native)
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?
                    != document)
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        Ok(())
    }
    fn allocate(
        &mut self,
        summary: BrowserTabSummary,
        url: Option<&colossus_contracts::BrowserUrl>,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if self.tabs.len() >= 8 || self.interrupted(control) {
            return Err(BrowserDriverError::Cancelled);
        }
        let native = self.next_tab;
        self.next_tab = self
            .next_tab
            .checked_add(1)
            .ok_or(BrowserDriverError::LimitExceeded)?;
        let destination =
            CString::new(url.map_or("about:blank", colossus_contracts::BrowserUrl::as_str))
                .map_err(|_| BrowserDriverError::Denied)?;
        // SAFETY: main-thread private allocation; native parent zero is the headless placement.
        status_result(unsafe {
            ffi::colossus_cef_create(
                native,
                1,
                1,
                0,
                ffi::Bounds {
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 800,
                },
                destination.as_ptr(),
            )
        })
        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        // Record ownership before waiting so canceled/failed creation is still drained.
        let identity = summary.tab_id.clone();
        self.tabs.insert(
            identity.clone(),
            Tab {
                native,
                summary,
                revision: 0,
                native_document: 0,
                snapshot: None,
                elements: HashMap::new(),
            },
        );
        self.until(Some(control), Duration::from_secs(60), |state| {
            state.failed.contains(&native)
                || (state.created.contains(&native)
                    && !state.loading.contains(&native)
                    && state.completed.get(&native).copied().unwrap_or_default() > 0)
        })?;
        if self.state(|state| state.failed.contains(&native))? {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let (origin, title, revision) = self.metadata(native)?;
        let native_document = self.native_document(native)?;
        let tab = self
            .tabs
            .get_mut(&identity)
            .ok_or(BrowserDriverError::Failed)?;
        tab.summary.origin = origin;
        tab.summary.title = title;
        tab.revision = revision;
        tab.native_document = native_document;
        Ok(tab.summary.clone())
    }
    pub fn open(
        &mut self,
        request: BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if self.session.is_some()
            || (request.options.mode != colossus_contracts::BrowserMode::Headless
                && !(request.options.mode == colossus_contracts::BrowserMode::Embedded
                    && self.presentation.enabled))
            || request.options.allowed_origins.is_empty()
            || request.options.allowed_origins.len() > 32
        {
            return Err(BrowserDriverError::Denied);
        }
        self.presentation.human = self.presentation.input_admitted && request.run_id.is_none();
        if self.presentation.human {
            self.presentation.handoff.admit(
                request.binding.clone(),
                request.session_id.clone(),
                colossus_contracts::BrowserTarget {
                    tab_id: request.tab_id.clone(),
                    document_id: request.document_id.clone(),
                },
            );
        }
        *self
            .callbacks
            .origins
            .lock()
            .map_err(|_| BrowserDriverError::Failed)? = request.options.allowed_origins;
        self.session = Some(request.session_id);
        self.allocate(
            BrowserTabSummary {
                tab_id: request.tab_id,
                document_id: request.document_id,
                origin: None,
                title: String::new(),
            },
            request.options.initial_url.as_ref(),
            control,
        )
        .map_err(|_| BrowserDriverError::OutcomeUnknown)
    }
    fn node(
        &mut self,
        tab: u64,
        backend: i32,
        control: &BrowserDriverControl,
    ) -> Result<i32, BrowserDriverError> {
        self.method(tab, "DOM.getDocument", json!({"depth":0}), control)?;
        let result = self.method(
            tab,
            "DOM.pushNodesByBackendIdsToFrontend",
            json!({"backendNodeIds":[backend]}),
            control,
        )?;
        result
            .get("nodeIds")
            .and_then(Value::as_array)
            .and_then(|ids| ids.first())
            .and_then(Value::as_i64)
            .and_then(|id| i32::try_from(id).ok())
            .filter(|id| *id > 0)
            .ok_or(BrowserDriverError::Stale)
    }
    fn attributes(
        &mut self,
        tab: u64,
        backend: i32,
        control: &BrowserDriverControl,
    ) -> Result<Vec<String>, BrowserDriverError> {
        let node = self.node(tab, backend, control)?;
        let result = self.method(tab, "DOM.getAttributes", json!({"nodeId":node}), control)?;
        let values = result
            .get("attributes")
            .and_then(Value::as_array)
            .ok_or(BrowserDriverError::Stale)?;
        if values.len() > 256 || values.len() % 2 != 0 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        let mut retained = Vec::new();
        let mut total = 0;
        for pair in values.chunks_exact(2) {
            let key = pair[0].as_str().ok_or(BrowserDriverError::Failed)?;
            let value = pair[1].as_str().ok_or(BrowserDriverError::Failed)?;
            total += key.len() + value.len();
            if total > 32 * 1024 {
                return Err(BrowserDriverError::LimitExceeded);
            }
            if matches!(
                key.to_ascii_lowercase().as_str(),
                "type"
                    | "autocomplete"
                    | "name"
                    | "id"
                    | "contenteditable"
                    | "multiple"
                    | "disabled"
                    | "readonly"
            ) {
                if value.len() > 4096 {
                    return Err(BrowserDriverError::LimitExceeded);
                }
                retained.push(key.to_owned());
                retained.push(value.to_owned());
            }
        }
        Ok(retained)
    }
    fn backend(
        &self,
        identity: &BrowserTabId,
        element: &BrowserElementRef,
    ) -> Result<i32, BrowserDriverError> {
        let tab = self.tabs.get(identity).ok_or(BrowserDriverError::Stale)?;
        if element.document_id != tab.summary.document_id
            || tab.snapshot.as_ref() != Some(&element.snapshot_id)
        {
            return Err(BrowserDriverError::Stale);
        }
        tab.elements
            .get(&element.element_id)
            .copied()
            .ok_or(BrowserDriverError::Stale)
    }
    fn focused_backend(
        &mut self,
        tab: u64,
        control: &BrowserDriverControl,
    ) -> Result<Option<i32>, BrowserDriverError> {
        let tree = self.method(tab, "Accessibility.getFullAXTree", json!({}), control)?;
        let nodes = tree
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or(BrowserDriverError::Failed)?;
        if nodes.len() > 16_384 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        let mut focused = None;
        for node in nodes {
            if node.pointer("/role/value").and_then(Value::as_str) == Some("RootWebArea") {
                continue;
            }
            let is_focused = node
                .get("properties")
                .and_then(Value::as_array)
                .is_some_and(|properties| {
                    properties.iter().any(|property| {
                        property.get("name").and_then(Value::as_str) == Some("focused")
                            && property.pointer("/value/value").and_then(Value::as_bool)
                                == Some(true)
                    })
                });
            if is_focused {
                let backend = node
                    .get("backendDOMNodeId")
                    .and_then(Value::as_i64)
                    .and_then(|backend| i32::try_from(backend).ok())
                    .filter(|backend| *backend > 0)
                    .ok_or(BrowserDriverError::Denied)?;
                if focused.replace(backend).is_some() {
                    return Err(BrowserDriverError::Denied);
                }
            }
        }
        Ok(focused)
    }
    fn editable(
        &mut self,
        tab: u64,
        backend: i32,
        control: &BrowserDriverControl,
    ) -> Result<(), BrowserDriverError> {
        let attributes = self.attributes(tab, backend, control)?;
        if semantic::protected(&attributes)
            || attribute(&attributes, "disabled").is_some()
            || attribute(&attributes, "readonly").is_some()
        {
            return Err(BrowserDriverError::Denied);
        }
        let node = self.method(
            tab,
            "DOM.describeNode",
            json!({"backendNodeId":backend,"depth":0}),
            control,
        )?;
        let name = node
            .pointer("/node/nodeName")
            .and_then(Value::as_str)
            .ok_or(BrowserDriverError::Stale)?;
        let ordinary = name == "TEXTAREA"
            || (name == "INPUT"
                && matches!(
                    attribute(&attributes, "type").unwrap_or("text"),
                    "text" | "search" | "email" | "url" | "tel" | "number"
                ))
            || attribute(&attributes, "contenteditable") == Some("true");
        if !ordinary {
            return Err(BrowserDriverError::Unsupported);
        }
        Ok(())
    }
    fn key(
        &mut self,
        tab: u64,
        key: &str,
        code: &str,
        modifiers: u8,
        control: &BrowserDriverControl,
    ) -> Result<(), BrowserDriverError> {
        let virtual_key = match key {
            "Enter" => 13,
            "Tab" => 9,
            "Escape" => 27,
            "Backspace" => 8,
            "Delete" => 46,
            "ArrowUp" => 38,
            "ArrowDown" => 40,
            "ArrowLeft" => 37,
            "ArrowRight" => 39,
            "Home" => 36,
            "End" => 35,
            "PageUp" => 33,
            "PageDown" => 34,
            " " => 32,
            "a" => 65,
            _ => return Err(BrowserDriverError::Denied),
        };
        self.method(
            tab,
            "Input.dispatchKeyEvent",
            json!({"type":"keyDown","key":key,"code":code,"modifiers":modifiers,"windowsVirtualKeyCode":virtual_key}),
            control,
        )?;
        self.method(
            tab,
            "Input.dispatchKeyEvent",
            json!({"type":"keyUp","key":key,"code":code,"modifiers":modifiers,"windowsVirtualKeyCode":virtual_key}),
            control,
        )?;
        Ok(())
    }

    fn select(
        &mut self,
        native: u64,
        backend: i32,
        values: &[String],
        control: &BrowserDriverControl,
    ) -> Result<(), BrowserDriverError> {
        if values.len() != 1 || values[0].len() > 1024 {
            return Err(BrowserDriverError::Unsupported);
        }
        let attributes = self.attributes(native, backend, control)?;
        if semantic::protected(&attributes)
            || attributes
                .chunks_exact(2)
                .any(|pair| pair[0].eq_ignore_ascii_case("disabled"))
        {
            return Err(BrowserDriverError::Denied);
        }
        if attributes
            .chunks_exact(2)
            .any(|pair| pair[0].eq_ignore_ascii_case("multiple"))
        {
            return Err(BrowserDriverError::Unsupported);
        }
        let described = self.method(
            native,
            "DOM.describeNode",
            json!({"backendNodeId":backend,"depth":4,"pierce":false}),
            control,
        )?;
        let node = described.get("node").ok_or(BrowserDriverError::Stale)?;
        if node.get("nodeName").and_then(Value::as_str) != Some("SELECT") {
            return Err(BrowserDriverError::Unsupported);
        }
        let mut options = Vec::new();
        collect_options(node, false, &mut options)?;
        let chosen = options
            .iter()
            .find(|option| option.value == values[0])
            .ok_or(BrowserDriverError::Denied)?;
        if chosen.disabled {
            return Err(BrowserDriverError::Denied);
        }
        let desired = chosen.backend;
        let arrows = options
            .iter()
            .filter(|option| !option.disabled)
            .position(|option| option.backend == desired)
            .ok_or(BrowserDriverError::Denied)?;
        self.method(
            native,
            "DOM.scrollIntoViewIfNeeded",
            json!({"backendNodeId":backend}),
            control,
        )?;
        self.method(
            native,
            "DOM.focus",
            json!({"backendNodeId":backend}),
            control,
        )?;
        if semantic::protected(&self.attributes(native, backend, control)?) {
            return Err(BrowserDriverError::Denied);
        }
        self.key(native, "Home", "Home", 0, control)?;
        for _ in 0..arrows {
            self.key(native, "ArrowDown", "ArrowDown", 0, control)?;
        }
        self.key(native, "Enter", "Enter", 0, control)?;
        let captured = self.method(
            native,
            "DOMSnapshot.captureSnapshot",
            json!({"computedStyles":[]}),
            control,
        )?;
        let documents = captured
            .get("documents")
            .and_then(Value::as_array)
            .ok_or(BrowserDriverError::OutcomeUnknown)?;
        let mut selected = HashSet::new();
        for document in documents {
            let nodes = document
                .get("nodes")
                .ok_or(BrowserDriverError::OutcomeUnknown)?;
            let backends = nodes
                .get("backendNodeId")
                .and_then(Value::as_array)
                .ok_or(BrowserDriverError::OutcomeUnknown)?;
            let indices = nodes
                .get("optionSelected")
                .and_then(|selected| selected.get("index"))
                .and_then(Value::as_array);
            for index in indices.into_iter().flatten() {
                let selected_backend = index
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok())
                    .and_then(|index| backends.get(index))
                    .and_then(Value::as_i64)
                    .and_then(|backend| i32::try_from(backend).ok())
                    .ok_or(BrowserDriverError::OutcomeUnknown)?;
                selected.insert(selected_backend);
            }
        }
        if !selected.contains(&desired)
            || options
                .iter()
                .any(|option| option.backend != desired && selected.contains(&option.backend))
        {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        Ok(())
    }
    pub fn execute(
        &mut self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.screenshot = None;
        self.transfer.upload = None;
        self.transfer.download = None;
        if self.session.as_ref() != Some(&command.session_id) || self.interrupted(control) {
            return Err(BrowserDriverError::Cancelled);
        }
        self.revoke_human_presentation();
        self.presentation.control_generation = command.control_generation;
        self.presentation.handoff.revoke();
        let identity = command.target.tab_id.clone();
        let (native, revision, native_document, document) = self
            .tabs
            .get(&identity)
            .map(|tab| {
                (
                    tab.native,
                    tab.revision,
                    tab.native_document,
                    tab.summary.document_id.clone(),
                )
            })
            .ok_or(BrowserDriverError::Stale)?;
        if self.state(|state| state.failed.contains(&native))? {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        let actual_native_document = self.native_document(native)?;
        let actual_revision = self.metadata(native)?.2;
        if document != command.target.document_id {
            return Err(BrowserDriverError::Stale);
        }
        let recovering_snapshot = matches!(command.action, BrowserAction::Snapshot { .. });
        let document = if recovering_snapshot {
            crate::document::snapshot_document(
                &command.action,
                &document,
                &command.target.document_id,
                &command.next_document_id,
                actual_native_document != native_document || actual_revision != revision,
            )?
        } else {
            if actual_native_document != native_document {
                return Err(BrowserDriverError::Stale);
            }
            document
        };
        if actual_revision != revision
            && !recovering_snapshot
            && !matches!(
                command.action,
                BrowserAction::Navigate { .. }
                    | BrowserAction::Back {}
                    | BrowserAction::Forward {}
                    | BrowserAction::Reload {}
                    | BrowserAction::Stop {}
            )
        {
            return Err(BrowserDriverError::Stale);
        }
        self.expected_revision = Some((native, actual_revision, actual_native_document));
        let mut snapshot = None;
        let mut selected = identity.clone();
        match &command.action {
            BrowserAction::Screenshot { .. }
            | BrowserAction::Upload { .. }
            | BrowserAction::Download { .. } => return Err(BrowserDriverError::Unsupported),
            BrowserAction::Navigate { url } => {
                self.expected_revision = None;
                let completed =
                    self.state(|state| state.completed.get(&native).copied().unwrap_or_default())?;
                let url = CString::new(url.as_str()).map_err(|_| BrowserDriverError::Denied)?;
                // SAFETY: main-thread exact owned native tab; native URL policy also checks.
                status_result(unsafe { ffi::colossus_cef_navigate(native, 1, url.as_ptr()) })?;
                self.until(Some(control), Duration::from_secs(60), |state| {
                    !state.loading.contains(&native)
                        && state.completed.get(&native).copied().unwrap_or_default() > completed
                })?;
            }
            BrowserAction::Back {}
            | BrowserAction::Forward {}
            | BrowserAction::Reload {}
            | BrowserAction::Stop {} => {
                self.expected_revision = None;
                let action = match command.action {
                    BrowserAction::Back {} => 1,
                    BrowserAction::Forward {} => 2,
                    BrowserAction::Reload {} => 3,
                    _ => 4,
                };
                // SAFETY: private main-thread navigation operation on exact owned tab.
                status_result(unsafe { ffi::colossus_cef_control(native, 1, action) })?;
                self.until(Some(control), Duration::from_secs(60), |state| {
                    !state.loading.contains(&native)
                })?;
            }
            BrowserAction::Snapshot { max_nodes } => {
                let tree =
                    self.method(native, "Accessibility.getFullAXTree", json!({}), control)?;
                let (candidates, truncated) = semantic::candidates(&tree, *max_nodes)?;
                let mut attributes = HashMap::new();
                for candidate in &candidates {
                    if let Ok(value) = self.attributes(native, candidate.backend, control) {
                        attributes.insert(candidate.backend, value);
                    }
                    if self.interrupted(control) {
                        return Err(BrowserDriverError::Cancelled);
                    }
                }
                let snapshot_id = command
                    .snapshot_id
                    .clone()
                    .ok_or(BrowserDriverError::Denied)?;
                let (value, mapping) = semantic::snapshot(
                    candidates,
                    &attributes,
                    document.clone(),
                    snapshot_id.clone(),
                    truncated,
                )?;
                let tab = self
                    .tabs
                    .get_mut(&identity)
                    .ok_or(BrowserDriverError::Stale)?;
                tab.snapshot = Some(snapshot_id);
                tab.elements = mapping;
                snapshot = Some(value);
            }
            BrowserAction::Click { element } => {
                let backend = self.backend(&identity, element)?;
                self.method(
                    native,
                    "DOM.scrollIntoViewIfNeeded",
                    json!({"backendNodeId":backend}),
                    control,
                )?;
                let model = self.method(
                    native,
                    "DOM.getBoxModel",
                    json!({"backendNodeId":backend}),
                    control,
                )?;
                let (x, y) = center(&model)?;
                self.method(
                    native,
                    "Input.dispatchMouseEvent",
                    json!({"type":"mousePressed","button":"left","clickCount":1,"x":x,"y":y}),
                    control,
                )?;
                self.method(
                    native,
                    "Input.dispatchMouseEvent",
                    json!({"type":"mouseReleased","button":"left","clickCount":1,"x":x,"y":y}),
                    control,
                )?;
            }
            BrowserAction::Fill { element, text } => {
                if text.len() > 8192 {
                    return Err(BrowserDriverError::LimitExceeded);
                }
                let backend = self.backend(&identity, element)?;
                self.editable(native, backend, control)?;
                self.method(
                    native,
                    "DOM.focus",
                    json!({"backendNodeId":backend}),
                    control,
                )?;
                self.editable(native, backend, control)?;
                if self.focused_backend(native, control)? != Some(backend) {
                    return Err(BrowserDriverError::Denied);
                }
                self.key(native, "a", "KeyA", 2, control)?;
                self.editable(native, backend, control)?;
                if self.focused_backend(native, control)? != Some(backend) {
                    return Err(BrowserDriverError::Denied);
                }
                self.method(native, "Input.insertText", json!({"text":text}), control)?;
            }
            BrowserAction::Select { element, values } => {
                let backend = self.backend(&identity, element)?;
                self.select(native, backend, values, control)?;
            }
            BrowserAction::Press { key } => {
                if !matches!(key, BrowserKey::Tab | BrowserKey::Escape)
                    && let Some(focused) = self.focused_backend(native, control)?
                    && semantic::protected(&self.attributes(native, focused, control)?)
                {
                    return Err(BrowserDriverError::Denied);
                }
                let (key, code) = safe_key(*key);
                self.key(native, key, code, 0, control)?;
            }
            BrowserAction::Scroll { x, y } => {
                if x.unsigned_abs() > 10_000 || y.unsigned_abs() > 10_000 {
                    return Err(BrowserDriverError::LimitExceeded);
                }
                self.method(
                    native,
                    "Input.dispatchMouseEvent",
                    json!({"type":"mouseWheel","x":640,"y":400,"deltaX":x,"deltaY":y}),
                    control,
                )?;
            }
            BrowserAction::Wait {
                condition,
                timeout_ms,
            } => {
                if !(1..=30_000).contains(timeout_ms) {
                    return Err(BrowserDriverError::LimitExceeded);
                }
                match condition {
                    BrowserWaitCondition::Load {} => self.until(
                        Some(control),
                        Duration::from_millis(u64::from(*timeout_ms)),
                        |state| !state.loading.contains(&native),
                    )?,
                    BrowserWaitCondition::ElementVisible { element } => {
                        let backend = self.backend(&identity, element)?;
                        let deadline =
                            Instant::now() + Duration::from_millis(u64::from(*timeout_ms));
                        loop {
                            if self
                                .method(
                                    native,
                                    "DOM.getBoxModel",
                                    json!({"backendNodeId":backend}),
                                    control,
                                )
                                .is_ok_and(|value| center(&value).is_ok())
                            {
                                break;
                            }
                            if self.interrupted(control) || Instant::now() >= deadline {
                                return Err(BrowserDriverError::Cancelled);
                            }
                            self.pump()?;
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
            }
            BrowserAction::TabOpen { url } => {
                let summary = command.new_tab.clone().ok_or(BrowserDriverError::Denied)?;
                selected = summary.tab_id.clone();
                self.allocate(summary, url.as_ref(), control)?;
            }
            BrowserAction::TabSelect { tab_id } => {
                if !self.tabs.contains_key(tab_id) {
                    return Err(BrowserDriverError::Stale);
                }
                selected = tab_id.clone();
            }
            BrowserAction::TabClose { tab_id } => {
                let closing = self
                    .tabs
                    .get(tab_id)
                    .ok_or(BrowserDriverError::Stale)?
                    .native;
                // SAFETY: native close on main thread, with private exact tab identity.
                status_result(unsafe { ffi::colossus_cef_close(closing, 1) })?;
                self.until(None, Duration::from_secs(10), |state| {
                    state.closed.contains(&closing)
                })?;
                // The coordinator expects metadata for the closed target until it removes it.
                let summary = self
                    .tabs
                    .remove(tab_id)
                    .ok_or(BrowserDriverError::Stale)?
                    .summary;
                return Ok(BrowserObservation {
                    session_id: command.session_id,
                    tab: summary,
                    snapshot: None,
                    truncated: false,
                });
            }
        }
        // Drain immediate native document changes before committing opaque state.
        self.pump()?;
        if recovering_snapshot {
            self.check_document_fence()?;
        }
        let selected_native = self
            .tabs
            .get(&selected)
            .ok_or(BrowserDriverError::Stale)?
            .native;
        let (origin, title, new_revision) = self.metadata(selected_native)?;
        let new_native_document = self.native_document(selected_native)?;
        let tab = self
            .tabs
            .get_mut(&selected)
            .ok_or(BrowserDriverError::Stale)?;
        if new_revision != tab.revision
            || new_native_document != tab.native_document
            || matches!(
                command.action,
                BrowserAction::Navigate { .. }
                    | BrowserAction::Back {}
                    | BrowserAction::Forward {}
                    | BrowserAction::Reload {}
            )
        {
            tab.summary.document_id = command.next_document_id;
            if !recovering_snapshot {
                tab.snapshot = None;
                tab.elements.clear();
                snapshot = None;
            }
        } else if !matches!(
            command.action,
            BrowserAction::Snapshot { .. }
                | BrowserAction::Wait { .. }
                | BrowserAction::TabSelect { .. }
        ) {
            tab.snapshot = None;
            tab.elements.clear();
        }
        tab.revision = new_revision;
        tab.native_document = new_native_document;
        tab.summary.origin = origin;
        tab.summary.title = title;
        Ok(BrowserObservation {
            session_id: command.session_id,
            tab: tab.summary.clone(),
            truncated: snapshot.as_ref().is_some_and(|snapshot| snapshot.truncated),
            snapshot,
        })
    }
    pub fn close(&mut self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        if self.session.as_ref().is_some_and(|owned| owned != session) {
            return Err(BrowserDriverError::Denied);
        }
        self.close_all()?;
        self.shutdown()
    }
    pub fn close_all(&mut self) -> Result<(), BrowserDriverError> {
        self.presentation.handoff.revoke();
        self.screenshot = None;
        self.transfer.upload = None;
        self.transfer.download = None;
        self.revoke_human_presentation();
        self.callbacks
            .identity_revoked
            .store(true, Ordering::Release);
        if self.tabs.is_empty() {
            self.session = None;
            return Ok(());
        }
        let tabs: Vec<_> = self.tabs.values().map(|tab| tab.native).collect();
        for tab in &tabs {
            // SAFETY: exact owned native tab on its main thread; repeated close is accepted.
            let status = unsafe { ffi::colossus_cef_close(*tab, 1) };
            if !matches!(status, 0 | 3) {
                return Err(BrowserDriverError::OutcomeUnknown);
            }
        }
        self.until(None, Duration::from_secs(15), |state| {
            tabs.iter().all(|tab| state.closed.contains(tab))
        })?;
        self.tabs.clear();
        self.session = None;
        Ok(())
    }
    pub fn shutdown(&mut self) -> Result<(), BrowserDriverError> {
        if !self.initialized {
            return self.finish_transfers();
        }
        self.close_all()?;
        // SAFETY: main-thread shutdown only after all native tab close acknowledgements.
        if unsafe { ffi::colossus_cef_shutdown() } != 0 {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        self.initialized = false;
        self.finish_transfers()
    }
    pub fn idle_pump(&mut self) -> Result<(), BrowserDriverError> {
        self.retire_capture();
        self.retire_transfers();
        if self.initialized {
            self.pump()
        } else {
            Ok(())
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // Native callbacks and staged files must outlive actual CEF shutdown on
        // every early-return/unwind path. The supervisor owns fail-stop cleanup.
        if self.shutdown().is_err() {
            eprintln!("browser outcome unknown");
            std::process::abort();
        }
    }
}

fn status_result(status: i32) -> Result<(), BrowserDriverError> {
    match status {
        0 => Ok(()),
        1 | 5 => Err(BrowserDriverError::Denied),
        2 => Err(BrowserDriverError::Unavailable),
        3 => Err(BrowserDriverError::Stale),
        _ => Err(BrowserDriverError::OutcomeUnknown),
    }
}
struct SelectOption {
    backend: i32,
    value: String,
    disabled: bool,
}
fn attribute<'a>(attributes: &'a [String], name: &str) -> Option<&'a str> {
    attributes
        .chunks_exact(2)
        .find(|pair| pair[0].eq_ignore_ascii_case(name))
        .map(|pair| pair[1].as_str())
}
fn collect_options(
    node: &Value,
    disabled: bool,
    result: &mut Vec<SelectOption>,
) -> Result<(), BrowserDriverError> {
    let attributes = node.get("attributes").and_then(Value::as_array);
    let disabled = disabled
        || attributes.is_some_and(|attributes| {
            attributes.chunks_exact(2).any(|pair| {
                pair[0]
                    .as_str()
                    .is_some_and(|key| key.eq_ignore_ascii_case("disabled"))
            })
        });
    if node.get("nodeName").and_then(Value::as_str) == Some("OPTION") {
        if result.len() >= 256 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        let backend = node
            .get("backendNodeId")
            .and_then(Value::as_i64)
            .and_then(|backend| i32::try_from(backend).ok())
            .filter(|backend| *backend > 0)
            .ok_or(BrowserDriverError::Stale)?;
        let declared = attributes.and_then(|attributes| {
            attributes
                .chunks_exact(2)
                .find(|pair| {
                    pair[0]
                        .as_str()
                        .is_some_and(|key| key.eq_ignore_ascii_case("value"))
                })
                .and_then(|pair| pair[1].as_str())
        });
        let value = if let Some(value) = declared {
            value.to_owned()
        } else {
            let children = node
                .get("children")
                .and_then(Value::as_array)
                .ok_or(BrowserDriverError::Unsupported)?;
            let text: String = children
                .iter()
                .map(|child| {
                    child
                        .get("nodeValue")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                })
                .collect();
            text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
        };
        if value.len() > 1024 {
            return Err(BrowserDriverError::LimitExceeded);
        }
        result.push(SelectOption {
            backend,
            value,
            disabled,
        });
    } else if let Some(children) = node.get("children").and_then(Value::as_array) {
        for child in children {
            match child.get("nodeName").and_then(Value::as_str) {
                Some("OPTION" | "OPTGROUP") => collect_options(child, disabled, result)?,
                Some("#text" | "#comment") => {}
                _ => return Err(BrowserDriverError::Unsupported),
            }
        }
    }
    Ok(())
}
fn center(value: &Value) -> Result<(f64, f64), BrowserDriverError> {
    let points = value
        .get("model")
        .and_then(|model| model.get("content"))
        .and_then(Value::as_array)
        .filter(|points| points.len() == 8)
        .ok_or(BrowserDriverError::Stale)?;
    let point = |index: usize| {
        points[index]
            .as_f64()
            .filter(|value| value.is_finite() && value.abs() <= 1_000_000.0)
            .ok_or(BrowserDriverError::Stale)
    };
    let center = ((point(0)? + point(2)?) / 2.0, (point(1)? + point(5)?) / 2.0);
    if !(0.0..1280.0).contains(&center.0) || !(0.0..800.0).contains(&center.1) {
        return Err(BrowserDriverError::Stale);
    }
    Ok(center)
}
fn safe_key(key: BrowserKey) -> (&'static str, &'static str) {
    match key {
        BrowserKey::Enter => ("Enter", "Enter"),
        BrowserKey::Tab => ("Tab", "Tab"),
        BrowserKey::Escape => ("Escape", "Escape"),
        BrowserKey::Backspace => ("Backspace", "Backspace"),
        BrowserKey::Delete => ("Delete", "Delete"),
        BrowserKey::ArrowUp => ("ArrowUp", "ArrowUp"),
        BrowserKey::ArrowDown => ("ArrowDown", "ArrowDown"),
        BrowserKey::ArrowLeft => ("ArrowLeft", "ArrowLeft"),
        BrowserKey::ArrowRight => ("ArrowRight", "ArrowRight"),
        BrowserKey::Home => ("Home", "Home"),
        BrowserKey::End => ("End", "End"),
        BrowserKey::PageUp => ("PageUp", "PageUp"),
        BrowserKey::PageDown => ("PageDown", "PageDown"),
        BrowserKey::Space => (" ", "Space"),
    }
}
