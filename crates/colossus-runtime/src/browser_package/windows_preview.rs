//! Signed-sidecar-bound, non-admin Windows functional preview discovery.
use super::windows_preview_slot::Slot;
use super::{BrowserPackageError, RuntimeBrowserHost, windows_preview_manifest as descriptor};
use colossus_browser_bridge::BrowserHostFactory as _;
use colossus_contracts::{BrowserActionKind, BrowserCapabilities, BrowserLimits, BrowserMode};
use colossus_ports::BrowserDriverError;
use colossus_sandbox::{BrowserEgressLimits, WindowsBrowserConfig, WindowsBrowserSupervisor};
use colossus_windows_native::{BoundPath, PrivateDirectoryCreation};
use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// Owns the exact installed preview and all native cleanup through worker exit.
/// This edition makes no whole-host egress or private-certificate custody claim.
pub struct WindowsFunctionalPreviewBrowserOwner {
    host: RuntimeBrowserHost,
    cleanup: Arc<Cleanup>,
    settled: AtomicBool,
}
impl WindowsFunctionalPreviewBrowserOwner {
    /// Clone the same authenticated automation and native presentation owner.
    #[must_use]
    pub fn host(&self) -> RuntimeBrowserHost {
        self.host.clone()
    }

    /// Acknowledge whole-Job/session cleanup and retire only our exact empty state.
    pub async fn shutdown(&self) -> Result<(), BrowserDriverError> {
        self.cleanup.shutdown().await?;
        self.settled.store(true, Ordering::Release);
        Ok(())
    }
}
impl Drop for WindowsFunctionalPreviewBrowserOwner {
    fn drop(&mut self) {
        if !self.settled.load(Ordering::Acquire) {
            retain(Arc::clone(&self.cleanup));
        }
    }
}

struct Cleanup {
    supervisor: Option<Arc<WindowsBrowserSupervisor>>,
    state: Mutex<Option<PrivateDirectoryCreation>>,
    _image: BoundPath,
    slot: Mutex<Option<Slot<'static>>>,
}

