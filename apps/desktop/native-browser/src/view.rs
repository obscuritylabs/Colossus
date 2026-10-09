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
}

impl BrowserView {
    /// Navigate through the native guest's closed human interface.
    ///
    /// # Errors
    /// Fails when the guest has closed or navigation is invalid.
    pub fn navigate(&self, url: Url) -> Result<(), BrowserError> {
        match self {
            Self::System(view) => view.navigate(url).map_err(|_| BrowserError::Closed),
            #[cfg(feature = "cef-preview")]
            Self::Chromium(view) => view.navigate(&url),
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
        }
    }

    /// Existing guest handle for sharing only the legacy engine's native context.
    #[must_use]
    pub fn system_view(&self) -> Option<&Webview> {
        match self {
            Self::System(view) => Some(view),
            #[cfg(feature = "cef-preview")]
            Self::Chromium(_) => None,
        }
    }
}
