//! Explicit compiler-selected, standard-user Windows browser development.
use super::{BrowserPackageError, RuntimeBrowserHost};
use colossus_browser_bridge::BrowserHostFactory as _;
use colossus_contracts::{BrowserActionKind, BrowserCapabilities, BrowserLimits, BrowserMode};
use colossus_ports::BrowserDriverError;
use colossus_sandbox::{BrowserEgressLimits, WindowsBrowserConfig, WindowsBrowserSupervisor};
use serde::Deserialize;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    component_root: PathBuf,
    state_root: PathBuf,
    manifest_sha256: String,
}

/// Retains the exact standard-user browser supervisor through managed worker exit.
/// This development binding does not satisfy installed or OS containment acceptance.
pub struct WindowsDevelopmentBrowserOwner {
    host: RuntimeBrowserHost,
    supervisor: Arc<WindowsBrowserSupervisor>,
    settled: AtomicBool,
}
impl WindowsDevelopmentBrowserOwner {
    /// Clone the same authenticated factory and native presenter for Core.
    #[must_use]
    pub fn host(&self) -> RuntimeBrowserHost {
        self.host.clone()
    }

    /// Await full Job/private-I/O/profile cleanup for every retained session.
    pub async fn shutdown(&self) -> Result<(), BrowserDriverError> {
        self.supervisor.shutdown().await?;
        self.settled.store(true, Ordering::Release);
        Ok(())
    }
}
impl Drop for WindowsDevelopmentBrowserOwner {
    fn drop(&mut self) {
        if !self.settled.load(Ordering::Acquire) {
            retain(Arc::clone(&self.supervisor));
        }
    }
}

pub(super) async fn install(
    bytes: &'static [u8],
) -> Result<Arc<WindowsDevelopmentBrowserOwner>, BrowserPackageError> {
    if bytes.is_empty() || bytes.len() > 8192 {
        return Err(BrowserPackageError::Invalid);
    }
    let selection: Selection =
        serde_json::from_slice(bytes).map_err(|_| BrowserPackageError::Invalid)?;
    if selection.manifest_sha256.len() != 64
        || !selection
            .manifest_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(BrowserPackageError::Invalid);
    }
    let manifest_sha256 = hex::decode(selection.manifest_sha256)
        .map_err(|_| BrowserPackageError::Invalid)?
        .try_into()
        .map_err(|_| BrowserPackageError::Invalid)?;
    let limits = BrowserLimits::default();
    let supervisor = Arc::new(
        WindowsBrowserSupervisor::install_for_development(WindowsBrowserConfig {
            component_root: selection.component_root,
            state_root: selection.state_root,
            manifest_sha256,
            capabilities: BrowserCapabilities {
                available: true,
                engine_version: Some("154.0.34+g14c5a08+chromium-154.0.8037.98".into()),
                modes: vec![BrowserMode::Embedded],
                actions: vec![
                    BrowserActionKind::Navigate,
                    BrowserActionKind::Snapshot,
                    BrowserActionKind::Screenshot,
                    BrowserActionKind::Click,
                    BrowserActionKind::Evaluate,
                    BrowserActionKind::Cdp,
                    BrowserActionKind::Hover,
                    BrowserActionKind::DoubleClick,
                    BrowserActionKind::RightClick,
                    BrowserActionKind::Drag,
                    BrowserActionKind::ClickAt,
                    BrowserActionKind::DragAt,
                    BrowserActionKind::Annotate,
                    BrowserActionKind::AnnotationClear,
                    BrowserActionKind::AnnotatedScreenshot,
                    BrowserActionKind::Fill,
                    BrowserActionKind::Select,
                    BrowserActionKind::Press,
                    BrowserActionKind::Scroll,
                    BrowserActionKind::Wait,
                    BrowserActionKind::Back,
                    BrowserActionKind::Forward,
                    BrowserActionKind::Reload,
                    BrowserActionKind::Stop,
                    BrowserActionKind::TabOpen,
                    BrowserActionKind::TabSelect,
                    BrowserActionKind::TabClose,
                    BrowserActionKind::Upload,
                    BrowserActionKind::Download,
                ],
                private_ca_trust: false,
                client_identities: true,
                restrictive_egress: false,
                limits: limits.clone(),
            },
            memory_bytes: 1024 * 1024 * 1024,
            max_processes: 64,
            egress: BrowserEgressLimits {
                max_connections: 16,
                lifetime: Duration::from_millis(u64::from(limits.max_lease_ms)),
                connection_timeout: Duration::from_secs(15),
                max_connection_bytes: 16 * 1024 * 1024,
            },
        })
        .await
        .map_err(|_| BrowserPackageError::Invalid)?,
    );
    assert!(!supervisor.capabilities().restrictive_egress);
    let host = match RuntimeBrowserHost::from_supervised_factory(supervisor.clone()) {
        Ok(host) => host.with_presenter(supervisor.clone()),
        Err(_) => {
            retain(supervisor);
            return Err(BrowserPackageError::CleanupUnknown);
        }
    };
    Ok(Arc::new(WindowsDevelopmentBrowserOwner {
        host,
        supervisor,
        settled: AtomicBool::new(false),
    }))
}

struct Keeper {
    owners: Mutex<VecDeque<Arc<WindowsBrowserSupervisor>>>,
    running: AtomicBool,
}
static KEEPER: OnceLock<Keeper> = OnceLock::new();
fn retain(owner: Arc<WindowsBrowserSupervisor>) {
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
        .name("colossus-windows-browser-cleanup".into())
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
