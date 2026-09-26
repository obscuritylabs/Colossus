//! Actual engine evidence: this module is never linked by a normal Desktop build.

use super::dto::BrowserAction;
use crate::state::AppState;
use std::time::Duration;
use tauri::{Manager as _, Webview};

pub(crate) fn run() {
    println!("Starting native browser acceptance");
    let address = std::env::args().nth(1).expect("native browser fixture URL");
    let url = tauri::Url::parse(&address).expect("fixture URL");
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    let home =
        std::path::PathBuf::from(std::env::var_os("COLOSSUS_HOME").expect("isolated test home"));
    assert!(!home.exists(), "test home must be new");
    #[cfg(windows)]
    colossus_windows_native::create_private_directory(&home).expect("private acceptance home");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&home)
            .expect("private acceptance home");
    }
    let mut context = tauri::generate_context!();
    context.config_mut().build.dev_url = None;
    context.config_mut().app.windows[0].data_directory = Some(home.join("controller"));
    context.config_mut().app.windows[0].incognito = true;
    let application = tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            crate::browser::commands::browser_context,
            crate::browser::commands::browser_command,
            crate::browser::commands::browser_viewport
        ])
        .setup(move |app| {
            println!("Native app initialized");
            let app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let result = exercise(&app, &address).await;
                match result {
                    Ok(()) => {
                        println!("native browser acceptance passed");
                        app.exit(0);
                    }
                    Err(error) => {
                        eprintln!("native browser acceptance failed: {error:#}");
                        app.exit(1);
                    }
                }
            });
            Ok(())
        })
        .build(context)
        .expect("native browser acceptance app");
    application.run(|_, _| {});
}

async fn action(
    app: &tauri::AppHandle,
    action: BrowserAction,
) -> anyhow::Result<super::dto::BrowserSnapshotDto> {
    let state = app.state::<AppState>();
    let generation = state
        .browser
        .snapshot()
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?
        .generation;
    state
        .browser
        .apply(app, generation, action)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    state
        .browser
        .snapshot()
        .await
        .map_err(|e| anyhow::anyhow!(e.message))
}

async fn evaluate(view: &Webview, script: &str) -> anyhow::Result<serde_json::Value> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let send = std::sync::Mutex::new(Some(send));
    view.eval_with_callback(script, move |result| {
        if let Some(send) = send.lock().expect("acceptance callback").take() {
            let _ = send.send(result);
        }
    })?;
    let value = tokio::time::timeout(Duration::from_secs(5), receive).await??;
    Ok(serde_json::from_str(&value)?)
}

