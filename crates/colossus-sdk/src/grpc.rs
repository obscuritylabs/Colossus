use crate::{
    AgentRunClient, ApiError, ApiErrorCode, ApiErrorReason, ApiResult, ApprovalInteraction,
    ApprovalRisk, ArchiveThreadRequest, ArtifactClient, ArtifactPurpose, ArtifactReference,
    ArtifactState, Backend, BackendKind, CancelRunRequest, CancelRunResponse, CreateRunRequest,
    CreateRunResponse, CredentialProvider, DownloadedArtifact, FieldViolation, GetRunRequest,
    GetRunResponse, InputContentPart, Interaction, InteractionAnswer, InteractionContent,
    InteractionKind, InteractionStatus, ListRunsRequest, ListRunsResponse, MessageContentPart,
    MessageRole, OutcomeCertainty, PageResponse, PlanExecutionStrategy, PlanRunAction, PlanStatus,
    PromptAnswer, PromptChoice, ResearchDepth, ResearchSourceKind, RespondInteractionRequest,
    RespondInteractionResponse, RestoreThreadRequest, Run, RunBranchContextMode, RunCancellation,
    RunFailure, RunMode, RunResult, RunStatus, RunTerminal, RunUpdate, RunUpdateKind,
    RunUpdateStream, SdkError, SdkResult, ServerCapabilities, SessionMessage, ThreadLifecycle,
    TlsFingerprint, TokenUsage, ToolActivity, ToolActivityState, UploadArtifactRequest,
    WatchRunRequest,
};
use async_trait::async_trait;
use colossus_api::{RequestId, validate_public_approval_display};
use colossus_api_proto::{
    google_rpc::Status as RichStatus,
    v1alpha1::{
        self as proto, ColossusErrorDetail, agent_run_service_client::AgentRunServiceClient,
        artifact_service_client::ArtifactServiceClient, content_part, interaction, plan_run_action,
        prompt_answer, respond_interaction_request, run, run_update,
        system_service_client::SystemServiceClient,
    },
};
use colossus_grpc::{PUBLIC_API_VERSION, validate_endpoint_certificate_pem};
use futures::{StreamExt as _, stream};
use prost::Message as _;
use prost_types::Timestamp;
use rustls::{
    CertificateError, DigitallySignedStruct, Error as RustlsError, RootCertStore, SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject as _},
};
use sha2::{Digest as _, Sha256};
use std::{fmt, sync::Arc, time::Duration};
use tokio::sync::watch;
use tonic::{
    Code, Request, Status,
    metadata::{Ascii, MetadataValue},
    transport::{Channel, ClientTlsConfig, Endpoint},
};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_HEADER_LIST_BYTES: u32 = 16 * 1024;
const MAX_CERTIFICATE_PEM_BYTES: usize = 256 * 1024;
const MAX_ERROR_DETAILS_BYTES: usize = 64 * 1024;
const MAX_ERROR_MESSAGE_BYTES: usize = 4 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 256;
const MAX_OPAQUE_BYTES: usize = 2 * 1024;
const MAX_VISIBLE_TEXT_BYTES: usize = 1024 * 1024;
const MAX_SUMMARY_BYTES: usize = 64 * 1024;
const MAX_COLLECTION_ITEMS: usize = 1024;
const MAX_ARTIFACT_BYTES: usize = 16 * 1_048_576;
const DEFAULT_ARTIFACT_CHUNK_BYTES: usize = 256 * 1024;
const ERROR_DETAIL_TYPE_URL: &str = "type.googleapis.com/colossus.api.v1alpha1.ColossusErrorDetail";

/// Complete material required to establish one pinned authenticated gRPC channel.
///
/// The certificate is public TLS material loaded from a separately protected discovery
/// file. The credential provider normally reads an application bearer from an OS
/// credential store; neither value belongs in the endpoint descriptor, argv, or the
/// environment.
pub struct GrpcConnectOptions {
    backend_kind: BackendKind,
    expected_instance_id: crate::InstanceId,
    api_major: crate::ApiMajor,
    endpoint: Url,
    tls_fingerprint: TlsFingerprint,
    certificate_pem: Vec<u8>,
    credential_provider: Arc<dyn CredentialProvider>,
}

impl GrpcConnectOptions {
    /// Validate an exact loopback endpoint and pinned leaf certificate.
    pub fn new(
        backend_kind: BackendKind,
        expected_instance_id: crate::InstanceId,
        api_major: crate::ApiMajor,
        endpoint: Url,
        tls_fingerprint: TlsFingerprint,
        certificate_pem: impl Into<Vec<u8>>,
        credential_provider: Arc<dyn CredentialProvider>,
    ) -> SdkResult<Self> {
        if backend_kind == BackendKind::Embedded {
            return Err(SdkError::InvalidConfiguration(
                "an embedded backend cannot use the gRPC transport",
            ));
        }
        expected_instance_id.validate()?;
        if api_major.get() != 1 {
            return Err(SdkError::VersionMismatch);
        }
        crate::daemon::validate_loopback_endpoint(&endpoint)?;
        let certificate_pem = certificate_pem.into();
        validate_certificate_pin(&certificate_pem, tls_fingerprint)?;
        Ok(Self {
            backend_kind,
            expected_instance_id,
            api_major,
            endpoint,
            tls_fingerprint,
            certificate_pem,
            credential_provider,
        })
    }
}

impl fmt::Debug for GrpcConnectOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GrpcConnectOptions")
            .field("backend_kind", &self.backend_kind)
            .field("expected_instance_id", &self.expected_instance_id)
            .field("api_major", &self.api_major)
            .field("endpoint", &self.endpoint)
            .field("tls_fingerprint", &self.tls_fingerprint)
            .field("certificate_pem_bytes", &self.certificate_pem.len())
            .field("credential_provider", &"[REDACTED]")
            .finish()
    }
}

/// Concrete pinned-TLS, bearer-authenticated public gRPC backend.
pub struct GrpcBackend {
    kind: BackendKind,
    agent_runs: Arc<GrpcAgentRunClient>,
    artifacts: Arc<GrpcArtifactClient>,
    capabilities: ServerCapabilities,
    closed: watch::Sender<bool>,
}

impl GrpcBackend {
    /// Establish a real bounded gRPC channel from already security-validated material.
    pub async fn connect(options: GrpcConnectOptions) -> SdkResult<Self> {
        validate_certificate_pin(&options.certificate_pem, options.tls_fingerprint)?;
        let host = options
            .endpoint
            .host_str()
            .ok_or(SdkError::InvalidConfiguration(
                "gRPC endpoint must have a loopback host",
            ))?
            .to_owned();
        let verifier = pinned_server_verifier(&options.certificate_pem, options.tls_fingerprint)?;
        let tls = ClientTlsConfig::new().domain_name(host);
        let channel = Endpoint::from_shared(options.endpoint.to_string())
            .map_err(|_| SdkError::InvalidConfiguration("gRPC endpoint URI is invalid"))?
            .tls_config_with_verifier(tls, verifier)
            .map_err(|_| SdkError::IdentityMismatch)?
            .connect_timeout(Duration::from_secs(5))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .http2_max_header_list_size(MAX_HEADER_LIST_BYTES)
            .keep_alive_timeout(Duration::from_secs(10))
            .tcp_nodelay(true)
            .connect()
            .await
            .map_err(|_| SdkError::Transport)?;
        let (closed, _) = watch::channel(false);
        let credential_provider = options.credential_provider;
        let artifacts = Arc::new(GrpcArtifactClient {
            channel: channel.clone(),
            credential_provider: Arc::clone(&credential_provider),
            closed: closed.clone(),
        });
        let agent_runs = Arc::new(GrpcAgentRunClient {
            channel,
            credential_provider,
            closed: closed.clone(),
        });
        let capabilities = verify_server_identity(
            agent_runs.as_ref(),
            options.expected_instance_id,
            options.api_major,
            options.backend_kind,
        )
        .await?;
        Ok(Self {
            kind: options.backend_kind,
            agent_runs,
            artifacts,
            capabilities,
            closed,
        })
    }

    /// Connect to one daemon descriptor after its discovery adapter validated identity.
    #[cfg(feature = "daemon")]
    pub async fn connect_daemon(
        options: &crate::DaemonConnectOptions,
        descriptor: &crate::DaemonDescriptor,
        certificate_pem: impl Into<Vec<u8>>,
    ) -> SdkResult<Self> {
        descriptor.validate_for(options)?;
        Self::connect(GrpcConnectOptions::new(
            BackendKind::Daemon,
            options.instance_id(),
            options.api_major(),
            descriptor.endpoint().clone(),
            options.expected_tls_fingerprint(),
            certificate_pem,
            options.credential_provider_arc(),
        )?)
        .await
    }
}

impl fmt::Debug for GrpcBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GrpcBackend")
            .field("kind", &self.kind)
            .field("credential", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Backend for GrpcBackend {
    fn kind(&self) -> BackendKind {
        self.kind
    }

    fn agent_runs(&self) -> Arc<dyn AgentRunClient> {
        self.agent_runs.clone()
    }

    fn capabilities(&self) -> ServerCapabilities {
        self.capabilities.clone()
    }

    fn artifacts(&self) -> Option<Arc<dyn ArtifactClient>> {
        (self.capabilities.contains("artifacts.read")
            || self.capabilities.contains("artifacts.upload"))
        .then(|| self.artifacts.clone() as Arc<dyn ArtifactClient>)
    }

    async fn close(&self) -> SdkResult<()> {
        self.closed.send_replace(true);
        Ok(())
    }
}

struct GrpcAgentRunClient {
    channel: Channel,
    credential_provider: Arc<dyn CredentialProvider>,
    closed: watch::Sender<bool>,
}

struct GrpcArtifactClient {
    channel: Channel,
    credential_provider: Arc<dyn CredentialProvider>,
    closed: watch::Sender<bool>,
}

struct PinnedServerCertVerifier {
    inner: Arc<WebPkiServerVerifier>,
    expected_leaf_sha256: [u8; 32],
}

impl fmt::Debug for PinnedServerCertVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinnedServerCertVerifier")
            .field("expected_leaf_sha256", &"[PINNED]")
            .finish_non_exhaustive()
    }
}

impl ServerCertVerifier for PinnedServerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        if !intermediates.is_empty()
            || Sha256::digest(end_entity.as_ref()).as_slice() != self.expected_leaf_sha256
        {
            return Err(RustlsError::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ));
        }
        self.inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _certificate: &CertificateDer<'_>,
        _signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        reject_tls12_signature()
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        self.inner
            .verify_tls13_signature(message, certificate, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }

    fn requires_raw_public_keys(&self) -> bool {
        self.inner.requires_raw_public_keys()
    }

    fn root_hint_subjects(&self) -> Option<&[rustls::DistinguishedName]> {
        self.inner.root_hint_subjects()
    }
}

fn reject_tls12_signature() -> Result<HandshakeSignatureValid, RustlsError> {
    Err(RustlsError::InvalidCertificate(
        CertificateError::ApplicationVerificationFailure,
    ))
}

