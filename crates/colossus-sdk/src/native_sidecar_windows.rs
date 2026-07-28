use crate::{
    AgentRunClient, ApiResult, ArtifactClient, Backend, BackendKind, CancelRunRequest,
    CancelRunResponse, CreateRunRequest, CreateRunResponse, CredentialProvider, GetRunRequest,
    GetRunResponse, GrpcBackend, GrpcConnectOptions, ListRunsRequest, ListRunsResponse,
    NativeSidecarFailure, NativeSidecarStatus, RespondInteractionRequest,
    RespondInteractionResponse, RunUpdateStream, SdkError, SdkResult, Secret, ServerCapabilities,
    SidecarBootstrapConfig, SidecarLifecycle, SidecarOptions, TlsFingerprint, WatchRunRequest,
};
use async_trait::async_trait;
use colossus_sidecar_protocol::{
    AckRequest, ActivatedResponse, ChildFrame, FailureCode, MAX_FRAME_BYTES, PROTOCOL_VERSION,
    ParentFrame, ReadyResponse, WorkspaceIdentity, decode_payload, encode_frame,
};
use colossus_windows_native::{BoundPath, FileIdentity, KillOnCloseJob};
use sha2::{Digest as _, Sha256};
use std::{
    fmt,
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::windows::named_pipe::{NamedPipeServer, ServerOptions},
    process::{Child, Command},
    sync::{Mutex, watch},
    time::{Instant, sleep, timeout},
};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);
const CONNECT_DEADLINE: Duration = Duration::from_secs(10);
const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_secs(40);
const MAX_VERIFIED_EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PIPE_NAME_BYTES: usize = 256;
const PIPE_ENVIRONMENT: &str = "COLOSSUS_WINDOWS_BOOTSTRAP_PIPE_V1";
const PARENT_ENVIRONMENT: &str = "COLOSSUS_WINDOWS_BOOTSTRAP_PARENT_PID_V1";
const PIPE_PREFIX: &str = r"\\.\pipe\colossus-managed-";
const PUBLIC_API_DIRECTORY: &str = "public-api";
const DESCRIPTOR_FILENAME: &str = "endpoint.json";
const CERTIFICATE_FILENAME: &str = "certificate.pem";

/// Authenticated Windows lifecycle for one app-owned Managed Local runtime.
pub struct NativeSidecarLifecycle {
    bootstrap: Arc<SidecarBootstrapConfig>,
    status: watch::Sender<NativeSidecarStatus>,
}

impl NativeSidecarLifecycle {
    /// Create a native lifecycle for one app-owned runtime configuration.
    pub fn new(bootstrap: SidecarBootstrapConfig) -> Self {
        let (status, _) = watch::channel(NativeSidecarStatus::Starting);
        Self {
            bootstrap: Arc::new(bootstrap),
            status,
        }
    }

    /// Return the lifecycle's current secret-free supervision state.
    pub fn status(&self) -> NativeSidecarStatus {
        *self.status.borrow()
    }

    /// Subscribe to supervision state changes without exposing process or transport data.
    pub fn subscribe_status(&self) -> watch::Receiver<NativeSidecarStatus> {
        self.status.subscribe()
    }
}

impl fmt::Debug for NativeSidecarLifecycle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSidecarLifecycle")
            .field("bootstrap", &self.bootstrap)
            .field("status", &self.status())
            .finish()
    }
}

#[async_trait]
impl SidecarLifecycle for NativeSidecarLifecycle {
    async fn start_verified(&self, options: &SidecarOptions) -> SdkResult<Arc<dyn Backend>> {
        self.status.send_replace(NativeSidecarStatus::Starting);
        match launch(options, &self.bootstrap, self.status.clone()).await {
            Ok(backend) => {
                self.status.send_replace(NativeSidecarStatus::Ready);
                Ok(Arc::new(backend))
            }
            Err(error) => {
                let failure = if matches!(error, SdkError::WorkspaceIdentityChanged) {
                    NativeSidecarFailure::WorkspaceIdentityChanged
                } else {
                    NativeSidecarFailure::SupervisionFailed
                };
                self.status
                    .send_replace(NativeSidecarStatus::Failed(failure));
                Err(error)
            }
        }
    }
}

struct BoundWorkspace {
    binding: BoundPath,
    identity: WorkspaceIdentity,
}

