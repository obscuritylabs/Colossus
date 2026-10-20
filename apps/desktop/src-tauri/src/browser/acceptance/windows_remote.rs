//! Real Windows HWND rendering and native input for the supervised Core fixture.
use super::{
    BrowserRect, ExpectedFrame, PAGE_TIMEOUT, RemoteAcceptanceProbe, ViewportHeartbeat,
    accepted_frame, present, title,
};
use anyhow::{Context as _, ensure};
use colossus_native_browser::{presentation::WindowsScreenEvidence, remote::RemoteSurface};
use std::{fs::OpenOptions, path::Path, time::Duration};
use tauri::Manager as _;

async fn screen(
    app: &tauri::AppHandle,
    tab: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    expected: [u8; 4],
) -> anyhow::Result<WindowsScreenEvidence> {
    let deadline = tokio::time::Instant::now() + PAGE_TIMEOUT;
    let factor = app
        .get_window("main")
        .context("Desktop window missing")?
        .scale_factor()?;
    let expected_size = tauri::LogicalSize::new(rect.width, rect.height).to_physical::<u32>(factor);
    let mut last = String::from("no native screen receipt");
    loop {
        present(app, tab, rect).await?;
        if let Ok(screen) = view.acceptance_windows_screen().await {
            last = format!(
                "foreground={} visible={} hit_test={} {}x{} dpi={} center_bgra={:?}",
                screen.foreground,
                screen.visible,
                screen.hit_test,
                screen.width,
                screen.height,
                screen.dpi,
                screen.center_bgra
            );
            if screen.foreground
                && screen.visible
                && screen.hit_test
                && screen.center_bgra == expected
                && screen.width == expected_size.width
                && screen.height == expected_size.height
            {
                return Ok(screen);
            }
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "the actual foreground Windows desktop never showed the current browser pixels: {last}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn save(screen: &WindowsScreenEvidence, home: &Path, name: &str) -> anyhow::Result<()> {
    let mut rgba = screen.pixels.clone();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let image = image::RgbaImage::from_raw(screen.width, screen.height, rgba)
        .context("native desktop capture dimensions disagreed")?;
    let path = home.join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    image::DynamicImage::ImageRgba8(image).write_to(&mut file, image::ImageFormat::Png)?;
    file.sync_all()?;
    println!(
        "EVIDENCE foreground Windows viewport PNG {}",
        path.display()
    );
    Ok(())
}

fn cursor_edge(screen: &WindowsScreenEvidence) -> anyhow::Result<usize> {
    let dpi = u64::from(screen.dpi);
    let x = ((40 * dpi + 48) / 96)
        .checked_sub(1)
        .context("invalid native DPI")?;
    let y = (165 * dpi + 48) / 96;
    ensure!(
        x < u64::from(screen.width) && y < u64::from(screen.height),
        "cursor probe lies outside the actual native viewport"
    );
    Ok(usize::try_from((y * u64::from(screen.width) + x) * 4)?)
}

async fn background_visibility(
    app: &tauri::AppHandle,
    tab: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
) -> anyhow::Result<()> {
    let other = tauri::WindowBuilder::new(app, "browser-focus-fixture")
        .title("Browser focus acceptance")
        .inner_size(160.0, 100.0)
        .build()?;
    other.set_focus()?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let result = async {
        let controller = app
            .get_webview("main")
            .context("Desktop controller missing")?;
        ensure!(
            !colossus_native_browser::is_active(&controller).await?,
            "fixture must transfer foreground focus"
        );
        present(app, tab, rect).await?;
        ensure!(
            view.acceptance_probe().await?.visible,
            "switching apps hid the browser pixels"
        );
        ensure!(
            !colossus_native_browser::is_active(&controller).await?,
            "background presentation stole foreground focus"
        );
        ensure!(
            view.acceptance_windows_input().await.is_err(),
            "background browser accepted human input"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    other.close()?;
    super::focus(app).await?;
    result?;
    let _ = screen(app, tab, view, rect, [255, 241, 231, 255]).await?;
    println!("PASS background browser retains visible pixels without accepting human input");
    Ok(())
}

pub(super) async fn human(
    app: &tauri::AppHandle,
    tab: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    home: &Path,
    heartbeat: &ViewportHeartbeat,
) -> anyhow::Result<()> {
    // Windows resize and overlay assertions own their viewport pulses. Settle
    // the ordinary heartbeat so it cannot restore old geometry or show an overlay.
    heartbeat.pause().await?;
    let initial = screen(app, tab, view, rect, [255, 241, 231, 255]).await?;
    save(&initial, home, "windows-human-foreground.png")?;
    let smaller = BrowserRect {
        width: rect.width - 120.0,
        height: rect.height - 80.0,
        ..rect
    };
    let resized = screen(app, tab, view, smaller, [255, 241, 231, 255]).await?;
    let factor = f64::from(resized.dpi) / 96.0;
    let expected_size =
        tauri::LogicalSize::new(smaller.width, smaller.height).to_physical::<u32>(factor);
    ensure!(
        resized.width == expected_size.width
            && resized.height == expected_size.height
            && resized.width < initial.width
            && resized.height < initial.height,
        "native HWND resizing/DPI conversion disagreed with the actual client rectangle"
    );
    println!(
        "PASS real HWND resize and current display DPI: {}x{} -> {}x{} dpi={}",
        initial.width, initial.height, resized.width, resized.height, resized.dpi
    );
    let _ = screen(app, tab, view, rect, [255, 241, 231, 255]).await?;
    let previous = view.acceptance_probe().await?;
    let controller = app
        .get_webview("main")
        .context("Desktop controller missing")?;
    // UI dispatch acknowledges this pulse before its asynchronous remote resize
    // settles. The overlay can revoke that in-flight presentation immediately.
    present(app, tab, smaller).await?;
    controller.eval("(()=>{const overlay=document.createElement('dialog');overlay.id='owned-windows-overlay';overlay.dataset.browserOccluded='';overlay.style.cssText='position:fixed;inset:0;margin:0;width:100vw;height:100vh;max-width:none;max-height:none;background:var(--surface-primary,#fff);color:var(--text-primary,#111)';overlay.textContent='Colossus Desktop native overlay acceptance';document.body.appendChild(overlay);overlay.showModal();})()")?;
    hide_viewport(app, tab).await?;
    require_modal_overlay(&controller).await?;
    let hidden = poll_hidden_overlay(app, tab, view, &previous).await?;
    save(&hidden, home, "windows-overlay-foreground.png")?;
    println!("PASS actual Desktop overlay occludes the native browser HWND");
    controller.eval("(()=>{const overlay=document.getElementById('owned-windows-overlay');overlay?.close();overlay?.remove();})()")?;
    restore_overlay(app, tab, view, rect, smaller, &previous).await?;
    heartbeat.resume(tab, rect).await?;
    background_visibility(app, tab, view, rect).await?;
    view.acceptance_windows_keyboard_reentrancy()
        .await
        .context("Tao parent Return key reentrancy did not acknowledge completion")?;
    // A returning native call alone is insufficient: require a fresh UI dispatch
    // and visible browser pixels after the sent keyup re-entered Tao's keyboard handler.
    let _ = screen(app, tab, view, rect, [255, 241, 231, 255]).await?;
    println!("PASS Tao parent Return key handles nested cross-thread keyup and remains responsive");
    view.acceptance_windows_input().await?;
    title(view, "Human input Ab3Ω😀").await?;
    println!(
        "PASS native HWND mouse/key dispatch, Unicode surrogate IME commit and IME cancel reached real Chromium"
    );
    Ok(())
}

async fn require_modal_overlay(controller: &tauri::Webview) -> anyhow::Result<()> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let send = std::sync::Mutex::new(Some(send));
    controller.eval_with_callback(
        "document.getElementById('owned-windows-overlay')?.matches('dialog:modal') === true",
        move |result| {
            if let Some(send) = send.lock().expect("overlay callback").take() {
                let _ = send.send(result);
            }
        },
    )?;
    let value = tokio::time::timeout(Duration::from_secs(2), receive)
        .await
        .context("native overlay modal query timed out")??;
    ensure!(
        serde_json::from_str::<bool>(&value)?,
        "the real Desktop HTML dialog did not enter its modal top layer"
    );
    Ok(())
}

async fn poll_hidden_overlay(
    app: &tauri::AppHandle,
    tab: &str,
    view: &RemoteSurface,
    previous: &RemoteAcceptanceProbe,
) -> anyhow::Result<WindowsScreenEvidence> {
    let started = tokio::time::Instant::now();
    // The renderer repeats hidden viewport pulses independently of its context
    // polling. Neither path may show or renew the expired visibility lease.
    let pulses = async {
        for index in 0..8 {
            tokio::time::sleep_until(started + Duration::from_millis(index * 400)).await;
            hide_viewport(app, tab).await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    let contexts = async {
        for index in 0..4 {
            tokio::time::sleep_until(started + Duration::from_millis(index * 750)).await;
            let caller = app
                .get_webview("main")
                .context("Desktop controller missing")?;
            let snapshot = crate::browser::commands::browser_context(
                app.clone(),
                caller,
                app.state::<crate::state::AppState>(),
                None,
                Some(view.conversation_id().into()),
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.message))?;
            ensure!(
                snapshot.selected_tab_id.as_deref() == Some(tab)
                    && snapshot.tabs.iter().any(|candidate| {
                        candidate.id == tab
                            && candidate.session_id == view.session_id()
                            && candidate.control == "human"
                            && candidate.error.is_none()
                            && candidate.page.url == previous.page.url
                            && candidate.page.title == previous.page.title
                    }),
                "hidden controller polling lost or failed the exact human browser session"
            );
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::try_join!(pulses, contexts)?;
    tokio::time::sleep_until(started + Duration::from_secs(3)).await;
    let hidden = view.acceptance_windows_screen().await?;
    ensure!(
        hidden.foreground
            && !hidden.visible
            && !hidden.hit_test
            && hidden.center_bgra != [255, 241, 231, 255],
        "native browser pixels remained above the real Desktop overlay"
    );
    println!(
        "PASS hidden human browser survived controller context polling beyond the native visibility TTL"
    );
    Ok(hidden)
}

async fn hide_viewport(app: &tauri::AppHandle, tab: &str) -> anyhow::Result<()> {
    let state = app.state::<crate::state::AppState>();
    let generation = state
        .browser
        .lock()
        .map_err(|error| anyhow::anyhow!(error.message))?
        .generation;
    state
        .browser
        .viewport(
            app,
            &super::BrowserViewportRequest {
                generation,
                tab_id: Some(tab.into()),
                rect: None,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.message))
}

async fn restore_overlay(
    app: &tauri::AppHandle,
    tab: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    smaller: BrowserRect,
    previous: &RemoteAcceptanceProbe,
) -> anyhow::Result<()> {
    // Do not serialize each remote apply before the next trusted visibility
    // change. A fresh pulse must recover without reviving an obsolete lease.
    present(app, tab, smaller).await?;
    hide_viewport(app, tab).await?;
    let prior = previous
        .frame
        .context("human overlay baseline frame missing")?;
    let restored = accepted_frame(
        app,
        tab,
        view,
        rect,
        ExpectedFrame {
            title: &previous.page.title,
            url: &previous.page.url,
            center_bgra: [255, 241, 231, 255],
            core_target: Some(&previous.page.target),
            previous: None,
            after: Some(prior),
        },
    )
    .await?;
    ensure!(
        view.human_control_available()
            && restored.frame.is_some_and(|frame| {
                frame.lease.viewport_generation > prior.lease.viewport_generation
                    && frame.lease.control_generation == 0
            }),
        "human overlay restoration did not acquire a fresh presentation lease"
    );
    let _ = screen(app, tab, view, rect, [255, 241, 231, 255]).await?;
    println!(
        "PASS live human browser restored the same authenticated page and HWND pixels after overlapping overlay viewport updates"
    );
    Ok(())
}

pub(super) async fn agent(
    app: &tauri::AppHandle,
    tab: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    home: &Path,
) -> anyhow::Result<()> {
    let screen = screen(app, tab, view, rect, [216, 247, 217, 255]).await?;
    ensure!(
        view.acceptance_windows_input().await.is_err(),
        "agent viewer accepted human input"
    );
    save(&screen, home, "windows-agent-foreground.png")?;
    println!(
        "PASS actual foreground Windows desktop shows the Core-controlled browser: {}x{} dpi={} center_bgra={:?}",
        screen.width, screen.height, screen.dpi, screen.center_bgra
    );
    Ok(())
}

pub(super) async fn agent_click_indicator(
    app: &tauri::AppHandle,
    tab: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    home: &Path,
) -> anyhow::Result<()> {
    println!("EVIDENCE native agent click title acknowledged; checking foreground cursor pixels");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let mut candidate_saved = false;
    let marked = loop {
        present(app, tab, rect).await?;
        let screen = match view.acceptance_windows_screen().await {
            Ok(screen) => screen,
            Err(error) => {
                if tokio::time::Instant::now() >= deadline {
                    println!("EVIDENCE cursor foreground capture failure: {error:?}");
                    anyhow::bail!("Core click agent cursor capture failed: {error:?}");
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
                continue;
            }
        };
        // The synthetic button begins at DIP (40,160). Its outer agent outline
        // lies outside the page's own border, so this cannot pass on HTML focus CSS.
        let index = cursor_edge(&screen)?;
        if !candidate_saved {
            save(&screen, home, "windows-agent-cursor-candidate.png")?;
            candidate_saved = true;
        }
        if screen.foreground
            && screen.visible
            && screen.hit_test
            && screen.pixels.get(index..index + 4) == Some(&[0, 0, 0, 255])
        {
            break screen;
        }
        if tokio::time::Instant::now() >= deadline {
            println!(
                "EVIDENCE cursor foreground failure: foreground={} visible={} hit_test={} {}x{} sample={:?}",
                screen.foreground,
                screen.visible,
                screen.hit_test,
                screen.width,
                screen.height,
                screen.pixels.get(index..index + 4)
            );
            anyhow::bail!("Core click agent outline never reached actual Windows desktop pixels");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    save(&marked, home, "windows-agent-cursor-foreground.png")?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let retained = view.acceptance_windows_screen().await?;
    let index = cursor_edge(&retained)?;
    ensure!(
        retained.pixels.get(index..index + 4) == Some(&[0, 0, 0, 255]),
        "agent cursor disappeared between actions before its eight-second deadline"
    );
    tokio::time::sleep(Duration::from_millis(5400)).await;
    let cleared = view
        .acceptance_windows_screen()
        .await
        .context("stage=agent-cursor-expiry-capture")?;
    let index = cursor_edge(&cleared)?;
    ensure!(
        cleared.pixels.get(index..index + 4) == Some(&[255, 241, 231, 255]),
        "expired agent cursor remained painted on the native viewer"
    );
    save(&cleared, home, "windows-agent-cursor-expired.png")?;
    println!(
        "PASS Core click keeps agent cursor/target outline through a subsequent snapshot in actual foreground Windows pixels and removes it after expiry"
    );
    Ok(())
}

pub(super) async fn agent_ready(
    app: &tauri::AppHandle,
    tab: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
) -> anyhow::Result<()> {
    let _ = screen(app, tab, view, rect, [255, 241, 231, 255]).await?;
    println!("PASS replacement agent viewer is foreground before Core input begins");
    Ok(())
}

pub(super) async fn selected_tab(
    app: &tauri::AppHandle,
    tab: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    color: [u8; 4],
) -> anyhow::Result<()> {
    let frame = screen(app, tab, view, rect, color).await?;
    println!(
        "PASS Core-selected tab reached the actual foreground Windows HWND: {}x{} dpi={} center_bgra={:?}",
        frame.width, frame.height, frame.dpi, frame.center_bgra,
    );
    Ok(())
}
