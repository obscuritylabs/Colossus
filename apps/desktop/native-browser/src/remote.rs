//! Trusted Desktop attachment to an independently supervised offscreen host.
mod actor;
mod handoff;
mod lifecycle;
mod viewport;
use crate::{
    BrowserError, EventSink, NavigationAction, PageState, contained::PresentationRole,
    presentation::PresentationSurface,
};
use colossus_browser_presentation::{Configure, HumanCommand, Input, Lease, PresentationClient};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
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
    bounds: Option<Bounds>,
    applied: Option<Bounds>,
    visible: bool,
    heartbeat: Instant,
    surface: Option<Arc<PresentationSurface>>,
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
}
/// Cloneable human guest handle; page pixels travel only on the private native channel.
pub struct RemoteSurface(Arc<Owner>);
impl Clone for RemoteSurface {
    fn clone(&self) -> Self {
        self.0.handles.fetch_add(1, Ordering::Relaxed);
        Self(Arc::clone(&self.0))
    }
}
impl RemoteSurface {
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
            .map_err(|_| BrowserError::Closed)?;
        let (events, receiver) = mpsc::channel(32);
        let surface =
            actor::create_surface(window, &client, &events, lease, digest, !read_only).await?;
        let viewport = configure.viewport_generation;
        let owner = Arc::new(Owner {
            window: window.clone(),
            client,
            digest,
            native,
            sink,
            state: Mutex::new(State {
                configure,
                lease,
                bounds: None,
                applied: None,
                visible: false,
                heartbeat: Instant::now(),
                surface: Some(Arc::new(surface)),
            }),
            events,
            revision: AtomicU64::new(1),
            viewport: AtomicU64::new(viewport),
            handles: AtomicUsize::new(1),
            closing: AtomicBool::new(false),
            updating: AtomicBool::new(false),
            read_only: AtomicBool::new(read_only),
            operation: tokio::sync::Mutex::new(()),
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
            state.visible = true;
            state.heartbeat = Instant::now();
        }
        self.0.revision.fetch_add(1, Ordering::AcqRel);
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
        self.0
            .state
            .lock()
            .map_err(|_| BrowserError::Closed)?
            .bounds = Some(bounds);
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
