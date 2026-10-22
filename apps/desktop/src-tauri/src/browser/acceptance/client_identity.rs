//! Synthetic Windows child/IPC acceptance. No TLS, certificate store, or key use.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use colossus_native_browser::{BrowserView, NavigationPolicy, PageState};
use tauri::{AppHandle, Manager as _, Webview, WebviewUrl, webview::WebviewBuilder};

use super::{
    dto::{BrowserRect, BrowserTabDto, BrowserViewportRequest},
    registry::Tab,
};
use crate::{
    client_identity_review::{self, ClientIdentityDisplay, ClientIdentityDisplayCandidate},
    state::AppState,
};

const SCOPE: &str = "synthetic-client-identity-workspace";
const GUEST: &str = "client-identity-test-guest";

pub(crate) fn run() -> i32 {
    println!("CLIENT_IDENTITY_REVIEW_HOST_PID {}", std::process::id());
    let home = PathBuf::from(std::env::var_os("COLOSSUS_HOME").expect("fresh isolated test home"));
    assert!(
        home.is_absolute() && !home.exists(),
        "test home must be a fresh absolute path"
    );
    colossus_windows_native::create_private_directory(&home).expect("private acceptance home");
    let mut context = crate::app_context::create();
    context.config_mut().identifier =
        format!("com.colossus.identity-test.{}", uuid::Uuid::new_v4());
    context.config_mut().build.dev_url = None;
    context.config_mut().app.windows[0].data_directory = Some(home.join("controller"));
    context.config_mut().app.windows[0].incognito = true;
    context.config_mut().app.windows[0].width = 880.0;
    context.config_mut().app.windows[0].height = 640.0;
    let completed = Arc::new(AtomicBool::new(false));
    let completion = completed.clone();
    let application = tauri::Builder::default()
        .register_uri_scheme_protocol(
            client_identity_review::protocol::SCHEME,
            |context, request| client_identity_review::protocol::respond(&context, &request),
        )
        .manage(AppState::default())
        .manage(client_identity_review::ClientIdentityReviewState::default())
        .manage(crate::terminal_commands::pane::TerminalPaneState::default())
        .invoke_handler(tauri::generate_handler![
            client_identity_review::client_identity_review_context,
            client_identity_review::finish_client_identity_review,
            client_identity_review::sync_client_identity_appearance
        ])
        .on_window_event(client_identity_review::handle_window_event)
        .setup(move |app| {
            println!(
                "CLIENT_IDENTITY_REVIEW_APP_DATA {}",
                app.path().app_local_data_dir()?.display()
            );
            let app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let result =
                    tokio::time::timeout(Duration::from_mins(1), exercise(&app, &home)).await;
                let passed = matches!(result, Ok(Ok(())));
                if let Ok(Err(error)) = result {
                    eprintln!("client identity child acceptance failed: {error:#}");
                } else if result.is_err() {
                    eprintln!("client identity child acceptance exceeded its fixed deadline");
                }
                completion.store(passed, Ordering::Release);
                app.exit(i32::from(!passed));
            });
            Ok(())
        })
        .build(context)
        .expect("synthetic client identity app");
    application.run_return(|_, _| {});
    let passed = completed.load(Ordering::Acquire);
    if passed {
        println!("PASS trusted in-window client identity acceptance and application exit");
    }
    i32::from(!passed)
}

fn display(remaining_ms: u32) -> ClientIdentityDisplay {
    ClientIdentityDisplay {
        origin: "https://client-identity.example.test:8443".into(),
        candidates: vec![
            ClientIdentityDisplayCandidate {
                label: "Synthetic Alice — Example issuing CA".into(),
                fingerprint_sha256: "a1".repeat(32),
            },
            ClientIdentityDisplayCandidate {
                label: "Synthetic Alice — Other issuing CA".into(),
                fingerprint_sha256: "b2".repeat(32),
            },
        ],
        remaining_ms,
    }
}

