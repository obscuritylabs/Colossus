use tauri::Webview;

use crate::{BrowserError, EventSink, NavigationAction, NavigationPolicy, PageState};

#[cfg(target_os = "macos")]
use crate::macos as platform;
#[cfg(windows)]
use crate::windows as platform;

/// Configure a blank, hidden guest before its first external navigation.
///
/// # Errors
/// Fails closed if mandatory platform security controls cannot be installed.
pub async fn harden(
    view: &Webview,
    policy: NavigationPolicy,
    sink: EventSink,
) -> Result<(), BrowserError> {
    dispatch(view, move |native| platform::harden(&native, policy, sink)).await?;
    #[cfg(windows)]
    platform::restrict_file_picker(view).await?;
    Ok(())
}

/// Harden an explicitly selected human `WebView2` profile. This grants no agent
/// handoff and never changes the default temporary Chromium session policy.
///
/// # Errors
/// Fails closed unless the actual native privacy mode and User-Agent match.
#[cfg(windows)]
pub async fn harden_human_profile(
    view: &Webview,
    policy: NavigationPolicy,
    sink: EventSink,
    persistent: bool,
    user_agent: String,
) -> Result<(), BrowserError> {
    dispatch(view, move |native| {
        platform::harden_human_profile(&native, policy, sink, persistent, &user_agent)
    })
    .await?;
    platform::restrict_file_picker(view).await
}

/// Clear the selected human profile through `WebView2`'s native profile API.
///
/// # Errors
/// A missing or failed completion is reported; it is never treated as a clear.
#[cfg(windows)]
pub async fn clear_site_data(view: &Webview) -> Result<(), BrowserError> {
    platform::clear_site_data(view).await
}

/// Apply one closed navigation operation on the `WebView`'s owning thread.
///
/// # Errors
/// Returns an error when the guest has closed or its engine is unavailable.
pub async fn control(view: &Webview, action: NavigationAction) -> Result<(), BrowserError> {
    dispatch(view, move |native| platform::control(&native, action)).await
}

/// Read bounded navigation metadata from the engine, never page-authored IPC.
///
/// # Errors
/// Returns an error if the guest no longer exists or fails to respond.
pub async fn inspect(view: &Webview) -> Result<PageState, BrowserError> {
    dispatch(view, |native| platform::inspect(&native)).await
}

/// Whether the guest's containing OS window is active, including child focus.
///
/// # Errors
/// Returns an error when the native window has closed or cannot be queried.
pub async fn is_active(view: &Webview) -> Result<bool, BrowserError> {
    dispatch(view, |native| platform::is_active(&native)).await
}

/// Detach native delegates immediately before destroying a guest.
///
/// # Errors
/// Returns an error if the owning engine has already terminated.
pub async fn release(view: &Webview) -> Result<(), BrowserError> {
    #[cfg(target_os = "macos")]
    return dispatch(view, |native| platform::release(&native)).await;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = view;
        Ok(())
    }
}

/// Reuse an existing guest's temporary session, never the app's native profile.
///
/// # Errors
/// Fails when the original guest has closed or native configuration is unavailable.
pub async fn share_session(
    builder: tauri::webview::WebviewBuilder<tauri::Wry>,
    source: &Webview,
) -> Result<tauri::webview::WebviewBuilder<tauri::Wry>, BrowserError> {
    dispatch(source, move |native| {
        #[cfg(windows)]
        return Ok(builder.with_environment(native.environment()));
        #[cfg(target_os = "macos")]
        return platform::share_session(builder, &native);
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (builder, native);
            Err(BrowserError::Unavailable)
        }
    })
    .await
}

/// Hand a validated HTTP(S) address to the OS without a command shell.
///
/// # Errors
/// Rejects invalid addresses and reports an unavailable OS handler.
pub async fn open_external(view: &Webview, address: &str) -> Result<(), BrowserError> {
    let url = crate::parse_address(address)?;
    dispatch(view, move |_| platform::open_external(url.as_str())).await
}

async fn dispatch<T: Send + 'static>(
    view: &Webview,
    action: impl FnOnce(tauri::webview::PlatformWebview) -> Result<T, BrowserError> + Send + 'static,
) -> Result<T, BrowserError> {
    let (send, receive) = tokio::sync::oneshot::channel();
    view.with_webview(move |native| {
        let _ = send.send(action(native));
    })
    .map_err(|_| BrowserError::Closed)?;
    tokio::time::timeout(std::time::Duration::from_secs(5), receive)
        .await
        .map_err(|_| BrowserError::TimedOut)?
        .map_err(|_| BrowserError::Closed)?
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::{BrowserError, EventSink, NavigationAction, NavigationPolicy, PageState};
    pub fn harden(
        _: &tauri::webview::PlatformWebview,
        _: NavigationPolicy,
        _: EventSink,
    ) -> Result<(), BrowserError> {
        Err(BrowserError::Unavailable)
    }
    pub fn control(
        _: &tauri::webview::PlatformWebview,
        _: NavigationAction,
    ) -> Result<(), BrowserError> {
        Err(BrowserError::Unavailable)
    }
    pub fn inspect(_: &tauri::webview::PlatformWebview) -> Result<PageState, BrowserError> {
        Err(BrowserError::Unavailable)
    }
    pub fn open_external(_: &str) -> Result<(), BrowserError> {
        Err(BrowserError::Unavailable)
    }
    pub fn is_active(_: &tauri::webview::PlatformWebview) -> Result<bool, BrowserError> {
        Err(BrowserError::Unavailable)
    }
}