impl BoundWorkspace {
    fn open(path: &Path, expected: Option<&WorkspaceIdentity>) -> SdkResult<Self> {
        let binding =
            BoundPath::open_directory(path).map_err(|_| SdkError::WorkspaceIdentityChanged)?;
        let kernel = binding.identity();
        let identity =
            WorkspaceIdentity::from_windows_parts(kernel.volume_serial_number, kernel.file_id)
                .map_err(|_| SdkError::WorkspaceIdentityChanged)?;
        if expected.is_some_and(|expected| expected != &identity) {
            return Err(SdkError::WorkspaceIdentityChanged);
        }
        binding
            .revalidate()
            .map_err(|_| SdkError::WorkspaceIdentityChanged)?;
        Ok(Self { binding, identity })
    }

    fn revalidate(&self) -> SdkResult<()> {
        self.binding
            .revalidate()
            .map_err(|_| SdkError::WorkspaceIdentityChanged)
    }
}

struct VerifiedImage {
    identity: FileIdentity,
    _snapshot: File,
}

fn verify_executable(executable: &crate::VerifiedExecutable) -> SdkResult<VerifiedImage> {
    let binding =
        BoundPath::open_file(executable.path()).map_err(|_| SdkError::IdentityMismatch)?;
    let mut source = binding
        .try_clone_file()
        .map_err(|_| SdkError::IdentityMismatch)?;
    let metadata = source.metadata().map_err(|_| SdkError::IdentityMismatch)?;
    if metadata.len() == 0 || metadata.len() > MAX_VERIFIED_EXECUTABLE_BYTES {
        return Err(SdkError::IdentityMismatch);
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|_| SdkError::IdentityMismatch)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    if digest.finalize().as_slice() != executable.sha256().as_bytes() {
        return Err(SdkError::IdentityMismatch);
    }
    source
        .seek(SeekFrom::Start(0))
        .map_err(|_| SdkError::IdentityMismatch)?;
    binding
        .revalidate()
        .map_err(|_| SdkError::IdentityMismatch)?;
    Ok(VerifiedImage {
        identity: binding.identity(),
        _snapshot: source,
    })
}