async fn wait_title(view: &Webview, title: &str) -> anyhow::Result<()> {
    for _ in 0..100 {
        if colossus_native_browser::inspect(view).await?.title == title {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("expected native title {title:?}")
}

async fn exercise(app: &tauri::AppHandle, address: &str) -> anyhow::Result<()> {
    println!("Creating first native browser tab");
    let state = app.state::<AppState>();
    state.select_target(Some("acceptance-a".into())).await;
    let first = action(
        app,
        BrowserAction::New {
            url: format!("{address}/first"),
        },
    )
    .await?;
    let a = first.selected_tab_id.clone().expect("first tab");
    let first_view = app.get_webview(&a).expect("first native guest");
    wait_title(&first_view, "First page").await?;
    println!("PASS native create and load");
    history(app, address, &a, &first_view).await?;
    sessions(app, address, first.generation).await?;
    Ok(())
}

async fn history(
    app: &tauri::AppHandle,
    address: &str,
    a: &str,
    first_view: &Webview,
) -> anyhow::Result<()> {
    evaluate(
        first_view,
        "document.cookie = 'browser_probe=workspace_a; path=/'; true",
    )
    .await?;
    action(
        app,
        BrowserAction::Navigate {
            tab_id: a.to_owned(),
            url: format!("{address}/second"),
        },
    )
    .await?;
    wait_title(first_view, "Second page").await?;
    anyhow::ensure!(
        colossus_native_browser::inspect(first_view)
            .await?
            .can_go_back,
        "native back history"
    );
    action(
        app,
        BrowserAction::Back {
            tab_id: a.to_owned(),
        },
    )
    .await?;
    wait_title(first_view, "First page").await?;
    action(
        app,
        BrowserAction::Forward {
            tab_id: a.to_owned(),
        },
    )
    .await?;
    wait_title(first_view, "Second page").await?;
    action(
        app,
        BrowserAction::Reload {
            tab_id: a.to_owned(),
        },
    )
    .await?;
    wait_title(first_view, "Second page").await?;
    println!("PASS native back, forward, and reload");
    Ok(())
}

async fn sessions(app: &tauri::AppHandle, address: &str, generation: u64) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let second = action(
        app,
        BrowserAction::New {
            url: format!("{address}/first"),
        },
    )
    .await?;
    let sibling = app
        .get_webview(second.selected_tab_id.as_deref().unwrap())
        .unwrap();
    wait_title(&sibling, "First page").await?;
    anyhow::ensure!(
        evaluate(
            &sibling,
            "document.cookie.includes('browser_probe=workspace_a')"
        )
        .await?
            == true,
        "same-workspace browser session was not shared"
    );
    state.select_target(Some("acceptance-b".into())).await;
    anyhow::ensure!(
        state.browser.validate(generation).is_err(),
        "stale workspace command was accepted"
    );
    let second_scope = action(
        app,
        BrowserAction::New {
            url: format!("{address}/first"),
        },
    )
    .await?;
    let b = second_scope.selected_tab_id.unwrap();
    let other = app.get_webview(&b).unwrap();
    wait_title(&other, "First page").await?;
    anyhow::ensure!(
        evaluate(&other, "document.cookie === ''").await? == true,
        "browser cookies crossed workspace boundary"
    );
    println!("PASS workspace isolation and temporary cookie sharing");
    anyhow::ensure!(
        state.browser.tab(sibling.label(), "acceptance-b").is_err(),
        "foreign tab was accessible"
    );
    evaluate(
        &other,
        "document.cookie = 'browser_probe=workspace_b; path=/'; true",
    )
    .await?;
    permissions(&other).await?;
    viewport(app, &other).await?;
    guest_denial(app, address, &b, &other).await?;
    handoffs(app, &b, &other, address).await?;
    cleanup(app, address).await?;
    Ok(())
}

async fn permissions(view: &Webview) -> anyhow::Result<()> {
    view.set_bounds(tauri::Rect {
        position: tauri::LogicalPosition::new(700.0, 200.0).into(),
        size: tauri::LogicalSize::new(500.0, 400.0).into(),
    })?;
    view.show()?;
    // The runner hides its console through STARTUPINFO on Windows. Force a
    // visibility transition so that flag cannot hide the first native window.
    view.window().hide()?;
    view.window().show()?;
    view.window().set_focus()?;
    view.set_focus()?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    // Geolocation is available on the loopback secure-context fixture. This
    // probes the actual native permission callback without an OS device/account.
    evaluate(view, "window.__permissionProbe = 'pending'; navigator.geolocation.getCurrentPosition(() => window.__permissionProbe = 'allowed', e => window.__permissionProbe = e.code === 1 ? 'denied' : 'unexpected'); true").await?;
    for _ in 0..50 {
        let result = evaluate(view, "window.__permissionProbe").await?;
        if result == "denied" {
            evaluate(view, "alert('Synthetic dialog suppression probe'); true").await?;
            println!("PASS native permission denial and suppressed script dialogs");
            view.hide()?;
            return Ok(());
        }
        anyhow::ensure!(
            result == "pending",
            "native permission was not denied: {result}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("native permission callback did not resolve")
}

async fn viewport(app: &tauri::AppHandle, view: &Webview) -> anyhow::Result<()> {
    use super::dto::{BrowserRect, BrowserViewportRequest};
    let state = app.state::<AppState>();
    let controller = app.get_webview("main").expect("controller");
    let active = colossus_native_browser::is_active(view).await?;
    anyhow::ensure!(
        active == colossus_native_browser::is_active(&controller).await?,
        "guest and controller disagree about window activation"
    );
    if std::env::var_os("COLOSSUS_BROWSER_INTERACTIVE_ACCEPTANCE").is_some() {
        anyhow::ensure!(
            active,
            "interactive acceptance requires the native window in the foreground"
        );
    } else if !active {
        println!("INFO no foreground native window: interactive focus acceptance remains required");
    }
    let generation = state
        .browser
        .lock()
        .map_err(|e| anyhow::anyhow!(e.message))?
        .generation;
    state
        .browser
        .viewport(
            app,
            &BrowserViewportRequest {
                generation,
                tab_id: Some(view.label().to_owned()),
                rect: Some(BrowserRect {
                    x: 700.0,
                    y: 200.0,
                    width: 500.0,
                    height: 400.0,
                }),
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    let leased = state
        .browser
        .lock()
        .map_err(|e| anyhow::anyhow!(e.message))?
        .tabs
        .iter()
        .find(|tab| tab.dto.id == view.label())
        .is_some_and(|tab| tab.heartbeat.is_some());
    anyhow::ensure!(
        leased == active,
        "viewport visibility did not follow OS window activation"
    );
    state.browser.hide_all();
    println!("PASS native viewport activation boundary");
    Ok(())
}

async fn guest_denial(
    app: &tauri::AppHandle,
    address: &str,
    b: &str,
    other: &Webview,
) -> anyhow::Result<()> {
    let forbidden = action(
        app,
        BrowserAction::Navigate {
            tab_id: b.to_owned(),
            url: "http://tauri.localhost/".into(),
        },
    )
    .await;
    anyhow::ensure!(forbidden.is_err(), "app origin navigation was accepted");
    evaluate(other, "window.__browserProbe = 'blocked'; if (window.__TAURI_INTERNALS__) { window.__TAURI_INTERNALS__.invoke('browser_context').then(() => window.__browserProbe = 'leaked', () => window.__browserProbe = 'denied'); } true").await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    anyhow::ensure!(
        evaluate(other, "window.__browserProbe !== 'leaked'").await? == true,
        "guest accessed privileged browser context"
    );
    evaluate(
        other,
        "window.location.href = 'http://tauri.localhost/'; true",
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    anyhow::ensure!(
        colossus_native_browser::inspect(other)
            .await?
            .url
            .starts_with(address),
        "script navigated into app origin"
    );
    println!("PASS guest IPC and app-origin navigation denial");
    Ok(())
}

async fn handoffs(
    app: &tauri::AppHandle,
    b: &str,
    other: &Webview,
    address: &str,
) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    evaluate(other, "window.open('/second'); true").await?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    anyhow::ensure!(
        app.webviews().len() == 4,
        "popup created an unmanaged native view"
    );
    action(
        app,
        BrowserAction::Navigate {
            tab_id: b.to_owned(),
            url: format!("{address}/download"),
        },
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let snapshot = state
        .browser
        .snapshot()
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    anyhow::ensure!(
        snapshot
            .tabs
            .iter()
            .any(|t| t.notice.as_deref().is_some_and(|n| n.contains("download"))),
        "download lacked a visible fallback"
    );
    println!("PASS popup and download handling");
    Ok(())
}

async fn cleanup(app: &tauri::AppHandle, address: &str) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    action(app, BrowserAction::Clear).await?;
    let reset = action(
        app,
        BrowserAction::New {
            url: format!("{address}/first"),
        },
    )
    .await?;
    let cleared = app
        .get_webview(reset.selected_tab_id.as_deref().unwrap())
        .unwrap();
    wait_title(&cleared, "First page").await?;
    anyhow::ensure!(
        evaluate(&cleared, "document.cookie === ''").await? == true,
        "clear session retained cookies"
    );
    action(app, BrowserAction::Clear).await?;
    state.select_target(Some("acceptance-a".into())).await;
    action(app, BrowserAction::Clear).await?;
    anyhow::ensure!(app.webviews().len() == 1, "guest views survived close");
    println!("PASS clear and close lifecycle");
    Ok(())
}
