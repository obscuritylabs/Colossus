use crate::{
    ApiMajor, AppPrivateInstanceDir, Backend, BackendKind, Colossus, InstanceId, SdkError,
    SdkResult, VerifiedExecutable,
};
use async_trait::async_trait;
use colossus_api::{ApiScope, ApplicationKind, scopes};
use colossus_sidecar_protocol::{
    BootstrapGrant, BootstrapRequest, HostCredential, PROTOCOL_VERSION, SecretString,
    encode_worker_authentication,
};
use std::sync::Arc;
use std::{fmt, path::PathBuf};
use uuid::Uuid;
use zeroize::Zeroizing;

pub use colossus_sidecar_protocol::{
    ManagedAccessProfile, ManagedChatCompletionsOutputTokenParameter, ManagedModelCapabilities,
    ManagedModelConfig, ManagedProviderConfig, ManagedProviderKind, ManagedRuntimeConfig,
    REMOTE_PROVIDER_TIMEOUT_MS, WorkspaceIdentity, default_managed_provider_timeout_ms,
    validate_managed_model_identifier, validate_managed_provider_base_url,
};

/// Fixed secret-free runtime configuration filename written inside the instance directory.
pub const MANAGED_CONFIG_FILENAME: &str = "managed-config.yaml";

/// Sanitized reason why one supervised native sidecar lifecycle failed closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSidecarFailure {
    /// The selected pathname no longer names the host-attested workspace object.
    WorkspaceIdentityChanged,
    /// Process cleanup, launch, transport, or another non-actionable supervision step failed.
    SupervisionFailed,
}

/// Secret-free state of one supervised native sidecar lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSidecarStatus {
    /// Executable verification, process launch, or authenticated bootstrap is active.
    Starting,
    /// The exact verified child is authenticated and serving API requests.
    Ready,
    /// An unexpected exit is consuming the bounded lifecycle-wide restart budget.
    Restarting,
    /// Explicit close or owner drop is stopping the managed process tree.
    Stopping,
    /// Initial launch or bounded recovery failed closed with one sanitized reason.
    Failed(NativeSidecarFailure),
}

/// Isolated bundled-sidecar launch policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidecarOptions {
    instance_id: InstanceId,
    instance_dir: AppPrivateInstanceDir,
    executable: VerifiedExecutable,
    api_major: ApiMajor,
}

impl SidecarOptions {
    /// Create policy for an explicit application-private sidecar instance.
    pub fn new(
        instance_id: InstanceId,
        instance_dir: AppPrivateInstanceDir,
        executable: VerifiedExecutable,
        api_major: ApiMajor,
    ) -> SdkResult<Self> {
        instance_id.validate()?;
        Ok(Self {
            instance_id,
            instance_dir,
            executable,
            api_major,
        })
    }

    /// Isolated instance identity.
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    /// Application-private canonical state directory.
    pub const fn instance_dir(&self) -> &AppPrivateInstanceDir {
        &self.instance_dir
    }

    /// Exact bundled executable and required digest.
    pub const fn executable(&self) -> &VerifiedExecutable {
        &self.executable
    }

    /// Required public API major.
    pub const fn api_major(&self) -> ApiMajor {
        self.api_major
    }

    /// Native-only path to the generated secret-free configuration used by the TUI.
    pub fn managed_config_path(&self) -> PathBuf {
        self.instance_dir.as_path().join(MANAGED_CONFIG_FILENAME)
    }
}

/// Exact public API authority assigned to the supervising application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidecarApplicationGrant {
    application_id: String,
    scopes: Vec<String>,
    allowed_roles: Vec<String>,
    allowed_tools: Vec<String>,
}

impl SidecarApplicationGrant {
    /// Validate one sidecar application identity and its immutable authority ceilings.
    pub fn new(
        application_id: impl Into<String>,
        scopes: impl IntoIterator<Item = ApiScope>,
        allowed_roles: impl IntoIterator<Item = String>,
        allowed_tools: impl IntoIterator<Item = String>,
    ) -> SdkResult<Self> {
        let application_id = application_id.into();
        let scopes = scopes.into_iter().collect::<Vec<_>>();
        let allowed_roles = allowed_roles.into_iter().collect::<Vec<_>>();
        let allowed_tools = allowed_tools.into_iter().collect::<Vec<_>>();
        colossus_grpc::ApplicationGrant::new(
            application_id.clone(),
            ApplicationKind::Sidecar,
            scopes.clone(),
            allowed_roles.clone(),
            allowed_tools.clone(),
        )
        .map_err(|_| SdkError::InvalidConfiguration("sidecar application grant is invalid"))?;
        Ok(Self {
            application_id,
            scopes: scopes
                .into_iter()
                .map(|scope| scope.as_str().to_owned())
                .collect(),
            allowed_roles,
            allowed_tools,
        })
    }

