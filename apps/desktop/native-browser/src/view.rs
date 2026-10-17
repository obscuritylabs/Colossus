//! Safe presentation handle shared by the existing guest and the Chromium preview.

use tauri::{Rect, Webview};
use url::Url;

use crate::{BrowserError, NavigationAction, PageState};

/// One actual native guest. Chromium never navigates a second `WebKit` page.
#[derive(Clone)]
#[allow(clippy::large_enum_variant)] // Unlinked preview surface is an inert zero-sized stub.
pub enum BrowserView {
    /// Existing human browsing adapter retained until native Chromium acceptance.
    System(Webview),
    /// Explicitly enabled embedded Chromium preview.
    #[cfg(feature = "cef-preview")]
    Chromium(crate::chromium::Surface),
    /// Trusted attachment to a separately contained Chromium host.
    #[cfg(all(
        feature = "cef-preview",
        colossus_cef_linked,
        any(windows, target_os = "macos")
    ))]
    Remote(crate::remote::RemoteSurface),
}

impl BrowserView {
    /// Exact canonical conversation of an independently admitted browser context.
    #[must_use]
    pub fn conversation_id(&self) -> Option<&str> {
        #[cfg(all(
            feature = "cef-preview",
            colossus_cef_linked,
            any(windows, target_os = "macos")
        ))]
        if let Self::Remote(view) = self {
            return Some(view.conversation_id());
        }
        None
    }

    /// Exact independently contained runtime session, when admitted.
    #[must_use]
    pub fn contained_session_id(&self) -> Option<&str> {
        #[cfg(all(
            feature = "cef-preview",
            colossus_cef_linked,
            any(windows, target_os = "macos")
        ))]
        if let Self::Remote(view) = self {
            return Some(view.session_id());
        }
        None
    }

    /// Transfer an independently admitted human page to a registered run.
    ///
    /// # Errors
    /// UI preview pages and unverified runtime contexts cannot grant automation.
    #[cfg_attr(
        not(all(
            feature = "cef-preview",
            colossus_cef_linked,
            any(windows, target_os = "macos")
        )),
        allow(clippy::unused_async)
    )]
    pub async fn handoff(&self, run_id: &str) -> Result<(), BrowserError> {
        #[cfg(all(
            feature = "cef-preview",
            colossus_cef_linked,
            any(windows, target_os = "macos")
        ))]
        if let Self::Remote(view) = self {
            return view.handoff(run_id).await;
        }
        let _ = run_id;
        Err(BrowserError::Unavailable)
    }

    /// Whether the native attachment admits human navigation and keyboard input.
    #[must_use]
    pub fn human_control_available(&self) -> bool {
        #[cfg(all(
            feature = "cef-preview",
            colossus_cef_linked,
            any(windows, target_os = "macos")
        ))]
        if let Self::Remote(view) = self {
            return view.human_control_available();
        }
        true
    }
    /// Cancel native identity review before controller ownership changes.
    pub fn revoke_client_identity(&self) {
        #[cfg(feature = "cef-preview")]
        if let Self::Chromium(view) = self {
            view.revoke_client_identity();
        }
    }

    /// Navigate through the native guest's closed human interface.
    ///
    /// # Errors
    /// Fails when the guest has closed or navigation is invalid.
    pub fn navigate(&self, url: Url) -> Result<(), BrowserError> {
        match self {
            Self::System(view) => view.navigate(url).map_err(|_| BrowserError::Closed),
            #[cfg(feature = "cef-preview")]
            Self::Chromium(view) => view.navigate(&url),
            #[cfg(all(
                feature = "cef-preview",
                colossus_cef_linked,
                any(windows, target_os = "macos")
            ))]
            Self::Remote(view) => view.navigate(&url),
        }
    }

    /// Hide the real native page and prevent guest input.
    ///
    /// # Errors
    /// Returns an error when the guest no longer exists.
    pub fn hide(&self) -> Result<(), BrowserError> {
        match self {
            Self::System(view) => view.hide().map_err(|_| BrowserError::Closed),
            #[cfg(feature = "cef-preview")]
            Self::Chromium(view) => view.hide(),
            #[cfg(all(
                feature = "cef-preview",
                colossus_cef_linked,
                any(windows, target_os = "macos")
            ))]
            Self::Remote(view) => view.hide(),
        }
    }

    /// Display the native page after the controller has validated its viewport.
    ///
    /// # Errors
    /// Returns an error when the guest no longer exists.
    pub fn show(&self) -> Result<(), BrowserError> {
        match self {
            Self::System(view) => view.show().map_err(|_| BrowserError::Closed),
            #[cfg(feature = "cef-preview")]
            Self::Chromium(view) => view.show(),
            #[cfg(all(
                feature = "cef-preview",
                colossus_cef_linked,
                any(windows, target_os = "macos")
            ))]
            Self::Remote(view) => view.show(),
        }
    }

    /// Apply validated logical viewport bounds on the native UI thread.
    ///
    /// # Errors
    /// Returns an error when the guest no longer exists or its bounds are invalid.
    pub fn set_bounds(&self, bounds: Rect) -> Result<(), BrowserError> {
        match self {
            Self::System(view) => view.set_bounds(bounds).map_err(|_| BrowserError::Closed),
            #[cfg(feature = "cef-preview")]
            Self::Chromium(view) => view.set_bounds(bounds),
            #[cfg(all(
                feature = "cef-preview",
                colossus_cef_linked,
                any(windows, target_os = "macos")
            ))]
            Self::Remote(view) => view.set_bounds(bounds),
        }
    }

    /// Stop and destroy the native guest before releasing its profile.
    ///
    /// # Errors
    /// Returns an error when native teardown cannot complete.
    pub async fn close(&self) -> Result<(), BrowserError> {
        match self {
            Self::System(view) => {
                let _ = crate::release(view).await;
                view.close().map_err(|_| BrowserError::Closed)
            }
            #[cfg(feature = "cef-preview")]
            Self::Chromium(view) => view.close().await,
            #[cfg(all(
                feature = "cef-preview",
                colossus_cef_linked,
                any(windows, target_os = "macos")
            ))]
            Self::Remote(view) => view.close().await,
        }
    }

    /// Read native navigation metadata, never renderer-authored authority.
    ///
    /// # Errors
    /// Fails if the guest is unavailable or its observation deadline expires.
    pub async fn inspect(&self) -> Result<PageState, BrowserError> {
        match self {
            Self::System(view) => crate::inspect(view).await,
            #[cfg(feature = "cef-preview")]
            Self::Chromium(view) => view.inspect().await,
            #[cfg(all(
                feature = "cef-preview",
                colossus_cef_linked,
                any(windows, target_os = "macos")
            ))]
            Self::Remote(view) => view.inspect().await,
        }
    }

    /// Apply a human navigation control on this same guest.
    ///
    /// # Errors
    /// Fails if the guest is unavailable or its operation deadline expires.
    pub async fn control(&self, action: NavigationAction) -> Result<(), BrowserError> {
        match self {
            Self::System(view) => crate::control(view, action).await,
            #[cfg(feature = "cef-preview")]
            Self::Chromium(view) => view.control(action).await,
            #[cfg(all(
                feature = "cef-preview",
                colossus_cef_linked,
                any(windows, target_os = "macos")
            ))]
            Self::Remote(view) => view.control(action).await,
        }
    }

    /// Existing guest handle for sharing only the legacy engine's native context.
    #[must_use]
    pub fn system_view(&self) -> Option<&Webview> {
        match self {
            Self::System(view) => Some(view),
            #[cfg(feature = "cef-preview")]
            Self::Chromium(_) => None,
            #[cfg(all(
                feature = "cef-preview",
                colossus_cef_linked,
                any(windows, target_os = "macos")
            ))]
            Self::Remote(_) => None,
        }
    }
}
