//! Real Windows HWND rendering and native input for the supervised Core fixture.
use super::{BrowserRect, PAGE_TIMEOUT, present, title};
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

pub(super) async fn human(
    app: &tauri::AppHandle,
    tab: &str,
    view: &RemoteSurface,
    rect: BrowserRect,
    home: &Path,
) -> anyhow::Result<()> {
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
    view.acceptance_windows_input().await?;
    title(app, tab, view, rect, "Human input Ab3Ω😀").await?;
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