    pub(crate) fn wire(&self) -> BootstrapGrant {
        BootstrapGrant {
            application_id: self.application_id.clone(),
            scopes: self.scopes.clone(),
            allowed_roles: self.allowed_roles.clone(),
            allowed_tools: self.allowed_tools.clone(),
        }
    }
}

/// Native-only authority for submitting effect approval decisions.
///
/// This is intentionally a separate credential from the supervising application's
/// ordinary run client. Its scope and empty tool ceiling are fixed by this type; the
/// sidecar protocol additionally binds its application and roles to the primary grant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SidecarApprovalBrokerGrant {
    application_id: String,
    allowed_roles: Vec<String>,
}

impl SidecarApprovalBrokerGrant {
    /// Create a broker grant with only `approvals:respond` and no tool authority.
    pub fn new(
        application_id: impl Into<String>,
        allowed_roles: impl IntoIterator<Item = String>,
    ) -> SdkResult<Self> {
        let application_id = application_id.into();
        let allowed_roles = allowed_roles.into_iter().collect::<Vec<_>>();
        colossus_grpc::ApplicationGrant::new(
            application_id.clone(),
            ApplicationKind::Sidecar,
            [ApiScope::new(scopes::APPROVALS_RESPOND).map_err(|_| {
                SdkError::InvalidConfiguration("sidecar approval broker scope is invalid")
            })?],
            allowed_roles.clone(),
            Vec::<String>::new(),
        )
        .map_err(|_| SdkError::InvalidConfiguration("sidecar approval broker grant is invalid"))?;
        Ok(Self {
            application_id,
            allowed_roles,
        })
    }

    fn wire(&self) -> BootstrapGrant {
        BootstrapGrant {
            application_id: self.application_id.clone(),
            scopes: vec![scopes::APPROVALS_RESPOND.into()],
            allowed_roles: self.allowed_roles.clone(),
            allowed_tools: Vec::new(),
        }
    }
}

/// One host-resolved provider credential retained only in zeroizing native memory.
pub struct SidecarHostCredential {
    id: String,
    secret: SecretString,
}

impl SidecarHostCredential {
    /// Bind a credential to an opaque identifier referenced as `host:<id>`.
    pub fn new(id: impl Into<String>, secret: crate::Secret) -> SdkResult<Self> {
        let secret = std::str::from_utf8(secret.expose())
            .map_err(|_| SdkError::InvalidConfiguration("host credential must be UTF-8"))?;
        let credential = HostCredential::new(
            id,
            SecretString::new(secret.to_owned())
                .map_err(|_| SdkError::InvalidConfiguration("host credential is invalid"))?,
        )
        .map_err(|_| SdkError::InvalidConfiguration("host credential identifier is invalid"))?;
        Ok(Self {
            id: credential.id,
            secret: credential.secret,
        })
    }

    pub(crate) fn wire(&self) -> SdkResult<HostCredential> {
        HostCredential::new(
            self.id.clone(),
            SecretString::new(self.secret.expose().to_owned())
                .map_err(|_| SdkError::SidecarFailed)?,
        )
        .map_err(|_| SdkError::SidecarFailed)
    }
}

impl fmt::Debug for SidecarHostCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SidecarHostCredential")
            .field("id", &self.id)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

/// App-owned runtime configuration sent only through the inherited bootstrap channel.
pub struct SidecarBootstrapConfig {
    workspace: PathBuf,
    runtime: ManagedRuntimeConfig,
    grant: SidecarApplicationGrant,
    expected_workspace_identity: Option<WorkspaceIdentity>,
    ca_bundle_path: Option<PathBuf>,
    approval_broker_grant: Option<SidecarApprovalBrokerGrant>,
    host_credentials: Vec<SidecarHostCredential>,
    worker_ipc_authentication: Option<SecretString>,
}

