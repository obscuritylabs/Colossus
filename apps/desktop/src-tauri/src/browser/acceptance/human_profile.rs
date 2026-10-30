//! Separate-process real `WebView2` profile acceptance; never part of shipping code.

use crate::{
    browser::{
        dto::BrowserAction,
        profile::{HumanBrowserPreferences, ProfileStore},
    },
    state::AppState,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::Manager as _;

#[path = "human_controller.rs"]
mod controller;

const SCOPE: &str = "human-profile-acceptance-a";
const USER_AGENT: &str = "Colossus-Human-Profile-Acceptance/1.0";

pub(super) fn run() -> i32 {
    let address = std::env::args().nth(2).expect("owned loopback fixture URL");
    let phase = std::env::args()
        .nth(3)
        .expect("seed, restart, clear or cleared");
    let url = tauri::Url::parse(&address).expect("fixture URL");
    let https = matches!(
        phase.as_str(),
        "mtls_seed" | "mtls_restart" | "tls_rejected"
    );
    assert_eq!(url.scheme(), if https { "https" } else { "http" });
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert!(matches!(
        phase.as_str(),
        "seed" | "restart" | "clear" | "cleared" | "mtls_seed" | "mtls_restart" | "tls_rejected"
    ));
    let home = PathBuf::from(std::env::var_os("COLOSSUS_HOME").expect("owned isolated home"));
    assert!(
        home.is_absolute() && home.is_dir(),
        "driver must allocate the private test home"
    );
    let _home_custody =
        colossus_home::ConfinedRoot::bind(&home).expect("private test home custody");
    if matches!(phase.as_str(), "seed" | "mtls_seed" | "tls_rejected") {
        let store = ProfileStore::open(SCOPE).expect("fixture profile store");
        assert_eq!(
            store.read().unwrap(),
            HumanBrowserPreferences::default(),
            "seed requires a new profile"
        );
        store
            .write(&HumanBrowserPreferences {
                remember_sign_ins: true,
                user_agent: USER_AGENT.into(),
            })
            .unwrap();
    }
    let mut context = crate::app_context::create();
    let _unused_application_assets = context.set_assets(Box::new(controller::ControllerAssets));
    context.config_mut().build.dev_url = None;
    context.config_mut().app.windows[0].data_directory = Some(home.join("controller"));
    context.config_mut().app.windows[0].incognito = true;
    context.config_mut().app.windows[0].visible = phase.starts_with("mtls_");
    context.config_mut().app.windows[0].title = "Colossus Human Browser Fixture".into();
    let completed = Arc::new(AtomicBool::new(false));
    let completion = completed.clone();
    let application = tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            crate::browser::commands::browser_context,
            crate::browser::commands::browser_command,
            crate::browser::preferences::browser_preferences,
        ])
        .setup(move |app| {
            let app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let result = exercise(&app, &address, &phase).await;
                match result {
                    Ok(()) => {
                        completion.store(true, Ordering::Release);
                        app.exit(0);
                    }
                    Err(error) => {
                        eprintln!("human profile acceptance failed: {error:#}");
                        app.exit(1);
                    }
                }
            });
            Ok(())
        })
        .build(context)
        .expect("human browser acceptance app");
    super::run_lifecycle(application, &completed)
}

async fn open(
    app: &tauri::AppHandle,
    scope: &str,
    address: &str,
    expected_title: &str,
) -> anyhow::Result<String> {
    let state = app.state::<AppState>();
    state.select_target(Some(scope.into())).await;
    let snapshot = super::action(
        app,
        BrowserAction::New {
            conversation_id: None,
            url: address.into(),
        },
    )
    .await?;
    anyhow::ensure!(snapshot.engine.kind == "webview2" && !snapshot.engine.agent_control_available);
    let id = snapshot
        .selected_tab_id
        .ok_or_else(|| anyhow::anyhow!("native tab missing"))?;
    if app
        .get_window("main")
        .is_some_and(|window| window.is_visible().unwrap_or(false))
    {
        let view = app
            .get_webview(&id)
            .ok_or_else(|| anyhow::anyhow!("native guest missing"))?;
        view.set_position(tauri::LogicalPosition::new(16.0, 80.0))?;
        view.set_size(tauri::LogicalSize::new(600.0, 360.0))?;
        view.show()?;
    }
    wait_title(app, &id, expected_title).await?;
    Ok(id)
}

