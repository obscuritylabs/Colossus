//! Trusted Desktop attachment to an independently supervised offscreen host.
mod actor;
mod handoff;
mod input_queue;
mod lifecycle;
mod viewport;
#[cfg(all(windows, feature = "native-test-driver"))]
mod windows_acceptance;
use crate::{
    BrowserError, EventSink, NavigationAction, PageState, contained::PresentationRole,
    presentation::PresentationSurface,
};
use colossus_browser_presentation::{Configure, HumanCommand, Input, Lease, PresentationClient};
#[cfg(all(any(windows, target_os = "macos"), feature = "native-test-driver"))]
use colossus_browser_presentation::{PageState as NativePageState, PresentationError};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
#[cfg(all(
    any(windows, target_os = "macos"),
    feature = "native-test-driver",
    debug_assertions
))]
use std::time::Duration;
use std::time::Instant;
use tauri::{Rect, Window};
use tokio::sync::mpsc;
use url::Url;

pub use crate::contained::RemoteHostOwner;
#[derive(Clone, Copy, PartialEq)]
struct Bounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_milli: u32,
}
struct State {
    configure: Configure,
    lease: Lease,
    confirmed_hidden_lease: Option<Lease>,
    bounds: Option<Bounds>,
    applied: Option<Bounds>,
    visible: bool,
    heartbeat: Instant,
    surface: Option<Arc<PresentationSurface>>,
}
impl State {
    fn new(configure: Configure, lease: Lease, surface: PresentationSurface) -> Self {
        Self {
            configure,
            lease,
            confirmed_hidden_lease: None,
            bounds: None,
            applied: None,
            visible: false,
            heartbeat: Instant::now(),
            surface: Some(Arc::new(surface)),
        }
    }
}
enum Event {
    Focus(Lease, bool),
    Input(Lease, Input, u32),
    Human(HumanCommand),
}
struct Owner {
    window: Window,
    client: PresentationClient,
    digest: [u8; 32],
    native: Arc<dyn RemoteHostOwner>,
    sink: EventSink,
    state: Mutex<State>,
    events: mpsc::Sender<Event>,
    revision: AtomicU64,
    viewport: AtomicU64,
    handles: AtomicUsize,
    closing: AtomicBool,
    updating: AtomicBool,
    read_only: AtomicBool,
    operation: tokio::sync::Mutex<()>,
    #[cfg(all(
        any(windows, target_os = "macos"),
        feature = "native-test-driver",
        debug_assertions
    ))]
    diagnostic_failures: std::sync::atomic::AtomicU8,
    #[cfg(windows)]
    completed: Arc<crate::presentation::CompletedCache>,
}

