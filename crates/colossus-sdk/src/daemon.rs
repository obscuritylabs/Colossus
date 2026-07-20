use crate::{
    ApiMajor, AppPrivateInstanceDir, Backend, Colossus, CredentialProvider, InstanceId, SdkError,
    SdkResult, TlsFingerprint, VerifiedExecutable, config::absolute_non_root_path,
};
use async_trait::async_trait;
use std::{
    fmt,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Arc,
};
use url::{Host, Url};

/// Connection policy for one installed shared daemon.
pub struct DaemonConnectOptions {
    instance_id: InstanceId,
    descriptor_path: PathBuf,
    certificate_path: Option<PathBuf>,
    expected_tls_fingerprint: TlsFingerprint,
    api_major: ApiMajor,
    credential_provider: Arc<dyn CredentialProvider>,
}

impl DaemonConnectOptions {
    /// Create connection policy for an exact instance and owner-only descriptor.
    pub fn new(
        instance_id: InstanceId,
        descriptor_path: impl Into<PathBuf>,
        expected_tls_fingerprint: TlsFingerprint,
        api_major: ApiMajor,
        credential_provider: Arc<dyn CredentialProvider>,
    ) -> SdkResult<Self> {
        instance_id.validate()?;
        let descriptor_path = absolute_non_root_path(descriptor_path.into())?;
        if descriptor_path.file_name().is_none() {
            return Err(SdkError::InvalidConfiguration(
                "daemon descriptor must be a file path",
            ));
        }
        Ok(Self {
            instance_id,
            descriptor_path,
            certificate_path: None,
            expected_tls_fingerprint,
            api_major,
            credential_provider,
        })
    }

    /// Select the exact separately protected public-certificate file.
    pub fn with_certificate_path(
        mut self,
        certificate_path: impl Into<PathBuf>,
    ) -> SdkResult<Self> {
        let certificate_path = absolute_non_root_path(certificate_path.into())?;
        if certificate_path.file_name().is_none() || certificate_path == self.descriptor_path {
            return Err(SdkError::InvalidConfiguration(
                "daemon certificate must be a distinct file path",
            ));
        }
        self.certificate_path = Some(certificate_path);
        Ok(self)
    }

    /// Expected stable instance identity.
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    /// Exact owner-only descriptor path.
    pub fn descriptor_path(&self) -> &Path {
        &self.descriptor_path
    }

    /// Exact owner-only public-certificate path, when native connection is configured.
    pub fn certificate_path(&self) -> Option<&Path> {
        self.certificate_path.as_deref()
    }

    /// Independently provisioned expected TLS leaf identity.
    pub const fn expected_tls_fingerprint(&self) -> TlsFingerprint {
        self.expected_tls_fingerprint
    }

    /// Required public API major.
    pub const fn api_major(&self) -> ApiMajor {
        self.api_major
    }

    /// Protected application credential source.
    pub fn credential_provider(&self) -> &dyn CredentialProvider {
        self.credential_provider.as_ref()
    }

    pub(crate) fn credential_provider_arc(&self) -> Arc<dyn CredentialProvider> {
        Arc::clone(&self.credential_provider)
    }
}

impl fmt::Debug for DaemonConnectOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonConnectOptions")
            .field("instance_id", &self.instance_id)
            .field("descriptor_path", &self.descriptor_path)
            .field("certificate_path", &self.certificate_path)
            .field("expected_tls_fingerprint", &self.expected_tls_fingerprint)
            .field("api_major", &self.api_major)
            .field("credential_provider", &"[REDACTED]")
            .finish()
    }
}

/// Validated public fields from an owner-only daemon descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonDescriptor {
    instance_id: InstanceId,
    pid: NonZeroU32,
    endpoint: Url,
    api_major: ApiMajor,
    tls_fingerprint: TlsFingerprint,
}

impl DaemonDescriptor {
    /// Validate a descriptor's portable content.
    ///
    /// The discovery adapter must separately validate file ownership, permissions,
    /// symlink safety, freshness, and process identity before constructing this value.
    pub fn new(
        instance_id: InstanceId,
        pid: NonZeroU32,
        endpoint: Url,
        api_major: ApiMajor,
        tls_fingerprint: TlsFingerprint,
    ) -> SdkResult<Self> {
        instance_id.validate()?;
        validate_loopback_endpoint(&endpoint)?;
        Ok(Self {
            instance_id,
            pid,
            endpoint,
            api_major,
            tls_fingerprint,
        })
    }

    /// Stable instance identity asserted by the daemon.
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    /// Live process identifier verified by the discovery adapter.
    pub const fn pid(&self) -> NonZeroU32 {
        self.pid
    }

    /// HTTPS endpoint using an IP-literal loopback host and explicit port.
    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    /// Public API major advertised by the daemon.
    pub const fn api_major(&self) -> ApiMajor {
        self.api_major
    }

    /// TLS identity to pin for the connection.
    pub const fn tls_fingerprint(&self) -> TlsFingerprint {
        self.tls_fingerprint
    }

    pub(crate) fn validate_for(&self, options: &DaemonConnectOptions) -> SdkResult<()> {
        if self.instance_id != options.instance_id {
            return Err(SdkError::IdentityMismatch);
        }
        if self.api_major != options.api_major {
            return Err(SdkError::VersionMismatch);
        }
        if self.tls_fingerprint != options.expected_tls_fingerprint {
            return Err(SdkError::IdentityMismatch);
        }
        validate_loopback_endpoint(&self.endpoint)
    }
}

/// Result of security-checked descriptor discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaemonDiscovery {
    /// No descriptor and no endpoint exists.
    Absent,
    /// An owner-only, live descriptor was verified.
    Present(DaemonDescriptor),
}

