use crate::{
    ApiMajor, Backend, Colossus, DaemonConnectOptions, DaemonDescriptor, DaemonDiscovery,
    DaemonLaunchGuard, DaemonLaunchOptions, DaemonLifecycle, GrpcBackend, InstanceId, SdkError,
    SdkResult, TlsFingerprint,
};
use async_trait::async_trait;
use colossus_grpc::{EndpointDescriptorError, read_endpoint_certificate, read_endpoint_descriptor};
use std::{num::NonZeroU32, sync::Arc};
use sysinfo::{Pid, System};
use url::Url;

/// First-party native installed-daemon discovery and authenticated connection.
///
/// The connector reuses Colossus' owner/link/mode-checked descriptor storage, verifies
/// the advertised process is live, requires its advertised pin and the separately stored
/// certificate to match an independently provisioned expected fingerprint, and then loads
/// bearer credentials only from the configured [`crate::CredentialProvider`]. It never
/// derives trust from descriptor content or falls back to environment variables, argv,
/// unprotected files, another endpoint, or automatic downloads.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeDaemonLifecycle;

#[async_trait]
impl DaemonLifecycle for NativeDaemonLifecycle {
    async fn discover(&self, options: &DaemonConnectOptions) -> SdkResult<DaemonDiscovery> {
        let descriptor = match read_endpoint_descriptor(options.descriptor_path()) {
            Ok(descriptor) => descriptor,
            Err(EndpointDescriptorError::Storage(error))
                if error.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(DaemonDiscovery::Absent);
            }
            Err(error) => return Err(map_discovery_error(error)),
        };
        if !process_is_live(descriptor.pid()) {
            return Err(SdkError::Unavailable);
        }
        let descriptor = DaemonDescriptor::new(
            InstanceId::from_uuid(descriptor.instance_id()),
            NonZeroU32::new(descriptor.pid()).ok_or(SdkError::IdentityMismatch)?,
            Url::parse(descriptor.endpoint()).map_err(|_| SdkError::IdentityMismatch)?,
            ApiMajor::new(1)?,
            TlsFingerprint::from_hex(descriptor.certificate_sha256())
                .map_err(|_| SdkError::IdentityMismatch)?,
        )
        .map_err(|_| SdkError::IdentityMismatch)?;
        descriptor.validate_for(options)?;
        Ok(DaemonDiscovery::Present(descriptor))
    }

    async fn connect_verified(
        &self,
        options: &DaemonConnectOptions,
        descriptor: &DaemonDescriptor,
    ) -> SdkResult<Arc<dyn Backend>> {
        descriptor.validate_for(options)?;
        let certificate_path = options
            .certificate_path()
            .ok_or(SdkError::InvalidConfiguration(
                "native daemon connection requires a public-certificate path",
            ))?;
        let certificate =
            read_endpoint_certificate(certificate_path).map_err(map_certificate_error)?;
        let backend = GrpcBackend::connect_daemon(options, descriptor, certificate).await?;
        Ok(Arc::new(backend))
    }

    async fn acquire_launch_guard(
        &self,
        _options: &DaemonLaunchOptions,
    ) -> SdkResult<DaemonLaunchGuard> {
        Err(SdkError::LaunchFailed)
    }

    async fn launch_verified(
        &self,
        _options: &DaemonLaunchOptions,
        _guard: &DaemonLaunchGuard,
    ) -> SdkResult<()> {
        Err(SdkError::LaunchFailed)
    }

    async fn wait_until_ready(
        &self,
        _options: &DaemonConnectOptions,
        _guard: &DaemonLaunchGuard,
    ) -> SdkResult<DaemonDescriptor> {
        Err(SdkError::LaunchFailed)
    }
}

impl Colossus {
    /// Securely discover and connect to an already installed running daemon.
    pub async fn connect_installed(options: DaemonConnectOptions) -> SdkResult<Self> {
        Self::connect(&NativeDaemonLifecycle, options).await
    }
}

fn process_is_live(pid: u32) -> bool {
    let mut system = System::new();
    system.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
    );
    system.process(Pid::from_u32(pid)).is_some()
}

fn map_discovery_error(error: EndpointDescriptorError) -> SdkError {
    match error {
        EndpointDescriptorError::UnsupportedPlatform => SdkError::InvalidConfiguration(
            "native secure daemon discovery is unavailable on this platform",
        ),
        EndpointDescriptorError::InvalidPath => {
            SdkError::InvalidConfiguration("daemon descriptor path is invalid")
        }
        EndpointDescriptorError::InvalidDescriptor(_)
        | EndpointDescriptorError::InvalidEndpoint
        | EndpointDescriptorError::InvalidEncoding
        | EndpointDescriptorError::InvalidCertificatePem
        | EndpointDescriptorError::Storage(_) => SdkError::IdentityMismatch,
    }
}

fn map_certificate_error(error: EndpointDescriptorError) -> SdkError {
    match error {
        EndpointDescriptorError::UnsupportedPlatform => SdkError::InvalidConfiguration(
            "native secure certificate storage is unavailable on this platform",
        ),
        EndpointDescriptorError::InvalidPath => {
            SdkError::InvalidConfiguration("daemon certificate path is invalid")
        }
        EndpointDescriptorError::InvalidDescriptor(_)
        | EndpointDescriptorError::InvalidEndpoint
        | EndpointDescriptorError::InvalidEncoding
        | EndpointDescriptorError::InvalidCertificatePem
        | EndpointDescriptorError::Storage(_) => SdkError::IdentityMismatch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CredentialProvider, Secret};
    use std::path::PathBuf;

    struct NeverCredential;

    #[async_trait]
    impl CredentialProvider for NeverCredential {
        async fn load(&self) -> SdkResult<Secret> {
            panic!("path validation must happen before credential loading")
        }
    }

    #[test]
    fn certificate_path_is_explicit_distinct_and_redacted_credential_remains_hidden() {
        let descriptor = std::env::temp_dir().join("colossus-native-endpoint.json");
        let options = DaemonConnectOptions::new(
            InstanceId::from_uuid(uuid::Uuid::now_v7()),
            &descriptor,
            TlsFingerprint::from_bytes([9; 32]),
            ApiMajor::new(1).expect("major"),
            Arc::new(NeverCredential),
        )
        .expect("options");
        assert!(options.certificate_path().is_none());
        assert!(
            options
                .with_certificate_path(PathBuf::from(&descriptor))
                .is_err()
        );
    }
}
