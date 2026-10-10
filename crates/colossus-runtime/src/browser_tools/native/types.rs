use super::super::RuntimeBrowserTools;
use async_trait::async_trait;
use colossus_browser::BrowserError;
use colossus_browser_presentation::{
    Configure, Lease, PageState, PresentationClient, PresentationError,
};
use colossus_contracts::{
    BrowserControlLease, BrowserSessionBinding, BrowserSessionId, BrowserTarget, BrowserUrl,
};
use colossus_ports::{BrowserDriver, BrowserDriverError, BrowserDriverOpenRequest, RunControl};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

/// Private presentation selected by the verified process supervisor, never a renderer.
#[async_trait]
pub trait RuntimeBrowserPresenter: Send + Sync {
    /// Acquire the original dedicated host's private presentation channel.
    async fn acquire(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(PresentationClient, [u8; 32]), BrowserDriverError>;
}

#[cfg(target_os = "linux")]
#[async_trait]
impl RuntimeBrowserPresenter for colossus_sandbox::OciBrowserSupervisor {
    async fn acquire(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(PresentationClient, [u8; 32]), BrowserDriverError> {
        self.presentation(request).await
    }
}
#[cfg(windows)]
#[async_trait]
impl RuntimeBrowserPresenter for colossus_sandbox::WindowsBrowserSupervisor {
    async fn acquire(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(PresentationClient, [u8; 32]), BrowserDriverError> {
        self.presentation(request).await
    }
}

/// Categorical native errors contain no page, private protocol or certificate data.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RuntimeNativeBrowserError {
    /// Verified Embedded host and private presenter are absent.
    #[error("native browser unavailable")]
    Unavailable,
    /// Sealed application, conversation, run or destination authority was rejected.
    #[error("native browser authority denied")]
    Denied,
    /// Target, ownership or authenticated presentation lease changed.
    #[error("native browser state is stale")]
    Stale,
    /// Another operation owns this context.
    #[error("native browser busy")]
    Busy,
    /// Bounded dimensions or resource limits were rejected before effect.
    #[error("native browser bounds exceeded")]
    LimitExceeded,
    /// Native effect or cleanup was not positively acknowledged; never retry automatically.
    #[error("native browser outcome unknown")]
    OutcomeUnknown,
}
impl From<BrowserError> for RuntimeNativeBrowserError {
    fn from(e: BrowserError) -> Self {
        match e {
            BrowserError::Unavailable => Self::Unavailable,
            BrowserError::Busy => Self::Busy,
            BrowserError::LimitExceeded | BrowserError::InvalidArguments => Self::LimitExceeded,
            BrowserError::StaleControl
            | BrowserError::StaleDocument
            | BrowserError::SnapshotRequired => Self::Stale,
            BrowserError::OutcomeUnknown | BrowserError::InvalidEvidence => Self::OutcomeUnknown,
            BrowserError::Driver(e) => e.into(),
            _ => Self::Denied,
        }
    }
}
impl From<BrowserDriverError> for RuntimeNativeBrowserError {
    fn from(e: BrowserDriverError) -> Self {
        match e {
            BrowserDriverError::Unavailable | BrowserDriverError::Unsupported => Self::Unavailable,
            BrowserDriverError::Stale => Self::Stale,
            BrowserDriverError::LimitExceeded => Self::LimitExceeded,
            BrowserDriverError::OutcomeUnknown => Self::OutcomeUnknown,
            _ => Self::Denied,
        }
    }
}
impl From<PresentationError> for RuntimeNativeBrowserError {
    fn from(e: PresentationError) -> Self {
        match e {
            PresentationError::OutcomeUnknown => Self::OutcomeUnknown,
            PresentationError::LimitExceeded | PresentationError::Invalid => Self::LimitExceeded,
            _ => Self::Stale,
        }
    }
}

/// Sealed native-bootstrap role, bound to one runtime and Desktop instance.
/// Native composition must revoke its independent credential on channel loss.
#[derive(Clone)]
pub struct RuntimeNativeBrowserAuthority {
    pub(in crate::browser_tools) runtime_id: String,
    pub(in crate::browser_tools) application_id: String,
    pub(in crate::browser_tools) instance: Uuid,
}

/// Bounded user request. All ownership is derived from the sealed native role.
pub struct NativeBrowserOpenRequest {
    /// Existing canonical application conversation, or create a fresh owned conversation.
    pub conversation_id: Option<String>,
    /// Initial destination; the native envelope is this exact origin.
    pub url: BrowserUrl,
    /// Logical viewport width.
    pub width: u32,
    /// Logical viewport height.
    pub height: u32,
    /// Device scale times one thousand.
    pub scale_milli: u32,
    /// Positive presentation generation.
    pub viewport_generation: u64,
    /// Visibility heartbeat, at most 1500ms.
    pub lease_ms: u16,
}

/// Native-only shared session handle; secrets and process identifiers have no serialization.
#[derive(Clone)]
pub struct RuntimeNativeBrowserSession {
    pub(in crate::browser_tools) entry: Arc<Entry>,
    pub(in crate::browser_tools) client: PresentationClient,
    pub(in crate::browser_tools) digest: [u8; 32],
    pub(in crate::browser_tools) session_id: BrowserSessionId,
}
impl RuntimeNativeBrowserSession {
    /// Exact admitted opaque session.
    pub fn session_id(&self) -> &BrowserSessionId {
        &self.session_id
    }
    /// Canonical application-owned conversation created or verified at admission.
    pub fn conversation_id(&self) -> &str {
        &self.entry.conversation_id
    }
    /// Retained private presentation client for the trusted native relay.
    pub fn presentation(&self) -> PresentationClient {
        self.client.clone()
    }
    /// Exact authenticated native enrollment digest.
    pub fn enrollment_digest(&self) -> [u8; 32] {
        self.digest
    }
    /// Positive full-cleanup receipt; an uncertain close remains false.
    pub fn is_closed(&self) -> Result<bool, RuntimeNativeBrowserError> {
        Ok(self
            .entry
            .view
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
            .retired)
    }
    /// Current authoritative viewport plan; closed or uncertain contexts fail closed.
    pub fn configure(&self) -> Result<Configure, RuntimeNativeBrowserError> {
        let state = self
            .entry
            .view
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        if state.closed || state.fenced && state.agent_generation.is_none() {
            return Err(RuntimeNativeBrowserError::Stale);
        }
        state
            .configure
            .clone()
            .ok_or(RuntimeNativeBrowserError::Stale)
    }
    /// Register only an actual authenticated Configure acknowledgement from this client.
    /// Worker composition calls this after its native relay receives Configured.
    pub fn confirm_presentation_lease(
        &self,
        config: &Configure,
        lease: Lease,
    ) -> Result<(), RuntimeNativeBrowserError> {
        config.validate()?;
        lease.validate()?;
        let mut state = self
            .entry
            .view
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        if state.closed
            || state.fenced && state.agent_generation.is_none()
            || config.session != self.session_id
            || !config.accepts(lease)
            || config.control_generation != state.agent_generation.unwrap_or(0)
            || state.lease.is_some_and(|old| {
                config.viewport_generation <= old.viewport_generation
                    || lease.tab != old.tab
                    || lease.session_generation != old.session_generation
                    || lease.document_generation < old.document_generation
            })
        {
            return Err(RuntimeNativeBrowserError::Stale);
        }
        let expected = state
            .configure
            .as_ref()
            .ok_or(RuntimeNativeBrowserError::Stale)?;
        if config.target != expected.target
            || config.viewport_generation < expected.viewport_generation
        {
            return Err(RuntimeNativeBrowserError::Stale);
        }
        state.configure = Some(config.clone());
        state.lease = Some(lease);
        Ok(())
    }

