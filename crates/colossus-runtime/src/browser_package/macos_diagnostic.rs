//! Debug-only compiler-sealed macOS browser diagnostic composition.
//! Direct-child cleanup receipts never claim full helper-tree acceptance.

use super::*;
use colossus_browser_bridge::BrowserHostFactory;
use colossus_contracts::{
    BrowserActionKind, BrowserCapabilities, BrowserLimits, BrowserMode, BrowserOrigin,
    BrowserSessionId,
};
use colossus_home::ConfinedRoot;
use colossus_ports::{BrowserDriver, BrowserDriverControl, BrowserDriverOpenRequest};
use colossus_sandbox::{
    BrowserEgressLimits, MacosBrowserAcceptanceConfig, MacosBrowserShutdownReceipt,
    MacosBrowserSupervisor,
};
use serde::Deserialize;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Seal {
    pub(super) allocation_root: PathBuf,
    pub(super) app_bundle: PathBuf,
    pub(super) manifest_sha256: String,
    pub(super) front_cdhash: String,
    pub(super) body_cdhash: String,
    pub(super) policy_sha256: String,
    pub(super) fixture_origin: String,
}

struct FixtureHost {
    supervisor: Arc<MacosBrowserSupervisor>,
    origin: BrowserOrigin,
}

impl FixtureHost {
    fn admits(&self, request: &BrowserDriverOpenRequest) -> bool {
        request.options.allowed_origins.len() == 1
            && request.options.allowed_origins[0] == self.origin
            && request
                .options
                .initial_url
                .as_ref()
                .is_none_or(|url| url.origin() == self.origin)
    }
}

#[async_trait::async_trait]
impl BrowserHostFactory for FixtureHost {
    fn capabilities(&self) -> BrowserCapabilities {
        self.supervisor.capabilities()
    }