fn pinned_server_verifier(
    certificate_pem: &[u8],
    expected: TlsFingerprint,
) -> SdkResult<Arc<dyn ServerCertVerifier>> {
    validate_certificate_pin(certificate_pem, expected)?;
    let certificate = CertificateDer::pem_slice_iter(certificate_pem)
        .next()
        .ok_or(SdkError::IdentityMismatch)?
        .map_err(|_| SdkError::IdentityMismatch)?;
    let mut roots = RootCertStore::empty();
    roots
        .add(certificate)
        .map_err(|_| SdkError::IdentityMismatch)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
        .build()
        .map_err(|_| SdkError::IdentityMismatch)?;
    Ok(Arc::new(PinnedServerCertVerifier {
        inner,
        expected_leaf_sha256: *expected.as_bytes(),
    }))
}

impl fmt::Debug for GrpcAgentRunClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GrpcAgentRunClient")
            .field("credential", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl GrpcAgentRunClient {
    fn client(&self) -> AgentRunServiceClient<Channel> {
        AgentRunServiceClient::new(self.channel.clone())
            .max_decoding_message_size(MAX_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_MESSAGE_BYTES)
    }

    async fn request<T>(&self, message: T) -> ApiResult<Request<T>> {
        if *self.closed.borrow() {
            return Err(closed_error());
        }
        let credential = self
            .credential_provider
            .load()
            .await
            .map_err(|_| authentication_error())?;
        let mut bearer = Zeroizing::new(Vec::with_capacity(
            b"Bearer ".len().saturating_add(credential.expose().len()),
        ));
        bearer.extend_from_slice(b"Bearer ");
        bearer.extend_from_slice(credential.expose());
        let mut metadata = MetadataValue::<Ascii>::try_from(bearer.as_slice())
            .map_err(|_| authentication_error())?;
        metadata.set_sensitive(true);
        let mut request = Request::new(message);
        request.metadata_mut().insert("authorization", metadata);
        Ok(request)
    }
}

async fn verify_server_identity(
    client: &GrpcAgentRunClient,
    expected_instance_id: crate::InstanceId,
    api_major: crate::ApiMajor,
    backend_kind: BackendKind,
) -> SdkResult<ServerCapabilities> {
    let request = client
        .request(proto::GetServerInfoRequest {})
        .await
        .map_err(connect_api_error)?;
    let response = SystemServiceClient::new(client.channel.clone())
        .max_decoding_message_size(MAX_MESSAGE_BYTES)
        .max_encoding_message_size(MAX_MESSAGE_BYTES)
        .get_server_info(request)
        .await
        .map_err(api_error_from_status)
        .map_err(connect_api_error)?
        .into_inner();
    let info = response.server_info.ok_or(SdkError::IdentityMismatch)?;
    let instance_id = Uuid::parse_str(&info.instance_id).map_err(|_| SdkError::IdentityMismatch)?;
    if instance_id.is_nil()
        || instance_id.to_string() != info.instance_id
        || crate::InstanceId::from_uuid(instance_id) != expected_instance_id
    {
        return Err(SdkError::IdentityMismatch);
    }
    if api_major.get() != 1
        || info.api_packages.is_empty()
        || info.api_packages.len() > MAX_COLLECTION_ITEMS
        || !info
            .api_packages
            .iter()
            .any(|package| package == PUBLIC_API_VERSION)
        || info
            .api_packages
            .iter()
            .any(|package| package.len() > MAX_IDENTIFIER_BYTES)
    {
        return Err(SdkError::VersionMismatch);
    }
    let expected_deployment = match backend_kind {
        BackendKind::Daemon => proto::DeploymentMode::SharedDaemon,
        BackendKind::Sidecar => proto::DeploymentMode::Sidecar,
        BackendKind::Embedded => return Err(SdkError::IdentityMismatch),
    };
    if proto::DeploymentMode::try_from(info.deployment_mode).ok() != Some(expected_deployment) {
        return Err(SdkError::IdentityMismatch);
    }
    if info.capabilities.len() > MAX_COLLECTION_ITEMS
        || info.capabilities.iter().any(|capability| {
            validate_identifier(&capability.name).is_err()
                || capability.detail.len() > MAX_SUMMARY_BYTES
                || capability.detail.chars().any(char::is_control)
        })
    {
        return Err(SdkError::IdentityMismatch);
    }
    let capability_names = info
        .capabilities
        .iter()
        .map(|capability| capability.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if capability_names.len() != info.capabilities.len() {
        return Err(SdkError::IdentityMismatch);
    }
    let enabled = info
        .capabilities
        .into_iter()
        .filter(|capability| capability.enabled)
        .map(|capability| capability.name)
        .collect::<Vec<_>>();
    Ok(ServerCapabilities::from_enabled(enabled))
}

fn connect_api_error(error: ApiError) -> SdkError {
    match error.code {
        ApiErrorCode::Unauthenticated => SdkError::Authentication,
        ApiErrorCode::Unavailable => SdkError::Transport,
        _ => SdkError::IdentityMismatch,
    }
}

fn cancel_watch_on_close(
    stream: RunUpdateStream,
    closed: watch::Receiver<bool>,
) -> RunUpdateStream {
    struct State {
        stream: RunUpdateStream,
        closed: watch::Receiver<bool>,
        done: bool,
    }

    Box::pin(stream::unfold(
        State {
            stream,
            closed,
            done: false,
        },
        |mut state| async move {
            loop {
                if state.done {
                    return None;
                }
                if *state.closed.borrow() {
                    state.done = true;
                    return Some((Err(closed_error()), state));
                }
                tokio::select! {
                    biased;
                    changed = state.closed.changed() => {
                        if changed.is_err() || *state.closed.borrow() {
                            state.done = true;
                            return Some((Err(closed_error()), state));
                        }
                    }
                    item = state.stream.next() => {
                        return item.map(|item| (item, state));
                    }
                }
            }
        },
    ))
}

#[async_trait]
impl AgentRunClient for GrpcAgentRunClient {
    async fn create_run(&self, request: CreateRunRequest) -> ApiResult<CreateRunResponse> {
        let request = proto_create_request(request)?;
        let request = self.request(request).await?;
        let response = self
            .client()
            .create_run(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        Ok(CreateRunResponse {
            run: run_from_proto(required(response.run)?)?,
        })
    }

    async fn get_run(&self, request: GetRunRequest) -> ApiResult<GetRunResponse> {
        validate_identifier(&request.run_id)?;
        let request = self
            .request(proto::GetRunRequest {
                run_id: request.run_id,
            })
            .await?;
        let response = self
            .client()
            .get_run(request)
            .await
            .map_err(read_error_from_status)?
            .into_inner();
        let run = run_from_proto(required(response.run)?)?;
        let pending_interactions = response
            .pending_interactions
            .into_iter()
            .map(interaction_from_proto)
            .collect::<ApiResult<Vec<_>>>()?;
        if pending_interactions.len() > MAX_COLLECTION_ITEMS
            || u32::try_from(pending_interactions.len()).ok() != Some(run.pending_interaction_count)
        {
            return Err(protocol_error());
        }
        Ok(GetRunResponse {
            run,
            pending_interactions,
        })
    }

    async fn list_runs(&self, request: ListRunsRequest) -> ApiResult<ListRunsResponse> {
        if let Some(session_id) = request.session_id.as_deref() {
            validate_identifier(session_id)?;
        }
        if request.statuses.len() > MAX_COLLECTION_ITEMS {
            return Err(invalid_request("statuses", "too many status filters"));
        }
        let statuses = request
            .statuses
            .into_iter()
            .map(proto_run_status)
            .map(|status| status as i32)
            .collect();
        let page = request.page.map(|page| proto::PageRequest {
            page_size: page.page_size,
            page_token: page.page_token,
        });
        let request = self
            .request(proto::ListRunsRequest {
                session_id: request.session_id,
                statuses,
                page,
                include_archived: request.include_archived,
            })
            .await?;
        let response = self
            .client()
            .list_runs(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        if response.runs.len() > MAX_COLLECTION_ITEMS {
            return Err(protocol_error());
        }
        Ok(ListRunsResponse {
            runs: response
                .runs
                .into_iter()
                .map(run_from_proto)
                .collect::<ApiResult<Vec<_>>>()?,
            page: response.page.map(|page| PageResponse {
                next_page_token: page.next_page_token,
            }),
        })
    }

    async fn watch_run(&self, request: WatchRunRequest) -> ApiResult<RunUpdateStream> {
        validate_identifier(&request.run_id)?;
        let request = self
            .request(proto::WatchRunRequest {
                run_id: request.run_id,
                after_sequence: request.after_sequence,
            })
            .await?;
        let stream = self
            .client()
            .watch_run(request)
            .await
            .map_err(read_error_from_status)?
            .into_inner()
            .map(|item| {
                let response = item.map_err(read_error_from_status)?;
                update_from_proto(required(response.update)?)
            });
        Ok(cancel_watch_on_close(
            Box::pin(stream),
            self.closed.subscribe(),
        ))
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
        validate_identifier(&request.run_id)?;
        let request = self
            .request(proto::CancelRunRequest {
                run_id: request.run_id,
                idempotency_key: request.idempotency_key.as_str().into(),
            })
            .await?;
        let response = self
            .client()
            .cancel_run(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        Ok(CancelRunResponse {
            run: run_from_proto(required(response.run)?)?,
        })
    }

    async fn archive_thread(&self, request: ArchiveThreadRequest) -> ApiResult<ThreadLifecycle> {
        validate_identifier(&request.run_id)?;
        let request = self
            .request(proto::ArchiveThreadRequest {
                run_id: request.run_id,
                idempotency_key: request.idempotency_key.as_str().into(),
            })
            .await?;
        let response = self
            .client()
            .archive_thread(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        thread_lifecycle_from_proto(required(response.thread)?)
    }

    async fn restore_thread(&self, request: RestoreThreadRequest) -> ApiResult<ThreadLifecycle> {
        validate_identifier(&request.run_id)?;
        let request = self
            .request(proto::RestoreThreadRequest {
                run_id: request.run_id,
                idempotency_key: request.idempotency_key.as_str().into(),
            })
            .await?;
        let response = self
            .client()
            .restore_thread(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        thread_lifecycle_from_proto(required(response.thread)?)
    }

    async fn respond_interaction(
        &self,
        request: RespondInteractionRequest,
    ) -> ApiResult<RespondInteractionResponse> {
        validate_identifier(&request.run_id)?;
        validate_identifier(&request.interaction_id)?;
        validate_opaque(&request.etag)?;
        let response = match request.response {
            InteractionAnswer::Prompt(answer) => {
                let answer = match answer {
                    PromptAnswer::Choice(choice) => {
                        prompt_answer::Answer::Choice(proto::PromptChoiceAnswer {
                            choice_id: choice.choice_id,
                            label: choice.label,
                        })
                    }
                    PromptAnswer::FreeForm(text) => prompt_answer::Answer::FreeFormText(text),
                };
                respond_interaction_request::Response::PromptAnswer(proto::PromptAnswer {
                    answer: Some(answer),
                })
            }
            InteractionAnswer::Approval {
                approved,
                request_hash,
            } => respond_interaction_request::Response::ApprovalAnswer(proto::ApprovalAnswer {
                approved,
                request_hash,
            }),
        };
        let request = self
            .request(proto::RespondInteractionRequest {
                run_id: request.run_id,
                interaction_id: request.interaction_id,
                etag: request.etag,
                idempotency_key: request.idempotency_key.as_str().into(),
                response: Some(response),
            })
            .await?;
        let response = self
            .client()
            .respond_interaction(request)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        let interaction = interaction_from_proto(required(response.interaction)?)?;
        if interaction.status == InteractionStatus::Pending
            || interaction.respondable_by_caller
            || !interaction.etag.is_empty()
        {
            return Err(protocol_error());
        }
        Ok(RespondInteractionResponse { interaction })
    }
}

impl fmt::Debug for GrpcArtifactClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GrpcArtifactClient")
            .field("credential", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl GrpcArtifactClient {
    fn client(&self) -> ArtifactServiceClient<Channel> {
        ArtifactServiceClient::new(self.channel.clone())
            .max_decoding_message_size(MAX_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_MESSAGE_BYTES)
    }

    async fn request<T>(&self, message: T) -> ApiResult<Request<T>> {
        if *self.closed.borrow() {
            return Err(closed_error());
        }
        let credential = self
            .credential_provider
            .load()
            .await
            .map_err(|_| authentication_error())?;
        let mut bearer = Zeroizing::new(Vec::with_capacity(
            b"Bearer ".len().saturating_add(credential.expose().len()),
        ));
        bearer.extend_from_slice(b"Bearer ");
        bearer.extend_from_slice(credential.expose());
        let mut metadata = MetadataValue::<Ascii>::try_from(bearer.as_slice())
            .map_err(|_| authentication_error())?;
        metadata.set_sensitive(true);
        let mut request = Request::new(message);
        request.metadata_mut().insert("authorization", metadata);
        Ok(request)
    }
}

#[async_trait]
impl ArtifactClient for GrpcArtifactClient {
    async fn upload(&self, request: UploadArtifactRequest) -> ApiResult<ArtifactReference> {
        if request.bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(invalid_request(
                "bytes",
                "artifact content exceeds the configured bound",
            ));
        }
        let sha256 = hex::encode(Sha256::digest(&request.bytes));
        let purpose = proto_artifact_purpose(request.purpose);
        let create = self
            .request(proto::CreateArtifactUploadRequest {
                file_name: request.file_name,
                media_type: request.media_type,
                size_bytes: request.bytes.len() as u64,
                sha256,
                purpose: purpose as i32,
                idempotency_key: request.idempotency_key.as_str().into(),
            })
            .await?;
        let reservation = self
            .client()
            .create_artifact_upload(create)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        validate_identifier(&reservation.upload_id)?;
        let chunk_size = usize::try_from(reservation.chunk_size_bytes)
            .ok()
            .filter(|size| *size > 0 && *size <= DEFAULT_ARTIFACT_CHUNK_BYTES)
            .ok_or_else(protocol_error)?;
        let mut chunks = request
            .bytes
            .chunks(chunk_size)
            .enumerate()
            .map(|(index, data)| {
                let offset = u64::try_from(index)
                    .ok()
                    .and_then(|index| index.checked_mul(chunk_size as u64))
                    .ok_or_else(protocol_error)?;
                Ok(proto::UploadArtifactRequest {
                    upload_id: reservation.upload_id.clone(),
                    offset,
                    data: data.to_vec(),
                })
            })
            .collect::<ApiResult<Vec<_>>>()?;
        if chunks.is_empty() {
            chunks.push(proto::UploadArtifactRequest {
                upload_id: reservation.upload_id,
                offset: 0,
                data: Vec::new(),
            });
        }
        let upload = self.request(stream::iter(chunks)).await?;
        let response = self
            .client()
            .upload_artifact(upload)
            .await
            .map_err(api_error_from_status)?
            .into_inner();
        artifact_from_proto(required(response.artifact)?)
    }

    async fn get(&self, artifact_id: &str) -> ApiResult<ArtifactReference> {
        validate_identifier(artifact_id)?;
        let request = self
            .request(proto::GetArtifactRequest {
                artifact_id: artifact_id.into(),
            })
            .await?;
        let response = self
            .client()
            .get_artifact(request)
            .await
            .map_err(read_error_from_status)?
            .into_inner();
        artifact_from_proto(required(response.artifact)?)
    }

    async fn download(&self, artifact_id: &str) -> ApiResult<DownloadedArtifact> {
        let artifact = self.get(artifact_id).await?;
        if artifact.state != ArtifactState::Available
            || artifact.size_bytes > MAX_ARTIFACT_BYTES as u64
        {
            return Err(protocol_error());
        }
        let request = self
            .request(proto::DownloadArtifactRequest {
                artifact_id: artifact_id.into(),
                offset: 0,
            })
            .await?;
        let mut stream = self
            .client()
            .download_artifact(request)
            .await
            .map_err(read_error_from_status)?
            .into_inner();
        let mut bytes =
            Vec::with_capacity(usize::try_from(artifact.size_bytes).map_err(|_| protocol_error())?);
        let mut expected_offset = 0_u64;
        while let Some(chunk) = stream.message().await.map_err(read_error_from_status)? {
            if chunk.offset != expected_offset || chunk.data.len() > DEFAULT_ARTIFACT_CHUNK_BYTES {
                return Err(protocol_error());
            }
            expected_offset = expected_offset
                .checked_add(u64::try_from(chunk.data.len()).map_err(|_| protocol_error())?)
                .ok_or_else(protocol_error)?;
            if expected_offset > artifact.size_bytes {
                return Err(protocol_error());
            }
            bytes.extend_from_slice(&chunk.data);
        }
        if expected_offset != artifact.size_bytes
            || hex::encode(Sha256::digest(&bytes)) != artifact.sha256
        {
            return Err(protocol_error());
        }
        Ok(DownloadedArtifact { artifact, bytes })
    }
}

fn proto_artifact_purpose(value: ArtifactPurpose) -> proto::ArtifactPurpose {
    match value {
        ArtifactPurpose::RunInput => proto::ArtifactPurpose::RunInput,
        ArtifactPurpose::RunOutput => proto::ArtifactPurpose::RunOutput,
        ArtifactPurpose::Workflow => proto::ArtifactPurpose::Workflow,
        ArtifactPurpose::Extension => proto::ArtifactPurpose::Extension,
        ArtifactPurpose::Archive => proto::ArtifactPurpose::Archive,
    }
}

fn proto_create_request(value: CreateRunRequest) -> ApiResult<proto::CreateRunRequest> {
    if value.input.is_empty() || value.input.len() > MAX_COLLECTION_ITEMS {
        return Err(invalid_request(
            "input",
            "input must contain a bounded number of content parts",
        ));
    }
    let input = value
        .input
        .into_iter()
        .map(|part| match part {
            InputContentPart::Text(text) => {
                if text.trim().is_empty() || text.len() > MAX_VISIBLE_TEXT_BYTES {
                    return Err(invalid_request("input.text", "input text is invalid"));
                }
                Ok(proto::ContentPart {
                    content: Some(content_part::Content::Text(proto::TextContent { text })),
                })
            }
            InputContentPart::Artifact(artifact_id) => {
                validate_identifier(&artifact_id)?;
                Ok(proto::ContentPart {
                    content: Some(content_part::Content::Artifact(proto::ArtifactReference {
                        artifact_id,
                        ..Default::default()
                    })),
                })
            }
        })
        .collect::<ApiResult<Vec<_>>>()?;
    if value.selected_skills.len() > MAX_COLLECTION_ITEMS {
        return Err(invalid_request(
            "selected_skills",
            "too many skills were selected",
        ));
    }
    let plan_action = value
        .plan_action
        .map(|action| {
            let (source_run_id, expected_revision, action) = match action {
                PlanRunAction::Revise {
                    source_run_id,
                    expected_revision,
                } => (
                    source_run_id,
                    expected_revision,
                    plan_run_action::Action::Revise(proto::RevisePlanAction {}),
                ),
                PlanRunAction::Execute {
                    source_run_id,
                    expected_revision,
                    strategy,
                } => {
                    let (strategy, max_goal_iterations) = match strategy {
                        PlanExecutionStrategy::Direct => (proto::PlanExecutionStrategy::Direct, 0),
                        PlanExecutionStrategy::Goal { max_iterations } => {
                            if !(1..=50).contains(&max_iterations) {
                                return Err(invalid_request(
                                    "plan_action.strategy.max_iterations",
                                    "Goal iterations must be in 1..=50",
                                ));
                            }
                            (
                                proto::PlanExecutionStrategy::Goal,
                                u32::from(max_iterations),
                            )
                        }
                    };
                    (
                        source_run_id,
                        expected_revision,
                        plan_run_action::Action::Execute(proto::ExecutePlanAction {
                            strategy: strategy as i32,
                            max_goal_iterations,
                        }),
                    )
                }
            };
            validate_identifier(&source_run_id)?;
            if expected_revision == 0 {
                return Err(invalid_request(
                    "plan_action.expected_revision",
                    "Plan revision must be greater than zero",
                ));
            }
            Ok(proto::PlanRunAction {
                source_run_id,
                expected_revision,
                action: Some(action),
            })
        })
        .transpose()?;
    let branch = value
        .branch
        .map(|branch| {
            validate_identifier(&branch.source_run_id)?;
            Ok(proto::RunBranch {
                source_run_id: branch.source_run_id,
                source_message_count: branch.source_message_count,
                context_mode: match branch.context_mode {
                    RunBranchContextMode::Exact => proto::RunBranchContextMode::Exact as i32,
                    RunBranchContextMode::Conversation => {
                        proto::RunBranchContextMode::Conversation as i32
                    }
                    RunBranchContextMode::SourceRunConversation => {
                        proto::RunBranchContextMode::SourceRunConversation as i32
                    }
                },
            })
        })
        .transpose()?;
    Ok(proto::CreateRunRequest {
        input,
        session_id: value.session_id,
        end_user_id: value.end_user_id,
        role: value.role,
        mode: proto_run_mode(value.mode) as i32,
        selected_skills: value.selected_skills,
        max_turns: value.max_turns,
        idempotency_key: value.idempotency_key.as_str().into(),
        plan_action,
        branch,
        research_depth: value.research_depth.map_or(
            proto::ResearchDepth::Unspecified as i32,
            |depth| match depth {
                ResearchDepth::Quick => proto::ResearchDepth::Quick as i32,
                ResearchDepth::Standard => proto::ResearchDepth::Standard as i32,
                ResearchDepth::Deep => proto::ResearchDepth::Deep as i32,
            },
        ),
        research_sources: value
            .research_sources
            .into_iter()
            .map(|kind| match kind {
                ResearchSourceKind::Repo => proto::ResearchSourceKind::Repo as i32,
                ResearchSourceKind::Web => proto::ResearchSourceKind::Web as i32,
                ResearchSourceKind::Mcp => proto::ResearchSourceKind::Mcp as i32,
            })
            .collect(),
    })
}

fn run_from_proto(value: proto::Run) -> ApiResult<Run> {
    validate_identifier(&value.run_id)?;
    validate_identifier(&value.session_id)?;
    validate_identifier(&value.role)?;
    let title = if value.title.trim().is_empty() {
        value.role.clone()
    } else {
        validate_text(&value.title, 256)?;
        value.title
    };
    validate_opaque(&value.etag)?;
    if value.selected_skills.len() > MAX_COLLECTION_ITEMS || value.pending_interaction_count > 1 {
        return Err(protocol_error());
    }
    for skill in &value.selected_skills {
        validate_identifier(skill)?;
    }
    let status = run_status_from_proto(value.status)?;
    let terminal = value.terminal.map(terminal_from_proto).transpose()?;
    match (status, &terminal) {
        (RunStatus::Completed, Some(RunTerminal::Result(_)))
        | (
            RunStatus::Failed | RunStatus::Interrupted | RunStatus::OutcomeUnknown,
            Some(RunTerminal::Failure(_)),
        )
        | (RunStatus::Cancelled, Some(RunTerminal::Cancellation(_))) => {}
        (
            RunStatus::Queued | RunStatus::Running | RunStatus::Waiting | RunStatus::Cancelling,
            None,
        ) => {}
        _ => return Err(protocol_error()),
    }
    Ok(Run {
        run_id: value.run_id,
        session_id: value.session_id,
        title,
        role: value.role,
        mode: run_mode_from_proto(value.mode)?,
        status,
        created_at: timestamp(required(value.created_at)?)?,
        updated_at: timestamp(required(value.updated_at)?)?,
        started_at: value.started_at.map(timestamp).transpose()?,
        finished_at: value.finished_at.map(timestamp).transpose()?,
        last_sequence: value.last_sequence,
        pending_interaction_count: value.pending_interaction_count,
        terminal,
        etag: value.etag,
        selected_skills: value.selected_skills,
        archived: value.archived,
    })
}

fn thread_lifecycle_from_proto(value: proto::ThreadLifecycle) -> ApiResult<ThreadLifecycle> {
    validate_identifier(&value.session_id)?;
    Ok(ThreadLifecycle {
        session_id: value.session_id,
        archived: value.archived,
    })
}

fn terminal_from_proto(value: run::Terminal) -> ApiResult<RunTerminal> {
    match value {
        run::Terminal::Result(result) => run_result_from_proto(result).map(RunTerminal::Result),
        run::Terminal::Failure(failure) => {
            run_failure_from_proto(failure).map(RunTerminal::Failure)
        }
        run::Terminal::Cancellation(cancellation) => Ok(RunTerminal::Cancellation(
            cancellation_from_proto(cancellation)?,
        )),
    }
}

fn run_result_from_proto(value: proto::RunResult) -> ApiResult<RunResult> {
    validate_text(&value.output, MAX_VISIBLE_TEXT_BYTES)?;
    if let Some(plan_id) = value.plan_id.as_deref() {
        validate_identifier(plan_id)?;
    }
    if let Some(goal_id) = value.goal_id.as_deref() {
        validate_identifier(goal_id)?;
    }
    let plan_status = plan_status_from_proto(value.plan_status)?;
    validate_plan_lineage(
        value.plan_id.as_deref(),
        value.plan_revision,
        plan_status,
        value.goal_id.as_deref(),
    )?;
    validate_identifier(&value.profile)?;
    validate_identifier(&value.model_profile)?;
    validate_identifier(&value.provider_profile)?;
    validate_identifier(&value.model)?;
    if value.profile != value.model_profile
        || !value.elapsed_seconds.is_finite()
        || value.elapsed_seconds < 0.0
    {
        return Err(protocol_error());
    }
    Ok(RunResult {
        output: value.output,
        plan_id: value.plan_id,
        plan_revision: value.plan_revision,
        plan_status,
        goal_id: value.goal_id,
        profile: value.profile,
        model_profile: value.model_profile,
        provider_profile: value.provider_profile,
        model: value.model,
        elapsed_seconds: value.elapsed_seconds,
    })
}

fn run_failure_from_proto(value: proto::RunFailure) -> ApiResult<RunFailure> {
    validate_identifier(&value.reason)?;
    validate_text(&value.message, MAX_SUMMARY_BYTES)?;
    let http_status = value
        .http_status
        .map(u16::try_from)
        .transpose()
        .map_err(|_| protocol_error())?;
    if http_status.is_some_and(|status| !(100..=599).contains(&status)) {
        return Err(protocol_error());
    }
    Ok(RunFailure {
        reason: value.reason,
        message: value.message,
        outcome_certainty: outcome_from_proto(value.outcome_certainty)?,
        recoverable: value.recoverable,
        http_status,
        retry_after_ms: value.retry_after_ms,
    })
}

fn cancellation_from_proto(value: proto::RunCancellation) -> ApiResult<RunCancellation> {
    validate_text(&value.message, MAX_SUMMARY_BYTES)?;
    if let Some(plan_id) = value.plan_id.as_deref() {
        validate_identifier(plan_id)?;
    }
    if let Some(goal_id) = value.goal_id.as_deref() {
        validate_identifier(goal_id)?;
    }
    let plan_status = plan_status_from_proto(value.plan_status)?;
    validate_plan_lineage(
        value.plan_id.as_deref(),
        value.plan_revision,
        plan_status,
        value.goal_id.as_deref(),
    )?;
    Ok(RunCancellation {
        turn: value.turn,
        message: value.message,
        plan_id: value.plan_id,
        plan_revision: value.plan_revision,
        plan_status,
        goal_id: value.goal_id,
    })
}

fn plan_status_from_proto(value: i32) -> ApiResult<Option<PlanStatus>> {
    match proto::PlanStatus::try_from(value) {
        Ok(proto::PlanStatus::Unspecified) => Ok(None),
        Ok(proto::PlanStatus::Draft) => Ok(Some(PlanStatus::Draft)),
        Ok(proto::PlanStatus::Approved) => Ok(Some(PlanStatus::Approved)),
        Ok(proto::PlanStatus::Executed) => Ok(Some(PlanStatus::Executed)),
        Ok(proto::PlanStatus::Discarded) => Ok(Some(PlanStatus::Discarded)),
        Err(_) => Err(protocol_error()),
    }
}

fn validate_plan_lineage(
    plan_id: Option<&str>,
    plan_revision: Option<u64>,
    plan_status: Option<PlanStatus>,
    goal_id: Option<&str>,
) -> ApiResult<()> {
    let has_complete_metadata = plan_revision.is_some() && plan_status.is_some();
    if plan_revision.is_some() != plan_status.is_some()
        || plan_revision == Some(0)
        || (plan_id.is_none() && (has_complete_metadata || goal_id.is_some()))
        || (goal_id.is_some() && !has_complete_metadata)
    {
        return Err(protocol_error());
    }
    Ok(())
}

fn interaction_from_proto(value: proto::Interaction) -> ApiResult<Interaction> {
    validate_identifier(&value.interaction_id)?;
    validate_identifier(&value.run_id)?;
    validate_opaque_allow_empty(&value.etag)?;
    let kind = match proto::InteractionKind::try_from(value.kind) {
        Ok(proto::InteractionKind::UserPrompt) => InteractionKind::UserPrompt,
        Ok(proto::InteractionKind::Approval) => InteractionKind::Approval,
        Ok(proto::InteractionKind::Unspecified) | Err(_) => return Err(protocol_error()),
    };
    let status = match proto::InteractionStatus::try_from(value.status) {
        Ok(proto::InteractionStatus::Pending) => InteractionStatus::Pending,
        Ok(proto::InteractionStatus::Answered) => InteractionStatus::Answered,
        Ok(proto::InteractionStatus::Expired) => InteractionStatus::Expired,
        Ok(proto::InteractionStatus::Cancelled) => InteractionStatus::Cancelled,
        Ok(proto::InteractionStatus::Unspecified) | Err(_) => return Err(protocol_error()),
    };
    if status != InteractionStatus::Pending
        && (value.respondable_by_caller || !value.etag.is_empty())
    {
        return Err(protocol_error());
    }
    let content = match (kind, required(value.content)?) {
        (InteractionKind::UserPrompt, interaction::Content::UserPrompt(prompt)) => {
            if prompt.choices.len() > MAX_COLLECTION_ITEMS {
                return Err(protocol_error());
            }
            let choices = prompt
                .choices
                .into_iter()
                .map(|choice| {
                    validate_opaque(&choice.choice_id)?;
                    validate_text(&choice.label, MAX_SUMMARY_BYTES)?;
                    Ok(PromptChoice {
                        choice_id: choice.choice_id,
                        label: choice.label,
                    })
                })
                .collect::<ApiResult<Vec<_>>>()?;
            InteractionContent::UserPrompt(crate::UserPromptInteraction {
                question: prompt.question,
                choices,
                allow_free_form: prompt.allow_free_form,
            })
        }
        (InteractionKind::Approval, interaction::Content::Approval(approval)) => {
            validate_text(&approval.reason, MAX_SUMMARY_BYTES)?;
            validate_text(&approval.action, MAX_SUMMARY_BYTES)?;
            validate_text(&approval.resource, MAX_SUMMARY_BYTES)?;
            if validate_public_approval_display(&approval.action, &approval.resource).is_err() {
                return Err(protocol_error());
            }
            validate_opaque(&approval.request_hash)?;
            let risk = match proto::ApprovalRisk::try_from(approval.risk) {
                Ok(proto::ApprovalRisk::Unspecified) => None,
                Ok(proto::ApprovalRisk::Low) => Some(ApprovalRisk::Low),
                Ok(proto::ApprovalRisk::Medium) => Some(ApprovalRisk::Medium),
                Ok(proto::ApprovalRisk::High) => Some(ApprovalRisk::High),
                Err(_) => return Err(protocol_error()),
            };
            InteractionContent::Approval(ApprovalInteraction {
                reason: approval.reason,
                action: approval.action,
                resource: approval.resource,
                risk,
                request_hash: approval.request_hash,
            })
        }
        _ => return Err(protocol_error()),
    };
    Ok(Interaction {
        interaction_id: value.interaction_id,
        run_id: value.run_id,
        kind,
        status,
        created_at: timestamp(required(value.created_at)?)?,
        expires_at: timestamp(required(value.expires_at)?)?,
        respondable_by_caller: value.respondable_by_caller,
        etag: value.etag,
        content,
    })
}

fn update_from_proto(value: proto::RunUpdate) -> ApiResult<RunUpdate> {
    validate_identifier(&value.run_id)?;
    if value.sequence == 0 {
        return Err(protocol_error());
    }
    let update = match required(value.update)? {
        run_update::Update::State(state) => {
            RunUpdateKind::State(run_status_from_proto(state.status)?)
        }
        run_update::Update::OutputDelta(delta) => {
            validate_text(&delta.text, MAX_VISIBLE_TEXT_BYTES)?;
            RunUpdateKind::OutputDelta(delta.text)
        }
        run_update::Update::ReasoningSummary(summary) => {
            validate_text(&summary.summary, MAX_SUMMARY_BYTES)?;
            RunUpdateKind::ReasoningSummary(summary.summary)
        }
        run_update::Update::ToolActivity(activity) => {
            validate_identifier(&activity.call_id)?;
            validate_identifier(&activity.tool_name)?;
            validate_text(&activity.summary, MAX_SUMMARY_BYTES)?;
            if let Some(input) = &activity.input {
                validate_text(input, MAX_SUMMARY_BYTES)?;
            }
            if let Some(preview) = &activity.preview {
                validate_text(preview, MAX_SUMMARY_BYTES)?;
            }
            RunUpdateKind::ToolActivity(ToolActivity {
                call_id: activity.call_id,
                tool_name: activity.tool_name,
                state: tool_state_from_proto(activity.state)?,
                summary: activity.summary,
                input: activity.input,
                preview: activity.preview,
            })
        }
        run_update::Update::Usage(usage) => RunUpdateKind::Usage(TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.total_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            reasoning_tokens: usage.reasoning_tokens,
        }),
        run_update::Update::Interaction(interaction) => {
            let interaction = interaction_from_proto(interaction)?;
            if interaction.run_id != value.run_id {
                return Err(protocol_error());
            }
            RunUpdateKind::Interaction(interaction)
        }
        run_update::Update::Message(message) => {
            let message = message_from_proto(message)?;
            if message.run_id != value.run_id {
                return Err(protocol_error());
            }
            RunUpdateKind::Message(message)
        }
        run_update::Update::Notice(notice) => {
            validate_identifier(&notice.reason)?;
            validate_text(&notice.message, MAX_SUMMARY_BYTES)?;
            RunUpdateKind::Notice {
                reason: notice.reason,
                message: notice.message,
            }
        }
        run_update::Update::Result(result) => RunUpdateKind::Result(run_result_from_proto(result)?),
        run_update::Update::Failure(failed) => {
            let status = run_status_from_proto(failed.status)?;
            if !matches!(
                status,
                RunStatus::Failed | RunStatus::Interrupted | RunStatus::OutcomeUnknown
            ) {
                return Err(protocol_error());
            }
            RunUpdateKind::Failure {
                status,
                failure: run_failure_from_proto(required(failed.failure)?)?,
            }
        }
        run_update::Update::Cancellation(cancellation) => {
            RunUpdateKind::Cancellation(cancellation_from_proto(cancellation)?)
        }
    };
    Ok(RunUpdate {
        run_id: value.run_id,
        sequence: value.sequence,
        created_at: timestamp(required(value.created_at)?)?,
        update,
    })
}

fn message_from_proto(value: proto::SessionMessage) -> ApiResult<SessionMessage> {
    validate_identifier(&value.session_id)?;
    validate_identifier(&value.run_id)?;
    if value.sequence == 0 || value.content.len() > MAX_COLLECTION_ITEMS {
        return Err(protocol_error());
    }
    let role = match proto::MessageRole::try_from(value.role) {
        Ok(proto::MessageRole::User) => MessageRole::User,
        Ok(proto::MessageRole::Assistant) => MessageRole::Assistant,
        Ok(proto::MessageRole::Tool) => MessageRole::Tool,
        Ok(proto::MessageRole::System) => MessageRole::System,
        Ok(proto::MessageRole::Unspecified) | Err(_) => return Err(protocol_error()),
    };
    let content = value
        .content
        .into_iter()
        .map(|part| match required(part.content)? {
            content_part::Content::Text(text) => {
                validate_text(&text.text, MAX_VISIBLE_TEXT_BYTES)?;
                Ok(MessageContentPart::Text(text.text))
            }
            content_part::Content::Artifact(artifact) => {
                artifact_from_proto(artifact).map(MessageContentPart::Artifact)
            }
        })
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(SessionMessage {
        session_id: value.session_id,
        run_id: value.run_id,
        sequence: value.sequence,
        role,
        content,
        created_at: timestamp(required(value.created_at)?)?,
    })
}

fn artifact_from_proto(value: proto::ArtifactReference) -> ApiResult<ArtifactReference> {
    validate_identifier(&value.artifact_id)?;
    validate_text(&value.file_name, MAX_SUMMARY_BYTES)?;
    validate_text(&value.media_type, MAX_SUMMARY_BYTES)?;
    if value.sha256.len() != 64
        || !value
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(protocol_error());
    }
    let purpose = match proto::ArtifactPurpose::try_from(value.purpose) {
        Ok(proto::ArtifactPurpose::RunInput) => ArtifactPurpose::RunInput,
        Ok(proto::ArtifactPurpose::RunOutput) => ArtifactPurpose::RunOutput,
        Ok(proto::ArtifactPurpose::Workflow) => ArtifactPurpose::Workflow,
        Ok(proto::ArtifactPurpose::Extension) => ArtifactPurpose::Extension,
        Ok(proto::ArtifactPurpose::Archive) => ArtifactPurpose::Archive,
        Ok(proto::ArtifactPurpose::Unspecified) | Err(_) => return Err(protocol_error()),
    };
    let state = match proto::ArtifactState::try_from(value.state) {
        Ok(proto::ArtifactState::Uploading) => ArtifactState::Uploading,
        Ok(proto::ArtifactState::Quarantined) => ArtifactState::Quarantined,
        Ok(proto::ArtifactState::Available) => ArtifactState::Available,
        Ok(proto::ArtifactState::Rejected) => ArtifactState::Rejected,
        Ok(proto::ArtifactState::Expired) => ArtifactState::Expired,
        Ok(proto::ArtifactState::Unspecified) | Err(_) => return Err(protocol_error()),
    };
    Ok(ArtifactReference {
        artifact_id: value.artifact_id,
        file_name: value.file_name,
        media_type: value.media_type,
        size_bytes: value.size_bytes,
        sha256: value.sha256,
        purpose,
        state,
        created_at: timestamp(required(value.created_at)?)?,
    })
}

fn proto_run_mode(value: RunMode) -> proto::RunMode {
    match value {
        RunMode::Execute => proto::RunMode::Execute,
        RunMode::Plan => proto::RunMode::Plan,
        RunMode::Research => proto::RunMode::Research,
    }
}

fn run_mode_from_proto(value: i32) -> ApiResult<RunMode> {
    match proto::RunMode::try_from(value) {
        Ok(proto::RunMode::Execute) => Ok(RunMode::Execute),
        Ok(proto::RunMode::Plan) => Ok(RunMode::Plan),
        Ok(proto::RunMode::Research) => Ok(RunMode::Research),
        Ok(proto::RunMode::Unspecified) | Err(_) => Err(protocol_error()),
    }
}

fn proto_run_status(value: RunStatus) -> proto::RunStatus {
    match value {
        RunStatus::Queued => proto::RunStatus::Queued,
        RunStatus::Running => proto::RunStatus::Running,
        RunStatus::Waiting => proto::RunStatus::Waiting,
        RunStatus::Cancelling => proto::RunStatus::Cancelling,
        RunStatus::Completed => proto::RunStatus::Completed,
        RunStatus::Failed => proto::RunStatus::Failed,
        RunStatus::Cancelled => proto::RunStatus::Cancelled,
        RunStatus::Interrupted => proto::RunStatus::Interrupted,
        RunStatus::OutcomeUnknown => proto::RunStatus::OutcomeUnknown,
    }
}

fn run_status_from_proto(value: i32) -> ApiResult<RunStatus> {
    match proto::RunStatus::try_from(value) {
        Ok(proto::RunStatus::Queued) => Ok(RunStatus::Queued),
        Ok(proto::RunStatus::Running) => Ok(RunStatus::Running),
        Ok(proto::RunStatus::Waiting) => Ok(RunStatus::Waiting),
        Ok(proto::RunStatus::Cancelling) => Ok(RunStatus::Cancelling),
        Ok(proto::RunStatus::Completed) => Ok(RunStatus::Completed),
        Ok(proto::RunStatus::Failed) => Ok(RunStatus::Failed),
        Ok(proto::RunStatus::Cancelled) => Ok(RunStatus::Cancelled),
        Ok(proto::RunStatus::Interrupted) => Ok(RunStatus::Interrupted),
        Ok(proto::RunStatus::OutcomeUnknown) => Ok(RunStatus::OutcomeUnknown),
        Ok(proto::RunStatus::Unspecified) | Err(_) => Err(protocol_error()),
    }
}

fn tool_state_from_proto(value: i32) -> ApiResult<ToolActivityState> {
    match proto::ToolActivityState::try_from(value) {
        Ok(proto::ToolActivityState::Requested) => Ok(ToolActivityState::Requested),
        Ok(proto::ToolActivityState::WaitingApproval) => Ok(ToolActivityState::WaitingApproval),
        Ok(proto::ToolActivityState::Started) => Ok(ToolActivityState::Started),
        Ok(proto::ToolActivityState::Completed) => Ok(ToolActivityState::Completed),
        Ok(proto::ToolActivityState::Cancelled) => Ok(ToolActivityState::Cancelled),
        Ok(proto::ToolActivityState::Failed) => Ok(ToolActivityState::Failed),
        Ok(proto::ToolActivityState::OutcomeUnknown) => Ok(ToolActivityState::OutcomeUnknown),
        Ok(proto::ToolActivityState::Unspecified) | Err(_) => Err(protocol_error()),
    }
}

fn outcome_from_proto(value: i32) -> ApiResult<OutcomeCertainty> {
    match proto::OutcomeCertainty::try_from(value) {
        Ok(proto::OutcomeCertainty::Known) => Ok(OutcomeCertainty::Known),
        Ok(proto::OutcomeCertainty::Unknown) => Ok(OutcomeCertainty::Unknown),
        Ok(proto::OutcomeCertainty::Unspecified) | Err(_) => Err(protocol_error()),
    }
}

fn timestamp(value: Timestamp) -> ApiResult<String> {
    if !(-62_135_596_800..=253_402_300_799).contains(&value.seconds)
        || !(0..1_000_000_000).contains(&value.nanos)
    {
        return Err(protocol_error());
    }
    Ok(value.to_string())
}

fn required<T>(value: Option<T>) -> ApiResult<T> {
    value.ok_or_else(protocol_error)
}

fn validate_identifier(value: &str) -> ApiResult<()> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(protocol_error())
    } else {
        Ok(())
    }
}

fn validate_opaque(value: &str) -> ApiResult<()> {
    if value.is_empty() {
        return Err(protocol_error());
    }
    validate_opaque_allow_empty(value)
}

fn validate_opaque_allow_empty(value: &str) -> ApiResult<()> {
    if value.len() > MAX_OPAQUE_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(protocol_error())
    } else {
        Ok(())
    }
}

fn validate_text(value: &str, maximum_bytes: usize) -> ApiResult<()> {
    if value.len() > maximum_bytes {
        Err(protocol_error())
    } else {
        Ok(())
    }
}

fn validate_certificate_pin(certificate_pem: &[u8], expected: TlsFingerprint) -> SdkResult<()> {
    if certificate_pem.is_empty()
        || certificate_pem.len() > MAX_CERTIFICATE_PEM_BYTES
        || !certificate_pem.is_ascii()
        || certificate_pem
            .windows(b"PRIVATE KEY".len())
            .any(|window| window == b"PRIVATE KEY")
    {
        return Err(SdkError::IdentityMismatch);
    }
    validate_endpoint_certificate_pem(certificate_pem).map_err(|_| SdkError::IdentityMismatch)?;
    let certificates = CertificateDer::pem_slice_iter(certificate_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SdkError::IdentityMismatch)?;
    if certificates.len() != 1 {
        return Err(SdkError::IdentityMismatch);
    }
    let actual = Sha256::digest(certificates[0].as_ref());
    if actual.as_slice() != expected.as_bytes() {
        return Err(SdkError::IdentityMismatch);
    }
    Ok(())
}

fn api_error_from_status(status: Status) -> ApiError {
    if status.details().is_empty() {
        return match status.code() {
            Code::Unauthenticated => authentication_error(),
            Code::Unavailable | Code::DeadlineExceeded => unavailable_error(),
            _ => protocol_error(),
        };
    }
    if status.details().len() > MAX_ERROR_DETAILS_BYTES
        || status.message().len() > MAX_ERROR_MESSAGE_BYTES
    {
        return protocol_error();
    }
    let Ok(rich) = RichStatus::decode(status.details()) else {
        return protocol_error();
    };
    if rich.code != status.code() as i32
        || rich.message != status.message()
        || rich.details.len() != 1
        || rich.details[0].type_url != ERROR_DETAIL_TYPE_URL
        || rich.details[0].value.len() > MAX_ERROR_DETAILS_BYTES
    {
        return protocol_error();
    }
    let Ok(detail) = ColossusErrorDetail::decode(rich.details[0].value.as_slice()) else {
        return protocol_error();
    };
    let Some(reason) = api_reason(&detail.reason) else {
        return protocol_error();
    };
    let outcome = match proto::OutcomeCertainty::try_from(detail.outcome_certainty) {
        Ok(proto::OutcomeCertainty::Known) => OutcomeCertainty::Known,
        Ok(proto::OutcomeCertainty::Unknown) => OutcomeCertainty::Unknown,
        Ok(proto::OutcomeCertainty::Unspecified) | Err(_) => return protocol_error(),
    };
    if (outcome == OutcomeCertainty::Unknown
        && (status.code() != Code::Unknown
            || reason != ApiErrorReason::OutcomeUnknown
            || detail.retryable))
        || (reason == ApiErrorReason::OutcomeUnknown && outcome != OutcomeCertainty::Unknown)
        || detail.retry_after.is_some()
        || detail.violations.len() > 64
    {
        return protocol_error();
    }
    let correlation_id = if detail.request_id.is_empty() {
        None
    } else {
        match RequestId::new(detail.request_id) {
            Ok(value) => Some(value),
            Err(_) => return protocol_error(),
        }
    };
    let violations = match detail
        .violations
        .into_iter()
        .map(|violation| {
            if violation.field.len() > MAX_IDENTIFIER_BYTES
                || violation.description.len() > MAX_SUMMARY_BYTES
            {
                Err(())
            } else {
                Ok(FieldViolation {
                    field: violation.field,
                    description: violation.description,
                })
            }
        })
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(value) => value,
        Err(()) => return protocol_error(),
    };
    let code = api_code(status.code(), reason, outcome);
    ApiError {
        code,
        reason,
        message: rich.message,
        correlation_id,
        retryable: detail.retryable,
        outcome: match outcome {
            OutcomeCertainty::Known => colossus_api::OutcomeCertainty::Known,
            OutcomeCertainty::Unknown => colossus_api::OutcomeCertainty::Unknown,
        },
        violations,
    }
}

fn read_error_from_status(status: Status) -> ApiError {
    let retryable_transport_failure = status.details().is_empty()
        && matches!(status.code(), Code::Unavailable | Code::DeadlineExceeded);
    let mut error = api_error_from_status(status);
    if retryable_transport_failure {
        error.retryable = true;
    }
    error
}

fn api_code(code: Code, reason: ApiErrorReason, outcome: OutcomeCertainty) -> ApiErrorCode {
    if reason == ApiErrorReason::OutcomeUnknown && outcome == OutcomeCertainty::Unknown {
        return ApiErrorCode::OutcomeUnknown;
    }
    match code {
        Code::InvalidArgument => ApiErrorCode::InvalidArgument,
        Code::Unauthenticated => ApiErrorCode::Unauthenticated,
        Code::PermissionDenied => ApiErrorCode::PermissionDenied,
        Code::NotFound => ApiErrorCode::NotFound,
        Code::AlreadyExists => ApiErrorCode::AlreadyExists,
        Code::Aborted => ApiErrorCode::Conflict,
        Code::FailedPrecondition => ApiErrorCode::FailedPrecondition,
        Code::ResourceExhausted => ApiErrorCode::ResourceExhausted,
        Code::Cancelled => ApiErrorCode::Cancelled,
        Code::Unavailable | Code::DeadlineExceeded => ApiErrorCode::Unavailable,
        _ => ApiErrorCode::Internal,
    }
}

fn api_reason(value: &str) -> Option<ApiErrorReason> {
    Some(match value {
        "invalid_argument" => ApiErrorReason::InvalidArgument,
        "authentication_required" => ApiErrorReason::AuthenticationRequired,
        "authentication_failed" => ApiErrorReason::AuthenticationFailed,
        "scope_denied" => ApiErrorReason::ScopeDenied,
        "role_denied" => ApiErrorReason::RoleDenied,
        "tool_denied" => ApiErrorReason::ToolDenied,
        "run_not_found" => ApiErrorReason::RunNotFound,
        "idempotency_key_reused" => ApiErrorReason::IdempotencyKeyReused,
        "concurrent_modification" => ApiErrorReason::ConcurrentModification,
        "invalid_run_transition" => ApiErrorReason::InvalidRunTransition,
        "interaction_unavailable" => ApiErrorReason::InteractionUnavailable,
        "capacity_exceeded" => ApiErrorReason::CapacityExceeded,
        "recovery_mode" => ApiErrorReason::RecoveryMode,
        "storage_failure" => ApiErrorReason::StorageFailure,
        "outcome_unknown" => ApiErrorReason::OutcomeUnknown,
        "internal_invariant" => ApiErrorReason::InternalInvariant,
        _ => return None,
    })
}

fn invalid_request(field: &str, description: &str) -> ApiError {
    ApiError::invalid(ApiErrorReason::InvalidArgument, field, description)
}

fn authentication_error() -> ApiError {
    ApiError {
        code: ApiErrorCode::Unauthenticated,
        reason: ApiErrorReason::AuthenticationFailed,
        message: "Colossus API authentication failed".into(),
        correlation_id: None,
        retryable: false,
        outcome: colossus_api::OutcomeCertainty::Known,
        violations: Vec::new(),
    }
}

fn unavailable_error() -> ApiError {
    ApiError {
        code: ApiErrorCode::Unavailable,
        reason: ApiErrorReason::InternalInvariant,
        message: "the Colossus API is unavailable".into(),
        correlation_id: None,
        retryable: false,
        outcome: colossus_api::OutcomeCertainty::Known,
        violations: Vec::new(),
    }
}

fn closed_error() -> ApiError {
    ApiError {
        code: ApiErrorCode::Unavailable,
        reason: ApiErrorReason::InternalInvariant,
        message: "the Colossus API client was closed".into(),
        correlation_id: None,
        retryable: false,
        outcome: colossus_api::OutcomeCertainty::Known,
        violations: Vec::new(),
    }
}

fn protocol_error() -> ApiError {
    ApiError {
        code: ApiErrorCode::Internal,
        reason: ApiErrorReason::InternalInvariant,
        message: "the Colossus API returned malformed public data".into(),
        correlation_id: None,
        retryable: false,
        outcome: colossus_api::OutcomeCertainty::Known,
        violations: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApiMajor, Colossus, InstanceId};
    use async_trait::async_trait;
    use colossus_api::{
        AgentRunApi as CoreAgentRunApi, ApiScope, ApplicationKind, CallerContext,
        CancelRunRequest as CoreCancelRunRequest, CreateRunRequest as CoreCreateRunRequest,
        CreateRunResponse as CoreCreateRunResponse, EventSourcedArtifactApi,
        GetRunRequest as CoreGetRunRequest, Interaction as CoreInteraction,
        ListRunsRequest as CoreListRunsRequest, ListRunsResponse as CoreListRunsResponse,
        RespondInteractionRequest as CoreRespondInteractionRequest, Run as CoreRun,
        RunUpdateStream as CoreRunUpdateStream, WatchRunRequest as CoreWatchRunRequest,
        scopes::RUNS_READ,
    };
    use colossus_api_proto::google_rpc::Status as RichStatus;
    use colossus_grpc::{
        ApplicationGrant, BoundPublicGrpcServer, CredentialAuthenticator, EndpointDescriptor,
        FixedReadiness, InMemoryCredentialRepository, SystemMetadata, SystemServiceAdapter,
        TlsIdentity, TlsKeySeed, write_endpoint_certificate, write_endpoint_descriptor,
    };
    use colossus_testkit::InMemoryEventJournal;
    use prost_types::Any;
    use rustls::ClientConfig;
    use std::sync::Arc;
    use tokio::{net::TcpStream, sync::oneshot};
    use tokio_rustls::TlsConnector;
    use zeroize::Zeroizing;

    struct StaticCredential {
        bytes: Zeroizing<Vec<u8>>,
    }

    impl StaticCredential {
        fn new(bytes: impl Into<Vec<u8>>) -> Self {
            Self {
                bytes: Zeroizing::new(bytes.into()),
            }
        }
    }

    impl fmt::Debug for StaticCredential {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("StaticCredential([REDACTED])")
        }
    }

    #[async_trait]
    impl CredentialProvider for StaticCredential {
        async fn load(&self) -> SdkResult<crate::Secret> {
            crate::Secret::new(self.bytes.as_slice().to_vec())
        }
    }

    #[tokio::test]
    async fn close_signal_terminates_an_active_watch_stream() {
        let (closed, receiver) = watch::channel(false);
        let pending: RunUpdateStream = Box::pin(stream::pending());
        let mut stream = cancel_watch_on_close(pending, receiver);
        let error = {
            let next = stream.next();
            tokio::pin!(next);
            tokio::task::yield_now().await;
            closed.send_replace(true);
            tokio::time::timeout(Duration::from_millis(100), &mut next)
                .await
                .expect("close wakes active watch")
                .expect("close error")
                .expect_err("closed watch is not successful")
        };
        assert_eq!(error.code, ApiErrorCode::Unavailable);
        assert!(!error.retryable);
        assert!(stream.next().await.is_none());
    }

    #[test]
    fn verifier_checks_the_live_presented_leaf_digest() {
        let pinned = TlsIdentity::from_seed(TlsKeySeed::new([0x21; 32])).expect("pinned identity");
        let fingerprint =
            TlsFingerprint::from_hex(pinned.certificate_sha256()).expect("fingerprint");
        let verifier =
            pinned_server_verifier(pinned.certificate_pem(), fingerprint).expect("verifier");
        let pinned_leaf = CertificateDer::pem_slice_iter(pinned.certificate_pem())
            .next()
            .expect("pinned leaf")
            .expect("valid pinned leaf");
        let server_name = ServerName::try_from("127.0.0.1").expect("server name");
        verifier
            .verify_server_cert(&pinned_leaf, &[], &server_name, &[], UnixTime::now())
            .expect("exact live leaf");

        let alternate =
            TlsIdentity::from_seed(TlsKeySeed::new([0x22; 32])).expect("alternate identity");
        let alternate_leaf = CertificateDer::pem_slice_iter(alternate.certificate_pem())
            .next()
            .expect("alternate leaf")
            .expect("valid alternate leaf");
        assert!(matches!(
            verifier.verify_server_cert(&alternate_leaf, &[], &server_name, &[], UnixTime::now()),
            Err(RustlsError::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure
            ))
        ));
        assert!(
            verifier
                .verify_server_cert(
                    &pinned_leaf,
                    std::slice::from_ref(&alternate_leaf),
                    &server_name,
                    &[],
                    UnixTime::now()
                )
                .is_err()
        );
    }

    #[test]
    fn verifier_refuses_tls12_even_for_an_otherwise_pinned_identity() {
        assert!(matches!(
            reject_tls12_signature(),
            Err(RustlsError::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure
            ))
        ));
    }

    struct ReadApi;

    #[async_trait]
    impl CoreAgentRunApi for ReadApi {
        async fn create_run(
            &self,
            _caller: &CallerContext,
            _request: CoreCreateRunRequest,
        ) -> ApiResult<CoreCreateRunResponse> {
            unreachable!("test calls only get_run")
        }

        async fn get_run(
            &self,
            _caller: &CallerContext,
            request: CoreGetRunRequest,
        ) -> ApiResult<CoreRun> {
            Ok(core_run(request.run_id))
        }

        async fn list_runs(
            &self,
            _caller: &CallerContext,
            _request: CoreListRunsRequest,
        ) -> ApiResult<CoreListRunsResponse> {
            unreachable!("test calls only get_run")
        }

        async fn watch_run(
            &self,
            _caller: &CallerContext,
            _request: CoreWatchRunRequest,
        ) -> ApiResult<CoreRunUpdateStream> {
            unreachable!("test calls only get_run")
        }

        async fn cancel_run(
            &self,
            _caller: &CallerContext,
            _request: CoreCancelRunRequest,
        ) -> ApiResult<CoreRun> {
            unreachable!("test calls only get_run")
        }

        async fn respond_interaction(
            &self,
            _caller: &CallerContext,
            _request: CoreRespondInteractionRequest,
        ) -> ApiResult<CoreInteraction> {
            unreachable!("test calls only get_run")
        }
    }

    fn core_run(run_id: String) -> CoreRun {
        CoreRun {
            id: run_id,
            session_id: "session-1".into(),
            title: "Test run".into(),
            status: colossus_api::RunStatus::Running,
            mode: colossus_api::RunMode::Execute,
            role: "assistant".into(),
            skill_ids: vec!["rust".into()],
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:01Z".into(),
            started_at: Some("2026-01-01T00:00:01Z".into()),
            finished_at: None,
            last_sequence: 2,
            result: None,
            failure: None,
            cancellation: None,
            pending_interaction: None,
            etag: "etag-2".into(),
            archived: false,
        }
    }

    #[test]
    fn rich_status_requires_exact_standard_envelope_and_type() {
        let detail = ColossusErrorDetail {
            reason: "scope_denied".into(),
            request_id: "request-1".into(),
            retryable: false,
            retry_after: None,
            outcome_certainty: proto::OutcomeCertainty::Known as i32,
            violations: Vec::new(),
        };
        let rich = RichStatus {
            code: Code::PermissionDenied as i32,
            message: "scope denied".into(),
            details: vec![Any {
                type_url: ERROR_DETAIL_TYPE_URL.into(),
                value: detail.encode_to_vec(),
            }],
        };
        let status = Status::with_details(
            Code::PermissionDenied,
            "scope denied",
            rich.encode_to_vec().into(),
        );
        let error = api_error_from_status(status);
        assert_eq!(error.code, ApiErrorCode::PermissionDenied);
        assert_eq!(error.reason, ApiErrorReason::ScopeDenied);

        let malformed = Status::with_details(
            Code::PermissionDenied,
            "scope denied",
            detail.encode_to_vec().into(),
        );
        assert_eq!(
            api_error_from_status(malformed).reason,
            ApiErrorReason::InternalInvariant
        );
    }

    #[test]
    fn bare_read_transport_failures_are_retryable_without_widening_effectful_errors() {
        for code in [Code::Unavailable, Code::DeadlineExceeded] {
            let read = read_error_from_status(Status::new(code, "transport unavailable"));
            assert_eq!(read.code, ApiErrorCode::Unavailable);
            assert!(read.retryable);

            let effectful = api_error_from_status(Status::new(code, "transport unavailable"));
            assert_eq!(effectful.code, ApiErrorCode::Unavailable);
            assert!(!effectful.retryable);
        }
    }

    #[test]
    fn capacity_status_maps_to_resource_exhausted() {
        let detail = ColossusErrorDetail {
            reason: "capacity_exceeded".into(),
            request_id: "request-capacity".into(),
            retryable: true,
            retry_after: None,
            outcome_certainty: proto::OutcomeCertainty::Known as i32,
            violations: Vec::new(),
        };
        let rich = RichStatus {
            code: Code::ResourceExhausted as i32,
            message: "public run capacity is exhausted".into(),
            details: vec![Any {
                type_url: ERROR_DETAIL_TYPE_URL.into(),
                value: detail.encode_to_vec(),
            }],
        };
        let error = api_error_from_status(Status::with_details(
            Code::ResourceExhausted,
            "public run capacity is exhausted",
            rich.encode_to_vec().into(),
        ));

        assert_eq!(error.code, ApiErrorCode::ResourceExhausted);
        assert_eq!(error.reason, ApiErrorReason::CapacityExceeded);
        assert!(error.retryable);
    }

    #[test]
    fn proto_run_rejects_terminal_payload_mismatch() {
        let run = proto::Run {
            run_id: "run-1".into(),
            session_id: "session-1".into(),
            title: "Test run".into(),
            role: "assistant".into(),
            mode: proto::RunMode::Execute as i32,
            status: proto::RunStatus::Running as i32,
            created_at: Some("2026-01-01T00:00:00Z".parse().expect("timestamp")),
            updated_at: Some("2026-01-01T00:00:00Z".parse().expect("timestamp")),
            started_at: None,
            finished_at: None,
            last_sequence: 1,
            pending_interaction_count: 0,
            terminal: Some(run::Terminal::Result(proto::RunResult {
                output: "unexpected".into(),
                plan_id: None,
                plan_revision: None,
                plan_status: proto::PlanStatus::Unspecified as i32,
                goal_id: None,
                profile: "default".into(),
                model_profile: "default".into(),
                provider_profile: "provider".into(),
                model: "model".into(),
                elapsed_seconds: 1.0,
            })),
            etag: "etag".into(),
            selected_skills: Vec::new(),
            archived: false,
        };
        assert_eq!(
            run_from_proto(run).expect_err("mismatch").reason,
            ApiErrorReason::InternalInvariant
        );
    }

    #[test]
    fn proto_run_uses_role_when_an_older_server_omits_title() {
        let run = proto::Run {
            run_id: "run-1".into(),
            session_id: "session-1".into(),
            title: String::new(),
            role: "assistant".into(),
            mode: proto::RunMode::Execute as i32,
            status: proto::RunStatus::Running as i32,
            created_at: Some("2026-01-01T00:00:00Z".parse().expect("timestamp")),
            updated_at: Some("2026-01-01T00:00:00Z".parse().expect("timestamp")),
            started_at: None,
            finished_at: None,
            last_sequence: 1,
            pending_interaction_count: 0,
            terminal: None,
            etag: "etag".into(),
            selected_skills: Vec::new(),
            archived: false,
        };

        assert_eq!(
            run_from_proto(run).expect("older server run").title,
            "assistant"
        );
    }

    #[test]
    fn proto_run_result_rejects_compatibility_profile_mismatch() {
        let error = run_result_from_proto(proto::RunResult {
            output: "answer".into(),
            plan_id: None,
            plan_revision: None,
            plan_status: proto::PlanStatus::Unspecified as i32,
            goal_id: None,
            profile: "legacy-alias".into(),
            model_profile: "different-model-profile".into(),
            provider_profile: "provider".into(),
            model: "model".into(),
            elapsed_seconds: 1.0,
        })
        .expect_err("profile mismatch");

        assert_eq!(error.reason, ApiErrorReason::InternalInvariant);
    }

    #[test]
    fn proto_run_result_preserves_optional_plan_identity() {
        let result = run_result_from_proto(proto::RunResult {
            output: "Plan saved".into(),
            plan_id: Some("plan-1".into()),
            plan_revision: Some(3),
            plan_status: proto::PlanStatus::Draft as i32,
            goal_id: None,
            profile: "default".into(),
            model_profile: "default".into(),
            provider_profile: "provider".into(),
            model: "model".into(),
            elapsed_seconds: 1.0,
        })
        .expect("plan result");

        assert_eq!(result.plan_id.as_deref(), Some("plan-1"));
    }

    #[test]
    fn proto_run_result_accepts_legacy_plan_identity_without_new_metadata() {
        let result = run_result_from_proto(proto::RunResult {
            output: "Plan saved".into(),
            plan_id: Some("plan-legacy".into()),
            plan_revision: None,
            plan_status: proto::PlanStatus::Unspecified as i32,
            goal_id: None,
            profile: "default".into(),
            model_profile: "default".into(),
            provider_profile: "provider".into(),
            model: "model".into(),
            elapsed_seconds: 1.0,
        })
        .expect("older server result");

        assert_eq!(result.plan_id.as_deref(), Some("plan-legacy"));
        assert_eq!(result.plan_revision, None);
        assert_eq!(result.plan_status, None);
    }

    #[test]
    fn proto_run_cancellation_preserves_optional_plan_identity() {
        let cancellation = cancellation_from_proto(proto::RunCancellation {
            turn: 2,
            message: "cancelled after persistence".into(),
            plan_id: Some("plan-1".into()),
            plan_revision: Some(2),
            plan_status: proto::PlanStatus::Draft as i32,
            goal_id: None,
        })
        .expect("plan cancellation");

        assert_eq!(cancellation.plan_id.as_deref(), Some("plan-1"));
    }

    #[test]
    fn proto_run_cancellation_accepts_legacy_plan_identity_without_new_metadata() {
        let cancellation = cancellation_from_proto(proto::RunCancellation {
            turn: 2,
            message: "cancelled after persistence".into(),
            plan_id: Some("plan-legacy".into()),
            plan_revision: None,
            plan_status: proto::PlanStatus::Unspecified as i32,
            goal_id: None,
        })
        .expect("older server cancellation");

        assert_eq!(cancellation.plan_id.as_deref(), Some("plan-legacy"));
        assert_eq!(cancellation.plan_revision, None);
        assert_eq!(cancellation.plan_status, None);
    }

    #[test]
    fn interaction_preserves_opaque_choice_id_and_label() {
        let interaction = proto::Interaction {
            interaction_id: "interaction-1".into(),
            run_id: "run-1".into(),
            kind: proto::InteractionKind::UserPrompt as i32,
            status: proto::InteractionStatus::Pending as i32,
            created_at: Some("2026-01-01T00:00:00Z".parse().expect("timestamp")),
            expires_at: Some("2026-01-01T00:01:00Z".parse().expect("timestamp")),
            respondable_by_caller: true,
            etag: "etag-1".into(),
            content: Some(interaction::Content::UserPrompt(
                proto::UserPromptInteraction {
                    question: "Choose".into(),
                    choices: vec![proto::PromptChoice {
                        choice_id: "opaque-server-choice".into(),
                        label: "Exact label".into(),
                    }],
                    allow_free_form: false,
                },
            )),
        };
        let converted = interaction_from_proto(interaction).expect("interaction");
        let InteractionContent::UserPrompt(prompt) = converted.content else {
            panic!("prompt");
        };
        assert_eq!(prompt.choices[0].choice_id, "opaque-server-choice");
        assert_eq!(prompt.choices[0].label, "Exact label");
    }

    #[test]
    fn approval_projection_rejects_spoofable_display_fields() {
        let approval = |action: &str, resource: &str| proto::Interaction {
            interaction_id: "interaction-1".into(),
            run_id: "run-1".into(),
            kind: proto::InteractionKind::Approval as i32,
            status: proto::InteractionStatus::Pending as i32,
            created_at: Some("2026-01-01T00:00:00Z".parse().expect("timestamp")),
            expires_at: Some("2026-01-01T00:01:00Z".parse().expect("timestamp")),
            respondable_by_caller: true,
            etag: "etag-1".into(),
            content: Some(interaction::Content::Approval(proto::ApprovalInteraction {
                reason: "A reviewed local effect requires permission.".into(),
                action: action.into(),
                resource: resource.into(),
                risk: proto::ApprovalRisk::Medium as i32,
                request_hash: "opaque-approval-binding".into(),
            })),
        };

        let valid = interaction_from_proto(approval("workspace.modify", "workspace resource"))
            .expect("canonical approval display");
        assert!(matches!(valid.content, InteractionContent::Approval(_)));

        for malformed in [
            approval("shell.run\nResource: harmless", "workspace resource"),
            approval("workspace.modify", "/private/secret/path"),
            approval("network.access", "https://user@example.com/"),
        ] {
            assert_eq!(
                interaction_from_proto(malformed)
                    .expect_err("malformed approval display")
                    .reason,
                ApiErrorReason::InternalInvariant
            );
        }
    }

    #[tokio::test]
    async fn live_pinned_tls_and_bearer_authentication_fail_closed() {
        let tls_identity =
            TlsIdentity::from_seed(TlsKeySeed::new([0x5a; 32])).expect("TLS identity");
        let certificate_pem = tls_identity.certificate_pem().to_vec();
        let fingerprint =
            TlsFingerprint::from_hex(tls_identity.certificate_sha256()).expect("fingerprint");
        let repository = Arc::new(InMemoryCredentialRepository::default());
        let authenticator = Arc::new(CredentialAuthenticator::new([0x37; 32], repository));
        let grant = ApplicationGrant::new(
            "app:rust-sdk-test",
            ApplicationKind::Enrolled,
            [
                ApiScope::new(RUNS_READ).expect("scope"),
                ApiScope::new(colossus_api::scopes::ARTIFACTS_READ).expect("scope"),
                ApiScope::new(colossus_api::scopes::ARTIFACTS_WRITE).expect("scope"),
            ],
            ["assistant".into()],
            Vec::<String>::new(),
        )
        .expect("grant");
        let issued = authenticator
            .issue_pending(&grant)
            .expect("issue pending credential");
        assert!(
            authenticator
                .activate(issued.credential_id())
                .expect("activate credential")
        );
        let credential_id = issued.credential_id().to_owned();
        let bearer = Zeroizing::new(issued.expose_token().as_bytes().to_vec());
        let server_instance_uuid =
            Uuid::parse_str("019f7d38-649a-7580-a30f-01157b719c2a").expect("instance UUID");
        let server_instance_id = InstanceId::from_uuid(server_instance_uuid);
        let system = SystemServiceAdapter::new(
            SystemMetadata {
                instance_id: server_instance_uuid.to_string(),
                server_version: "0.9.0".into(),
                deployment_mode: proto::DeploymentMode::SharedDaemon,
            },
            Arc::new(FixedReadiness::ready()),
        );
        let server = BoundPublicGrpcServer::bind(
            "127.0.0.1:0".parse().expect("address"),
            tls_identity,
            Arc::clone(&authenticator),
            system,
            Arc::new(ReadApi),
            Arc::new(EventSourcedArtifactApi::new(Arc::new(
                InMemoryEventJournal::default(),
            ))),
        )
        .await
        .expect("bind server");
        let address = server.local_addr();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            server
                .serve(async {
                    let _ = shutdown_rx.await;
                })
                .await
        });
        let endpoint = Url::parse(&format!("https://{address}/")).expect("endpoint");

        let slow_handshake = TcpStream::connect(address)
            .await
            .expect("slow TCP handshake");
        let good_provider = Arc::new(StaticCredential::new(bearer.as_slice()));
        let backend = tokio::time::timeout(
            Duration::from_secs(2),
            GrpcBackend::connect(
                GrpcConnectOptions::new(
                    BackendKind::Daemon,
                    server_instance_id,
                    ApiMajor::new(1).expect("major"),
                    endpoint.clone(),
                    fingerprint,
                    certificate_pem.clone(),
                    good_provider,
                )
                .expect("connect options"),
            ),
        )
        .await
        .expect("slow handshake must not block a valid client")
        .expect("connect");
        drop(slow_handshake);
        let response = backend
            .agent_runs()
            .get_run(GetRunRequest {
                run_id: "run-1".into(),
            })
            .await
            .expect("authenticated get");
        assert_eq!(response.run.run_id, "run-1");
        assert_eq!(response.run.selected_skills, ["rust"]);
        assert!(backend.capabilities().contains("artifacts.read"));
        assert!(backend.capabilities().contains("artifacts.upload"));
        let artifact = backend
            .artifacts()
            .expect("advertised artifact client")
            .upload(UploadArtifactRequest {
                file_name: "review.md".into(),
                media_type: "text/markdown".into(),
                purpose: ArtifactPurpose::RunInput,
                bytes: b"# Review\nready".to_vec(),
                idempotency_key: crate::IdempotencyKey::new("sdk-live-artifact")
                    .expect("idempotency key"),
            })
            .await
            .expect("authenticated artifact upload");
        let downloaded = backend
            .artifacts()
            .expect("artifact client")
            .download(&artifact.artifact_id)
            .await
            .expect("verified artifact download");
        assert_eq!(downloaded.bytes, b"# Review\nready");

        let identity_error = GrpcBackend::connect(
            GrpcConnectOptions::new(
                BackendKind::Daemon,
                InstanceId::from_uuid(Uuid::now_v7()),
                ApiMajor::new(1).expect("major"),
                endpoint.clone(),
                fingerprint,
                certificate_pem.clone(),
                Arc::new(StaticCredential::new(bearer.as_slice())),
            )
            .expect("mismatched identity remains well formed"),
        )
        .await
        .expect_err("authenticated server identity must match before returning a backend");
        assert!(matches!(identity_error, SdkError::IdentityMismatch));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let discovery = tempfile::tempdir().expect("private discovery directory");
            std::fs::set_permissions(discovery.path(), std::fs::Permissions::from_mode(0o700))
                .expect("private directory mode");
            let descriptor_path = discovery.path().join("endpoint.json");
            let certificate_path = discovery.path().join("certificate.pem");
            let descriptor = EndpointDescriptor::new(
                server_instance_uuid,
                endpoint.to_string(),
                std::process::id(),
                hex::encode(fingerprint.as_bytes()),
            )
            .expect("descriptor");
            write_endpoint_certificate(&certificate_path, &certificate_pem)
                .expect("protected certificate");
            write_endpoint_descriptor(&descriptor_path, &descriptor).expect("protected descriptor");
            let installed = Colossus::connect_installed(
                crate::DaemonConnectOptions::new(
                    server_instance_id,
                    descriptor_path,
                    fingerprint,
                    ApiMajor::new(1).expect("major"),
                    Arc::new(StaticCredential::new(bearer.as_slice())),
                )
                .expect("installed options")
                .with_certificate_path(certificate_path)
                .expect("certificate path"),
            )
            .await
            .expect("native installed connect");
            assert_eq!(
                installed
                    .get_run(GetRunRequest {
                        run_id: "run-native".into(),
                    })
                    .await
                    .expect("native authenticated get")
                    .run
                    .run_id,
                "run-native"
            );
        }

        let wrong_token = b"cls_v1.00000000-0000-7000-8000-000000000000.invalid";
        let wrong_error = GrpcBackend::connect(
            GrpcConnectOptions::new(
                BackendKind::Daemon,
                server_instance_id,
                ApiMajor::new(1).expect("major"),
                endpoint.clone(),
                fingerprint,
                certificate_pem.clone(),
                Arc::new(StaticCredential::new(wrong_token.as_slice())),
            )
            .expect("wrong bearer options remain non-secret"),
        )
        .await
        .expect_err("authentication is checked before a backend is exposed");
        assert!(matches!(wrong_error, SdkError::Authentication));
        assert!(!format!("{wrong_error:?}").contains("00000000-0000"));

        assert!(
            authenticator
                .revoke(&credential_id)
                .expect("revoke credential")
        );
        let revoked_error = backend
            .agent_runs()
            .get_run(GetRunRequest {
                run_id: "run-1".into(),
            })
            .await
            .expect_err("revoked bearer rejected");
        assert_eq!(revoked_error.code, ApiErrorCode::Unauthenticated);
        assert!(!format!("{revoked_error:?}").contains(issued.expose_token()));

        let wrong_pin = GrpcConnectOptions::new(
            BackendKind::Daemon,
            server_instance_id,
            ApiMajor::new(1).expect("major"),
            endpoint.clone(),
            TlsFingerprint::from_bytes([0x11; 32]),
            certificate_pem.clone(),
            Arc::new(StaticCredential::new(bearer.as_slice())),
        )
        .expect_err("wrong certificate pin rejected locally");
        assert!(matches!(wrong_pin, SdkError::IdentityMismatch));

        let mut roots = RootCertStore::empty();
        let leaf = CertificateDer::pem_slice_iter(&certificate_pem)
            .next()
            .expect("leaf")
            .expect("valid leaf");
        roots.add(leaf).expect("trust test leaf");
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls12 = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS12])
            .expect("TLS 1.2 config")
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tcp = TcpStream::connect(address).await.expect("TCP");
        let server_name = ServerName::try_from("127.0.0.1")
            .expect("server name")
            .to_owned();
        assert!(
            TlsConnector::from(Arc::new(tls12))
                .connect(server_name, tcp)
                .await
                .is_err(),
            "TLS 1.2 must be rejected by the actual acceptor"
        );

        let _ = shutdown_tx.send(());
        server_task
            .await
            .expect("server task")
            .expect("server shutdown");
    }
}