async fn exercise(app: &AppHandle, home: &Path) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    state.select_target(Some(SCOPE.into())).await;
    let epoch = *state.subscribe_selection().borrow();
    let controller = app.get_webview("main").expect("synthetic main document");
    wait_for(&controller, "document.readyState === 'complete'").await?;
    let guest = install_guest(app, home).await?;
    heartbeat(app, true).await?;
    wait_for(&guest, "document.visibilityState === 'visible'").await?;

    let task = start_review(app, epoch, 15_000);
    let review = wait_review(app).await?;
    anyhow::ensure!(
        review.window().label() == "main" && app.windows().len() == 1,
        "review created an extra OS window"
    );
    geometry(app, &review).await?;
    heartbeat(app, false).await?;
    wait_for(&guest, "document.visibilityState === 'hidden'").await?;
    println!("PASS actual main-window child geometry and native guest heartbeat occlusion");

    let context = invoke(
        &review,
        "client_identity_review_context",
        serde_json::json!({}),
    )
    .await?;
    anyhow::ensure!(context["ok"] == true, "trusted child IPC denied");
    let token = context["value"]["uiToken"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing opaque UI token"))?;
    for caller in [&controller, &guest] {
        anyhow::ensure!(
            invoke(
                caller,
                "client_identity_review_context",
                serde_json::json!({})
            )
            .await?["ok"]
                == false,
            "foreign document read review metadata"
        );
        anyhow::ensure!(
            invoke(
                caller,
                "finish_client_identity_review",
                serde_json::json!({"uiToken":token,"fingerprintSha256":"a1".repeat(32)})
            )
            .await?["ok"]
                == false,
            "foreign document selected a certificate"
        );
    }
    for (ui_token, fingerprint) in [
        ("forged-ui-token", "a1".repeat(32)),
        (token, "cc".repeat(32)),
    ] {
        anyhow::ensure!(
            invoke(
                &review,
                "finish_client_identity_review",
                serde_json::json!({"uiToken":ui_token,"fingerprintSha256":fingerprint})
            )
            .await?["ok"]
                == false,
            "forged choice was accepted"
        );
    }
    anyhow::ensure!(!task.is_finished(), "invalid IPC consumed the review");
    println!("PASS exact-document IPC denies ordinary controller, guest and forged choices");
    review.eval("document.querySelector('[role=radio]').click()")?;
    wait_for(&review, "Array.from(document.querySelectorAll('button')).some(b => b.textContent === 'Use certificate' && !b.disabled)").await?;
    review.eval("Array.from(document.querySelectorAll('button')).find(b => b.textContent === 'Use certificate').click()")?;
    anyhow::ensure!(
        answer(task).await? == Some("a1".repeat(32)),
        "explicit native choice did not return the selected fingerprint"
    );
    wait_closed(app).await?;
    println!("PASS actual bundled controls select exactly the reviewed fingerprint once");

    exercise_cancellation(app, epoch).await?;
    state.browser.hide_all();
    guest.close()?;
    Ok(())
}

async fn exercise_cancellation(app: &AppHandle, epoch: u64) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let cancelled = start_review(app, epoch, 15_000);
    let review = wait_review(app).await?;
    review.eval("Array.from(document.querySelectorAll('button')).find(b => b.textContent === 'Cancel').click()")?;
    anyhow::ensure!(
        answer(cancelled).await?.is_none(),
        "cancel selected an identity"
    );
    wait_closed(app).await?;
    println!("PASS actual Cancel returns no identity and closes only its trusted child");

    let changed = start_review(app, epoch, 15_000);
    wait_review(app).await?;
    state
        .select_target(Some("different-synthetic-workspace".into()))
        .await;
    anyhow::ensure!(changed.await?.is_err(), "workspace change retained consent");
    wait_closed(app).await?;
    state.select_target(Some(SCOPE.into())).await;
    let epoch = *state.subscribe_selection().borrow();
    let dropped = start_review(app, epoch, 15_000);
    wait_review(app).await?;
    dropped.abort();
    let _ = dropped.await;
    wait_closed(app).await?;
    let expired = start_review(app, epoch, 500);
    anyhow::ensure!(
        expired.await?.ok().flatten().is_none(),
        "expiry selected an identity"
    );
    wait_closed(app).await?;
    anyhow::ensure!(
        app.get_window("main").is_some() && app.windows().len() == 1,
        "cancellation closed the main window"
    );
    println!(
        "PASS workspace change, dropped future and fixed expiry retire native UI without selection"
    );
    Ok(())
}

fn start_review(
    app: &AppHandle,
    epoch: u64,
    remaining_ms: u32,
) -> tokio::task::JoinHandle<Result<Option<String>, crate::dto::CommandErrorDto>> {
    let app = app.clone();
    tokio::spawn(async move {
        client_identity_review::show(&app, SCOPE, epoch, display(remaining_ms)).await
    })
}

async fn answer(
    task: tokio::task::JoinHandle<Result<Option<String>, crate::dto::CommandErrorDto>>,
) -> anyhow::Result<Option<String>> {
    task.await?.map_err(|error| anyhow::anyhow!(error.message))
}