async fn launch(
    options: &SidecarOptions,
    bootstrap: &SidecarBootstrapConfig,
    status: watch::Sender<NativeSidecarStatus>,
) -> SdkResult<WindowsSidecarBackend> {
    let instance = BoundPath::open_directory(options.instance_dir().as_path())
        .map_err(|_| SdkError::IdentityMismatch)?;
    instance
        .validate_private_owner_dacl()
        .and_then(|()| instance.revalidate())
        .map_err(|_| SdkError::IdentityMismatch)?;
    if instance.canonical_path() != options.instance_dir().as_path() {
        return Err(SdkError::IdentityMismatch);
    }
    let workspace = BoundWorkspace::open(
        bootstrap.workspace(),
        bootstrap.expected_workspace_identity(),
    )?;
    let request = bootstrap.request(
        options,
        workspace.binding.canonical_path(),
        workspace.identity.clone(),
    )?;
    let executable = verify_executable(options.executable())?;

    let pipe_name = format!("{PIPE_PREFIX}{}", Uuid::now_v7());
    if pipe_name.len() > MAX_PIPE_NAME_BYTES {
        return Err(SdkError::SidecarFailed);
    }
    let mut pipe = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .access_inbound(true)
        .access_outbound(true)
        .create(&pipe_name)
        .map_err(|_| SdkError::SidecarFailed)?;
    let mut command = Command::new(options.executable().path());
    command
        .arg("__managed-sidecar-v1")
        .env_clear()
        .env(PIPE_ENVIRONMENT, &pipe_name)
        .env(PARENT_ENVIRONMENT, std::process::id().to_string())
        .current_dir(instance.canonical_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    colossus_windows_native::configure_suspended_process(command.as_std_mut());
    let mut child = command.spawn().map_err(|_| SdkError::SidecarFailed)?;
    let (job, process_id) =
        match KillOnCloseJob::assign_tokio_child_verify_and_resume(&child, executable.identity) {
            Ok(verified) => verified,
            Err(_) => {
                let _ = child.start_kill();
                return Err(SdkError::IdentityMismatch);
            }
        };
    timeout(BOOTSTRAP_TIMEOUT, pipe.connect())
        .await
        .map_err(|_| SdkError::SidecarFailed)?
        .map_err(|_| SdkError::SidecarFailed)?;
    colossus_windows_native::validate_named_pipe_client(&pipe, process_id)
        .map_err(|_| SdkError::IdentityMismatch)?;

    workspace.revalidate()?;
    write_async_frame(&mut pipe, &ParentFrame::Bootstrap(Box::new(request))).await?;
    let ready = match read_async_frame::<ChildFrame>(&mut pipe).await? {
        ChildFrame::Ready(ready) => ready,
        ChildFrame::Failed(failure) => return Err(map_child_failure(failure.code)),
        ChildFrame::Activated(_) => return Err(SdkError::IdentityMismatch),
    };
    validate_ready(options, &ready)?;
    validate_discovery_directory(instance.canonical_path())?;

    let endpoint = Url::parse(&ready.endpoint).map_err(|_| SdkError::IdentityMismatch)?;
    let fingerprint = TlsFingerprint::from_hex(&ready.certificate_sha256)
        .map_err(|_| SdkError::IdentityMismatch)?;
    let certificate_pem = ready.certificate_pem.as_bytes().to_vec();
    let primary_credential: Arc<dyn CredentialProvider> = Arc::new(MemoryCredentialProvider {
        bearer: Zeroizing::new(ready.bearer.expose().as_bytes().to_vec()),
    });
    let approval_credential = ready.approval_broker_bearer.as_ref().map(|bearer| {
        Arc::new(MemoryCredentialProvider {
            bearer: Zeroizing::new(bearer.expose().as_bytes().to_vec()),
        }) as Arc<dyn CredentialProvider>
    });
    sidecar_connect_options(
        options,
        &endpoint,
        fingerprint,
        &certificate_pem,
        Arc::clone(&primary_credential),
    )?;
    write_async_frame(
        &mut pipe,
        &ParentFrame::Ack(AckRequest {
            protocol_version: PROTOCOL_VERSION,
            exchange_id: ready.exchange_id.clone(),
            credential_id: ready.credential_id.clone(),
            approval_broker_credential_id: ready.approval_broker_credential_id.clone(),
        }),
    )
    .await?;
    let activated = match read_async_frame::<ChildFrame>(&mut pipe).await? {
        ChildFrame::Activated(activated) => activated,
        ChildFrame::Failed(failure) => return Err(map_child_failure(failure.code)),
        ChildFrame::Ready(_) => return Err(SdkError::IdentityMismatch),
    };
    validate_activated(
        &activated,
        &ready.exchange_id,
        &ready.credential_id,
        ready.approval_broker_credential_id.as_deref(),
    )?;

    let deadline = Instant::now() + CONNECT_DEADLINE;
    let primary = connect_sidecar(
        options,
        &endpoint,
        fingerprint,
        &certificate_pem,
        primary_credential,
        deadline,
    )
    .await?;
    let approval = if let Some(credential) = approval_credential {
        Some(
            connect_sidecar(
                options,
                &endpoint,
                fingerprint,
                &certificate_pem,
                credential,
                deadline,
            )
            .await?,
        )
    } else {
        None
    };
    let (agent_runs_closed, _) = watch::channel(false);
    let agent_runs = Arc::new(WindowsAgentRuns {
        primary: Arc::clone(&primary),
        approval: approval.clone(),
        closed: agent_runs_closed,
    });
    Ok(WindowsSidecarBackend {
        primary,
        approval,
        agent_runs,
        child: Mutex::new(Some(child)),
        pipe: Mutex::new(Some(pipe)),
        job: Mutex::new(Some(job)),
        instance_root: instance.canonical_path().to_owned(),
        closed: AtomicBool::new(false),
        status,
    })
}

struct MemoryCredentialProvider {
    bearer: Zeroizing<Vec<u8>>,
}

#[async_trait]
impl CredentialProvider for MemoryCredentialProvider {
    async fn load(&self) -> SdkResult<Secret> {
        Secret::new(self.bearer.to_vec())
    }
}

struct WindowsAgentRuns {
    primary: Arc<GrpcBackend>,
    approval: Option<Arc<GrpcBackend>>,
    closed: watch::Sender<bool>,
}

#[async_trait]
impl AgentRunClient for WindowsAgentRuns {
    async fn create_run(&self, request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        self.primary.agent_runs().create_run(request).await
    }

    async fn get_run(&self, request: GetRunRequest) -> ApiResult<GetRunResponse> {
        self.primary.agent_runs().get_run(request).await
    }

    async fn list_runs(&self, request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        self.primary.agent_runs().list_runs(request).await
    }

    async fn watch_run(&self, request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        self.primary.agent_runs().watch_run(request).await
    }

    fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    async fn wait_closed(&self) {
        let mut closed = self.closed.subscribe();
        if *closed.borrow() {
            return;
        }
        while closed.changed().await.is_ok() {
            if *closed.borrow() {
                return;
            }
        }
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> ApiResult<CancelRunResponse> {
        self.primary.agent_runs().cancel_run(request).await
    }

    async fn respond_interaction(
        &self,
        request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        if let Some(approval) = &self.approval {
            approval.agent_runs().respond_interaction(request).await
        } else {
            self.primary.agent_runs().respond_interaction(request).await
        }
    }
}

struct WindowsSidecarBackend {
    primary: Arc<GrpcBackend>,
    approval: Option<Arc<GrpcBackend>>,
    agent_runs: Arc<WindowsAgentRuns>,
    child: Mutex<Option<Child>>,
    pipe: Mutex<Option<NamedPipeServer>>,
    job: Mutex<Option<KillOnCloseJob>>,
    instance_root: PathBuf,
    closed: AtomicBool,
    status: watch::Sender<NativeSidecarStatus>,
}

impl fmt::Debug for WindowsSidecarBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowsSidecarBackend")
            .field("kind", &BackendKind::Sidecar)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Backend for WindowsSidecarBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Sidecar
    }

    fn agent_runs(&self) -> Arc<dyn AgentRunClient> {
        self.agent_runs.clone()
    }

    fn capabilities(&self) -> ServerCapabilities {
        self.primary.capabilities()
    }

    fn artifacts(&self) -> Option<Arc<dyn ArtifactClient>> {
        self.primary.artifacts()
    }

    async fn close(&self) -> SdkResult<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.status.send_replace(NativeSidecarStatus::Stopping);
        self.agent_runs.closed.send_replace(true);
        let _ = self.primary.close().await;
        if let Some(approval) = &self.approval {
            let _ = approval.close().await;
        }
        self.pipe.lock().await.take();
        let mut child = self.child.lock().await;
        if let Some(process) = child.as_mut()
            && timeout(GRACEFUL_CLOSE_TIMEOUT, process.wait())
                .await
                .is_err()
        {
            if let Some(job) = self.job.lock().await.as_ref() {
                let _ = job.terminate();
            }
            let _ = process.start_kill();
            let _ = process.wait().await;
        }
        child.take();
        self.job.lock().await.take();
        cleanup_discovery(&self.instance_root)?;
        Ok(())
    }
}