/// Opaque held launch lock preventing competing auto-start attempts.
pub struct DaemonLaunchGuard {
    _guard: Box<dyn Send>,
}

impl DaemonLaunchGuard {
    /// Wrap a platform lock guard. Dropping this value must release the lock.
    pub fn new(guard: impl Send + 'static) -> Self {
        Self {
            _guard: Box::new(guard),
        }
    }
}

impl fmt::Debug for DaemonLaunchGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DaemonLaunchGuard")
            .finish_non_exhaustive()
    }
}

/// Secure auto-start policy for an installed daemon.
#[derive(Debug)]
pub struct DaemonLaunchOptions {
    connect: DaemonConnectOptions,
    executable: VerifiedExecutable,
    instance_dir: AppPrivateInstanceDir,
}

impl DaemonLaunchOptions {
    /// Create an auto-start request with an exact executable and private state directory.
    pub fn new(
        connect: DaemonConnectOptions,
        executable: VerifiedExecutable,
        instance_dir: AppPrivateInstanceDir,
    ) -> Self {
        Self {
            connect,
            executable,
            instance_dir,
        }
    }

    /// Connection policy used before and after launch.
    pub const fn connect(&self) -> &DaemonConnectOptions {
        &self.connect
    }

    /// Exact path and digest that must be reverified immediately before execution.
    pub const fn executable(&self) -> &VerifiedExecutable {
        &self.executable
    }

    /// Exact owner-private canonical state directory.
    pub const fn instance_dir(&self) -> &AppPrivateInstanceDir {
        &self.instance_dir
    }
}

/// Platform-specific daemon discovery, authenticated transport, and verified launch.
///
/// Implementations are a security boundary. `discover` must distinguish true absence
/// from busy, malformed, unauthorized, or identity-mismatched endpoints. `launch` must
/// execute the exact pinned file without a shell, PATH search, ambient environment, or
/// automatic download.
#[async_trait]
pub trait DaemonLifecycle: Send + Sync {
    /// Inspect the exact descriptor and return `Absent` only after proving absence.
    async fn discover(&self, options: &DaemonConnectOptions) -> SdkResult<DaemonDiscovery>;

    /// Authenticate to a descriptor after pinning its exact TLS identity.
    async fn connect_verified(
        &self,
        options: &DaemonConnectOptions,
        descriptor: &DaemonDescriptor,
    ) -> SdkResult<Arc<dyn Backend>>;

    /// Acquire a per-instance, cross-process launch lock.
    async fn acquire_launch_guard(
        &self,
        options: &DaemonLaunchOptions,
    ) -> SdkResult<DaemonLaunchGuard>;

    /// Launch the exact pinned executable while the supplied guard is held.
    async fn launch_verified(
        &self,
        options: &DaemonLaunchOptions,
        guard: &DaemonLaunchGuard,
    ) -> SdkResult<()>;

    /// Wait boundedly for the newly launched daemon and verify its descriptor.
    async fn wait_until_ready(
        &self,
        options: &DaemonConnectOptions,
        guard: &DaemonLaunchGuard,
    ) -> SdkResult<DaemonDescriptor>;
}

impl Colossus {
    /// Connect only when an authenticated daemon already exists.
    pub async fn connect(
        lifecycle: &impl DaemonLifecycle,
        options: DaemonConnectOptions,
    ) -> SdkResult<Self> {
        match lifecycle.discover(&options).await? {
            DaemonDiscovery::Absent => Err(SdkError::Unavailable),
            DaemonDiscovery::Present(descriptor) => {
                connect_descriptor(lifecycle, &options, &descriptor).await
            }
        }
    }

    /// Connect to a shared daemon, starting it only after verified absence and lock recheck.
    pub async fn connect_or_start(
        lifecycle: &impl DaemonLifecycle,
        options: DaemonLaunchOptions,
    ) -> SdkResult<Self> {
        match lifecycle.discover(options.connect()).await? {
            DaemonDiscovery::Present(descriptor) => {
                return connect_descriptor(lifecycle, options.connect(), &descriptor).await;
            }
            DaemonDiscovery::Absent => {}
        }

        let guard = lifecycle.acquire_launch_guard(&options).await?;
        match lifecycle.discover(options.connect()).await? {
            DaemonDiscovery::Present(descriptor) => {
                return connect_descriptor(lifecycle, options.connect(), &descriptor).await;
            }
            DaemonDiscovery::Absent => {}
        }

        lifecycle.launch_verified(&options, &guard).await?;
        let descriptor = lifecycle
            .wait_until_ready(options.connect(), &guard)
            .await?;
        connect_descriptor(lifecycle, options.connect(), &descriptor).await
    }
}

async fn connect_descriptor(
    lifecycle: &impl DaemonLifecycle,
    options: &DaemonConnectOptions,
    descriptor: &DaemonDescriptor,
) -> SdkResult<Colossus> {
    descriptor.validate_for(options)?;
    lifecycle
        .connect_verified(options, descriptor)
        .await
        .map(Colossus::from_shared_backend)
}

pub(crate) fn validate_loopback_endpoint(endpoint: &Url) -> SdkResult<()> {
    if endpoint.scheme() != "https"
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || endpoint.port().is_none()
        || !matches!(endpoint.path(), "" | "/")
    {
        return Err(SdkError::InvalidConfiguration(
            "daemon endpoint must be a bare HTTPS loopback URL with an explicit port",
        ));
    }

    match endpoint.host() {
        Some(Host::Ipv4(address)) if address == std::net::Ipv4Addr::LOCALHOST => Ok(()),
        Some(Host::Ipv6(address)) if address == std::net::Ipv6Addr::LOCALHOST => Ok(()),
        _ => Err(SdkError::InvalidConfiguration(
            "daemon endpoint must use an IP-literal loopback host",
        )),
    }
}
