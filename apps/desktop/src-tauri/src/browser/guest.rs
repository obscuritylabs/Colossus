//! Native guest construction. Neither branch exposes app capability to a page.

use std::path::PathBuf;

#[cfg(any(
    windows,
    not(any(
        feature = "embedded-chromium-preview",
        any(
            feature = "macos-contained-browser-diagnostic",
            any(
                feature = "windows-browser-development",
                feature = "windows-browser-functional-preview"
            )
        )
    ))
))]
use colossus_native_browser::BrowserEvent;
use colossus_native_browser::{self as engine, BrowserView, EventSink, NavigationPolicy};
use tauri::{AppHandle, Manager as _, Webview};
#[cfg(any(
    windows,
    not(any(
        feature = "embedded-chromium-preview",
        any(
            feature = "macos-contained-browser-diagnostic",
            any(
                feature = "windows-browser-development",
                feature = "windows-browser-functional-preview"
            )
        )
    ))
))]
use tauri::{
    WebviewUrl,
    webview::{DownloadEvent, NewWindowResponse, WebviewBuilder},
};

use super::manager::error;
use crate::dto::CommandErrorDto;

pub(super) struct GuestRequest {
    pub id: String,
    pub generation: u64,
    pub scope: String,
    pub conversation_id: Option<String>,
    pub initial_url: Option<url::Url>,
    pub directory: PathBuf,
    pub source: Option<Webview>,
    pub human: super::profile::HumanBrowserPreferences,
    pub policy: NavigationPolicy,
    pub sink: EventSink,
}

// Compile-time engine branches share one guest entry point and authority check.
#[allow(clippy::too_many_lines)]
pub(super) async fn create(
    app: &AppHandle,
    request: GuestRequest,
) -> Result<BrowserView, CommandErrorDto> {
    let GuestRequest {
        id,
        generation,
        scope,
        conversation_id,
        initial_url,
        directory,
        source,
        human,
        policy,
        sink,
    } = request;
    if initial_url.is_none() {
        return Ok(BrowserView::Empty);
    }
    #[cfg(any(
        feature = "embedded-chromium-preview",
        feature = "macos-contained-browser-diagnostic",
        feature = "windows-browser-development",
        feature = "windows-browser-functional-preview"
    ))]
    let window = app
        .get_window("main")
        .ok_or_else(|| error("The Desktop window has closed."))?;
    #[cfg(windows)]
    if human.system_mode() {
        return create_system(app, id, directory, source, policy, sink, human).await;
    }
    #[cfg(not(windows))]
    let _ = &human;
    #[cfg(any(
        feature = "macos-contained-browser-diagnostic",
        any(
            feature = "windows-browser-development",
            feature = "windows-browser-functional-preview"
        )
    ))]
    {
        let _ = (directory, source);
        let composition = app
            .try_state::<engine::contained::ContainedBrowserComposition>()
            .ok_or_else(|| error("The supervised browser is unavailable."))?;
        let request = engine::contained::ContainedGuestRequest {
            id,
            generation,
            scope,
            conversation_id,
            initial_url,
            policy,
            sink,
        };
        if !composition
            .0
            .available(&window, &request)
            .await
            .map_err(|failure| error(&failure.to_string()))?
        {
            return Err(error("The supervised browser is unavailable."));
        }
        return composition
            .0
            .create(&window, request)
            .await
            .map_err(|failure| error(&failure.to_string()));
    }
    #[cfg(feature = "embedded-chromium-preview")]
    {
        if let Some(composition) = app.try_state::<engine::contained::ContainedBrowserComposition>()
        {
            let request = engine::contained::ContainedGuestRequest {
                id: id.clone(),
                generation,
                scope,
                conversation_id,
                initial_url: initial_url.clone(),
                policy: policy.clone(),
                sink: sink.clone(),
            };
            if composition
                .0
                .available(&window, &request)
                .await
                .map_err(|failure| error(&failure.to_string()))?
            {
                return composition
                    .0
                    .create(&window, request)
                    .await
                    .map_err(|failure| error(&failure.to_string()));
            }
        }
        let _ = source;
        return engine::chromium::Surface::create(
            &window, &id, generation, directory, policy, sink,
        )
        .await
        .map(BrowserView::Chromium)
        .map_err(|failure| error(&failure.to_string()));
    }
    #[cfg(not(any(
        feature = "embedded-chromium-preview",
        any(
            feature = "macos-contained-browser-diagnostic",
            any(
                feature = "windows-browser-development",
                feature = "windows-browser-functional-preview"
            )
        )
    )))]
    {
        let _ = (generation, scope, initial_url, conversation_id);
        create_system(app, id, directory, source, policy, sink, human).await
    }
}

#[cfg(any(
    windows,
    not(any(
        feature = "embedded-chromium-preview",
        feature = "macos-contained-browser-diagnostic",
        feature = "windows-browser-development",
        feature = "windows-browser-functional-preview"
    ))
))]
#[allow(clippy::too_many_arguments)]
pub(super) async fn create_system(
    app: &AppHandle,
    id: String,
    directory: PathBuf,
    source: Option<Webview>,
    policy: NavigationPolicy,
    sink: EventSink,
    human: super::profile::HumanBrowserPreferences,
) -> Result<BrowserView, CommandErrorDto> {
    let window = app
        .get_window("main")
        .ok_or_else(|| error("The Desktop window has closed."))?;
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
    .incognito(!human.remember_sign_ins)
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
    if !human.user_agent.is_empty() {
        builder = builder.user_agent(&human.user_agent);
    }
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
    #[cfg(windows)]
    let hardened = engine::harden_human_profile(
        &view,
        policy,
        sink,
        human.remember_sign_ins,
        human.user_agent,
    )
    .await;
    #[cfg(not(windows))]
    let hardened = engine::harden(&view, policy, sink).await;
    if let Err(failure) = hardened {
        let _ = engine::release(&view).await;
        let _ = view.close();
        return Err(error(&failure.to_string()));
    }
    Ok(BrowserView::System(view))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_tab_has_no_navigation_or_handoff_authority() {
        let view = BrowserView::Empty;
        assert!(view.conversation_id().is_none());
        assert!(view.contained_session_id().is_none());
        assert!(view.system_view().is_none());
        let page = view.inspect().await.unwrap();
        assert!(page.url.is_empty() && !page.loading);
        assert_eq!(
            view.navigate("https://example.com".parse().unwrap()),
            Err(engine::BrowserError::Unavailable)
        );
        assert_eq!(
            view.control(engine::NavigationAction::Reload).await,
            Err(engine::BrowserError::Unavailable)
        );
        assert_eq!(
            view.handoff("run").await,
            Err(engine::BrowserError::Unavailable)
        );
        view.close().await.unwrap();
    }
}