// Source-only, closed-vocabulary attach evidence. These enum variants contain
// no URL, enrollment material, path, native pointer or channel payload.
#[cfg(all(
    any(windows, target_os = "macos"),
    feature = "native-test-driver",
    debug_assertions
))]
enum AttachFailure {
    Configure(PresentationError),
    NativeView(BrowserError),
}
#[cfg(all(
    any(windows, target_os = "macos"),
    feature = "native-test-driver",
    debug_assertions
))]
impl AttachFailure {
    const fn token(self) -> &'static str {
        match self {
            Self::Configure(PresentationError::Expired) => "configure_expired",
            Self::Configure(PresentationError::Invalid) => "configure_invalid",
            Self::Configure(PresentationError::Stale) => "configure_stale",
            Self::Configure(PresentationError::Unauthenticated) => "configure_unauthenticated",
            Self::Configure(PresentationError::Hidden) => "configure_hidden",
            Self::Configure(PresentationError::LimitExceeded) => "configure_limit_exceeded",
            Self::Configure(PresentationError::OutcomeUnknown) => "configure_outcome_unknown",
            Self::NativeView(BrowserError::InvalidAddress) => "native_view_invalid_address",
            Self::NativeView(BrowserError::Unavailable) => "native_view_unavailable",
            Self::NativeView(BrowserError::Closed) => "native_view_closed",
            Self::NativeView(BrowserError::Expired) => "native_view_expired",
            Self::NativeView(BrowserError::Stale) => "native_view_stale",
            Self::NativeView(BrowserError::TimedOut) => "native_view_timed_out",
            Self::NativeView(BrowserError::ComponentMissing) => "native_view_component_missing",
            Self::NativeView(BrowserError::BootstrapRequired) => "native_view_bootstrap_required",
            Self::NativeView(BrowserError::NativeAcceptanceRequired) => {
                "native_view_acceptance_required"
            }
        }
    }
}
#[cfg(all(
    any(windows, target_os = "macos"),
    feature = "native-test-driver",
    debug_assertions
))]
fn trace_attach_failure(failure: AttachFailure) {
    eprintln!("EVIDENCE native-remote-attach failure={}", failure.token());
}
#[cfg(all(
    any(windows, target_os = "macos"),
    feature = "native-test-driver",
    debug_assertions
))]
fn trace_frame_channel_failure(stage: &'static str, error: PresentationError, elapsed: Duration) {
    let failure = presentation_failure_token(error);
    let timing = if elapsed >= Duration::from_millis(700) {
        "after_700ms"
    } else {
        "before_700ms"
    };
    eprintln!("EVIDENCE native-remote-frame stage={stage} failure={failure} timing={timing}");
}
#[cfg(all(
    any(windows, target_os = "macos"),
    feature = "native-test-driver",
    debug_assertions
))]
fn presentation_failure_token(error: PresentationError) -> &'static str {
    match error {
        PresentationError::Expired => "expired",
        PresentationError::Invalid => "invalid",
        PresentationError::Stale => "stale",
        PresentationError::Unauthenticated => "unauthenticated",
        PresentationError::Hidden => "hidden",
        PresentationError::LimitExceeded => "limit_exceeded",
        PresentationError::OutcomeUnknown => "outcome_unknown",
    }
}
#[cfg(all(
    any(windows, target_os = "macos"),
    feature = "native-test-driver",
    debug_assertions
))]
fn trace_owner_failure(
    owner: &Owner,
    stage: &'static str,
    failure: &'static str,
    lease: Lease,
    revision: u64,
    elapsed: Duration,
) {
    let bit = match stage {
        "hide_host" => 1,
        "viewport_renew_surface" => 2,
        "viewport_renew_host" => 4,
        _ => 8,
    };
    if owner.diagnostic_failures.fetch_or(bit, Ordering::AcqRel) & bit != 0 {
        return;
    }
    let (current_lease, requested_visible, heartbeat_expired) =
        owner.state.lock().map_or((false, false, false), |state| {
            (
                state.lease == lease,
                state.visible,
                state.heartbeat.elapsed() >= Duration::from_millis(1500),
            )
        });
    let revision_changed = owner.revision.load(Ordering::Acquire) != revision;
    let role = if owner.read_only.load(Ordering::Acquire) {
        "agent"
    } else {
        "human"
    };
    eprintln!(
        "EVIDENCE native-remote-owner stage={stage} failure={failure} role={role} current_lease={current_lease} requested_visible={requested_visible} revision_changed={revision_changed} heartbeat_expired={heartbeat_expired} elapsed_ms={}",
        elapsed.as_millis()
    );
}
#[cfg(all(
    any(windows, target_os = "macos"),
    feature = "native-test-driver",
    debug_assertions
))]
fn trace_frame_browser_failure(stage: &'static str, error: BrowserError) {
    eprintln!(
        "EVIDENCE native-remote-frame stage={stage} failure={}",
        AttachFailure::NativeView(error).token()
    );
}
/// Cloneable human guest handle; page pixels travel only on the private native channel.
pub struct RemoteSurface(Arc<Owner>);

/// Debug evidence that a supervised remote view submitted authenticated pixels
/// to a live native presenter. Foreground OS compositing needs separate evidence.
#[cfg(all(any(windows, target_os = "macos"), feature = "native-test-driver"))]
#[derive(Clone, Debug)]
pub struct RemoteAcceptanceProbe {
    /// Metadata and exact document target from the authenticated native channel.
    pub page: NativePageState,
    /// The trusted viewport remains visible and within its active lease.
    pub visible: bool,
    /// Native-applied logical viewport width.
    pub width: u32,
    /// Native-applied logical viewport height.
    pub height: u32,
    /// Last authenticated frame synchronously accepted by this native view.
    pub frame: Option<RemoteAcceptedFrame>,
}