    async fn launch(
        &self,
        request: &BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<Arc<dyn BrowserDriver>, BrowserDriverError> {
        if !self.admits(request) {
            return Err(BrowserDriverError::Denied);
        }
        self.supervisor.launch(request, control).await
    }

    async fn reap_failed_launch(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(), BrowserDriverError> {
        // Cleanup is never fenced by the admission check. An uncertain
        // partial launch always reaches the retained exact supervisor.
        self.supervisor.reap_failed_launch(request).await
    }
}

#[async_trait::async_trait]
impl crate::RuntimeBrowserPresenter for FixtureHost {
    async fn acquire(
        &self,
        request: &BrowserDriverOpenRequest,
    ) -> Result<(colossus_browser_presentation::PresentationClient, [u8; 32]), BrowserDriverError>
    {
        if !self.admits(request) {
            return Err(BrowserDriverError::Denied);
        }
        self.supervisor.presentation(request).await
    }
}

fn fixture_origin(value: &str) -> Result<BrowserOrigin, BrowserPackageError> {
    let origin = BrowserOrigin::parse(value).map_err(|_| BrowserPackageError::Invalid)?;
    let port = origin
        .as_str()
        .strip_prefix("http://127.0.0.1:")
        .and_then(|port| port.parse::<u16>().ok())
        .filter(|port| *port != 0 && *port != 80)
        .ok_or(BrowserPackageError::Invalid)?;
    let _ = port;
    if origin.as_str() != value {
        return Err(BrowserPackageError::Invalid);
    }
    Ok(origin)
}

fn digest<const N: usize>(value: &str) -> Result<[u8; N], BrowserPackageError> {
    hex::decode(value)
        .map_err(|_| BrowserPackageError::Invalid)?
        .try_into()
        .map_err(|_| BrowserPackageError::Invalid)
}

/// Separate native ownership with deliberately narrow shutdown evidence.
pub struct MacosDiagnosticBrowserOwner {
    host: RuntimeBrowserHost,
    supervisor: Arc<MacosBrowserSupervisor>,
    direct_child_shutdown: AtomicBool,
    owned_process: bool,
}

impl MacosDiagnosticBrowserOwner {
    /// Whether this owner uses the independent known-child creator/keeper lane.
    /// This never establishes production or complete browser-domain acceptance.
    #[must_use]
    pub fn is_owned_process_diagnostic(&self) -> bool {
        self.owned_process
    }
    /// Clone only the verified factory+presenter binding for Core composition.
    #[must_use]
    pub fn host(&self) -> RuntimeBrowserHost {
        self.host.clone()
    }

    /// Revoke egress and reap the direct host child. Helper-tree clearance is
    /// not established; the signed allocation is preserved for inspection.
    pub async fn shutdown_direct_child(
        &self,
    ) -> Result<Vec<(BrowserSessionId, Option<MacosBrowserShutdownReceipt>)>, BrowserDriverError>
    {
        let receipts = settle_direct_child(&self.supervisor).await?;
        self.direct_child_shutdown.store(true, Ordering::Release);
        Ok(receipts)
    }
    /// Read only completed native custody retirement evidence; opaque actor
    /// correlations cannot be used to enroll, sign or nominate an OS identity.
    pub async fn custody_retirement_receipts(
        &self,
    ) -> Vec<(
        BrowserSessionId,
        Option<colossus_sandbox::MacosBrowserCustodyRetirementReceipt>,
    )> {
        self.supervisor.custody_retirement_receipts().await
    }
}

async fn settle_direct_child(
    supervisor: &MacosBrowserSupervisor,
) -> Result<Vec<(BrowserSessionId, Option<MacosBrowserShutdownReceipt>)>, BrowserDriverError> {
    supervisor.shutdown().await?;
    let receipts = supervisor.shutdown_receipts().await;
    if receipts.iter().any(|(_, receipt)| receipt.is_none()) {
        return Err(BrowserDriverError::OutcomeUnknown);
    }
    Ok(receipts)
}

impl Drop for MacosDiagnosticBrowserOwner {
    fn drop(&mut self) {
        if !self.direct_child_shutdown.load(Ordering::Acquire) {
            retain(Arc::clone(&self.supervisor));
        }
    }
}

pub(super) async fn install(
    bytes: &'static [u8],
) -> Result<Arc<MacosDiagnosticBrowserOwner>, BrowserPackageError> {
    if bytes.is_empty() || bytes.len() > 4096 {
        return Err(BrowserPackageError::Invalid);
    }
    let candidate: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| BrowserPackageError::Invalid)?;
    if candidate.get("purpose").is_some() {
        let (seal, owned, tls) = super::macos_owned::verify(bytes)?;
        return install_metadata(seal, Some(owned), tls).await;
    }
    let seal: Seal = serde_json::from_slice(bytes).map_err(|_| BrowserPackageError::Invalid)?;
    install_metadata(seal, None, None).await
}

pub(super) async fn install_metadata(
    seal: Seal,
    owned: Option<colossus_sandbox::MacosBrowserOwnedProcessConfig>,
    tls: Option<colossus_sandbox::MacosBrowserTlsConfig>,
) -> Result<Arc<MacosDiagnosticBrowserOwner>, BrowserPackageError> {
    let owned_process = owned.is_some();
    let origin = if let Some(tls) = &tls {
        if tls.origin.as_str() != seal.fixture_origin {
            return Err(BrowserPackageError::Invalid);
        }
        tls.origin.clone()
    } else {
        fixture_origin(&seal.fixture_origin)?
    };
    // Protect the retained allocation and separately derived sibling custody
    // namespace from model-visible filesystem/process effects.
    // The browser supervisor independently verifies its native inventory.
    let protected_allocation =
        ConfinedRoot::bind(&seal.allocation_root).map_err(|_| BrowserPackageError::Invalid)?;
    let limits = BrowserLimits {
        max_sessions: 1,
        max_tabs: 2,
        ..BrowserLimits::default()
    };
    let supervisor = Arc::new(
        MacosBrowserSupervisor::install_for_acceptance(MacosBrowserAcceptanceConfig {
            owned_process: owned,
            tls_owner: None,
            tls_config: tls,
            allocation_root: seal.allocation_root,
            app_bundle: seal.app_bundle,
            manifest_sha256: digest(&seal.manifest_sha256)?,
            front_cdhash: digest(&seal.front_cdhash)?,
            body_cdhash: digest(&seal.body_cdhash)?,
            policy_sha256: digest(&seal.policy_sha256)?,
            capabilities: BrowserCapabilities {
                available: true,
                engine_version: Some("154.0.34+g14c5a08+chromium-154.0.8037.98".into()),
                modes: vec![BrowserMode::Embedded],
                actions: vec![
                    BrowserActionKind::Snapshot,
                    BrowserActionKind::Navigate,
                    BrowserActionKind::Click,
                    BrowserActionKind::Screenshot,
                    BrowserActionKind::TabOpen,
                    BrowserActionKind::TabSelect,
                    BrowserActionKind::TabClose,
                ],
                limits,
                private_ca_trust: false,
                client_identities: false,
                restrictive_egress: true,
            },
            egress: BrowserEgressLimits {
                max_connections: 4,
                // A source-only visual checkpoint can retain the fixed loopback
                // page for up to 40 seconds after Core startup and screenshot.
                lifetime: Duration::from_secs(180),
                connection_timeout: Duration::from_secs(10),
                max_connection_bytes: 1024 * 1024,
            },
        })
        .await
        .map_err(|_| BrowserPackageError::Invalid)?,
    );
    let fixture = Arc::new(FixtureHost {
        supervisor: Arc::clone(&supervisor),
        origin,
    });
    let (host, _retained) =
        RuntimeBrowserHost::macos_supervised_for_acceptance(fixture).map_err(|_| {
            retain(Arc::clone(&supervisor));
            BrowserPackageError::CleanupUnknown
        })?;
    let protected_custody = supervisor
        .protected_custody_root()
        .map_err(|_| BrowserPackageError::Invalid)?;
    let host = host.with_native_profile_protection(vec![protected_allocation, protected_custody]);
    Ok(Arc::new(MacosDiagnosticBrowserOwner {
        host,
        supervisor,
        direct_child_shutdown: AtomicBool::new(false),
        owned_process,
    }))
}

struct Keeper {
    owners: Mutex<VecDeque<Arc<MacosBrowserSupervisor>>>,
    running: AtomicBool,
}
static KEEPER: OnceLock<Keeper> = OnceLock::new();

fn keeper() -> &'static Keeper {
    KEEPER.get_or_init(|| Keeper {
        owners: Mutex::new(VecDeque::new()),
        running: AtomicBool::new(false),
    })
}

