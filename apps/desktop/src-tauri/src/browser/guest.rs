//! Native guest construction. Neither branch exposes app capability to a page.

use std::path::PathBuf;

#[cfg(not(feature = "embedded-chromium-preview"))]
use colossus_native_browser::BrowserEvent;
use colossus_native_browser::{self as engine, BrowserView, EventSink, NavigationPolicy};
use tauri::{AppHandle, Manager as _, Webview};
#[cfg(not(feature = "embedded-chromium-preview"))]
use tauri::{
    WebviewUrl,
    webview::{DownloadEvent, NewWindowResponse, WebviewBuilder},
};

use super::manager::error;
use crate::dto::CommandErrorDto;

pub(super) async fn create(
    app: &AppHandle,
    id: &str,
    generation: u64,
    directory: PathBuf,
    source: Option<Webview>,
    policy: NavigationPolicy,
    sink: EventSink,
) -> Result<BrowserView, CommandErrorDto> {
    let window = app
        .get_window("main")
        .ok_or_else(|| error("The Desktop window has closed."))?;
    #[cfg(feature = "embedded-chromium-preview")]
    {
        let _ = source;
        return engine::chromium::Surface::create(&window, id, generation, directory, policy, sink)
            .await
            .map(BrowserView::Chromium)
            .map_err(|failure| error(&failure.to_string()));
    }
    #[cfg(not(feature = "embedded-chromium-preview"))]
    {
        let _ = generation;
        let popup_sink = sink.clone();
        let download_sink = sink.clone();
        let navigation = policy.clone();
        let mut builder = WebviewBuilder::new(
            id,
            WebviewUrl::External(
                "about:blank"
                    .parse()
                    .map_err(|_| error("Browser initialization failed."))?,
            ),
        )
        .incognito(true)
        .data_directory(directory)
        .focused(false)
        .devtools(false)
        .on_navigation(move |url| navigation.allows(url.as_str()))
        .on_new_window(move |url, _| {
            popup_sink(BrowserEvent::Popup(url.to_string()));
            NewWindowResponse::Deny
        })
        .on_download(move |_, event| {
            if matches!(event, DownloadEvent::Requested { .. }) {
                download_sink(BrowserEvent::Download);
            }
            false
        });
        if let Some(source) = source {
            builder = engine::share_session(builder, &source)
                .await
                .map_err(|failure| error(&failure.to_string()))?;
        }
        let view = window
            .add_child(
                builder,
                tauri::LogicalPosition::new(-10_000.0, -10_000.0),
                tauri::LogicalSize::new(16.0, 16.0),
            )
            .map_err(|_| error("The browser engine could not start."))?;
        let _ = view.hide();
        if let Err(failure) = engine::harden(&view, policy, sink).await {
            let _ = engine::release(&view).await;
            let _ = view.close();
            return Err(error(&failure.to_string()));
        }
        Ok(BrowserView::System(view))
    }
}