/// Debug-only per-document pixels accepted by the native view; not OS-compositor evidence.
#[cfg(all(any(windows, target_os = "macos"), feature = "native-test-driver"))]
#[derive(Clone, Copy, Debug)]
pub struct RemoteAcceptedFrame {
    /// Exact authenticated lease, including native document and viewport generations.
    pub lease: Lease,
    /// Host sequence authenticated with the frame pixels and lease.
    pub sequence: u64,
    /// Center BGRA pixel copied from the accepted frame after native acknowledgement.
    pub center_bgra: [u8; 4],
    /// Synchronous frame-and-visible acknowledgements for this surface instance.
    pub count: u64,
}
impl Clone for RemoteSurface {
    fn clone(&self) -> Self {
        self.0.handles.fetch_add(1, Ordering::Relaxed);
        Self(Arc::clone(&self.0))
    }
}
impl RemoteSurface {
    /// Diagnostic loss of this viewer's outer stream. The separate native
    /// admission owner must still acknowledge detach; this does not close Core's writer.
    #[cfg(all(any(windows, target_os = "macos"), feature = "native-test-driver"))]
    pub fn acceptance_disconnect_presentation(&self) {
        self.0.client.disconnect();
    }
    /// Read native presentation evidence for a source-only acceptance fixture.
    ///
    /// # Errors
    /// Reports closed or poisoned native view state.
    #[cfg(all(any(windows, target_os = "macos"), feature = "native-test-driver"))]
    pub async fn acceptance_probe(&self) -> Result<RemoteAcceptanceProbe, BrowserError> {
        let lease = self.0.state.lock().map_err(|_| BrowserError::Closed)?.lease;
        #[cfg(all(any(windows, target_os = "macos"), debug_assertions))]
        let observed_at = Instant::now();
        let page = self.0.client.observe(lease).await.map_err(|error| {
            #[cfg(all(any(windows, target_os = "macos"), debug_assertions))]
            trace_frame_channel_failure("probe_observe", error, observed_at.elapsed());
            match error {
                PresentationError::Stale | PresentationError::Hidden => BrowserError::Stale,
                _ => BrowserError::Closed,
            }
        })?;
        let state = self.0.state.lock().map_err(|_| BrowserError::Closed)?;
        if state.lease != lease || state.configure.target != page.target {
            return Err(BrowserError::Stale);
        }
        let bounds = state.applied.ok_or(BrowserError::Unavailable)?;
        let surface = state.surface.as_ref().ok_or(BrowserError::Unavailable)?;
        let frame = surface
            .acceptance_accepted_frame()?
            .map(|accepted| RemoteAcceptedFrame {
                lease: accepted.lease,
                sequence: accepted.sequence,
                center_bgra: accepted.center_bgra,
                count: accepted.count,
            });
        if frame.is_some_and(|accepted| accepted.lease != lease) {
            return Err(BrowserError::Stale);
        }
        Ok(RemoteAcceptanceProbe {
            page,
            visible: state.visible
                && !self.0.closing.load(Ordering::Acquire)
                && state.heartbeat.elapsed() < std::time::Duration::from_millis(1500),
            width: bounds.width,
            height: bounds.height,
            frame,
        })
    }

