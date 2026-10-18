//! Real Windows HWND rendering and native input for the supervised Core fixture.
use super::{BrowserRect, PAGE_TIMEOUT, ViewportHeartbeat, present, title};
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
    let controller = app
        .get_webview("main")
        .context("Desktop controller missing")?;
    controller.eval("const overlay=document.createElement('div');overlay.id='owned-windows-overlay';overlay.style.cssText='position:fixed;inset:0;z-index:2147483647;background:var(--surface-primary,#fff);color:var(--text-primary,#111)';overlay.textContent='Colossus Desktop native overlay acceptance';document.body.appendChild(overlay);")?;
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
        .map_err(|error| anyhow::anyhow!(error.message))?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let hidden = view.acceptance_windows_screen().await?;
    ensure!(
        hidden.foreground
            && !hidden.visible
            && !hidden.hit_test
            && hidden.center_bgra != [255, 241, 231, 255],
        "native browser pixels remained above the real Desktop overlay"
    );
    save(&hidden, home, "windows-overlay-foreground.png")?;
    println!("PASS actual Desktop overlay occludes the native browser HWND");
    controller.eval("document.getElementById('owned-windows-overlay')?.remove();")?;
    let _ = screen(app, tab, view, rect, [255, 241, 231, 255]).await?;
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
    tokio::time::sleep(Duration::from_millis(2400)).await;
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