// Install hashes the full inventory on a blocking worker. A cancelled awaiting
// future must transfer the exact allocation before its best-effort Drop runs.
struct Setup(Option<(PrivateDirectoryCreation, BoundPath, Slot<'static>)>);
impl Drop for Setup {
    fn drop(&mut self) {
        if let Some((state, image, slot)) = self.0.take() {
            retain(Arc::new(Cleanup {
                supervisor: None,
                state: Mutex::new(Some(state)),
                _image: image,
                slot: Mutex::new(Some(slot)),
            }));
        }
    }
}
impl Cleanup {
    async fn shutdown(&self) -> Result<(), BrowserDriverError> {
        if let Some(supervisor) = &self.supervisor {
            supervisor.shutdown().await?;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(creation) = state.as_mut() {
            creation
                .remove_empty()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            state.take();
        }
        if let Some(slot) = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            slot.retire();
        }
        Ok(())
    }
}

pub(super) async fn install(
    bytes: &'static [u8],
    instance_root: &Path,
) -> Result<Option<Arc<WindowsFunctionalPreviewBrowserOwner>>, BrowserPackageError> {
    let seal = descriptor::Seal::parse(bytes).map_err(|_| BrowserPackageError::Invalid)?;
    let image = BoundPath::open_immutable_file(
        &std::env::current_exe().map_err(|_| BrowserPackageError::Invalid)?,
    )
    .map_err(|_| BrowserPackageError::Invalid)?;
    let component_root = image
        .canonical_path()
        .parent()
        .ok_or(BrowserPackageError::Invalid)?
        .join(descriptor::COMPONENT_PATH);
    let manifest_sha256 = hex::decode(seal.component_manifest_sha256)
        .map_err(|_| BrowserPackageError::Invalid)?
        .try_into()
        .map_err(|_| BrowserPackageError::Invalid)?;
    // Native FILE_CREATE returns custody before verification; existing names and
    // reparse parents cannot become this allocation. Never adopt a prior profile.
    let slot = Slot::reserve(&INSTALLATION_ACTIVE).ok_or(BrowserPackageError::CleanupUnknown)?;
    let state = match PrivateDirectoryCreation::create(
        &instance_root.join(format!("browser-preview-{}", uuid::Uuid::new_v4().simple())),
    ) {
        Ok(state) => state,
        Err(_) => {
            slot.retire();
            return Err(BrowserPackageError::Invalid);
        }
    };
    let state_root = state.path().to_owned();
    let mut setup = Setup(Some((state, image, slot)));
    let limits = BrowserLimits::default();
    let installation =
        WindowsBrowserSupervisor::install_for_functional_preview(WindowsBrowserConfig {
            component_root,
            state_root,
            manifest_sha256,
            capabilities: capabilities(limits.clone()),
            memory_bytes: 1024 * 1024 * 1024,
            max_processes: 64,
            egress: BrowserEgressLimits {
                max_connections: 16,
                lifetime: Duration::from_millis(u64::from(limits.max_lease_ms)),
                connection_timeout: Duration::from_secs(15),
                max_connection_bytes: 16 * 1024 * 1024,
            },
        })
        .await;
    let (state, image, slot) = setup.0.take().ok_or(BrowserPackageError::Invalid)?;
    let supervisor = Arc::new(match installation {
        Ok(supervisor) => supervisor,
        // Only the native preview constructor classifies ordinary-user desktop
        // prerequisites this way; invalid publisher bytes remain fatal.
        Err(error) => {
            let cleanup = Arc::new(Cleanup {
                supervisor: None,
                state: Mutex::new(Some(state)),
                _image: image,
                slot: Mutex::new(Some(slot)),
            });
            if cleanup.shutdown().await.is_err() {
                retain(cleanup);
                return Err(BrowserPackageError::CleanupUnknown);
            }
            return if error == BrowserDriverError::Unavailable {
                Ok(None)
            } else {
                Err(BrowserPackageError::Invalid)
            };
        }
    });
    let cleanup = Arc::new(Cleanup {
        supervisor: Some(Arc::clone(&supervisor)),
        state: Mutex::new(Some(state)),
        _image: image,
        slot: Mutex::new(Some(slot)),
    });
    if supervisor.capabilities().restrictive_egress {
        retain(cleanup);
        return Err(BrowserPackageError::Invalid);
    }
    let host = match RuntimeBrowserHost::from_supervised_factory(supervisor.clone()) {
        Ok(host) => host.with_presenter(supervisor),
        Err(_) => {
            retain(cleanup);
            return Err(BrowserPackageError::CleanupUnknown);
        }
    };
    Ok(Some(Arc::new(WindowsFunctionalPreviewBrowserOwner {
        host,
        cleanup,
        settled: AtomicBool::new(false),
    })))
}

fn capabilities(limits: BrowserLimits) -> BrowserCapabilities {
    use BrowserActionKind::*;
    BrowserCapabilities {
        available: true,
        engine_version: Some(descriptor::CEF_VERSION.into()),
        modes: vec![BrowserMode::Embedded],
        actions: vec![
            Navigate,
            Snapshot,
            Screenshot,
            Click,
            Evaluate,
            Cdp,
            Hover,
            DoubleClick,
            RightClick,
            Drag,
            ClickAt,
            DragAt,
            Annotate,
            AnnotationClear,
            AnnotatedScreenshot,
            Fill,
            Select,
            Press,
            Scroll,
            Wait,
            Back,
            Forward,
            Reload,
            Stop,
            TabOpen,
            TabSelect,
            TabClose,
            Upload,
            Download,
        ],
        private_ca_trust: false,
        client_identities: false,
        restrictive_egress: false,
        limits,
    }
}

struct Keeper {
    // Slot admits at most one unresolved allocation. Setup/Owner transfers its
    // cleanup once, and retries move that same Arc; no cap can discard custody.
    owners: Mutex<VecDeque<Arc<Cleanup>>>,
    running: AtomicBool,
}
static INSTALLATION_ACTIVE: AtomicBool = AtomicBool::new(false);
static KEEPER: OnceLock<Keeper> = OnceLock::new();
fn retain(owner: Arc<Cleanup>) {
    let keeper = KEEPER.get_or_init(|| Keeper {
        owners: Mutex::new(VecDeque::new()),
        running: AtomicBool::new(false),
    });
    keeper
        .owners
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push_back(owner);
    if keeper
        .running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    if std::thread::Builder::new()
        .name("colossus-browser-preview-cleanup".into())
        .spawn(move || retry(keeper))
        .is_err()
    {
        keeper.running.store(false, Ordering::Release);
    }
}
fn retry(keeper: &'static Keeper) {
    let runtime = loop {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => break runtime,
            Err(_) => std::thread::sleep(Duration::from_secs(1)),
        }
    };
    loop {
        let owner = keeper
            .owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
        if let Some(owner) = owner {
            if runtime.block_on(owner.shutdown()).is_err() {
                keeper
                    .owners
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push_back(owner);
                std::thread::sleep(Duration::from_secs(1));
            }
            continue;
        }
        keeper.running.store(false, Ordering::Release);
        if keeper
            .owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
            || keeper
                .running
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
    }
}