    /// Observe native pixel removal from the same view that accepted a frame
    /// before the Desktop viewport was occluded. A local hide request alone is
    /// insufficient evidence.
    ///
    /// # Errors
    /// Rejects a missing or retired native presenter.
    #[cfg(all(target_os = "macos", feature = "native-test-driver", debug_assertions))]
    pub async fn acceptance_hidden_after(
        &self,
        previous: RemoteAcceptedFrame,
    ) -> Result<bool, BrowserError> {
        let surface = {
            let state = self.0.state.lock().map_err(|_| BrowserError::Closed)?;
            if state.visible {
                return Ok(false);
            }
            state.surface.clone().ok_or(BrowserError::Unavailable)?
        };
        let accepted = surface.acceptance_accepted_frame()?;
        if !accepted.is_some_and(|accepted| {
            accepted.lease == previous.lease
                && accepted.sequence == previous.sequence
                && accepted.count == previous.count
        }) {
            return Ok(false);
        }
        surface.acceptance_hidden().await
    }
    /// Attach a supervisor-admitted human session to the actual Desktop native window.
    ///
    /// # Errors
    /// Rejects agent ownership, bad enrollment, unavailable native views or stale targets.
    pub async fn create(
        window: &Window,
        client: PresentationClient,
        configure: Configure,
        digest: [u8; 32],
        native: Arc<dyn RemoteHostOwner>,
        sink: EventSink,
    ) -> Result<Self, BrowserError> {
        Self::attach(window, client, configure, digest, native, sink, false).await
    }
    /// Attach a native-admitted view of the exact agent page with all human effects disabled.
    ///
    /// # Errors
    /// Requires independently admitted viewer enrollment and exact current control generation.
    pub async fn create_read_only(
        window: &Window,
        client: PresentationClient,
        configure: Configure,
        digest: [u8; 32],
        native: Arc<dyn RemoteHostOwner>,
        sink: EventSink,
    ) -> Result<Self, BrowserError> {
        Self::attach(window, client, configure, digest, native, sink, true).await
    }
    #[allow(clippy::too_many_arguments)]
    async fn attach(
        window: &Window,
        client: PresentationClient,
        configure: Configure,
        digest: [u8; 32],
        native: Arc<dyn RemoteHostOwner>,
        sink: EventSink,
        read_only: bool,
    ) -> Result<Self, BrowserError> {
        let mut pending = lifecycle::Pending(Some(Arc::clone(&native)));
        let role = if read_only {
            PresentationRole::ReadOnly
        } else {
            PresentationRole::Human
        };
        if !role.accepts_generation(configure.control_generation) {
            return Err(BrowserError::Unavailable);
        }
        configure
            .validate()
            .map_err(|_| BrowserError::Unavailable)?;
        let lease = client
            .configure(configure.clone())
            .await
            .map_err(|failure| {
                #[cfg(all(
                    any(windows, target_os = "macos"),
                    feature = "native-test-driver",
                    debug_assertions
                ))]
                trace_attach_failure(AttachFailure::Configure(failure));
                #[cfg(not(all(
                    any(windows, target_os = "macos"),
                    feature = "native-test-driver",
                    debug_assertions
                )))]
                let _ = failure;
                BrowserError::Closed
            })?;
        let (events, receiver) = mpsc::channel(input_queue::CAPACITY);
        #[cfg(windows)]
        let completed = Arc::new(crate::presentation::CompletedCache::default());
        #[cfg(windows)]
        if read_only {
            completed.activate(lease);
        }
        let surface = actor::create_surface(
            window,
            &client,
            &events,
            lease,
            digest,
            !read_only,
            #[cfg(windows)]
            &completed,
        )
        .await
        .inspect_err(|failure| {
            #[cfg(all(
                any(windows, target_os = "macos"),
                feature = "native-test-driver",
                debug_assertions
            ))]
            trace_attach_failure(AttachFailure::NativeView(*failure));
            #[cfg(not(all(
                target_os = "macos",
                feature = "native-test-driver",
                debug_assertions
            )))]
            let _ = failure;
        })?;
        let viewport = configure.viewport_generation;
        let owner = Arc::new(Owner {
            window: window.clone(),
            client,
            digest,
            native,
            sink,
            state: Mutex::new(State::new(configure, lease, surface)),
            events,
            revision: AtomicU64::new(1),
            viewport: AtomicU64::new(viewport),
            handles: AtomicUsize::new(1),
            closing: AtomicBool::new(false),
            updating: AtomicBool::new(false),
            read_only: AtomicBool::new(read_only),
            operation: tokio::sync::Mutex::new(()),
            #[cfg(all(
                any(windows, target_os = "macos"),
                feature = "native-test-driver",
                debug_assertions
            ))]
            diagnostic_failures: std::sync::atomic::AtomicU8::new(0),
            #[cfg(windows)]
            completed,
        });
        actor::start(&owner, receiver);
        pending.0.take();
        Ok(Self(owner))
    }
    /// Revoke human input immediately and remove native pixels through the owning UI loop.
    ///
    /// # Errors
    /// Reports a poisoned or unavailable native view.
    pub fn hide(&self) -> Result<(), BrowserError> {
        lifecycle::hide(&self.0)
    }
    /// Schedule fresh native visibility under the supervisor's current opaque target.
    ///
    /// # Errors
    /// Rejects closing handles and bounded native scheduling failure.
    pub fn show(&self) -> Result<(), BrowserError> {
        if self.0.closing.load(Ordering::Acquire) {
            return Err(BrowserError::Closed);
        }
        {
            let mut state = self.0.state.lock().map_err(|_| BrowserError::Closed)?;
            // A fresh pulse for unchanged visibility must not invalidate a
            // configuration already awaiting native acknowledgement. Bounds
            // and visibility transitions independently advance the revision.
            if !state.visible {
                self.0.revision.fetch_add(1, Ordering::AcqRel);
            }
            state.visible = true;
            state.heartbeat = Instant::now();
        }
        viewport::update(&self.0);
        Ok(())
    }
    /// Store validated native viewport bounds, independent of page-authored coordinates.
    ///
    /// # Errors
    /// Rejects invalid bounds, excessive pixel allocation or a closed window.
    pub fn set_bounds(&self, bounds: Rect) -> Result<(), BrowserError> {
        let scale = self
            .0
            .window
            .scale_factor()
            .map_err(|_| BrowserError::Closed)?;
        let position = bounds.position.to_logical::<f64>(scale);
        let size = bounds.size.to_logical::<f64>(scale);
        let values = [
            position.x,
            position.y,
            size.width,
            size.height,
            scale * 1000.0,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0 || *value > 16384.0)
            || size.width < 16.0
            || size.height < 16.0
        {
            return Err(BrowserError::Unavailable);
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bounds = Bounds {
            x: position.x.round() as i32,
            y: position.y.round() as i32,
            width: size.width.round() as u32,
            height: size.height.round() as u32,
            scale_milli: (scale * 1000.0).round() as u32,
        };
        let mut state = self.0.state.lock().map_err(|_| BrowserError::Closed)?;
        if state.bounds != Some(bounds) {
            state.bounds = Some(bounds);
            self.0.revision.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }
    /// Submit an explicitly human navigation without changing origin or agent grants.
    ///
    /// # Errors
    /// Rejects a closing view or full bounded native event queue.
    pub fn navigate(&self, url: &Url) -> Result<(), BrowserError> {
        if self.0.read_only.load(Ordering::Acquire) {
            return Err(BrowserError::Unavailable);
        }
        self.0
            .events
            .try_send(Event::Human(HumanCommand::Navigate {
                url: url.to_string(),
            }))
            .map_err(|_| BrowserError::Closed)
    }
    /// Apply the closed human navigation vocabulary on the actual contained page.
    ///
    /// # Errors
    /// Reports native or private-channel rejection without uncertain-effect retry.
    pub async fn control(&self, action: NavigationAction) -> Result<(), BrowserError> {
        if self.0.read_only.load(Ordering::Acquire) {
            return Err(BrowserError::Unavailable);
        }
        let command = match action {
            NavigationAction::Back => HumanCommand::Back,
            NavigationAction::Forward => HumanCommand::Forward,
            NavigationAction::Reload => HumanCommand::Reload,
            NavigationAction::Stop => HumanCommand::Stop,
        };
        actor::human(&self.0, command).await
    }
    /// Read bounded native metadata; renderer observations grant no authority.
    ///
    /// # Errors
    /// Reports closed or stale private native ownership.
    pub async fn inspect(&self) -> Result<PageState, BrowserError> {
        actor::inspect(&self.0).await
    }
    /// Hide through a native acknowledgment, then close the entire owned host envelope.
    ///
    /// # Errors
    /// Unknown cleanup stays retained by both this handle and native composition.
    pub async fn close(&self) -> Result<(), BrowserError> {
        lifecycle::close(&self.0).await
    }
    /// Whether trusted native human navigation/input was admitted for this attachment.
    #[must_use]
    pub fn human_control_available(&self) -> bool {
        !self.0.read_only.load(Ordering::Acquire)
    }

    #[cfg(windows)]
    pub(crate) async fn completed_snapshot(
        &self,
    ) -> Result<Option<crate::presentation::CompletedSurface>, BrowserError> {
        if !self.0.read_only.load(Ordering::Acquire) || self.0.closing.load(Ordering::Acquire) {
            return Ok(None);
        }
        let lease = self.0.state.lock().map_err(|_| BrowserError::Closed)?.lease;
        let snapshot = crate::presentation::CompletedSurface::from_cache(
            &self.0.window,
            &self.0.completed,
            lease,
        )
        .await?;
        if self.0.closing.load(Ordering::Acquire)
            || !self.0.state.lock().is_ok_and(|state| state.lease == lease)
        {
            if let Some(snapshot) = snapshot {
                snapshot.close().await?;
            }
            return Ok(None);
        }
        Ok(snapshot)
    }

    /// Canonical conversation retained by this exact native runtime admission.
    #[must_use]
    pub fn conversation_id(&self) -> &str {
        self.0.native.conversation_id()
    }

    /// Exact independently contained browser context.
    #[must_use]
    pub fn session_id(&self) -> &str {
        self.0.native.session_id()
    }

    /// Transfer this exact page to a registered run and retain a read-only native view.
    ///
    /// # Errors
    /// Uncertain transfer remains fenced and cannot restore human input.
    pub async fn handoff(&self, run_id: &str) -> Result<(), BrowserError> {
        handoff::transfer(&self.0, run_id).await
    }
}
impl Drop for RemoteSurface {
    fn drop(&mut self) {
        if self.0.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            let _ = lifecycle::hide(&self.0);
            let owner = Arc::clone(&self.0);
            tauri::async_runtime::spawn(async move {
                let _ = lifecycle::close(&owner).await;
            });
        }
    }
}
