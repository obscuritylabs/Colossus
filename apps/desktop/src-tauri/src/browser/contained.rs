//! Native-only composition of the attested managed sidecar and the OS pixel view.
use async_trait::async_trait;
use colossus_contracts::BrowserUrl;
use colossus_native_browser::{
    self as engine, BrowserError, BrowserView,
    contained::{ContainedGuestFactory, ContainedGuestRequest, RemoteHostOwner},
};
use colossus_sdk::{NativeBrowserClient, NativeBrowserOpenRequest, NativeBrowserSessionOwner};
use std::sync::Arc;
use tauri::{AppHandle, Manager as _, Window};

pub(crate) fn install(app: &AppHandle) -> Result<(), BrowserError> {
    engine::contained::ContainedBrowserComposition::install(app, Arc::new(ManagedFactory))
}

struct ManagedFactory;
async fn client(window: &Window, request: &ContainedGuestRequest) -> Option<NativeBrowserClient> {
    window
        .state::<crate::state::AppState>()
        .managed_browser_for(&request.scope)
        .await
}

#[async_trait]
impl ContainedGuestFactory for ManagedFactory {
    async fn available(
        &self,
        window: &Window,
        request: &ContainedGuestRequest,
    ) -> Result<bool, BrowserError> {
        if !engine::contained::native_presentation_available() {
            return Ok(false);
        }
        if request.initial_url.is_none() {
            return Ok(false);
        }
        let Some(client) = client(window, request).await else {
            return Ok(false);
        };
        client
            .available()
            .await
            .map_err(|_| BrowserError::Unavailable)
    }

    async fn create(
        &self,
        window: &Window,
        request: ContainedGuestRequest,
    ) -> Result<BrowserView, BrowserError> {
        let client = client(window, &request)
            .await
            .ok_or(BrowserError::Unavailable)?;
        let url = request
            .initial_url
            .as_ref()
            .ok_or(BrowserError::Unavailable)?;
        request.policy.authorize(url)?;
        let url = BrowserUrl::parse(url.as_str()).map_err(|_| BrowserError::Unavailable)?;
        let attachment = client
            .open_human(NativeBrowserOpenRequest {
                conversation_id: request.conversation_id,
                url,
                width: 800,
                height: 600,
                scale_milli: 1000,
                viewport_generation: 1,
                lease_ms: 1500,
            })
            .await
            .map_err(|_| BrowserError::Unavailable)?;
        let owner = Arc::new(ManagedOwner {
            client,
            conversation_id: attachment.conversation_id().to_owned(),
            session_id: attachment.owner().session_id().to_string(),
            native: attachment.owner(),
            digest: attachment.enrollment_digest(),
        });
        engine::contained::attach(
            window,
            attachment.presentation(),
            attachment.configure(),
            attachment.enrollment_digest(),
            owner,
            request.sink,
        )
        .await
    }
}

struct ManagedOwner {
    client: NativeBrowserClient,
    conversation_id: String,
    session_id: String,
    native: Arc<NativeBrowserSessionOwner>,
    digest: [u8; 32],
}
#[async_trait]
impl RemoteHostOwner for ManagedOwner {
    fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
    fn session_id(&self) -> &str {
        &self.session_id
    }
    async fn close(&self) -> Result<(), BrowserError> {
        self.native.detach().await.map_err(|_| BrowserError::Closed)
    }
    async fn handoff(
        &self,
        run_id: &str,
    ) -> Result<colossus_native_browser::contained::NativeConfigure, BrowserError> {
        let granted = self
            .client
            .handoff(self.native.session_id(), run_id, 60_000)
            .await
            .map_err(|_| BrowserError::Unavailable)?;
        if granted.enrollment_digest() != self.digest {
            return Err(BrowserError::Unavailable);
        }
        Ok(granted.configure())
    }
}
