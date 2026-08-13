use crate::{
    AppPrivateInstanceDir, Backend, BackendKind, Colossus, InstanceId, ManagedExecutionBoundary,
    SdkError, SdkResult,
};
use async_trait::async_trait;
use std::sync::Arc;

/// Isolated in-process runtime policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmbeddedOptions {
    instance_id: InstanceId,
    instance_dir: AppPrivateInstanceDir,
    application_id: String,
    execution_boundary: ManagedExecutionBoundary,
}

impl EmbeddedOptions {
    /// Create policy for an explicit application-private embedded instance.
    pub fn new(
        instance_id: InstanceId,
        instance_dir: AppPrivateInstanceDir,
        application_id: impl Into<String>,
    ) -> SdkResult<Self> {
        instance_id.validate()?;
        let application_id = application_id.into();
        let application_token = application_id.strip_prefix("app:");
        if application_id.len() > 128
            || application_token.is_none_or(str::is_empty)
            || !application_id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
            })
        {
            return Err(SdkError::InvalidConfiguration(
                "application ID must use the canonical public identifier grammar",
            ));
        }
        Ok(Self {
            instance_id,
            instance_dir,
            application_id,
            execution_boundary: ManagedExecutionBoundary::default(),
        })
    }

    /// Select the host execution boundary used by the embedded runtime.
    #[must_use]
    pub const fn with_execution_boundary(
        mut self,
        execution_boundary: ManagedExecutionBoundary,
    ) -> Self {
        self.execution_boundary = execution_boundary;
        self
    }

    /// Isolated instance identity.
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    /// Application-private canonical state directory.
    pub const fn instance_dir(&self) -> &AppPrivateInstanceDir {
        &self.instance_dir
    }

    /// Stable actor identity used for embedded application audit evidence.
    pub fn application_id(&self) -> &str {
        &self.application_id
    }

    /// Host execution boundary selected for this embedded runtime.
    pub const fn execution_boundary(&self) -> ManagedExecutionBoundary {
        self.execution_boundary
    }
}

/// Trusted runtime composition for an embedded Colossus instance.
///
/// Implementations must reject the default shared instance, validate private directory
/// ownership, acquire the ordinary exclusive writer lease before returning, and retain
/// the complete policy, permit, approval, sandbox, quarantine, and audit path. They must
/// apply [`EmbeddedOptions::execution_boundary`] exactly: full access selects the
/// acknowledged direct boundary, workspace isolation selects the platform-isolating
/// workspace profile, and offline isolation additionally withholds general model network
/// tools while retaining only configured provider/authentication transport. Failure must
/// never fall back to opening another instance, boundary, or transport.
#[async_trait]
pub trait EmbeddedLifecycle: Send + Sync {
    /// Open one isolated runtime and start its in-process coordinator.
    async fn open_isolated(&self, options: &EmbeddedOptions) -> SdkResult<Arc<dyn Backend>>;
}

impl Colossus {
    /// Open an isolated runtime directly in the application process.
    pub async fn open_embedded(
        lifecycle: &impl EmbeddedLifecycle,
        options: EmbeddedOptions,
    ) -> SdkResult<Self> {
        let backend = lifecycle.open_isolated(&options).await?;
        if backend.kind() != BackendKind::Embedded {
            let _ = backend.close().await;
            return Err(SdkError::IdentityMismatch);
        }
        Ok(Self::from_shared_backend(backend))
    }
}
