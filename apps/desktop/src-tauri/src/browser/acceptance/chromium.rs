//! Real CEF child views inside the Desktop Tauri/AppKit application.

use std::time::Duration;

use colossus_native_browser::{BrowserView, chromium::Surface};
use tauri::Manager as _;

use crate::{
    browser::dto::{BrowserAction, BrowserRect, BrowserViewportRequest},
    state::AppState,
};

use super::action;

pub(super) async fn exercise(app: &tauri::AppHandle, address: &str) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    state
        .select_target(Some("chromium-acceptance-a".into()))
        .await;
    let first = action(
        app,
        BrowserAction::New {
            url: format!("{address}/first"),
        },
    )
    .await?;
    anyhow::ensure!(
        first.engine.kind == "embedded_chromium" && first.engine.ready,
        "CEF preview is unavailable"
    );
    anyhow::ensure!(
        !first.engine.agent_control_available,
        "production browser automation became available"
    );
    let a = first
        .selected_tab_id
        .ok_or_else(|| anyhow::anyhow!("first tab missing"))?;
    let first_view = surface(app, &a, "chromium-acceptance-a")?;
    wait_title(&first_view, "First page").await?;

    let controller = app
        .get_webview("main")
        .ok_or_else(|| anyhow::anyhow!("Desktop controller missing"))?;
    match first_view.acceptance_probe().await {
        Ok(probe) => println!("EVIDENCE startup CEF probe {probe:?}"),
        Err(error) => eprintln!("EVIDENCE startup CEF probe unavailable: {error}"),
    }
    println!("CHECKPOINT waiting-for-native-foreground");
    focus_controller(&controller, &first_view).await?;
    let rect = bounds(app)?;
    viewport(app, Some(&a), Some(rect)).await?;
    checkpoint(app, "presented-native-child", Some(&a), Some(rect)).await?;
    pixels(app, &first_view, true, Some(rect)).await?;
    println!("PASS CEF page rendering inside Desktop Tauri/AppKit with sandbox preserved");
    checkpoint(app, "rendered-first-page", Some(&a), Some(rect)).await?;

    history(app, address, &a, &first_view).await?;
    tabs(app, address, &a, &first_view).await?;
    resizing(app, &a, &first_view).await?;
    overlays(app, &a, &first_view).await?;
    workspace_boundary(app, address, &a, first.generation).await?;

    state
        .browser
        .close_all_settled()
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    anyhow::ensure!(
        state
            .browser
            .lock()
            .map_err(|error| anyhow::anyhow!(error.message))?
            .tabs
            .is_empty(),
        "native tabs survived close acknowledgements"
    );
    anyhow::ensure!(
        app.webviews().len() == 1,
        "CEF created an unmanaged Tauri WebView"
    );
    println!("PASS CEF tab close acknowledgements and temporary profile teardown");
    retain_live_for_quit(app, address).await
}

async fn retain_live_for_quit(app: &tauri::AppHandle, address: &str) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let final_snapshot = action(
        app,
        BrowserAction::New {
            url: format!("{address}/first"),
        },
    )
    .await?;
    let final_id = final_snapshot
        .selected_tab_id
        .ok_or_else(|| anyhow::anyhow!("application-quit regression tab missing"))?;
    let final_view = surface(app, &final_id, "chromium-acceptance-b")?;
    wait_title(&final_view, "First page").await?;
    let rect = bounds(app)?;
    viewport(app, Some(&final_id), Some(rect)).await?;
    pixels(app, &final_view, true, Some(rect)).await?;
    anyhow::ensure!(
        state
            .browser
            .lock()
            .map_err(|error| anyhow::anyhow!(error.message))?
            .tabs
            .len()
            == 1,
        "application quit must begin with one live native browser tab"
    );
    println!("PASS live CEF tab retained for AppKit quit");
    Ok(())
}

pub(super) async fn terminate(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let scope = state
        .browser
        .lock()
        .map_err(|error| anyhow::anyhow!(error.message))?
        .scope
        .clone()
        .ok_or_else(|| anyhow::anyhow!("application quit scope missing"))?;
    let view = surface(app, &selected(app)?, &scope)?;
    view.acceptance_terminate_application().await?;
    println!("PASS AppKit terminate requested with a live CEF tab");
    Ok(())
}