async fn wait_title(app: &tauri::AppHandle, id: &str, expected: &str) -> anyhow::Result<()> {
    let view = app
        .get_webview(id)
        .ok_or_else(|| anyhow::anyhow!("native guest missing"))?;
    let interactive = app
        .get_window("main")
        .is_some_and(|window| window.is_visible().unwrap_or(false));
    let deadline =
        std::time::Instant::now() + Duration::from_secs(if interactive { 120 } else { 15 });
    loop {
        let snapshot = app
            .state::<AppState>()
            .browser
            .snapshot()
            .await
            .map_err(|failure| anyhow::anyhow!(failure.message))?;
        if colossus_native_browser::inspect(&view).await?.title == expected
            && snapshot
                .tabs
                .iter()
                .any(|tab| tab.id == id && !tab.page.loading)
            && snapshot
                .diagnostics
                .get(id)
                .is_some_and(|details| details.http_status.is_some())
        {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("expected categorical fixture result {expected}")
}

async fn exercise(app: &tauri::AppHandle, address: &str, phase: &str) -> anyhow::Result<()> {
    super::controller_authorization(app).await?;
    controller::ready(app).await?;
    if phase == "tls_rejected" {
        return rejected_tls(app, address).await;
    }
    let stored = ProfileStore::open(SCOPE)
        .map_err(|e| anyhow::anyhow!(e.message))?
        .read()
        .map_err(|e| anyhow::anyhow!(e.message))?;
    anyhow::ensure!(
        stored.remember_sign_ins && stored.user_agent == USER_AGENT,
        "native preferences did not survive restart"
    );
    let initial = if phase == "mtls_seed" {
        "mTLS seeded"
    } else if phase == "mtls_restart" {
        "Client certificate accepted"
    } else if phase == "seed" {
        "Seeded"
    } else if phase == "cleared" {
        "Signed out"
    } else {
        "Signed in"
    };
    let endpoint = if matches!(phase, "seed" | "mtls_seed") {
        "seed"
    } else {
        "verify"
    };
    let id = open(app, SCOPE, &format!("{address}/{endpoint}"), initial).await?;
    let state = app.state::<AppState>();
    let diagnostics = state
        .browser
        .snapshot()
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    let details = diagnostics
        .diagnostics
        .get(&id)
        .ok_or_else(|| anyhow::anyhow!("native diagnostics missing"))?;
    anyhow::ensure!(details.persistent && details.custom_user_agent && details.system_engine);
    let (view, _) = state
        .browser
        .tab(&id, SCOPE)
        .map_err(|e| anyhow::anyhow!(e.message))?;
    anyhow::ensure!(
        view.handoff("fixture-agent").await
            == Err(colossus_native_browser::BrowserError::Unavailable)
    );
    println!("PASS native saved human profile, exact User-Agent readback and agent handoff denial");
    if phase.starts_with("mtls_") {
        if phase == "mtls_seed" {
            anyhow::ensure!(
                details.client_certificate_requested,
                "native client-certificate challenge not observed"
            );
        }
        println!(
            "PASS native client-certificate chooser, strict HTTPS and Secure HttpOnly profile result"
        );
    } else if phase == "seed" {
        seed_checks(app, address, &id, &stored).await?;
    } else if phase == "restart" {
        println!("PASS saved HttpOnly sign-in survived a normal independent application restart");
    } else if phase == "clear" {
        let generation = state
            .browser
            .snapshot()
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?
            .generation;
        let request = serde_json::from_value(
            serde_json::json!({"generation": generation, "action": {"type": "clear_data"}}),
        )?;
        let controller = app
            .get_webview("main")
            .ok_or_else(|| anyhow::anyhow!("controller missing"))?;
        crate::browser::preferences::browser_preferences(
            app.clone(),
            controller,
            state.clone(),
            request,
        )
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
        println!("PASS actual trusted preferences command confirmed native browsing-data clear");
    } else {
        println!("PASS cleared sign-in stayed absent after an independent application restart");
    }
    Ok(())
}

async fn rejected_tls(app: &tauri::AppHandle, address: &str) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    state.select_target(Some(SCOPE.into())).await;
    let snapshot = super::action(
        app,
        BrowserAction::New {
            conversation_id: None,
            url: format!("{address}/verify"),
        },
    )
    .await?;
    let id = snapshot
        .selected_tab_id
        .ok_or_else(|| anyhow::anyhow!("native TLS tab missing"))?;
    for _ in 0..150 {
        let snapshot = state
            .browser
            .snapshot()
            .await
            .map_err(|failure| anyhow::anyhow!(failure.message))?;
        if snapshot
            .diagnostics
            .get(&id)
            .is_some_and(|details| details.tls_failed && details.http_status.is_none())
        {
            println!(
                "PASS native WebView2 rejected the untrusted fixture chain without a TLS bypass"
            );
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("native TLS rejection was not observed")
}

async fn seed_checks(
    app: &tauri::AppHandle,
    address: &str,
    id: &str,
    stored: &HumanBrowserPreferences,
) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    super::action(
        app,
        BrowserAction::Navigate {
            tab_id: id.to_owned(),
            url: format!("{address}/deny"),
        },
    )
    .await?;
    wait_title(app, id, "Denied").await?;
    let snapshot = state
        .browser
        .snapshot()
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    anyhow::ensure!(
        snapshot
            .diagnostics
            .get(id)
            .and_then(|value| value.http_status)
            == Some(401),
        "native HTTP 401 missing"
    );
    super::action(
        app,
        BrowserAction::Navigate {
            tab_id: id.to_owned(),
            url: format!("{address}/redirect?synthetic=must-not-appear"),
        },
    )
    .await?;
    wait_title(app, id, "Signed in").await?;
    let snapshot = state
        .browser
        .snapshot()
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    let details = snapshot.diagnostics.get(id).unwrap();
    anyhow::ensure!(details.redirects >= 1 && details.http_status == Some(200));
    let encoded = serde_json::to_string(details)?;
    anyhow::ensure!(!encoded.contains("synthetic") && !encoded.contains("must-not-appear"));
    println!("PASS native 401/redirect diagnostics omit callback paths and query values");
    let store =
        ProfileStore::open("human-profile-acceptance-b").map_err(|e| anyhow::anyhow!(e.message))?;
    store
        .write(stored)
        .map_err(|e| anyhow::anyhow!(e.message))?;
    open(
        app,
        "human-profile-acceptance-b",
        &format!("{address}/verify"),
        "Signed out",
    )
    .await?;
    println!("PASS second workspace cannot receive the first workspace's HttpOnly cookie");
    Ok(())
}
