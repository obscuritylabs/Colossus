//! Windows native frame acknowledgements; foreground pixels are checked separately.
use super::{
    BrowserError, BrowserRect, ExpectedFrame, PAGE_TIMEOUT, RemoteAcceptanceProbe, RemoteSurface,
    present, same_document_frame_after,
};
use anyhow::{Context as _, ensure};
use std::time::Duration;
use tauri::Manager as _;

pub(super) async fn focus(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let window = app.get_window("main").context("Desktop window missing")?;
    let controller = app
        .get_webview("main")
        .context("Desktop controller missing")?;
    window.show()?;
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    loop {
        if colossus_native_browser::is_active(&controller).await? {
            return Ok(());
        }
        window.set_focus()?;
        ensure!(
            tokio::time::Instant::now() < deadline,
            "owned browser acceptance requires a foreground Desktop window"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

pub(super) async fn accepted_frame(
    app: &tauri::AppHandle,
    id: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    expected: ExpectedFrame<'_>,
) -> anyhow::Result<RemoteAcceptanceProbe> {
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    loop {
        present(app, id, rect).await?;
        let (probe, observation) = match view.acceptance_probe().await {
            Ok(probe) => (Some(probe), "ready"),
            Err(BrowserError::Stale) => (None, "stale"),
            Err(BrowserError::Unavailable) => (None, "unavailable"),
            Err(error) => return Err(error.into()),
        };
        if let Some(probe) = probe.as_ref() {
            let target_matches = expected
                .core_target
                .is_none_or(|target| probe.page.target == *target);
            if let Some(frame) = probe.frame {
                let new_document = expected.previous.is_none_or(|previous| {
                    previous.frame.is_some_and(|prior| {
                        probe.page.target.document_id != previous.page.target.document_id
                            && frame.lease.document_generation > prior.lease.document_generation
                            && frame.lease.viewport_generation > prior.lease.viewport_generation
                            && frame.lease.control_generation > prior.lease.control_generation
                    })
                });
                if probe.visible
                    && probe.width > 0
                    && probe.height > 0
                    && !probe.page.loading
                    && probe.page.title == expected.title
                    && probe.page.url == expected.url
                    && target_matches
                    && new_document
                    && expected
                        .after
                        .is_none_or(|prior| same_document_frame_after(frame, prior))
                    && frame.sequence > 0
                    && frame.count > 0
                    && frame.center_bgra == expected.center_bgra
                {
                    return Ok(probe.clone());
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let controller = app
                .get_webview("main")
                .context("Desktop controller missing")?;
            let active = colossus_native_browser::is_active(&controller).await?;
            eprintln!(
                "EVIDENCE Windows frame timeout observation={observation} controller_active={active} visible={} loading={} target_match={} frame_present={} color_match={}",
                probe.as_ref().is_some_and(|probe| probe.visible),
                probe.as_ref().is_some_and(|probe| probe.page.loading),
                probe
                    .as_ref()
                    .is_some_and(|probe| probe.page.title == expected.title
                        && probe.page.url == expected.url),
                probe.as_ref().is_some_and(|probe| probe.frame.is_some()),
                probe
                    .as_ref()
                    .and_then(|probe| probe.frame)
                    .is_some_and(|frame| frame.center_bgra == expected.center_bgra),
            );
            anyhow::bail!(
                "native HWND did not accept an authenticated frame for the current document and fixture color"
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