impl Drop for WindowsSidecarBackend {
    fn drop(&mut self) {
        self.status.send_replace(NativeSidecarStatus::Stopping);
        self.agent_runs.closed.send_replace(true);
        if let Ok(mut pipe) = self.pipe.try_lock() {
            pipe.take();
        }
        if let Ok(mut job) = self.job.try_lock()
            && let Some(job) = job.take()
        {
            let _ = job.terminate();
        }
    }
}

fn validate_discovery_directory(instance_root: &Path) -> SdkResult<()> {
    let directory = BoundPath::open_directory(&instance_root.join(PUBLIC_API_DIRECTORY))
        .map_err(|_| SdkError::IdentityMismatch)?;
    directory
        .validate_private_owner_dacl()
        .and_then(|()| directory.revalidate())
        .map_err(|_| SdkError::IdentityMismatch)?;
    if directory.canonical_path().parent() != Some(instance_root) {
        return Err(SdkError::IdentityMismatch);
    }
    Ok(())
}

fn cleanup_discovery(instance_root: &Path) -> SdkResult<()> {
    let directory = instance_root.join(PUBLIC_API_DIRECTORY);
    for name in [DESCRIPTOR_FILENAME, CERTIFICATE_FILENAME] {
        let path = directory.join(name);
        match BoundPath::open_file(&path) {
            Ok(binding) => {
                binding.revalidate().map_err(|_| SdkError::CloseFailed)?;
                std::fs::remove_file(path).map_err(|_| SdkError::CloseFailed)?;
            }
            Err(_) if !path.exists() => {}
            Err(_) => return Err(SdkError::CloseFailed),
        }
    }
    Ok(())
}

