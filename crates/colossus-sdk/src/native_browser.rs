//! Native Desktop browser admission. No renderer credentials, executable selectors or receipts.
use crate::{NativeSidecarLifecycle, SdkError, SdkResult, SidecarOptions};
use colossus_browser_presentation::native_admission::{
    NativeBrowserEnrollment, NativeBrowserOpen, NativeBrowserReply, NativeBrowserRequest,
    client_handshake,
};
use colossus_browser_presentation::{Configure, PresentationChannel, PresentationClient};
use colossus_contracts::{BrowserControlLease, BrowserSessionId};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use zeroize::Zeroizing;

mod owner;
mod platform;
mod types;
pub use colossus_browser_presentation::native_admission::NativeBrowserOpen as NativeBrowserOpenRequest;
pub use owner::NativeBrowserSessionOwner;
pub use types::{NativeBrowserAttachment, NativeBrowserHandoff};

/// Separate native-only client, bound to one attested managed workspace and lifecycle.
#[derive(Clone)]
pub struct NativeBrowserClient {
    endpoint: String,
    authentication: Arc<Zeroizing<[u8; 32]>>,
    enrollment: NativeBrowserEnrollment,
    child: Arc<AtomicU32>,
}
impl std::fmt::Debug for NativeBrowserClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeBrowserClient")
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}
impl NativeSidecarLifecycle {
    /// Obtain a separately authenticated native client, never the ordinary worker/TUI key.
    /// Connections remain unavailable until this lifecycle's exact child is activated.
    pub fn native_browser_client(
        &self,
        options: &SidecarOptions,
    ) -> SdkResult<Option<NativeBrowserClient>> {
        let Some(browser) = &self.bootstrap.native_browser else {
            return Ok(None);
        };
        let workspace = self.bootstrap.expected_workspace_identity.as_ref().ok_or(
            SdkError::InvalidConfiguration("native browser requires an attested workspace"),
        )?;
        let generation =
            uuid::Uuid::parse_str(&browser.generation).map_err(|_| SdkError::IdentityMismatch)?;
        let workspace_digest = hex::decode(&workspace.sha256)
            .map_err(|_| SdkError::IdentityMismatch)?
            .try_into()
            .map_err(|_| SdkError::IdentityMismatch)?;
        let enrollment = NativeBrowserEnrollment {
            generation: *generation.as_bytes(),
            instance: *uuid::Uuid::parse_str(&options.instance_id().to_string())
                .map_err(|_| SdkError::IdentityMismatch)?
                .as_bytes(),
            application_id: self.bootstrap.grant.wire().application_id,
            workspace_version: workspace.version,
            workspace_digest,
            parent_process_id: browser.parent_process_id,
        };
        enrollment
            .validate()
            .map_err(|_| SdkError::IdentityMismatch)?;
        let endpoint = platform::endpoint(options.instance_dir().as_path(), &enrollment)?;
        Ok(Some(NativeBrowserClient {
            endpoint,
            enrollment,
            authentication: Arc::new(
                colossus_sidecar_protocol::decode_worker_authentication(&browser.authentication)
                    .map_err(|_| SdkError::IdentityMismatch)?,
            ),
            child: Arc::clone(&self.bootstrap.native_browser_child),
        }))
    }
}
impl NativeBrowserClient {
    async fn connect(
        &self,
    ) -> SdkResult<
        colossus_browser_presentation::native_admission::AdmissionChannel<platform::Stream>,
    > {
        if self.enrollment.parent_process_id != std::process::id() {
            return Err(SdkError::IdentityMismatch);
        }
        let child = self.child.load(Ordering::Acquire);
        if child == 0 {
            return Err(SdkError::SidecarFailed);
        }
        let stream = platform::connect(&self.endpoint, child).await?;
        client_handshake(
            stream,
            &self.authentication,
            self.enrollment
                .digest()
                .map_err(|_| SdkError::IdentityMismatch)?,
        )
        .await
        .map_err(|_| SdkError::SidecarFailed)
    }
    /// Query accepted Embedded availability without allocating or promoting a browser mode.
    pub async fn available(&self) -> SdkResult<bool> {
        match self
            .connect()
            .await?
            .request(&NativeBrowserRequest::Probe)
            .await
            .map_err(|_| SdkError::SidecarFailed)?
        {
            NativeBrowserReply::Available => Ok(true),
            NativeBrowserReply::Unavailable => Ok(false),
            _ => Err(SdkError::SidecarFailed),
        }
    }
    /// Admit a human page through the exact managed application and canonical conversation.
    /// A lost acknowledgment never replays allocation; the worker retains its cleanup intent.
    pub async fn open_human(
        &self,
        request: NativeBrowserOpen,
    ) -> SdkResult<NativeBrowserAttachment> {
        request
            .validate()
            .map_err(|_| SdkError::InvalidConfiguration("native browser intent is invalid"))?;
        let mut channel = self.connect().await?;
        let reply = channel
            .request(&NativeBrowserRequest::Open(request.clone()))
            .await
            .map_err(|_| SdkError::SidecarFailed)?;
        let NativeBrowserReply::Opened {
            conversation_id,
            configure,
            enrollment_digest,
        } = reply
        else {
            return Err(SdkError::SidecarFailed);
        };
        let owner = Arc::new(NativeBrowserSessionOwner::new(
            self.clone(),
            configure.session.clone(),
        ));
        configure
            .validate()
            .map_err(|_| SdkError::IdentityMismatch)?;
        if configure.control_generation != 0
            || conversation_id.is_empty()
            || conversation_id.len() > 256
            || conversation_id.chars().any(char::is_control)
            || request
                .conversation_id
                .as_ref()
                .is_some_and(|id| id != &conversation_id)
            || enrollment_digest == [0; 32]
            || configure.width != request.width
            || configure.height != request.height
            || configure.scale_milli != request.scale_milli
            || configure.viewport_generation != request.viewport_generation
            || configure.lease_ms != request.lease_ms
        {
            return Err(SdkError::IdentityMismatch);
        }
        let (stream, key) = channel.into_presentation();
        let (reader, writer) = tokio::io::split(stream);
        let presentation = PresentationClient::connect(
            PresentationChannel::new(reader, writer),
            key,
            enrollment_digest,
        )
        .await
        .map_err(|_| SdkError::SidecarFailed)?;
        Ok(NativeBrowserAttachment {
            conversation_id,
            presentation,
            configure,
            enrollment_digest,
            owner,
        })
    }
    pub(super) async fn close_session(&self, session: &BrowserSessionId) -> SdkResult<()> {
        match self
            .connect()
            .await?
            .request(&NativeBrowserRequest::Close {
                session: session.clone(),
            })
            .await
            .map_err(|_| SdkError::SidecarFailed)?
        {
            NativeBrowserReply::Closed => Ok(()),
            _ => Err(SdkError::SidecarFailed),
        }
    }
    pub(super) async fn detach_session(&self, session: &BrowserSessionId) -> SdkResult<()> {
        match self
            .connect()
            .await?
            .request(&NativeBrowserRequest::Detach {
                session: session.clone(),
            })
            .await
            .map_err(|_| SdkError::SidecarFailed)?
        {
            NativeBrowserReply::Detached => Ok(()),
            _ => Err(SdkError::SidecarFailed),
        }
    }
    /// Request gateway-authorized transfer to an existing registered run.
    /// Runtime obtains its own native fence; this API accepts no caller receipt or binding.
    pub async fn handoff(
        &self,
        session: &BrowserSessionId,
        run_id: &str,
        lease_ms: u32,
    ) -> SdkResult<NativeBrowserHandoff> {
        let request = NativeBrowserRequest::Handoff {
            session: session.clone(),
            run_id: run_id.to_owned(),
            lease_ms,
        };
        request
            .validate()
            .map_err(|_| SdkError::InvalidConfiguration("native handoff intent is invalid"))?;
        match self
            .connect()
            .await?
            .request(&request)
            .await
            .map_err(|_| SdkError::SidecarFailed)?
        {
            NativeBrowserReply::Granted {
                lease,
                configure,
                enrollment_digest,
            } if lease.session_id == *session
                && lease.run_id == run_id
                && configure.session == *session
                && configure.control_generation == lease.control_generation
                && configure.control_generation > 0
                && enrollment_digest != [0; 32] =>
            {
                configure
                    .validate()
                    .map_err(|_| SdkError::IdentityMismatch)?;
                Ok(NativeBrowserHandoff {
                    lease,
                    configure,
                    enrollment_digest,
                })
            }
            _ => Err(SdkError::SidecarFailed),
        }
    }
}