    /// Adopt authenticated Observe metadata from this same retained client.
    /// Core keeps its original document until the irreversible handoff transaction.
    pub fn confirm_presentation_state(
        &self,
        prior: Lease,
        page: &PageState,
    ) -> Result<(), RuntimeNativeBrowserError> {
        page.validate()?;
        let request = self
            .entry
            .request
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        let original = request.as_ref().ok_or(RuntimeNativeBrowserError::Stale)?;
        let url = BrowserUrl::parse(&page.url).map_err(|_| RuntimeNativeBrowserError::Stale)?;
        if !original.options.allowed_origins.contains(&url.origin())
            || page.target.tab_id != original.tab_id
        {
            return Err(RuntimeNativeBrowserError::Stale);
        }
        let mut state = self
            .entry
            .view
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        if state.closed
            || state.fenced && state.agent_generation.is_none()
            || state.lease != Some(prior)
        {
            return Err(RuntimeNativeBrowserError::Stale);
        }
        state
            .configure
            .as_mut()
            .ok_or(RuntimeNativeBrowserError::Stale)?
            .target = page.target.clone();
        Ok(())
    }
}

/// Authoritative transfer result for reconfiguring the same native GUI relay read-only.
pub struct RuntimeNativeBrowserGranted {
    /// Core's active single-writer grant.
    pub control: BrowserControlLease,
    /// Next viewport generation; native post-transfer configuration was already acknowledged.
    pub configure: Configure,
    /// Existing authenticated native enrollment.
    pub enrollment_digest: [u8; 32],
}
impl RuntimeNativeBrowserGranted {
    /// Actual Core grant selected by the authenticated run transaction.
    pub fn lease(&self) -> &BrowserControlLease {
        &self.control
    }
    /// Next read-only viewport plan for the same retained relay.
    pub fn configure(&self) -> &Configure {
        &self.configure
    }
    /// Exact native enrollment digest, unchanged by transfer.
    pub fn enrollment_digest(&self) -> [u8; 32] {
        self.enrollment_digest
    }
}

pub(in crate::browser_tools) struct NativeState {
    pub(in crate::browser_tools) driver: Arc<dyn BrowserDriver>,
    pub(in crate::browser_tools) presenter: Option<Arc<dyn RuntimeBrowserPresenter>>,
    pub(in crate::browser_tools) entries: Mutex<BTreeMap<Uuid, Arc<Entry>>>,
    pub(in crate::browser_tools) current: Mutex<Option<Arc<Entry>>>,
    pub(in crate::browser_tools) opening: AsyncMutex<()>,
    pub(in crate::browser_tools) stopped: AtomicBool,
}
impl NativeState {
    pub(in crate::browser_tools) fn new(
        driver: Arc<dyn BrowserDriver>,
        presenter: Option<Arc<dyn RuntimeBrowserPresenter>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            driver,
            presenter,
            entries: Mutex::new(BTreeMap::new()),
            current: Mutex::new(None),
            opening: AsyncMutex::new(()),
            stopped: AtomicBool::new(false),
        })
    }
    pub(in crate::browser_tools) fn active(&self) -> bool {
        self.entries
            .lock()
            .map_or(true, |entries| !entries.is_empty())
    }
    pub(in crate::browser_tools) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Ok(entries) = self.entries.lock() {
            for entry in entries.values() {
                super::lifecycle::fence_entry(entry);
            }
        }
    }
}
pub(in crate::browser_tools) struct Entry {
    pub(in crate::browser_tools) id: Uuid,
    pub(in crate::browser_tools) instance: Uuid,
    pub(in crate::browser_tools) binding: BrowserSessionBinding,
    pub(in crate::browser_tools) conversation_id: String,
    pub(in crate::browser_tools) request: Mutex<Option<BrowserDriverOpenRequest>>,
    pub(in crate::browser_tools) control: RunControl,
    pub(in crate::browser_tools) operation: AsyncMutex<()>,
    pub(in crate::browser_tools) view: Mutex<View>,
}
#[derive(Default)]
pub(in crate::browser_tools) struct View {
    pub(in crate::browser_tools) configure: Option<Configure>,
    pub(in crate::browser_tools) lease: Option<Lease>,
    pub(in crate::browser_tools) client: Option<PresentationClient>,
    pub(in crate::browser_tools) digest: Option<[u8; 32]>,
    pub(in crate::browser_tools) fenced: bool,
    pub(in crate::browser_tools) closed: bool,
    pub(in crate::browser_tools) retired: bool,
    pub(in crate::browser_tools) agent_generation: Option<u64>,
}
pub(in crate::browser_tools) fn target(request: &BrowserDriverOpenRequest) -> BrowserTarget {
    BrowserTarget {
        tab_id: request.tab_id.clone(),
        document_id: request.document_id.clone(),
    }
}
pub(in crate::browser_tools) fn authority(
    service: &RuntimeBrowserTools,
    role: &RuntimeNativeBrowserAuthority,
) -> Result<(), RuntimeNativeBrowserError> {
    service
        .identity
        .revalidate()
        .map_err(|_| RuntimeNativeBrowserError::Denied)?;
    if role.runtime_id != service.runtime_id
        || role.application_id.is_empty()
        || role.instance.is_nil()
    {
        Err(RuntimeNativeBrowserError::Denied)
    } else {
        Ok(())
    }
}
