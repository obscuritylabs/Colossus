use colossus_sdk::{Colossus, NativeSidecarFailure, NativeSidecarStatus};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, RwLockReadGuard, Semaphore, watch};

use crate::{
    desktop_dto::{ManagedRuntimeStateDto, RuntimeFailureCodeDto},
    terminal::{TerminalKind, TerminalManager, TerminalPlanContext, TerminalWorkspace},
};

pub(crate) const MANAGED_TARGET_ID: &str = "managed-local";
const MAX_NATIVE_WATCHES_PER_TARGET: usize = 8;
const MAX_NATIVE_UNARY_CALLS_PER_TARGET: usize = 16;
const MAX_CONCURRENT_EXTERNAL_PROBES: usize = 4;
const MAX_RUN_TARGET_BINDINGS: usize = 4_096;
const EXTERNAL_PROBE_COOLDOWN: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub(crate) struct TerminalLaunchRequest {
    pub(crate) generation: u64,
    delivered_generation: u64,
    pub(crate) window_epoch: u64,
    pub(crate) kind: TerminalKind,
    pub(crate) plan_context: Option<TerminalPlanContext>,
    pub(crate) pending: bool,
}

#[derive(Clone)]
pub(crate) struct TargetHandle {
    pub(crate) client: Colossus,
    pub(crate) consent: TargetConsentContext,
    limits: TargetLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TargetConsentContext {
    ManagedLocal,
    External {
        label: String,
        instance_id: String,
        certificate_sha256: String,
    },
}

/// A native selection lease held across one target operation.
///
/// Its read guard prevents a target switch from racing an in-flight create, hydrate,
/// or control request. The renderer-supplied target ID is advisory only: constructing
/// this value proves it still matches the native Work selection.
pub(crate) struct SelectedTargetLease<'a> {
    pub(crate) target: TargetHandle,
    target_id: String,
    epoch: u64,
    _selection: RwLockReadGuard<'a, Option<String>>,
}

impl SelectedTargetLease<'_> {
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }
}

#[derive(Clone)]
struct TargetLimits {
    watch_slots: Arc<Semaphore>,
    unary_slots: Arc<Semaphore>,
}

impl TargetHandle {
    fn new(client: Colossus, consent: TargetConsentContext) -> Self {
        Self {
            client,
            consent,
            limits: TargetLimits::new(),
        }
    }

    pub(crate) fn try_watch_slot(&self) -> Option<OwnedSemaphorePermit> {
        self.limits.try_watch_slot()
    }

    pub(crate) fn try_unary_slot(&self) -> Option<OwnedSemaphorePermit> {
        self.limits.try_unary_slot()
    }

    fn is_closed(&self) -> bool {
        self.client.agent_runs().is_closed()
    }
}

impl TargetLimits {
    fn new() -> Self {
        Self {
            watch_slots: Arc::new(Semaphore::new(MAX_NATIVE_WATCHES_PER_TARGET)),
            unary_slots: Arc::new(Semaphore::new(MAX_NATIVE_UNARY_CALLS_PER_TARGET)),
        }
    }

    pub(crate) fn try_watch_slot(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.watch_slots).try_acquire_owned().ok()
    }

    pub(crate) fn try_unary_slot(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.unary_slots).try_acquire_owned().ok()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ManagedHealth {
    pub(crate) state: ManagedRuntimeStateDto,
    pub(crate) message: String,
    pub(crate) failure_code: Option<RuntimeFailureCodeDto>,
}

