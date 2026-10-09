//! A late main-thread show cannot outlive workspace/overlay authority revocation.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use colossus_native_browser::{BrowserError, BrowserView};
use tauri::Window;

use super::dto::BrowserRect;

pub(super) async fn apply(
    window: &Window,
    view: BrowserView,
    lease: Arc<AtomicU64>,
    epoch: u64,
    bounds: Option<BrowserRect>,
) -> Result<(), BrowserError> {
    let (sender, receive) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let result = if !sender.is_closed() && lease.load(Ordering::Acquire) == epoch {
                if let Some(rect) = bounds {
                    view.set_bounds(tauri::Rect {
                        position: tauri::LogicalPosition::new(rect.x, rect.y).into(),
                        size: tauri::LogicalSize::new(rect.width, rect.height).into(),
                    })
                    .and_then(|()| {
                        // Resizing can invoke native callbacks. Recheck the
                        // current presentation owner immediately before show.
                        if !sender.is_closed() && lease.load(Ordering::Acquire) == epoch {
                            view.show()
                        } else {
                            view.hide()?;
                            Err(BrowserError::Closed)
                        }
                    })
                } else {
                    view.hide()
                }
            } else {
                Err(BrowserError::Closed)
            };
            let _ = sender.send(result);
        })
        .map_err(|_| BrowserError::Closed)?;
    tokio::time::timeout(std::time::Duration::from_secs(2), receive)
        .await
        .map_err(|_| BrowserError::TimedOut)?
        .map_err(|_| BrowserError::Closed)?
}