impl SidecarBootstrapConfig {
    /// Build a folder-first managed runtime bootstrap without provider credentials.
    pub fn new(
        workspace: impl Into<PathBuf>,
        runtime: ManagedRuntimeConfig,
        grant: SidecarApplicationGrant,
    ) -> SdkResult<Self> {
        let workspace = workspace.into();
        if !workspace.is_absolute() || runtime.validate().is_err() {
            return Err(SdkError::InvalidConfiguration(
                "sidecar bootstrap configuration is invalid",
            ));
        }
        Ok(Self {
            workspace,
            runtime,
            grant,
            expected_workspace_identity: None,
            ca_bundle_path: None,
            approval_broker_grant: None,
            host_credentials: Vec::new(),
            worker_ipc_authentication: None,
        })
    }

    /// Require launch and every supervised restart to use the exact workspace object
    /// previously attested by the native host.
    ///
    /// macOS requires the persisted birthtime-bound v2 identity. Other Unix hosts use
    /// the descriptor-lifetime v1 device/inode identity; unsupported platforms fail
    /// closed. The value is compared before provider or worker secrets are cloned into
    /// a bootstrap frame and before the verified child is spawned.
    pub fn with_expected_workspace_identity(
        mut self,
        identity: WorkspaceIdentity,
    ) -> SdkResult<Self> {
        validate_expected_workspace_identity(&identity)?;
        self.expected_workspace_identity = Some(identity);
        Ok(self)
    }

    /// Add one native-copied private CA bundle to every managed-runtime network client.
    pub fn with_additional_ca_bundle_path(mut self, path: impl Into<PathBuf>) -> SdkResult<Self> {
        let path = path.into();
        if !path.is_absolute()
            || path.parent().is_none()
            || path.to_str().is_none_or(|path| path.len() > 4_096)
        {
            return Err(SdkError::InvalidConfiguration(
                "additional CA bundle path is invalid",
            ));
        }
        self.ca_bundle_path = Some(path);
        Ok(self)
    }

    /// Attach a separate native approval broker bounded by the primary application.
    pub fn with_approval_broker_grant(
        mut self,
        grant: SidecarApprovalBrokerGrant,
    ) -> SdkResult<Self> {
        let primary_roles = self
            .grant
            .allowed_roles
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if grant.application_id != self.grant.application_id
            || grant
                .allowed_roles
                .iter()
                .any(|role| !primary_roles.contains(role))
            || self
                .grant
                .scopes
                .iter()
                .any(|scope| scope == scopes::APPROVALS_RESPOND)
        {
            return Err(SdkError::InvalidConfiguration(
                "sidecar approval broker must be bounded by the primary grant",
            ));
        }
        self.approval_broker_grant = Some(grant);
        Ok(self)
    }