impl Default for ManagedHealth {
    fn default() -> Self {
        Self {
            state: ManagedRuntimeStateDto::NeedsWorkspace,
            message: "Choose a workspace to configure Managed Local.".into(),
            failure_code: None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ExternalHealth {
    pub(crate) state: &'static str,
    pub(crate) message: String,
    pub(crate) failure_code: Option<RuntimeFailureCodeDto>,
    generation: u64,
    probe_not_before: Instant,
    automatic_probe: bool,
}

impl ExternalHealth {
    pub(crate) fn available() -> Self {
        Self {
            state: "available",
            message: "Authenticated daemon is reachable. Select it to connect.".into(),
            failure_code: None,
            generation: 0,
            probe_not_before: Instant::now() + EXTERNAL_PROBE_COOLDOWN,
            automatic_probe: true,
        }
    }

    pub(crate) fn connected() -> Self {
        Self {
            state: "ready",
            message: "Authenticated daemon connection is ready.".into(),
            failure_code: None,
            generation: 0,
            probe_not_before: Instant::now() + EXTERNAL_PROBE_COOLDOWN,
            automatic_probe: true,
        }
    }

    pub(crate) fn unreachable() -> Self {
        Self {
            state: "unreachable",
            message: "The saved daemon could not be reached.".into(),
            failure_code: Some(RuntimeFailureCodeDto::Transport),
            generation: 0,
            probe_not_before: Instant::now() + EXTERNAL_PROBE_COOLDOWN,
            automatic_probe: true,
        }
    }

    pub(crate) fn authentication_failed() -> Self {
        Self {
            state: "unreachable",
            message: "The saved daemon credential is unavailable or was rejected.".into(),
            failure_code: Some(RuntimeFailureCodeDto::Authentication),
            generation: 0,
            probe_not_before: Instant::now() + EXTERNAL_PROBE_COOLDOWN,
            automatic_probe: false,
        }
    }

    pub(crate) fn connection_failed(code: &str) -> Self {
        match code {
            "unauthenticated" => Self::authentication_failed(),
            "identity_mismatch" => Self::permanent_failure(
                "The saved daemon identity did not match its trust anchor.",
                RuntimeFailureCodeDto::Integrity,
            ),
            "version_mismatch" | "not_configured" | "invalid_argument" => Self::permanent_failure(
                "The saved daemon connection is not configured for this desktop build.",
                RuntimeFailureCodeDto::Configuration,
            ),
            "credential_reenrollment_required" => Self::permanent_failure(
                "Re-enroll this daemon for the Desktop-bound credential entry, then import it again.",
                RuntimeFailureCodeDto::Configuration,
            ),
            "transport" | "unavailable" => Self::unreachable(),
            _ => Self::permanent_failure(
                "The saved daemon connection failed safely.",
                RuntimeFailureCodeDto::Internal,
            ),
        }
    }

    fn permanent_failure(message: &str, failure_code: RuntimeFailureCodeDto) -> Self {
        Self {
            state: "unreachable",
            message: message.into(),
            failure_code: Some(failure_code),
            generation: 0,
            probe_not_before: Instant::now() + EXTERNAL_PROBE_COOLDOWN,
            automatic_probe: false,
        }
    }

    pub(crate) fn stalled() -> Self {
        Self {
            state: "unreachable",
            message: "The daemon health check did not finish. Reconnect it to retry.".into(),
            failure_code: Some(RuntimeFailureCodeDto::Transport),
            generation: 0,
            probe_not_before: Instant::now() + EXTERNAL_PROBE_COOLDOWN,
            // A timeout may leave a platform-keychain blocking job alive. Do not
            // schedule another background read for this target; an explicit native
            // reconnect begins a new generation when the user is ready to retry.
            automatic_probe: false,
        }
    }
}

/// Native-only authenticated clients and local process state shared by narrow commands.
pub(crate) struct AppState {
    targets: RwLock<HashMap<String, TargetHandle>>,
    selected_target_id: RwLock<Option<String>>,
    selection_epoch: AtomicU64,
    selection_updates: watch::Sender<u64>,
    run_targets: RwLock<HashMap<String, String>>,
    managed_health: RwLock<ManagedHealth>,
    managed_lifecycle_generation: AtomicU64,
    managed_lifecycle: StdMutex<Option<ManagedLifecycleObservation>>,
    external_health: RwLock<HashMap<String, ExternalHealth>>,
    external_health_generation: AtomicU64,
    external_probe_slots: Arc<Semaphore>,
    connect_guard: Mutex<()>,
    approval_guard: Mutex<()>,
    terminal_context_guard: Mutex<()>,
    terminal_window_guard: Mutex<()>,
    terminal_window_lifecycle: StdMutex<()>,
    terminal_manager: TerminalManager,
    terminal_workspace: RwLock<Option<TerminalWorkspace>>,
    terminal_context_generation: AtomicU64,
    terminal_window_epoch: AtomicU64,
    terminal_window_active: AtomicBool,
    terminal_document_ready: AtomicBool,
    terminal_enabled: AtomicBool,
    update_available: AtomicBool,
    update_guard: Mutex<()>,
    terminal_launch_request: StdMutex<TerminalLaunchRequest>,
}

struct ManagedLifecycleObservation {
    generation: u64,
    status: watch::Receiver<NativeSidecarStatus>,
}

impl Default for AppState {
    fn default() -> Self {
        let (selection_updates, _) = watch::channel(0);
        Self {
            targets: RwLock::new(HashMap::new()),
            selected_target_id: RwLock::new(None),
            selection_epoch: AtomicU64::new(0),
            selection_updates,
            run_targets: RwLock::new(HashMap::new()),
            managed_health: RwLock::new(ManagedHealth::default()),
            managed_lifecycle_generation: AtomicU64::new(0),
            managed_lifecycle: StdMutex::new(None),
            external_health: RwLock::new(HashMap::new()),
            external_health_generation: AtomicU64::new(0),
            external_probe_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_EXTERNAL_PROBES)),
            connect_guard: Mutex::new(()),
            approval_guard: Mutex::new(()),
            terminal_context_guard: Mutex::new(()),
            terminal_window_guard: Mutex::new(()),
            terminal_window_lifecycle: StdMutex::new(()),
            terminal_manager: TerminalManager::default(),
            terminal_workspace: RwLock::new(None),
            terminal_context_generation: AtomicU64::new(0),
            terminal_window_epoch: AtomicU64::new(0),
            terminal_window_active: AtomicBool::new(false),
            terminal_document_ready: AtomicBool::new(false),
            terminal_enabled: AtomicBool::new(false),
            update_available: AtomicBool::new(false),
            update_guard: Mutex::new(()),
            terminal_launch_request: StdMutex::new(TerminalLaunchRequest {
                generation: 0,
                delivered_generation: 0,
                window_epoch: 0,
                kind: TerminalKind::ColossusTui,
                plan_context: None,
                pending: false,
            }),
        }
    }
}

impl AppState {
    pub(crate) fn update_available(&self) -> bool {
        self.update_available.load(Ordering::Acquire)
    }

    pub(crate) fn set_update_available(&self, available: bool) {
        self.update_available.store(available, Ordering::Release);
    }

    pub(crate) fn try_update_guard(&self) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        self.update_guard.try_lock().ok()
    }

    pub(crate) async fn target(&self, target_id: &str) -> Option<TargetHandle> {
        self.targets
            .read()
            .await
            .get(target_id)
            .filter(|target| !target.is_closed())
            .cloned()
    }