async fn wait_review(app: &AppHandle) -> anyhow::Result<Webview> {
    for _ in 0..100 {
        if let Some(view) = app.get_webview(client_identity_review::WEBVIEW)
            && evaluate(&view, "!!document.querySelector('[role=dialog]')").await? == true
        {
            return Ok(view);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("trusted review document did not become ready")
}

async fn wait_closed(app: &AppHandle) -> anyhow::Result<()> {
    for _ in 0..100 {
        if app.get_webview(client_identity_review::WEBVIEW).is_none()
            && !client_identity_review::is_active(app)
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("retired review child survived cancellation")
}

async fn geometry(app: &AppHandle, review: &Webview) -> anyhow::Result<()> {
    let window = app.get_window("main").unwrap();
    for size in [(880.0, 640.0), (1040.0, 720.0)] {
        window.set_size(tauri::LogicalSize::new(size.0, size.1))?;
        let mut matched = false;
        for _ in 0..100 {
            if review.position()? == tauri::PhysicalPosition::new(0, 0)
                && review.size()? == window.inner_size()?
            {
                matched = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        anyhow::ensure!(matched, "native child did not follow main-window geometry");
    }
    Ok(())
}

async fn install_guest(app: &AppHandle, home: &Path) -> anyhow::Result<Webview> {
    let window = app.get_window("main").unwrap();
    let directory = home.join("guest");
    let view = tauri::async_runtime::spawn_blocking(move || {
        window.add_child(
            WebviewBuilder::new(
                GUEST,
                WebviewUrl::App("index.html?surface=client-identity-review".into()),
            )
            .incognito(true)
            .devtools(false)
            .focused(false)
            .data_directory(directory),
            tauri::LogicalPosition::new(32.0, 100.0),
            tauri::LogicalSize::new(400.0, 300.0),
        )
    })
    .await??;
    let state = app.state::<AppState>();
    let mut registry = state
        .browser
        .lock()
        .map_err(|error| anyhow::anyhow!(error.message))?;
    registry.selected.insert(SCOPE.into(), GUEST.into());
    registry.tabs.push(Tab {
        scope: SCOPE.into(),
        view: BrowserView::System(view.clone()),
        pending_completed_view: None,
        close_requested: false,
        agent_run_id: None,
        policy: NavigationPolicy::default(),
        dto: BrowserTabDto {
            id: GUEST.into(),
            session_id: "synthetic-session".into(),
            conversation_id: None,
            control: "human",
            completed_snapshot: false,
            page: PageState::default(),
            error: None,
            notice: None,
            popup_url: None,
        },
        heartbeat: None,
        presentation_epoch: Arc::new(AtomicU64::new(0)),
    });
    Ok(view)
}

async fn heartbeat(app: &AppHandle, expected: bool) -> anyhow::Result<()> {
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
                tab_id: Some(GUEST.into()),
                rect: Some(BrowserRect {
                    x: 32.0,
                    y: 100.0,
                    width: 400.0,
                    height: 300.0,
                }),
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let registry = state
        .browser
        .lock()
        .map_err(|error| anyhow::anyhow!(error.message))?;
    anyhow::ensure!(
        registry
            .tabs
            .iter()
            .find(|tab| tab.dto.id == GUEST)
            .unwrap()
            .heartbeat
            .is_some()
            == expected,
        "native guest visibility lease ignored review occlusion"
    );
    Ok(())
}

async fn evaluate(view: &Webview, script: &str) -> anyhow::Result<serde_json::Value> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Mutex::new(Some(sender));
    view.eval_with_callback(script, move |value| {
        if let Some(sender) = sender.lock().unwrap().take() {
            let _ = sender.send(value);
        }
    })?;
    Ok(serde_json::from_str(
        &tokio::time::timeout(Duration::from_secs(5), receiver).await??,
    )?)
}

async fn wait_for(view: &Webview, condition: &str) -> anyhow::Result<()> {
    for _ in 0..100 {
        if evaluate(view, condition).await? == true {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("native document condition did not settle: {condition}")
}

async fn invoke(
    view: &Webview,
    command: &str,
    args: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    view.eval(format!("window.__identityProbe = null; window.__TAURI_INTERNALS__.invoke({}, {}).then(value => window.__identityProbe = {{ok: true, value}}, () => window.__identityProbe = {{ok: false}})", serde_json::to_string(command)?, args))?;
    wait_for(view, "window.__identityProbe !== null").await?;
    evaluate(view, "window.__identityProbe").await
}