fn sidecar_connect_options(
    options: &SidecarOptions,
    endpoint: &Url,
    fingerprint: TlsFingerprint,
    certificate_pem: &[u8],
    credential: Arc<dyn CredentialProvider>,
) -> SdkResult<GrpcConnectOptions> {
    GrpcConnectOptions::new(
        BackendKind::Sidecar,
        options.instance_id(),
        options.api_major(),
        endpoint.clone(),
        fingerprint,
        certificate_pem.to_vec(),
        credential,
    )
}

async fn connect_sidecar(
    options: &SidecarOptions,
    endpoint: &Url,
    fingerprint: TlsFingerprint,
    certificate_pem: &[u8],
    credential: Arc<dyn CredentialProvider>,
    deadline: Instant,
) -> SdkResult<Arc<GrpcBackend>> {
    loop {
        let connect = GrpcBackend::connect(sidecar_connect_options(
            options,
            endpoint,
            fingerprint,
            certificate_pem,
            Arc::clone(&credential),
        )?);
        match timeout(CONNECT_ATTEMPT_TIMEOUT, connect).await {
            Ok(Ok(transport)) => return Ok(Arc::new(transport)),
            Ok(Err(SdkError::Transport)) | Err(_) if Instant::now() < deadline => {
                sleep(Duration::from_millis(50)).await;
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(SdkError::Transport),
        }
    }
}

fn map_child_failure(code: FailureCode) -> SdkError {
    match code {
        FailureCode::InvalidBootstrap | FailureCode::InvalidInstanceDirectory => {
            SdkError::IdentityMismatch
        }
        FailureCode::InvalidWorkspace => SdkError::WorkspaceIdentityChanged,
        FailureCode::InvalidConfiguration => {
            SdkError::InvalidConfiguration("managed sidecar configuration was rejected")
        }
        FailureCode::WorkspaceBusy => SdkError::Busy,
        FailureCode::PublicApiSetup | FailureCode::CredentialActivation => SdkError::Authentication,
        FailureCode::RuntimeFailed => SdkError::SidecarFailed,
    }
}

fn validate_ready(options: &SidecarOptions, ready: &ReadyResponse) -> SdkResult<()> {
    ready.validate().map_err(|_| SdkError::IdentityMismatch)?;
    if ready.instance_id != options.instance_id().to_string()
        || ready.api_major != options.api_major().get()
    {
        return Err(SdkError::IdentityMismatch);
    }
    Ok(())
}

fn validate_activated(
    activated: &ActivatedResponse,
    exchange_id: &str,
    credential_id: &str,
    approval_broker_credential_id: Option<&str>,
) -> SdkResult<()> {
    if activated.protocol_version != PROTOCOL_VERSION
        || activated.exchange_id != exchange_id
        || activated.credential_id != credential_id
        || activated.approval_broker_credential_id.as_deref() != approval_broker_credential_id
    {
        return Err(SdkError::IdentityMismatch);
    }
    Ok(())
}

async fn write_async_frame<T: serde::Serialize>(
    pipe: &mut NamedPipeServer,
    value: &T,
) -> SdkResult<()> {
    let frame = encode_frame(value).map_err(|_| SdkError::SidecarFailed)?;
    pipe.write_all(frame.as_slice())
        .await
        .map_err(|_| SdkError::SidecarFailed)?;
    pipe.flush().await.map_err(|_| SdkError::SidecarFailed)
}

async fn read_async_frame<T: serde::de::DeserializeOwned>(
    pipe: &mut NamedPipeServer,
) -> SdkResult<T> {
    let mut length = [0_u8; 4];
    pipe.read_exact(&mut length)
        .await
        .map_err(|_| SdkError::SidecarFailed)?;
    let length =
        usize::try_from(u32::from_be_bytes(length)).map_err(|_| SdkError::SidecarFailed)?;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(SdkError::SidecarFailed);
    }
    let mut payload = Zeroizing::new(vec![0_u8; length]);
    pipe.read_exact(payload.as_mut_slice())
        .await
        .map_err(|_| SdkError::SidecarFailed)?;
    decode_payload(payload.as_slice()).map_err(|_| SdkError::SidecarFailed)
}