    pub(crate) async fn selected_target(&self, target_id: &str) -> Option<SelectedTargetLease<'_>> {
        let selection = self.selected_target_id.read().await;
        if selection.as_deref() != Some(target_id) {
            return None;
        }
        let target = self
            .targets
            .read()
            .await
            .get(target_id)
            .filter(|target| !target.is_closed())
            .cloned()?;
        Some(SelectedTargetLease {
            target,
            target_id: target_id.to_owned(),
            epoch: self.selection_epoch.load(Ordering::Acquire),
            _selection: selection,
        })
    }

    pub(crate) async fn bind_runs(&self, lease: &SelectedTargetLease<'_>, run_ids: Vec<String>) {
        let mut bindings = self.run_targets.write().await;
        let new_bindings = run_ids
            .iter()
            .filter(|run_id| !bindings.contains_key(run_id.as_str()))
            .count();
        if bindings.len().saturating_add(new_bindings) > MAX_RUN_TARGET_BINDINGS {
            bindings.clear();
        }
        for run_id in run_ids {
            bindings.insert(run_id, lease.target_id.clone());
        }
    }

    pub(crate) async fn run_is_bound(&self, lease: &SelectedTargetLease<'_>, run_id: &str) -> bool {
        self.run_targets
            .read()
            .await
            .get(run_id)
            .is_some_and(|target_id| target_id == &lease.target_id)
    }

    pub(crate) fn subscribe_selection(&self) -> watch::Receiver<u64> {
        self.selection_updates.subscribe()
    }

    pub(crate) fn selection_is_current(&self, target_id: &str, epoch: u64) -> bool {
        self.selection_epoch.load(Ordering::Acquire) == epoch
            && self
                .selected_target_id
                .try_read()
                .is_ok_and(|selected| selected.as_deref() == Some(target_id))
    }

    pub(crate) async fn replace_target(
        &self,
        target_id: &str,
        client: Colossus,
        consent: TargetConsentContext,
    ) -> Option<TargetHandle> {
        self.targets
            .write()
            .await
            .insert(target_id.to_owned(), TargetHandle::new(client, consent))
    }

    pub(crate) async fn remove_target(&self, target_id: &str) -> Option<TargetHandle> {
        self.targets.write().await.remove(target_id)
    }

    pub(crate) async fn connected(&self, target_id: &str) -> bool {
        self.targets
            .read()
            .await
            .get(target_id)
            .is_some_and(|target| !target.is_closed())
    }

    pub(crate) async fn target_is_closed(&self, target_id: &str) -> bool {
        self.targets
            .read()
            .await
            .get(target_id)
            .is_some_and(TargetHandle::is_closed)
    }

    pub(crate) async fn selected_target_id(&self) -> Option<String> {
        self.selected_target_id.read().await.clone()
    }

    pub(crate) async fn select_target(&self, target_id: Option<String>) {
        let _context_guard = self.terminal_context_guard.lock().await;
        let mut selected = self.selected_target_id.write().await;
        if *selected == target_id {
            return;
        }
        *selected = target_id;
        self.run_targets.write().await.clear();
        let epoch = self
            .selection_epoch
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.selection_updates.send_replace(epoch);
        self.advance_terminal_context();
        drop(selected);
        self.terminal_manager.close_owner("terminal");
    }

    pub(crate) async fn managed_health(&self) -> ManagedHealth {
        self.managed_health.read().await.clone()
    }

    pub(crate) async fn set_managed_health(&self, health: ManagedHealth) {
        *self.managed_health.write().await = health;
    }

    pub(crate) fn begin_managed_lifecycle(&self) -> u64 {
        let generation = self
            .managed_lifecycle_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.managed_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        generation
    }

    pub(crate) fn observe_managed_lifecycle(
        &self,
        generation: u64,
        status: watch::Receiver<NativeSidecarStatus>,
    ) {
        if self.managed_lifecycle_generation.load(Ordering::Acquire) != generation {
            return;
        }
        self.managed_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .replace(ManagedLifecycleObservation { generation, status });
    }

    pub(crate) fn clear_managed_lifecycle(&self, generation: u64) {
        if self.managed_lifecycle_generation.load(Ordering::Acquire) != generation {
            return;
        }
        let mut observed = self
            .managed_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if observed
            .as_ref()
            .is_some_and(|observation| observation.generation == generation)
        {
            observed.take();
        }
    }

    pub(crate) async fn sync_managed_lifecycle_health(&self) {
        let observed = self
            .managed_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|observation| (observation.generation, *observation.status.borrow()));
        let Some((generation, status)) = observed else {
            return;
        };
        if self.managed_lifecycle_generation.load(Ordering::Acquire) != generation {
            return;
        }
        let health = match status {
            NativeSidecarStatus::Starting => ManagedHealth {
                state: ManagedRuntimeStateDto::Starting,
                message: "Starting the managed Colossus runtime…".into(),
                failure_code: None,
            },
            NativeSidecarStatus::Ready => ManagedHealth {
                state: ManagedRuntimeStateDto::Ready,
                message: "Managed Local is ready.".into(),
                failure_code: None,
            },
            NativeSidecarStatus::Restarting => ManagedHealth {
                state: ManagedRuntimeStateDto::Restarting,
                message: "Managed Local exited unexpectedly and is restarting safely…".into(),
                failure_code: None,
            },
            NativeSidecarStatus::Stopping => ManagedHealth {
                state: ManagedRuntimeStateDto::Stopping,
                message: "Stopping Managed Local safely…".into(),
                failure_code: None,
            },
            NativeSidecarStatus::Failed(NativeSidecarFailure::WorkspaceIdentityChanged) => {
                ManagedHealth {
                    state: ManagedRuntimeStateDto::Failed,
                    message: "The selected workspace changed. Choose the workspace again.".into(),
                    failure_code: Some(RuntimeFailureCodeDto::Permission),
                }
            }
            NativeSidecarStatus::Failed(NativeSidecarFailure::SupervisionFailed) => ManagedHealth {
                state: ManagedRuntimeStateDto::Failed,
                message:
                    "Managed Local stopped after repeated restart failures. Restart it to continue."
                        .into(),
                failure_code: Some(RuntimeFailureCodeDto::CrashLoop),
            },
        };
        *self.managed_health.write().await = health;
    }

    pub(crate) fn managed_lifecycle_ready(&self) -> bool {
        let current_generation = self.managed_lifecycle_generation.load(Ordering::Acquire);
        self.managed_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|observation| {
                observation.generation == current_generation
                    && *observation.status.borrow() == NativeSidecarStatus::Ready
            })
    }

    pub(crate) async fn begin_external_probe(&self, target_id: &str) -> u64 {
        let generation = self
            .external_health_generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        self.external_health.write().await.insert(
            target_id.to_owned(),
            ExternalHealth {
                state: "checking",
                message: "Checking the authenticated daemon connection…".into(),
                failure_code: None,
                generation,
                probe_not_before: Instant::now(),
                automatic_probe: false,
            },
        );
        generation
    }

    pub(crate) async fn try_begin_external_probe(&self, target_id: &str) -> Option<u64> {
        let mut current = self.external_health.write().await;
        let now = Instant::now();
        if current.get(target_id).is_some_and(|health| {
            health.state == "checking" || !health.automatic_probe || health.probe_not_before > now
        }) {
            return None;
        }
        let generation = self
            .external_health_generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        current.insert(
            target_id.to_owned(),
            ExternalHealth {
                state: "checking",
                message: "Checking the authenticated daemon connection…".into(),
                failure_code: None,
                generation,
                probe_not_before: now,
                automatic_probe: false,
            },
        );
        Some(generation)
    }

    pub(crate) async fn acquire_external_probe_slot(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.external_probe_slots)
            .acquire_owned()
            .await
            .ok()
    }

    pub(crate) async fn finish_external_probe(
        &self,
        target_id: &str,
        generation: u64,
        health: Option<ExternalHealth>,
    ) {
        let mut current = self.external_health.write().await;
        if current
            .get(target_id)
            .is_some_and(|value| value.generation == generation)
        {
            if let Some(mut health) = health {
                health.generation = generation;
                current.insert(target_id.to_owned(), health);
            } else {
                current.remove(target_id);
            }
        }
    }

    pub(crate) async fn external_probe_is_current(&self, target_id: &str, generation: u64) -> bool {
        self.external_health
            .read()
            .await
            .get(target_id)
            .is_some_and(|health| health.generation == generation)
    }

    pub(crate) async fn external_target_snapshot(
        &self,
        target_id: &str,
    ) -> (bool, Option<ExternalHealth>) {
        // Match the health -> target lock order used by failure retirement so a
        // renderer DTO is derived from one coherent native transition boundary.
        let health = self.external_health.read().await;
        let targets = self.targets.read().await;
        let connected = targets
            .get(target_id)
            .is_some_and(|target| !target.is_closed());
        (connected, health.get(target_id).cloned())
    }

    pub(crate) async fn clear_external_health(&self, target_id: &str) {
        self.external_health.write().await.remove(target_id);
    }

    pub(crate) fn try_connect_guard(&self) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        self.connect_guard.try_lock().ok()
    }

    pub(crate) fn try_approval_guard(&self) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        self.approval_guard.try_lock().ok()
    }

    pub(crate) async fn close_all(&self) {
        {
            // Terminal processes may hold worker IPC (notably the bundled TUI).
            // Revoke their document authority and tear down their process trees
            // before asking runtime transports to drain and join IPC tasks.
            let _context_guard = self.terminal_context_guard.lock().await;
            let _lifecycle = self
                .terminal_window_lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.terminal_enabled.store(false, Ordering::Release);
            self.terminal_document_ready.store(false, Ordering::Release);
            self.terminal_window_active.store(false, Ordering::Release);
            self.terminal_window_epoch.fetch_add(1, Ordering::AcqRel);
            self.terminal_launch_request
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pending = false;
            self.terminal_manager.close_owner("terminal");
            self.advance_terminal_context();
        }
        self.set_managed_health(ManagedHealth {
            state: ManagedRuntimeStateDto::Stopping,
            message: "Stopping Managed Local safely…".into(),
            failure_code: None,
        })
        .await;
        let clients = self
            .targets
            .write()
            .await
            .drain()
            .map(|(_, target)| target.client)
            .collect::<Vec<_>>();
        for client in clients {
            let _ = client.close().await;
        }
    }

    pub(crate) fn terminal_manager(&self) -> TerminalManager {
        self.terminal_manager.clone()
    }

    pub(crate) async fn configure_terminal_workspace(&self, workspace: TerminalWorkspace) {
        let _guard = self.terminal_context_guard.lock().await;
        self.terminal_manager
            .close_kind("terminal", TerminalKind::ColossusTui);
        let mut current = self.terminal_workspace.write().await;
        current.replace(workspace);
    }

    pub(crate) async fn clear_terminal_workspace(&self) {
        let _guard = self.terminal_context_guard.lock().await;
        let mut current = self.terminal_workspace.write().await;
        current.take();
        drop(current);
        self.terminal_manager
            .close_kind("terminal", TerminalKind::ColossusTui);
    }

    pub(crate) async fn workspace_for_terminal(
        &self,
        workspace_id: &str,
        context_generation: u64,
    ) -> Option<TerminalWorkspace> {
        if self.terminal_context_generation.load(Ordering::Acquire) != context_generation
            || !self.managed_lifecycle_ready()
        {
            return None;
        }
        self.terminal_workspace
            .read()
            .await
            .as_ref()
            .filter(|workspace| workspace.id == workspace_id)
            .cloned()
    }

    pub(crate) async fn terminal_workspace_context(
        &self,
    ) -> (u64, Option<TerminalWorkspace>, bool) {
        loop {
            let before = self.terminal_context_generation.load(Ordering::Acquire);
            let selected_managed =
                self.selected_target_id.read().await.as_deref() == Some(MANAGED_TARGET_ID);
            let workspace = self.terminal_workspace.read().await.clone();
            let after = self.terminal_context_generation.load(Ordering::Acquire);
            if before == after {
                return (after, workspace, selected_managed);
            }
        }
    }

    pub(crate) async fn lock_terminal_context(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.terminal_context_guard.lock().await
    }

    pub(crate) async fn lock_terminal_window(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.terminal_window_guard.lock().await
    }

    fn advance_terminal_context(&self) {
        self.terminal_context_generation
            .fetch_add(1, Ordering::Release);
    }

    pub(crate) async fn set_terminal_enabled(&self, enabled: bool) {
        let _context_guard = self.terminal_context_guard.lock().await;
        let changed = self.terminal_enabled.swap(enabled, Ordering::AcqRel) != enabled;
        if !enabled {
            self.terminal_manager.close_owner("terminal");
        }
        if changed {
            self.advance_terminal_context();
        }
    }

    pub(crate) fn terminal_enabled(&self) -> bool {
        self.terminal_enabled.load(Ordering::Acquire)
    }

    pub(crate) fn terminal_document_started_for_window(&self, window_epoch: u64) {
        let _lifecycle = self
            .terminal_window_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.terminal_window_active.load(Ordering::Acquire)
            || self.terminal_window_epoch.load(Ordering::Acquire) != window_epoch
        {
            return;
        }
        self.terminal_document_ready.store(false, Ordering::Release);
        self.terminal_manager.close_owner("terminal");
        self.advance_terminal_context();
    }

    pub(crate) fn terminal_document_finished_for_window(&self, window_epoch: u64) {
        let _lifecycle = self
            .terminal_window_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.terminal_window_active.load(Ordering::Acquire)
            && self.terminal_window_epoch.load(Ordering::Acquire) == window_epoch
        {
            self.terminal_document_ready.store(true, Ordering::Release);
        }
    }

    pub(crate) fn terminal_window_destroyed(&self, window_epoch: u64) {
        let _lifecycle = self
            .terminal_window_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.terminal_window_active.load(Ordering::Acquire)
            || self.terminal_window_epoch.load(Ordering::Acquire) != window_epoch
        {
            return;
        }
        self.terminal_manager.close_owner("terminal");
        self.advance_terminal_context();
        let mut request = self
            .terminal_launch_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if request.pending && request.window_epoch == window_epoch {
            request.pending = false;
        }
        self.terminal_document_ready.store(false, Ordering::Release);
        self.terminal_window_active.store(false, Ordering::Release);
        self.terminal_window_epoch.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn terminal_context_is_current(&self, generation: u64) -> bool {
        self.terminal_context_generation.load(Ordering::Acquire) == generation
    }

    pub(crate) fn next_terminal_window_epoch(&self) -> u64 {
        let _lifecycle = self
            .terminal_window_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.terminal_launch_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending = false;
        let epoch = self
            .terminal_window_epoch
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.terminal_document_ready.store(false, Ordering::Release);
        self.terminal_window_active.store(true, Ordering::Release);
        epoch
    }

    pub(crate) fn terminal_document_authority(&self) -> Option<(u64, u64)> {
        let _lifecycle = self
            .terminal_window_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.terminal_window_active.load(Ordering::Acquire)
            || !self.terminal_document_ready.load(Ordering::Acquire)
        {
            return None;
        }
        Some((
            self.terminal_window_epoch.load(Ordering::Acquire),
            self.terminal_context_generation.load(Ordering::Acquire),
        ))
    }

    #[cfg(test)]
    pub(crate) fn request_terminal_kind(
        &self,
        kind: TerminalKind,
        window_epoch: u64,
    ) -> Option<u64> {
        self.request_terminal_launch(kind, None, window_epoch)
    }

    pub(crate) fn request_terminal_launch(
        &self,
        kind: TerminalKind,
        plan_context: Option<TerminalPlanContext>,
        window_epoch: u64,
    ) -> Option<u64> {
        let _lifecycle = self
            .terminal_window_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if window_epoch == 0
            || !self.terminal_window_active.load(Ordering::Acquire)
            || self.terminal_window_epoch.load(Ordering::Acquire) != window_epoch
        {
            return None;
        }
        let mut request = self
            .terminal_launch_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if request.pending {
            return None;
        }
        request.generation = request.generation.wrapping_add(1);
        request.window_epoch = window_epoch;
        request.kind = kind;
        request.plan_context = plan_context;
        request.pending = true;
        Some(request.generation)
    }

    pub(crate) fn cancel_terminal_launch_request(&self, generation: u64) {
        let mut request = self
            .terminal_launch_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if request.pending && request.generation == generation {
            request.pending = false;
        }
    }

    pub(crate) fn take_terminal_launch_request_for_window(
        &self,
        window_epoch: u64,
        document_generation: u64,
    ) -> TerminalLaunchRequest {
        let _lifecycle = self
            .terminal_window_lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut request = self
            .terminal_launch_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let pending = request.clone();
        let claim = self.terminal_window_active.load(Ordering::Acquire)
            && self.terminal_document_ready.load(Ordering::Acquire)
            && pending.pending
            && pending.window_epoch == window_epoch
            && self.terminal_window_epoch.load(Ordering::Acquire) == window_epoch;
        let claim = claim
            && self.terminal_context_generation.load(Ordering::Acquire) == document_generation;
        if claim {
            request.pending = false;
            request.delivered_generation = request.generation;
            return TerminalLaunchRequest {
                pending: true,
                ..request.clone()
            };
        }
        TerminalLaunchRequest {
            generation: pending.delivered_generation,
            pending: false,
            ..pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_workspace_identity() -> colossus_sdk::WorkspaceIdentity {
        colossus_sdk::WorkspaceIdentity::from_macos_parts(1, 2, 1_700_000_000, 0)
            .expect("current workspace identity")
    }

    #[test]
    fn native_watch_admission_is_bounded_per_target() {
        let target = TargetLimits::new();
        let permits: Vec<_> = (0..MAX_NATIVE_WATCHES_PER_TARGET)
            .map(|_| target.try_watch_slot().expect("watch slot"))
            .collect();
        assert!(target.try_watch_slot().is_none());
        drop(permits);
        assert!(target.try_watch_slot().is_some());
    }

    #[test]
    fn native_unary_admission_is_bounded_per_target() {
        let target = TargetLimits::new();
        let permits: Vec<_> = (0..MAX_NATIVE_UNARY_CALLS_PER_TARGET)
            .map(|_| target.try_unary_slot().expect("unary slot"))
            .collect();
        assert!(target.try_unary_slot().is_none());
        drop(permits);
        assert!(target.try_unary_slot().is_some());
    }

    #[test]
    fn concurrent_connect_attempts_fail_fast() {
        let state = AppState::default();
        let guard = state.try_connect_guard().expect("first connect guard");
        assert!(state.try_connect_guard().is_none());
        drop(guard);
        assert!(state.try_connect_guard().is_some());
    }

    #[tokio::test]
    async fn stale_external_probe_cannot_overwrite_or_resurrect_health() {
        let state = AppState::default();
        let stale_generation = state.begin_external_probe("daemon-1").await;
        let current = state.begin_external_probe("daemon-1").await;
        state
            .finish_external_probe(
                "daemon-1",
                stale_generation,
                Some(ExternalHealth::unreachable()),
            )
            .await;
        assert_eq!(
            state
                .external_target_snapshot("daemon-1")
                .await
                .1
                .expect("current health")
                .state,
            "checking"
        );

        state.clear_external_health("daemon-1").await;
        state
            .finish_external_probe("daemon-1", current, Some(ExternalHealth::available()))
            .await;
        assert!(state.external_target_snapshot("daemon-1").await.1.is_none());
    }

    #[tokio::test]
    async fn target_switch_revokes_run_bindings_and_notifies_watchers() {
        let state = AppState::default();
        state
            .run_targets
            .write()
            .await
            .insert("run-1".into(), "external-1".into());
        let mut updates = state.subscribe_selection();

        state.select_target(Some(MANAGED_TARGET_ID.into())).await;

        assert!(state.run_targets.read().await.is_empty());
        updates.changed().await.expect("selection notification");
        assert_eq!(*updates.borrow_and_update(), 1);
    }

    #[tokio::test]
    async fn managed_health_follows_only_the_current_lifecycle_observer() {
        let state = AppState::default();
        let generation = state.begin_managed_lifecycle();
        let (status, receiver) = watch::channel(NativeSidecarStatus::Starting);
        state.observe_managed_lifecycle(generation, receiver);
        state.sync_managed_lifecycle_health().await;
        assert_eq!(
            state.managed_health().await.state,
            ManagedRuntimeStateDto::Starting
        );

        status.send_replace(NativeSidecarStatus::Restarting);
        state.sync_managed_lifecycle_health().await;
        assert_eq!(
            state.managed_health().await.state,
            ManagedRuntimeStateDto::Restarting
        );

        let replacement = state.begin_managed_lifecycle();
        status.send_replace(NativeSidecarStatus::Failed(
            NativeSidecarFailure::SupervisionFailed,
        ));
        state.sync_managed_lifecycle_health().await;
        assert_eq!(
            state.managed_health().await.state,
            ManagedRuntimeStateDto::Restarting,
            "an old lifecycle must not overwrite replacement startup"
        );
        assert_ne!(replacement, generation);
    }

    #[tokio::test]
    async fn workspace_replacement_failure_is_sanitized_and_actionable() {
        let state = AppState::default();
        let generation = state.begin_managed_lifecycle();
        let (status, receiver) = watch::channel(NativeSidecarStatus::Starting);
        state.observe_managed_lifecycle(generation, receiver);

        status.send_replace(NativeSidecarStatus::Restarting);
        state.sync_managed_lifecycle_health().await;
        status.send_replace(NativeSidecarStatus::Failed(
            NativeSidecarFailure::WorkspaceIdentityChanged,
        ));
        state.sync_managed_lifecycle_health().await;

        let health = state.managed_health().await;
        assert_eq!(health.state, ManagedRuntimeStateDto::Failed);
        assert_eq!(health.failure_code, Some(RuntimeFailureCodeDto::Permission));
        assert_eq!(
            health.message,
            "The selected workspace changed. Choose the workspace again."
        );
        assert!(!health.message.contains("restart failures"));
        assert!(!health.message.contains("/private/workspace"));
    }

    #[tokio::test]
    async fn selection_writer_waits_for_in_flight_native_operation() {
        let state = Arc::new(AppState::default());
        *state.selected_target_id.write().await = Some(MANAGED_TARGET_ID.into());
        let operation = state.selected_target_id.read().await;
        let switching = {
            let state = Arc::clone(&state);
            tokio::spawn(async move { state.select_target(None).await })
        };
        tokio::task::yield_now().await;
        assert!(!switching.is_finished());

        drop(operation);
        switching.await.expect("selection task");
        assert!(state.selected_target_id().await.is_none());
    }

    #[tokio::test]
    async fn periodic_external_probes_are_single_flight_and_generation_safe() {
        let state = AppState::default();
        let current = state
            .try_begin_external_probe("daemon-1")
            .await
            .expect("first periodic probe");
        assert!(
            state.try_begin_external_probe("daemon-1").await.is_none(),
            "a second status refresh must not create an overlapping probe"
        );

        state
            .finish_external_probe(
                "daemon-1",
                current.wrapping_sub(1),
                Some(ExternalHealth::unreachable()),
            )
            .await;
        assert_eq!(
            state
                .external_target_snapshot("daemon-1")
                .await
                .1
                .expect("current health")
                .state,
            "checking"
        );

        state
            .finish_external_probe("daemon-1", current, Some(ExternalHealth::unreachable()))
            .await;
        assert_eq!(
            state
                .external_target_snapshot("daemon-1")
                .await
                .1
                .expect("failed health")
                .state,
            "unreachable"
        );
        assert!(
            state.try_begin_external_probe("daemon-1").await.is_none(),
            "renderer refreshes must not bypass the native probe cooldown"
        );

        state.clear_external_health("daemon-1").await;
        let stalled_generation = state
            .try_begin_external_probe("daemon-1")
            .await
            .expect("probe after native removal");
        state
            .finish_external_probe(
                "daemon-1",
                stalled_generation,
                Some(ExternalHealth::stalled()),
            )
            .await;
        assert!(
            state.try_begin_external_probe("daemon-1").await.is_none(),
            "a timed-out keychain probe must remain single-flight until explicit reconnect"
        );
    }

    #[tokio::test]
    async fn terminal_workspace_lookup_is_opaque_and_exact() {
        let state = AppState::default();
        let generation = state.begin_managed_lifecycle();
        let (_status, ready) = watch::channel(NativeSidecarStatus::Ready);
        state.observe_managed_lifecycle(generation, ready);
        state
            .configure_terminal_workspace(TerminalWorkspace {
                id: "workspace:managed".into(),
                display_name: "Managed workspace".into(),
                workspace: "/private/tmp/workspace".into(),
                workspace_identity: test_workspace_identity(),
                config: Some("/private/tmp/config.yaml".into()),
                worker_authentication: None,
            })
            .await;
        let (generation, workspace, _) = state.terminal_workspace_context().await;
        assert_eq!(workspace.expect("active workspace").id, "workspace:managed");
        assert!(
            state
                .workspace_for_terminal("workspace:managed", generation)
                .await
                .is_some()
        );
        assert!(
            state
                .workspace_for_terminal("workspace:other", generation)
                .await
                .is_none()
        );

        state
            .configure_terminal_workspace(TerminalWorkspace {
                id: "workspace:managed".into(),
                display_name: "Managed workspace".into(),
                workspace: "/private/tmp/workspace".into(),
                workspace_identity: test_workspace_identity(),
                config: Some("/private/tmp/config-2.yaml".into()),
                worker_authentication: None,
            })
            .await;
        let restarted = state
            .workspace_for_terminal("workspace:managed", generation)
            .await
            .expect("current workspace with restarted TUI authority");
        assert_eq!(
            restarted.config,
            Some("/private/tmp/config-2.yaml".into()),
            "a new TUI must receive only the current runtime configuration"
        );
    }

    #[tokio::test]
    async fn terminal_context_generation_changes_for_selection_and_consent() {
        let state = AppState::default();
        let (initial, _, _) = state.terminal_workspace_context().await;
        state.set_terminal_enabled(true).await;
        let (enabled, _, _) = state.terminal_workspace_context().await;
        assert_ne!(enabled, initial);

        state.select_target(Some(MANAGED_TARGET_ID.into())).await;
        let (managed_selected, _, selected_managed) = state.terminal_workspace_context().await;
        assert_ne!(managed_selected, enabled);
        assert!(selected_managed);

        state.select_target(Some("external-target".into())).await;
        let (external_selected, _, selected_managed) = state.terminal_workspace_context().await;
        assert_ne!(external_selected, managed_selected);
        assert!(!selected_managed);

        state.clear_terminal_workspace().await;
        let (cleared, workspace, _) = state.terminal_workspace_context().await;
        assert_eq!(
            cleared, external_selected,
            "runtime-only TUI teardown must not revoke an unrelated shell context"
        );
        assert!(workspace.is_none());
    }

    #[tokio::test]
    async fn terminal_document_reload_invalidates_existing_renderer_authority() {
        let state = AppState::default();
        let window_epoch = state.next_terminal_window_epoch();
        assert!(state.terminal_document_authority().is_none());
        state.terminal_document_finished_for_window(window_epoch);
        let (before, _, _) = state.terminal_workspace_context().await;

        state.terminal_document_started_for_window(window_epoch);

        let (after, _, _) = state.terminal_workspace_context().await;
        assert_ne!(after, before);
        assert!(!state.terminal_context_is_current(before));
        assert!(state.terminal_context_is_current(after));
        assert!(
            state.terminal_document_authority().is_none(),
            "navigation start must revoke terminal IPC until the new document finishes"
        );

        state
            .request_terminal_kind(TerminalKind::ColossusTui, window_epoch)
            .expect("new document launch request");
        let old_document_claim =
            state.take_terminal_launch_request_for_window(window_epoch, before);
        assert!(
            !old_document_claim.pending,
            "an old document poll must not claim a post-reload launch request"
        );
        state.terminal_document_finished_for_window(window_epoch);
        let current_document_claim =
            state.take_terminal_launch_request_for_window(window_epoch, after);
        assert!(
            current_document_claim.pending,
            "the current document must receive its launch request"
        );
        assert_ne!(
            old_document_claim.generation, current_document_claim.generation,
            "a failed claim must not expose the still-pending launch generation"
        );
    }

    #[tokio::test]
    async fn app_shutdown_revokes_terminal_document_authority_before_transport_drain() {
        let state = AppState::default();
        state.set_terminal_enabled(true).await;
        let window_epoch = state.next_terminal_window_epoch();
        state.terminal_document_finished_for_window(window_epoch);
        assert!(state.terminal_document_authority().is_some());

        state.close_all().await;

        assert!(!state.terminal_enabled());
        assert!(state.terminal_document_authority().is_none());
        assert!(
            state
                .request_terminal_kind(TerminalKind::ColossusTui, window_epoch)
                .is_none(),
            "shutdown must tombstone the terminal before any runtime close can wait on TUI IPC"
        );
    }

    #[tokio::test]
    async fn terminal_launch_requests_are_monotonic_and_keep_the_requested_kind() {
        let state = AppState::default();
        let initial = state.take_terminal_launch_request_for_window(0, 0);
        assert!(!initial.pending);
        let first_window = state.next_terminal_window_epoch();
        state.terminal_document_finished_for_window(first_window);
        let (_, first_document) = state
            .terminal_document_authority()
            .expect("first document ready");

        state
            .request_terminal_kind(TerminalKind::ColossusTui, first_window)
            .expect("TUI launch request");
        assert!(
            state
                .request_terminal_kind(TerminalKind::ColossusTui, first_window)
                .is_none(),
            "a second command must fail fast instead of overwriting accepted intent"
        );
        let tui = state.take_terminal_launch_request_for_window(first_window, first_document);
        assert_ne!(tui.generation, initial.generation);
        assert_eq!(tui.kind, TerminalKind::ColossusTui);
        assert!(tui.pending);

        let consumed = state.take_terminal_launch_request_for_window(first_window, first_document);
        assert_eq!(consumed.generation, tui.generation);
        assert!(!consumed.pending);

        let plan_context = TerminalPlanContext {
            session_id: "session-1".into(),
            plan_id: "plan-1".into(),
        };
        state
            .request_terminal_launch(
                TerminalKind::ColossusTui,
                Some(plan_context.clone()),
                first_window,
            )
            .expect("second TUI launch request");
        let second_tui =
            state.take_terminal_launch_request_for_window(first_window, first_document);
        assert_ne!(second_tui.generation, tui.generation);
        assert_eq!(second_tui.kind, TerminalKind::ColossusTui);
        assert_eq!(second_tui.plan_context, Some(plan_context));
        assert!(second_tui.pending);

        let pending = state
            .request_terminal_kind(TerminalKind::ColossusTui, first_window)
            .expect("cancelled launch request");
        state.cancel_terminal_launch_request(pending);
        assert!(
            !state
                .take_terminal_launch_request_for_window(first_window, first_document)
                .pending
        );

        state
            .request_terminal_kind(TerminalKind::ColossusTui, first_window)
            .expect("stale launch request");
        let replacement_window = state.next_terminal_window_epoch();
        state.terminal_document_finished_for_window(replacement_window);
        let (_, replacement_document) = state
            .terminal_document_authority()
            .expect("replacement document ready");
        state
            .request_terminal_kind(TerminalKind::ColossusTui, replacement_window)
            .expect("replacement launch request must retire stale pending intent");
        let (replacement_context, _, _) = state.terminal_workspace_context().await;
        state.terminal_window_destroyed(first_window);
        assert!(
            !state
                .take_terminal_launch_request_for_window(first_window, first_document)
                .pending,
            "an old renderer poll must not claim a replacement window request"
        );
        assert!(
            state
                .take_terminal_launch_request_for_window(replacement_window, replacement_document,)
                .pending,
            "delayed destruction cleanup must not clear a replacement window request"
        );
        assert_eq!(
            state.terminal_context_generation.load(Ordering::Acquire),
            replacement_context,
            "delayed destruction cleanup must not invalidate replacement authority"
        );

        state.terminal_window_destroyed(replacement_window);
        assert!(
            state
                .request_terminal_kind(TerminalKind::ColossusTui, replacement_window)
                .is_none(),
            "a destroyed window epoch must never accept another launch intent"
        );
        assert!(
            state
                .request_terminal_kind(
                    TerminalKind::ColossusTui,
                    state.terminal_window_epoch.load(Ordering::Acquire),
                )
                .is_none(),
            "the destruction tombstone must not act as a live window epoch"
        );
    }
}