fn retain(owner: Arc<MacosBrowserSupervisor>) {
    let keeper = keeper();
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
    if thread::Builder::new()
        .name("colossus-mac-browser-diagnostic-cleanup".into())
        .spawn(move || retry(keeper))
        .is_err()
    {
        // A failed thread spawn never drops the process-static owner.
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
            Err(_) => thread::sleep(Duration::from_secs(1)),
        }
    };
    loop {
        let owner = keeper
            .owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
        if let Some(owner) = owner {
            if runtime.block_on(settle_direct_child(&owner)).is_err() {
                keeper
                    .owners
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push_back(owner);
                thread::sleep(Duration::from_secs(1));
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

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::{
        BrowserDocumentId, BrowserOpenOptions, BrowserScope, BrowserSessionBinding, BrowserTabId,
        BrowserUrl,
    };

    #[test]
    fn compiler_seal_accepts_only_one_canonical_numeric_loopback_http_origin() {
        assert_eq!(
            fixture_origin("http://127.0.0.1:60924").unwrap().as_str(),
            "http://127.0.0.1:60924"
        );
        for denied in [
            "http://localhost:60924",
            "http://127.0.0.1:80",
            "http://127.0.0.1:0",
            "http://127.0.0.1:60924/",
            "http://127.0.0.1:60924/path",
            "http://user@127.0.0.1:60924",
            "https://127.0.0.1:60924",
            "http://[::1]:60924",
        ] {
            assert!(fixture_origin(denied).is_err(), "{denied}");
        }
    }

    #[test]
    fn diagnostic_factory_rejects_every_unsealed_destination_before_launch() {
        let origin = fixture_origin("http://127.0.0.1:60924").unwrap();
        let fixture = FixtureHost {
            supervisor: Arc::new(MacosBrowserSupervisor::unavailable()),
            origin: origin.clone(),
        };
        let mut request = BrowserDriverOpenRequest {
            binding: BrowserSessionBinding {
                runtime_id: "fixture-runtime".into(),
                workspace_id: "fixture-workspace".into(),
                application_id: "fixture-application".into(),
                scope: BrowserScope::Conversation {
                    id: "fixture-conversation".into(),
                },
            },
            run_id: None,
            session_id: BrowserSessionId::parse(format!("bs_{}", "1".repeat(32))).unwrap(),
            tab_id: BrowserTabId::parse(format!("bt_{}", "2".repeat(32))).unwrap(),
            document_id: BrowserDocumentId::parse(format!("bd_{}", "3".repeat(32))).unwrap(),
            options: BrowserOpenOptions {
                mode: BrowserMode::Embedded,
                allowed_origins: vec![origin],
                initial_url: Some(BrowserUrl::parse("http://127.0.0.1:60924/first").unwrap()),
                profile: Default::default(),
            },
        };
        assert!(fixture.admits(&request));
        request.options.initial_url =
            Some(BrowserUrl::parse("http://127.0.0.1:60925/first").unwrap());
        assert!(!fixture.admits(&request));
        request.options.initial_url = None;
        request.options.allowed_origins =
            vec![BrowserOrigin::parse("http://127.0.0.1:60925").unwrap()];
        assert!(!fixture.admits(&request));
        request.options.allowed_origins.push(fixture.origin.clone());
        assert!(!fixture.admits(&request));
    }
}