async fn focus_controller(controller: &tauri::Webview, view: &Surface) -> anyhow::Result<()> {
    let window = controller.window();
    window.show()?;
    let wait_ms = std::env::var("COLOSSUS_BROWSER_ACCEPTANCE_FOREGROUND_WAIT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(15_000)
        .min(60_000);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
    let mut attempt = 0;
    while tokio::time::Instant::now() < deadline {
        if colossus_native_browser::is_active(controller).await? {
            break;
        }
        if attempt % 20 == 0 {
            window.set_focus()?;
            view.acceptance_activate_parent().await?;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        attempt += 1;
    }
    if !colossus_native_browser::is_active(controller).await? {
        if let Ok(probe) = view.acceptance_probe().await {
            println!("EVIDENCE foreground timeout after {wait_ms}ms native={probe:?}");
        }
        anyhow::bail!("native Chromium acceptance requires a foreground Desktop window");
    }
    Ok(())
}

fn surface(app: &tauri::AppHandle, id: &str, scope: &str) -> anyhow::Result<Surface> {
    let (view, _) = app
        .state::<AppState>()
        .browser
        .tab(id, scope)
        .map_err(|error| anyhow::anyhow!(error.message))?;
    match view {
        BrowserView::Chromium(surface) => Ok(surface),
        BrowserView::System(_) => anyhow::bail!("CEF preview fell back to a platform WebView"),
    }
}

async fn wait_title(view: &Surface, title: &str) -> anyhow::Result<()> {
    for _ in 0..100 {
        let page = view.inspect().await?;
        if page.title == title && !page.loading {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("expected loaded CEF title {title:?}")
}

fn bounds(app: &tauri::AppHandle) -> anyhow::Result<BrowserRect> {
    let window = app
        .get_window("main")
        .ok_or_else(|| anyhow::anyhow!("Desktop window missing"))?;
    let size = window
        .inner_size()?
        .to_logical::<f64>(window.scale_factor()?);
    anyhow::ensure!(
        size.width >= 300.0 && size.height >= 300.0,
        "Desktop acceptance window too small"
    );
    Ok(BrowserRect {
        x: 24.0,
        y: 96.0,
        width: size.width - 48.0,
        height: size.height - 120.0,
    })
}

async fn viewport(
    app: &tauri::AppHandle,
    id: Option<&str>,
    rect: Option<BrowserRect>,
) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let generation = state
        .browser
        .lock()
        .map_err(|error| anyhow::anyhow!(error.message))?
        .generation;
    state
        .browser
        .viewport(
            app,
            &BrowserViewportRequest {
                generation,
                tab_id: id.map(str::to_owned),
                rect,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.message))
}

async fn pixels(
    app: &tauri::AppHandle,
    view: &Surface,
    visible: bool,
    rect: Option<BrowserRect>,
) -> anyhow::Result<()> {
    let started = tokio::time::Instant::now();
    let deadline = started + Duration::from_secs(5);
    let mut first_probe = true;
    loop {
        let probe = probe_with_heartbeat(app, view, visible, rect).await?;
        if first_probe {
            println!(
                "EVIDENCE first CEF probe expected_visible={visible} expected_bounds={rect:?} elapsed_ms={} actual={probe:?}",
                started.elapsed().as_millis()
            );
            first_probe = false;
        }
        anyhow::ensure!(
            probe.cef_application && probe.tauri_event_loop && probe.parent_attached,
            "CEF and Tauri do not share the supported AppKit application/parent: {probe:?}"
        );
        let correct_bounds = rect.is_none_or(|rect| {
            (probe.x - rect.x).abs() <= 1.0
                && (probe.y - rect.y).abs() <= 1.0
                && (probe.width - rect.width).abs() <= 1.0
                && (probe.height - rect.height).abs() <= 1.0
        });
        if probe.visible == visible
            && correct_bounds
            && (!visible || (probe.magenta_pixels > 32 && probe.green_pixels > 32))
        {
            println!(
                "EVIDENCE CEF visible={} bounds={},{},{},{} magenta_pixels={} green_pixels={} cef_application={} tauri_event_loop={} parent_attached={}",
                probe.visible,
                probe.x,
                probe.y,
                probe.width,
                probe.height,
                probe.magenta_pixels,
                probe.green_pixels,
                probe.cef_application,
                probe.tauri_event_loop,
                probe.parent_attached
            );
            return Ok(());
        }
        anyhow::ensure!(
            tokio::time::Instant::now() < deadline,
            "CEF rendered pixels, visibility, or native bounds did not match: expected_visible={visible}, expected_bounds={rect:?}, elapsed_ms={}, actual={probe:?}",
            started.elapsed().as_millis()
        );
        if let Some(rect) = rect {
            viewport(app, Some(selected(app)?.as_str()), Some(rect)).await?;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn probe_with_heartbeat(
    app: &tauri::AppHandle,
    view: &Surface,
    visible: bool,
    rect: Option<BrowserRect>,
) -> anyhow::Result<colossus_native_browser::chromium::AcceptanceProbe> {
    if visible && let Some(rect) = rect {
        let controller = app
            .get_webview("main")
            .ok_or_else(|| anyhow::anyhow!("Desktop controller missing"))?;
        if !colossus_native_browser::is_active(&controller).await? {
            focus_controller(&controller, view).await?;
        }
        viewport(app, Some(selected(app)?.as_str()), Some(rect)).await?;
    }
    let probe = view.acceptance_probe();
    tokio::pin!(probe);
    loop {
        tokio::select! {
            result = &mut probe => return Ok(result?),
            () = tokio::time::sleep(Duration::from_millis(250)), if visible => {
                // The real renderer keeps renewing its viewport lease while a
                // page paints. A native screenshot must not suspend that owner.
                if let Some(rect) = rect {
                    let controller = app.get_webview("main").ok_or_else(|| anyhow::anyhow!("Desktop controller missing"))?;
                    if !colossus_native_browser::is_active(&controller).await? {
                        focus_controller(&controller, view).await?;
                    }
                    viewport(app, Some(selected(app)?.as_str()), Some(rect)).await?;
                }
            }
        }
    }
}

fn selected(app: &tauri::AppHandle) -> anyhow::Result<String> {
    app.state::<AppState>()
        .browser
        .lock()
        .map_err(|error| anyhow::anyhow!(error.message))?
        .snapshot()
        .selected_tab_id
        .ok_or_else(|| anyhow::anyhow!("selected CEF tab missing"))
}

async fn history(
    app: &tauri::AppHandle,
    address: &str,
    id: &str,
    view: &Surface,
) -> anyhow::Result<()> {
    action(
        app,
        BrowserAction::Navigate {
            tab_id: id.to_owned(),
            url: format!("{address}/second"),
        },
    )
    .await?;
    wait_title(view, "Second page").await?;
    anyhow::ensure!(
        view.inspect().await?.can_go_back,
        "CEF history did not retain the first page"
    );
    action(
        app,
        BrowserAction::Back {
            tab_id: id.to_owned(),
        },
    )
    .await?;
    wait_title(view, "First page").await?;
    anyhow::ensure!(
        view.inspect().await?.can_go_forward,
        "CEF forward history missing"
    );
    action(
        app,
        BrowserAction::Forward {
            tab_id: id.to_owned(),
        },
    )
    .await?;
    wait_title(view, "Second page").await?;
    action(
        app,
        BrowserAction::Reload {
            tab_id: id.to_owned(),
        },
    )
    .await?;
    wait_title(view, "Second page").await?;
    viewport(app, Some(id), Some(bounds(app)?)).await?;
    pixels(app, view, true, Some(bounds(app)?)).await?;
    println!("PASS CEF navigation, back, forward, and reload");
    Ok(())
}

async fn tabs(
    app: &tauri::AppHandle,
    address: &str,
    a: &str,
    first: &Surface,
) -> anyhow::Result<()> {
    let second = action(
        app,
        BrowserAction::New {
            url: format!("{address}/first"),
        },
    )
    .await?;
    let b = second
        .selected_tab_id
        .ok_or_else(|| anyhow::anyhow!("second tab missing"))?;
    let sibling = surface(app, &b, "chromium-acceptance-a")?;
    wait_title(&sibling, "First page").await?;
    anyhow::ensure!(
        second.tabs.len() == 2 && second.tabs[0].session_id == second.tabs[1].session_id,
        "CEF tabs lack shared workspace session ownership"
    );
    let rect = bounds(app)?;
    viewport(app, Some(&b), Some(rect)).await?;
    pixels(app, first, false, None).await?;
    pixels(app, &sibling, true, Some(rect)).await?;
    action(
        app,
        BrowserAction::Select {
            tab_id: a.to_owned(),
        },
    )
    .await?;
    viewport(app, Some(a), Some(rect)).await?;
    pixels(app, &sibling, false, None).await?;
    pixels(app, first, true, Some(rect)).await?;
    action(app, BrowserAction::Close { tab_id: b }).await?;
    anyhow::ensure!(
        sibling.inspect().await.is_err(),
        "closed CEF tab retained engine authority"
    );
    println!("PASS CEF tab creation, selection, hidden sibling, and close");
    Ok(())
}

async fn resizing(app: &tauri::AppHandle, a: &str, view: &Surface) -> anyhow::Result<()> {
    let window = app
        .get_window("main")
        .ok_or_else(|| anyhow::anyhow!("Desktop window missing"))?;
    window.set_size(tauri::LogicalSize::new(1050.0, 760.0))?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let rect = bounds(app)?;
    viewport(app, Some(a), Some(rect)).await?;
    pixels(app, view, true, Some(rect)).await?;
    println!("PASS CEF native bounds and rendered content after Desktop resize");
    checkpoint(app, "resized-page", Some(a), Some(rect)).await?;
    Ok(())
}

async fn overlays(app: &tauri::AppHandle, a: &str, view: &Surface) -> anyhow::Result<()> {
    let controller = app
        .get_webview("main")
        .ok_or_else(|| anyhow::anyhow!("Desktop controller missing"))?;
    let rect = bounds(app)?;
    focus_controller(&controller, view).await?;
    viewport(app, Some(a), Some(rect)).await?;
    pixels(app, view, true, Some(rect)).await?;
    require_active(&controller, "overlay visibility baseline").await?;
    controller.eval("const overlay = document.createElement('div'); overlay.id = 'chromium-acceptance-overlay'; overlay.style.cssText = 'position:fixed;inset:0;z-index:2147483647;display:grid;place-items:center;background:var(--surface-primary,#fff);color:var(--text-primary,#111);font:24px sans-serif'; overlay.textContent = 'Colossus Desktop overlay: Chromium must be occluded'; document.body.appendChild(overlay);")?;
    viewport(app, None, None).await?;
    require_active(&controller, "overlay visibility revocation").await?;
    pixels(app, view, false, None).await?;
    require_active(&controller, "overlay hidden capture").await?;
    println!("PASS Desktop overlay revokes native Chromium visibility");
    checkpoint(app, "desktop-overlay-occlusion", None, None).await?;
    controller.eval("document.getElementById('chromium-acceptance-overlay')?.remove();")?;
    viewport(app, Some(a), Some(rect)).await?;
    pixels(app, view, true, Some(rect)).await?;
    require_active(&controller, "viewport expiry visibility baseline").await?;
    anyhow::ensure!(leased(app, a)?, "visible CEF tab lacks a viewport lease");
    // Do not renew presentation or reactivate the window during this interval.
    // Otherwise inactive-window hiding could masquerade as watchdog expiry.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(2600);
    while tokio::time::Instant::now() < deadline {
        require_active(&controller, "viewport heartbeat expiry interval").await?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    require_active(&controller, "viewport heartbeat expiry capture").await?;
    pixels(app, view, false, None).await?;
    require_active(&controller, "expired viewport hidden capture").await?;
    anyhow::ensure!(
        !leased(app, a)?,
        "watchdog retained an expired viewport lease"
    );
    viewport(app, Some(a), Some(rect)).await?;
    pixels(app, view, true, Some(rect)).await?;
    println!("PASS stale viewport heartbeat hides CEF and fresh lease restores it");
    Ok(())
}

async fn require_active(controller: &tauri::Webview, scenario: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        colossus_native_browser::is_active(controller).await?,
        "Desktop lost foreground during {scenario}; leave the test window in front and rerun"
    );
    Ok(())
}

fn leased(app: &tauri::AppHandle, id: &str) -> anyhow::Result<bool> {
    Ok(app
        .state::<AppState>()
        .browser
        .lock()
        .map_err(|error| anyhow::anyhow!(error.message))?
        .tabs
        .iter()
        .find(|tab| tab.dto.id == id)
        .is_some_and(|tab| tab.heartbeat.is_some()))
}

async fn workspace_boundary(
    app: &tauri::AppHandle,
    address: &str,
    a: &str,
    old_generation: u64,
) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    state
        .select_target(Some("chromium-acceptance-b".into()))
        .await;
    anyhow::ensure!(
        state.browser.validate(old_generation).is_err(),
        "stale workspace command retained browser authority"
    );
    anyhow::ensure!(
        state.browser.tab(a, "chromium-acceptance-b").is_err(),
        "foreign workspace could access CEF tab"
    );
    let snapshot = action(
        app,
        BrowserAction::New {
            url: format!("{address}/first"),
        },
    )
    .await?;
    anyhow::ensure!(
        snapshot.tabs.len() == 1 && !snapshot.engine.agent_control_available,
        "workspace selection or disabled production automation changed"
    );
    let b = snapshot
        .selected_tab_id
        .ok_or_else(|| anyhow::anyhow!("other workspace tab missing"))?;
    wait_title(&surface(app, &b, "chromium-acceptance-b")?, "First page").await?;
    anyhow::ensure!(
        action(
            app,
            BrowserAction::Navigate {
                tab_id: b,
                url: "http://tauri.localhost/".into()
            }
        )
        .await
        .is_err(),
        "CEF accepted privileged app origin"
    );
    println!(
        "PASS CEF workspace generation, foreign tab, app-origin denial, and disabled production automation"
    );
    Ok(())
}

async fn checkpoint(
    app: &tauri::AppHandle,
    label: &str,
    id: Option<&str>,
    rect: Option<BrowserRect>,
) -> anyhow::Result<()> {
    println!("CHECKPOINT {label}");
    let pause = std::env::var("COLOSSUS_BROWSER_ACCEPTANCE_PAUSE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .min(30_000);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(pause);
    while tokio::time::Instant::now() < deadline {
        viewport(app, id, rect).await?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}