    /// Attach a bounded exact map of host-resolved provider credentials.
    pub fn with_host_credentials(
        mut self,
        credentials: Vec<SidecarHostCredential>,
    ) -> SdkResult<Self> {
        if credentials.len() > colossus_sidecar_protocol::MAX_HOST_CREDENTIALS {
            return Err(SdkError::InvalidConfiguration(
                "too many sidecar host credentials",
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        if credentials
            .iter()
            .any(|credential| !ids.insert(&credential.id))
        {
            return Err(SdkError::InvalidConfiguration(
                "sidecar host credential identifiers must be unique",
            ));
        }
        self.host_credentials = credentials;
        Ok(self)
    }

    /// Supply an independent worker IPC key for a bundled native TUI.
    ///
    /// The key remains in zeroizing bootstrap memory and is delivered only through
    /// the inherited sidecar channel. It is never serialized into managed YAML.
    pub fn with_worker_ipc_authentication(
        mut self,
        authentication: crate::Secret,
    ) -> SdkResult<Self> {
        let authentication =
            Zeroizing::new(<[u8; 32]>::try_from(authentication.expose()).map_err(|_| {
                SdkError::InvalidConfiguration("worker IPC authentication must be 32 bytes")
            })?);
        self.worker_ipc_authentication =
            Some(encode_worker_authentication(&authentication).map_err(|_| {
                SdkError::InvalidConfiguration("worker IPC authentication is invalid")
            })?);
        Ok(self)
    }

    pub(crate) fn request(
        &self,
        options: &SidecarOptions,
        canonical_workspace: &std::path::Path,
        workspace_identity: WorkspaceIdentity,
    ) -> SdkResult<BootstrapRequest> {
        let request = BootstrapRequest {
            protocol_version: PROTOCOL_VERSION,
            exchange_id: Uuid::now_v7().to_string(),
            instance_id: options.instance_id().to_string(),
            api_major: options.api_major().get(),
            instance_dir: options
                .instance_dir()
                .as_path()
                .to_str()
                .ok_or(SdkError::InvalidConfiguration(
                    "sidecar instance path must be UTF-8",
                ))?
                .to_owned(),
            workspace: canonical_workspace
                .to_str()
                .ok_or(SdkError::InvalidConfiguration(
                    "sidecar workspace path must be UTF-8",
                ))?
                .to_owned(),
            workspace_identity,
            ca_bundle_path: self
                .ca_bundle_path
                .as_ref()
                .map(|path| {
                    path.to_str()
                        .map(str::to_owned)
                        .ok_or(SdkError::InvalidConfiguration(
                            "additional CA bundle path must be UTF-8",
                        ))
                })
                .transpose()?,
            runtime: self.runtime.clone(),
            grant: self.grant.wire(),
            approval_broker_grant: self
                .approval_broker_grant
                .as_ref()
                .map(SidecarApprovalBrokerGrant::wire),
            host_credentials: self
                .host_credentials
                .iter()
                .map(SidecarHostCredential::wire)
                .collect::<SdkResult<Vec<_>>>()?,
            worker_ipc_authentication: self
                .worker_ipc_authentication
                .as_ref()
                .map(|authentication| SecretString::new(authentication.expose().to_owned()))
                .transpose()
                .map_err(|_| SdkError::SidecarFailed)?,
        };
        request.validate().map_err(|_| SdkError::SidecarFailed)?;
        Ok(request)
    }

    pub(crate) fn workspace(&self) -> &std::path::Path {
        &self.workspace
    }

    pub(crate) fn expected_workspace_identity(&self) -> Option<&WorkspaceIdentity> {
        self.expected_workspace_identity.as_ref()
    }
}

#[cfg(target_os = "macos")]
fn validate_expected_workspace_identity(identity: &WorkspaceIdentity) -> SdkResult<()> {
    if identity.is_current_macos() {
        Ok(())
    } else {
        Err(SdkError::InvalidConfiguration(
            "expected sidecar workspace identity is invalid",
        ))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn validate_expected_workspace_identity(identity: &WorkspaceIdentity) -> SdkResult<()> {
    identity.validate().map_err(|_| {
        SdkError::InvalidConfiguration("expected sidecar workspace identity is invalid")
    })?;
    if identity.is_legacy_v1() {
        Ok(())
    } else {
        Err(SdkError::InvalidConfiguration(
            "expected sidecar workspace identity is invalid",
        ))
    }
}

#[cfg(windows)]
fn validate_expected_workspace_identity(identity: &WorkspaceIdentity) -> SdkResult<()> {
    if identity.is_current_windows() {
        Ok(())
    } else {
        Err(SdkError::InvalidConfiguration(
            "expected sidecar workspace identity is invalid",
        ))
    }
}

#[cfg(not(any(unix, windows)))]
fn validate_expected_workspace_identity(identity: &WorkspaceIdentity) -> SdkResult<()> {
    let _ = identity;
    Err(SdkError::InvalidConfiguration(
        "expected sidecar workspace identity is invalid",
    ))
}

impl fmt::Debug for SidecarBootstrapConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SidecarBootstrapConfig")
            .field("workspace", &"[PRIVATE PATH]")
            .field(
                "expected_workspace_identity",
                &self
                    .expected_workspace_identity
                    .as_ref()
                    .map(|_| "[OPAQUE IDENTITY]"),
            )
            .field("ca_bundle_configured", &self.ca_bundle_path.is_some())
            .field("runtime", &self.runtime)
            .field("grant", &self.grant)
            .field("approval_broker_grant", &self.approval_broker_grant)
            .field("host_credentials", &"[REDACTED]")
            .field("worker_ipc_authentication", &"[REDACTED]")
            .finish()
    }
}

/// Platform sidecar launcher, bootstrap channel, and guardian implementation.
///
/// Implementations must verify the executable immediately before launching it without a
/// shell. They must create a one-use bootstrap secret internally, transfer it only over
/// an inherited pipe or handle, exchange it for a memory-only scoped credential, and
/// retain a guardian whose EOF requests clean shutdown. Bootstrap material must never
/// enter `SidecarOptions`, argv, the environment, discovery files, or debug output.
#[async_trait]
pub trait SidecarLifecycle: Send + Sync {
    /// Start, authenticate, and supervise an isolated sidecar.
    async fn start_verified(&self, options: &SidecarOptions) -> SdkResult<Arc<dyn Backend>>;
}

impl Colossus {
    /// Start an authenticated, isolated application-bundled sidecar.
    pub async fn start_sidecar(
        lifecycle: &impl SidecarLifecycle,
        options: SidecarOptions,
    ) -> SdkResult<Self> {
        let backend = lifecycle.start_verified(&options).await?;
        if backend.kind() != BackendKind::Sidecar {
            let _ = backend.close().await;
            return Err(SdkError::IdentityMismatch);
        }
        Ok(Self::from_shared_backend(backend))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn primary_grant(scopes: &[&str]) -> SidecarApplicationGrant {
        SidecarApplicationGrant::new(
            "app:desktop",
            scopes
                .iter()
                .map(|scope| ApiScope::new(*scope).expect("scope")),
            ["primary".into()],
            ["shell.run".into()],
        )
        .expect("primary grant")
    }

    fn runtime() -> ManagedRuntimeConfig {
        ManagedRuntimeConfig::echo(ManagedAccessProfile::Minimal)
    }

    #[test]
    fn approval_broker_scope_and_tools_are_fixed_by_type() {
        let broker =
            SidecarApprovalBrokerGrant::new("app:desktop", ["primary".into()]).expect("broker");
        let wire = broker.wire();
        assert_eq!(wire.scopes, [scopes::APPROVALS_RESPOND]);
        assert!(wire.allowed_tools.is_empty());
        assert_eq!(wire.allowed_roles, ["primary"]);
    }

    #[test]
    fn worker_ipc_authentication_is_exact_and_redacted() {
        let bootstrap = SidecarBootstrapConfig::new(
            "/tmp/colossus-sdk-sidecar-workspace",
            runtime(),
            primary_grant(&[scopes::RUNS_READ]),
        )
        .expect("bootstrap");
        assert!(
            bootstrap
                .with_worker_ipc_authentication(
                    crate::Secret::new(vec![0x5a; 31]).expect("bounded secret"),
                )
                .is_err()
        );

        let bootstrap = SidecarBootstrapConfig::new(
            "/tmp/colossus-sdk-sidecar-workspace",
            runtime(),
            primary_grant(&[scopes::RUNS_READ]),
        )
        .expect("bootstrap")
        .with_worker_ipc_authentication(crate::Secret::new(vec![0x5a; 32]).expect("bounded secret"))
        .expect("worker authentication");
        let encoded = bootstrap
            .worker_ipc_authentication
            .as_ref()
            .expect("worker authentication");
        assert_eq!(
            *colossus_sidecar_protocol::decode_worker_authentication(encoded)
                .expect("decode worker authentication"),
            [0x5a; 32]
        );
        let debug = format!("{bootstrap:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(&"5a".repeat(32)));
    }

    #[test]
    fn additional_ca_bundle_path_is_native_only_and_redacted() {
        let path = if cfg!(windows) {
            r"C:\private\app\trust\company-ca.pem"
        } else {
            "/private/app/trust/company-ca.pem"
        };
        let bootstrap = SidecarBootstrapConfig::new(
            "/tmp/colossus-sdk-sidecar-workspace",
            runtime(),
            primary_grant(&[scopes::RUNS_READ]),
        )
        .expect("bootstrap")
        .with_additional_ca_bundle_path(path)
        .expect("CA bundle path");

        let debug = format!("{bootstrap:?}");
        assert!(debug.contains("ca_bundle_configured: true"));
        assert!(!debug.contains(path));
        assert!(
            SidecarBootstrapConfig::new(
                "/tmp/colossus-sdk-sidecar-workspace",
                runtime(),
                primary_grant(&[scopes::RUNS_READ]),
            )
            .expect("bootstrap")
            .with_additional_ca_bundle_path("../company-ca.pem")
            .is_err()
        );
    }

    #[test]
    fn expected_workspace_identity_is_validated_and_redacted() {
        let v2_identity = WorkspaceIdentity::from_macos_parts(42, 84, 1_700_000_000, 123_456_789)
            .expect("current identity");
        let v1_identity = WorkspaceIdentity::from_unix_parts(42, 84);
        #[cfg(target_os = "macos")]
        let accepted_identity = v2_identity.clone();
        #[cfg(all(unix, not(target_os = "macos")))]
        let accepted_identity = v1_identity.clone();
        #[cfg(unix)]
        let bootstrap = SidecarBootstrapConfig::new(
            "/tmp/colossus-sdk-sidecar-workspace",
            runtime(),
            primary_grant(&[scopes::RUNS_READ]),
        )
        .expect("bootstrap")
        .with_expected_workspace_identity(accepted_identity.clone())
        .expect("expected workspace identity");
        #[cfg(unix)]
        assert_eq!(
            bootstrap.expected_workspace_identity(),
            Some(&accepted_identity)
        );
        #[cfg(unix)]
        let debug = format!("{bootstrap:?}");
        #[cfg(unix)]
        assert!(debug.contains("[OPAQUE IDENTITY]"));
        #[cfg(unix)]
        assert!(!debug.contains(&accepted_identity.sha256));

        #[cfg(target_os = "macos")]
        assert!(
            SidecarBootstrapConfig::new(
                "/tmp/colossus-sdk-sidecar-workspace",
                runtime(),
                primary_grant(&[scopes::RUNS_READ]),
            )
            .expect("bootstrap")
            .with_expected_workspace_identity(v1_identity.clone())
            .is_err()
        );
        #[cfg(not(target_os = "macos"))]
        assert!(
            SidecarBootstrapConfig::new(
                "/tmp/colossus-sdk-sidecar-workspace",
                runtime(),
                primary_grant(&[scopes::RUNS_READ]),
            )
            .expect("bootstrap")
            .with_expected_workspace_identity(v2_identity.clone())
            .is_err()
        );
        #[cfg(not(unix))]
        assert!(
            SidecarBootstrapConfig::new(
                "/tmp/colossus-sdk-sidecar-workspace",
                runtime(),
                primary_grant(&[scopes::RUNS_READ]),
            )
            .expect("bootstrap")
            .with_expected_workspace_identity(v1_identity)
            .is_err()
        );

        let mut malformed = v2_identity;
        malformed.version = u16::MAX;
        assert!(
            SidecarBootstrapConfig::new(
                "/tmp/colossus-sdk-sidecar-workspace",
                runtime(),
                primary_grant(&[scopes::RUNS_READ]),
            )
            .expect("bootstrap")
            .with_expected_workspace_identity(malformed)
            .is_err()
        );
    }

    #[test]
    fn approval_broker_must_match_and_not_widen_the_primary_grant() {
        let bootstrap = SidecarBootstrapConfig::new(
            "/tmp/colossus-sdk-sidecar-workspace",
            runtime(),
            primary_grant(&[
                scopes::RUNS_EXECUTE,
                scopes::RUNS_READ,
                scopes::RUNS_CONTROL,
                scopes::PROMPTS_RESPOND,
            ]),
        )
        .expect("bootstrap");
        bootstrap
            .with_approval_broker_grant(
                SidecarApprovalBrokerGrant::new("app:desktop", ["primary".into()]).expect("broker"),
            )
            .expect("bounded broker");

        let bootstrap = SidecarBootstrapConfig::new(
            "/tmp/colossus-sdk-sidecar-workspace",
            runtime(),
            primary_grant(&[scopes::RUNS_READ]),
        )
        .expect("bootstrap");
        assert!(
            bootstrap
                .with_approval_broker_grant(
                    SidecarApprovalBrokerGrant::new("app:other", ["primary".into()])
                        .expect("shape")
                )
                .is_err()
        );

        let bootstrap = SidecarBootstrapConfig::new(
            "/tmp/colossus-sdk-sidecar-workspace",
            runtime(),
            primary_grant(&[scopes::RUNS_READ, scopes::APPROVALS_RESPOND]),
        )
        .expect("bootstrap");
        assert!(
            bootstrap
                .with_approval_broker_grant(
                    SidecarApprovalBrokerGrant::new("app:desktop", ["primary".into()])
                        .expect("broker")
                )
                .is_err()
        );
    }
}
