use super::{Owner, RemoteHostOwner};
use crate::{BrowserError, presentation::PresentationSurface};
use std::sync::{Arc, atomic::Ordering};

pub(super) struct Pending(pub Option<Arc<dyn RemoteHostOwner>>);
impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(native) = self.0.take() {
            tauri::async_runtime::spawn(async move {
                let _ = native.close().await;
            });
        }
    }
}
pub(super) fn hide(owner: &Arc<Owner>) -> Result<(), BrowserError> {
    owner.revision.fetch_add(1, Ordering::AcqRel);
    let (surface, lease) = {
        let mut state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
        state.visible = false;
        state.applied = None;
        (state.surface.clone(), state.lease)
    };
    let result = surface.as_ref().map_or(Ok(()), |surface| surface.hide());
    let owner = Arc::clone(owner);
    tauri::async_runtime::spawn(async move {
        let _operation = owner.operation.lock().await;
        if owner.state.lock().is_ok_and(|state| state.lease == lease)
            && hide_host(&owner, lease).await.is_err()
        {
            owner.client.disconnect();
        }
    });
    result
}
pub(super) async fn close(owner: &Arc<Owner>) -> Result<(), BrowserError> {
    owner.closing.store(true, Ordering::Release);
    hide(owner)?;
    let _operation = owner.operation.lock().await;
    let (surface, lease) = {
        let state = owner.state.lock().map_err(|_| BrowserError::Closed)?;
        (state.surface.clone(), state.lease)
    };
    if let Some(surface) = surface {
        surface.hide_confirmed().await?;
    }
    let hidden = hide_host(owner, lease).await;
    owner.client.disconnect();
    owner.native.close().await?;
    if owner.read_only.load(Ordering::Acquire) {
        hidden?;
    }
    owner
        .state
        .lock()
        .map_err(|_| BrowserError::Closed)?
        .surface
        .take();
    Ok(())
}
pub(super) async fn hide_host(
    owner: &Owner,
    lease: colossus_browser_presentation::Lease,
) -> Result<(), BrowserError> {
    match owner.client.hide(lease).await {
        Ok(())
        | Err(
            colossus_browser_presentation::PresentationError::Stale
            | colossus_browser_presentation::PresentationError::Hidden,
        ) => Ok(()),
        Err(_) => Err(BrowserError::Closed),
    }
}
pub(super) async fn hide_surface(
    surface: Option<&Arc<PresentationSurface>>,
) -> Result<(), BrowserError> {
    if let Some(surface) = surface {
        surface.hide_confirmed().await?;
    }
    Ok(())
}
