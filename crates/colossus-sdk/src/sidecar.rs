use crate::{
    ApiMajor, AppPrivateInstanceDir, Backend, BackendKind, Colossus, InstanceId, SdkError,
    SdkResult, VerifiedExecutable,
};
use async_trait::async_trait;
use std::sync::Arc;

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
