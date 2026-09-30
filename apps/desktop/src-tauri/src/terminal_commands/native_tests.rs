//! Exercise the bundled terminal document in real `WebView2`, without user state.

use super::*;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[test]
#[ignore = "requires Windows WebView2, an interactive desktop, and npm run build"]
fn native_terminal_bundled_document_loads_and_connects_to_ipc() {
    let profile = tempfile::tempdir().expect("isolated WebView2 profile");
    let mut context = crate::app_context::create();
    context.config_mut().app.windows.clear();
    let outcome = Arc::new(Mutex::new(None));
    let result = outcome.clone();
    let data_directory = profile.path().to_owned();
    let application = tauri::Builder::default()
        .any_thread()
        .manage(AppState::default())
        .register_uri_scheme_protocol(terminal_protocol::SCHEME, |context, request| {
            terminal_protocol::respond(&context, &request)
        })
        .invoke_handler(tauri::generate_handler![terminal_context])
        .setup(move |app| {
            let epoch = app.state::<AppState>().next_terminal_window_epoch();
            // Exercise the release protocol even in a debug test: a dev server
            // cannot catch WebView2's custom-origin mapping.
            let window = WebviewWindowBuilder::new(
                app,
                TERMINAL_WEBVIEW,
                tauri::WebviewUrl::CustomProtocol(
                    "colossus-terminal://localhost/index.html?surface=terminal".parse()?,
                ),
            )
            .use_https_scheme(false)
            .on_navigation(|url| terminal_navigation_allowed_for_profile(url, false))
            .on_page_load(move |window, payload| match payload.event() {
                PageLoadEvent::Started => window
                    .state::<AppState>()
                    .terminal_document_started_for_window(epoch),
                PageLoadEvent::Finished => window
                    .state::<AppState>()
                    .terminal_document_finished_for_window(epoch),
            })
            .data_directory(data_directory)
            .title("Colossus terminal acceptance")
            .build()?;
            let app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let checked =
                    tokio::time::timeout(Duration::from_secs(30), check_document(&window))
                        .await
                        .map_err(|_| "terminal document timed out".to_owned())
                        .and_then(|result| result);
                *result.lock().expect("test result") = Some(checked);
                app.exit(0);
            });
            Ok(())
        })
        .build(context)
        .expect("native terminal test app");
    application.run_return(|_, _| {});
    let result = outcome
        .lock()
        .expect("test outcome")
        .take()
        .expect("test completed");
    result.expect("bundled terminal renderer and native IPC");
}

async fn check_document(window: &tauri::WebviewWindow) -> Result<(), String> {
    loop {
        let (send, receive) = tokio::sync::oneshot::channel();
        let send = Mutex::new(Some(send));
        // This text requires a successful real terminal_context invocation.
        // The test grants no workspace or PTY permission.
        window.eval_with_callback(
            "JSON.stringify({origin: location.origin, rendered: !!document.querySelector('.terminal-window-shell'), connected: document.body.innerText.includes('Local terminal access is disabled or has no active workspace.')})",
            move |value| {
                if let Some(send) = send.lock().expect("callback").take() {
                    let _ = send.send(value);
                }
            },
        ).map_err(|error| error.to_string())?;
        let value = receive.await.map_err(|error| error.to_string())?;
        let encoded: String = serde_json::from_str(&value).map_err(|error| error.to_string())?;
        let state: serde_json::Value =
            serde_json::from_str(&encoded).map_err(|error| error.to_string())?;
        if state["rendered"] == true && state["connected"] == true {
            if state["origin"] != "http://colossus-terminal.localhost" {
                return Err(format!("unexpected terminal origin: {}", state["origin"]));
            }
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(debug_assertions)]
fn tui_fixture() -> (
    tempfile::TempDir,
    SettingsStore,
    crate::desktop_settings::DesktopSettings,
) {
    use crate::desktop_settings::{
        DesktopSettings, LOCAL_TERMINAL_CONSENT_VERSION, ModelCapabilitiesSetting, ModelSetting,
        ProviderKindSetting, ProviderSetting, validate_workspace,
    };

    let parent = std::env::var_os("LOCALAPPDATA").expect("LocalAppData test parent");
    let root = tempfile::Builder::new()
        .prefix("ColossusTerminalTest-")
        .tempdir_in(parent)
        .expect("isolated test root");
    let home = root.path().join("home");
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).expect("test workspace");
    let store = SettingsStore::open_test_home(home.clone()).expect("isolated Desktop home");
    let mut settings = DesktopSettings {
        terminal_enabled: true,
        local_terminal_consent_version: LOCAL_TERMINAL_CONSENT_VERSION,
        providers: vec![ProviderSetting {
            profile: "fixture".into(),
            kind: ProviderKindSetting::Compatible,
            base_url: "http://127.0.0.1:9/v1".into(),
            credential_required: false,
            credential_id: None,
            timeout_ms: Some(1_000),
        }],
        models: vec![ModelSetting {
            profile: "primary".into(),
            provider_profile: "fixture".into(),
            model: "terminal-fixture".into(),
            context_window_tokens: 32768,
            max_output_tokens: 1024,
            capabilities: ModelCapabilitiesSetting {
                tool_calls: true,
                streaming: false,
                image_inputs: false,
            },
            reasoning_effort: None,
        }],
        model_roles: [("primary".into(), "primary".into())].into(),
        ..DesktopSettings::default()
    };
    settings
        .add_space(validate_workspace(&workspace).expect("workspace identity"))
        .expect("test workspace settings");
    store.save(&settings).expect("save fixture");
    (root, store, settings)
}

#[cfg(debug_assertions)]
#[tokio::test]
#[ignore = "requires prepared debug Desktop CLI/sidecar binaries and loopback access"]
async fn native_terminal_authenticated_tui_draws_through_conpty() {
    let (_root, store, settings) = tui_fixture();
    let state = AppState::default();
    state
        .select_target(settings.selected_target_id.clone())
        .await;
    let started = crate::managed_runtime::start(&state, &store, &settings, false).await;
    let result = if let Err(error) = started {
        Err(error.message)
    } else {
        let (_, workspace, _) = state.terminal_workspace_context().await;
        let manager = state.terminal_manager();
        tauri::async_runtime::spawn_blocking(move || {
            let workspace = workspace.ok_or("terminal workspace unavailable")?;
            let (send, receive) = std::sync::mpsc::sync_channel(128);
            let session = manager
                .open(
                    TERMINAL_WEBVIEW,
                    &workspace,
                    TerminalKind::ColossusTui,
                    30,
                    120,
                    Arc::new(move |event| send.try_send(event).is_ok()),
                )
                .map_err(TerminalError::message)?;
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            let mut output = Vec::new();
            let rendered = loop {
                let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now())
                else {
                    break Err("TUI did not render before the deadline");
                };
                match receive.recv_timeout(remaining) {
                    Ok(TerminalEvent::Output { bytes, .. }) => {
                        output.extend_from_slice(&bytes);
                        if String::from_utf8_lossy(&output).contains("COLOSSUS") {
                            break Ok(());
                        }
                        if output.len() > 1024 * 1024 {
                            break Err("unexpectedly large TUI startup output");
                        }
                    }
                    _ => break Err("TUI exited or failed before rendering"),
                }
            };
            manager
                .close(TERMINAL_WEBVIEW, &session)
                .map_err(TerminalError::message)?;
            rendered
        })
        .await
        .expect("TUI test task")
        .map_err(str::to_owned)
    };
    state.close_all().await;
    // Remove only the generated home, after closing the actual CLI and worker.
    crate::uninstall::cleanup(store.home_root().expect("test home"))
        .expect("clean up test Desktop home");
    result.expect("authenticated Windows TUI rendered through ConPTY");
}

#[cfg(debug_assertions)]
#[test]
#[ignore = "requires Windows WebView2, bundled frontend and prepared debug sidecars"]
fn native_terminal_pane_keeps_pty_and_rejects_main_authority() {
    let profile = tempfile::tempdir().expect("isolated controller profile");
    let mut context = crate::app_context::create();
    context.config_mut().build.dev_url = None;
    context.config_mut().app.windows[0].data_directory = Some(profile.path().to_owned());
    let outcome = Arc::new(Mutex::new(None));
    let result = outcome.clone();
    let application = tauri::Builder::default()
        .any_thread()
        .manage(AppState::default())
        .manage(pane::TerminalPaneState::default())
        .invoke_handler(tauri::generate_handler![
            terminal_context,
            open_terminal,
            write_terminal,
            resize_terminal,
            signal_terminal,
            close_terminal,
            pane::mount_terminal_pane,
            pane::terminal_pane_viewport
        ])
        .setup(move |app| {
            let app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let (_root, store, settings) = tui_fixture();
                let state = app.state::<AppState>();
                state
                    .select_target(settings.selected_target_id.clone())
                    .await;
                let checked = async {
                    crate::managed_runtime::start(&state, &store, &settings, false)
                        .await
                        .map_err(|e| e.message)?;
                    let main = app.get_webview("main").ok_or("main view missing")?;
                    assert!(
                        terminal_context(main.clone(), app.state()).await.is_err(),
                        "main acquired PTY authority"
                    );
                    check_stale_pane_request(&app, &main, Some("old-workspace".into())).await?;
                    assert!(app.get_webview(TERMINAL_WEBVIEW).is_none());
                    let lease = pane::mount_terminal_pane(
                        app.clone(),
                        main.clone(),
                        app.state(),
                        app.state(),
                        None,
                        0,
                        settings.selected_target_id.clone(),
                    )
                    .await
                    .map_err(|e| e.message)?;
                    let view = app
                        .get_webview(TERMINAL_WEBVIEW)
                        .ok_or("terminal child missing")?;
                    assert_eq!(view.window().label(), "main");
                    check_pane_viewport(&app, &main, &view, lease).await?;
                    check_pane_remount(&app, main.clone(), lease).await?;
                    state.select_target(None).await;
                    check_stale_pane_request(&app, &main, settings.selected_target_id.clone())
                        .await?;
                    assert!(
                        !state
                            .terminal_manager()
                            .has_owner_sessions(TERMINAL_WEBVIEW)
                            .unwrap(),
                        "workspace change retained terminal authority"
                    );
                    Ok::<(), String>(())
                }
                .await;
                state.close_all().await;
                crate::uninstall::cleanup(store.home_root().expect("test home"))
                    .expect("test cleanup");
                *result.lock().expect("test result") = Some(checked);
                app.exit(0);
            });
            Ok(())
        })
        .build(context)
        .expect("native terminal pane test app");
    application.run_return(|_, _| {});
    outcome
        .lock()
        .expect("test result")
        .take()
        .expect("test finished")
        .expect("native terminal pane");
}

#[cfg(debug_assertions)]
async fn check_pane_viewport(
    app: &AppHandle,
    main: &Webview,
    view: &Webview,
    lease: u64,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if state
            .terminal_manager()
            .has_owner_sessions(TERMINAL_WEBVIEW)
            .map_err(|e| e.message().to_owned())?
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let (send, receive) = tokio::sync::oneshot::channel();
            let send = Mutex::new(Some(send));
            view.eval_with_callback(
                "JSON.stringify({url:location.href,text:document.body.innerText})",
                move |value| {
                    if let Some(send) = send.lock().unwrap().take() {
                        let _ = send.send(value);
                    }
                },
            )
            .unwrap();
            let diagnostic = tokio::time::timeout(Duration::from_secs(3), receive).await;
            return Err(format!(
                "terminal child did not open its authenticated PTY: {diagnostic:?}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    main.window().hide().unwrap();
    main.window().show().unwrap();
    main.window().set_focus().unwrap();
    main.set_focus().unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    let active = colossus_native_browser::is_active(main)
        .await
        .map_err(|e| e.to_string())?;
    // Compact layout starts directly below the shared 48px header.
    pane::terminal_pane_viewport(
        app.clone(),
        main.clone(),
        app.state(),
        app.state(),
        serde_json::from_value(serde_json::json!({
            "epoch": lease,
            "rect": {"x": 0, "y": 48, "width": 700, "height": 500}
        }))
        .unwrap(),
    )
    .await
    .map_err(|e| e.message)?;
    assert_eq!(
        app.state::<pane::TerminalPaneState>().visible_lease(),
        active
    );
    if active {
        assert!(
            (view
                .position()
                .unwrap()
                .to_logical::<f64>(main.window().scale_factor().unwrap())
                .y
                - 48.0)
                .abs()
                < 0.01
        );
    }
    Ok(())
}

#[cfg(debug_assertions)]
async fn check_pane_remount(app: &AppHandle, main: Webview, lease: u64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let next = pane::mount_terminal_pane(
        app.clone(),
        main.clone(),
        app.state(),
        app.state(),
        None,
        0,
        state.selected_target_id().await,
    )
    .await
    .map_err(|e| e.message)?;
    assert_ne!(lease, next);
    assert_eq!(
        app.webviews().len(),
        2,
        "switching tools created another terminal"
    );
    pane::terminal_pane_viewport(
        app.clone(),
        main,
        app.state(),
        app.state(),
        serde_json::from_value(serde_json::json!({"epoch": lease, "rect": null})).unwrap(),
    )
    .await
    .map_err(|e| e.message)?;
    assert!(
        state
            .terminal_manager()
            .has_owner_sessions(TERMINAL_WEBVIEW)
            .unwrap(),
        "switching tools closed the PTY"
    );
    Ok(())
}

#[cfg(debug_assertions)]
async fn check_stale_pane_request(
    app: &AppHandle,
    main: &Webview,
    scope: Option<String>,
) -> Result<(), String> {
    let request = serde_json::from_value(serde_json::json!({
        "kind": "colossus_tui", "sessionId": "session-A", "planId": "plan-A"
    }))
    .unwrap();
    let error = pane::mount_terminal_pane(
        app.clone(),
        main.clone(),
        app.state(),
        app.state(),
        Some(request),
        42,
        scope,
    )
    .await
    .expect_err("stale workspace queued a terminal launch");
    assert_eq!(error.code, "terminal_workspace_changed");
    Ok(())
}

#[cfg(debug_assertions)]
fn shell_fixture() -> (tempfile::TempDir, TerminalWorkspace) {
    use crate::desktop_settings::revalidate_workspace;
    let (root, store, settings) = tui_fixture();
    let selected = settings.workspace.as_ref().expect("selected workspace");
    let workspace = TerminalWorkspace {
        id: selected.id.clone(),
        display_name: selected.display_name.clone(),
        workspace: revalidate_workspace(selected).expect("bound workspace"),
        workspace_identity: selected.identity.clone().expect("identity"),
        colossus_home: store.home_root().unwrap().to_owned(),
        config: None,
        worker_authentication: None,
    };
    (root, workspace)
}

#[cfg(debug_assertions)]
#[test]
fn native_powershell_runs_commands_in_workspace_without_worker_authority() {
    use crate::terminal::TerminalManager;
    let (_root, workspace) = shell_fixture();
    let manager = TerminalManager::default();
    let mut privileged = workspace.clone();
    privileged.config = Some(workspace.workspace.join("unused.yaml"));
    assert_eq!(
        manager.open(
            TERMINAL_WEBVIEW,
            &privileged,
            TerminalKind::Shell,
            24,
            80,
            Arc::new(|_| true)
        ),
        Err(TerminalError::InvalidConfiguration)
    );
    let (send, receive) = std::sync::mpsc::sync_channel(256);
    let session = manager
        .open(
            TERMINAL_WEBVIEW,
            &workspace,
            TerminalKind::Shell,
            30,
            140,
            Arc::new(move |event| send.try_send(event).is_ok()),
        )
        .expect("real Windows PowerShell starts without a managed runtime or CLI");
    manager
        .resize(TERMINAL_WEBVIEW, &session, 40, 160)
        .expect("resize shell");
    // Construct markers in the shell, so echoed commands cannot satisfy assertions.
    let expected = workspace
        .workspace
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\'', "''");
    // Encode a physical Enter with its virtual key and scan code. A bare CR
    // makes ConPTY infer the key through the host keyboard layout, which is not
    // reliable on headless Windows Server runners. This does not bypass the PTY.
    let enter = "\x1b[13;28;13;1;0;1_\x1b[13;28;13;0;0;1_";
    let command = format!(
        "if ((Get-Location).Path -ne '{expected}') {{ exit 71 }}; if (Test-Path Env:COLOSSUS_DESKTOP_TUI_AUTH_INPUT_HANDLE_V1) {{ exit 72 }}; if (Test-Path Env:COLOSSUS_DESKTOP_TUI_AUTH_OUTPUT_HANDLE_V1) {{ exit 73 }}; Write-Output ('SHELL_' + (20 + 22)); exit 0{enter}"
    );
    // Wait for the interactive prompt before typing the user command.
    let prompt = format!(
        "PS {}>",
        workspace
            .workspace
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
    );
    let mut submitted = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut output = Vec::new();
    let result = loop {
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            break Err("PowerShell did not finish before the deadline".to_owned());
        };
        match receive.recv_timeout(remaining) {
            Ok(TerminalEvent::Output { bytes, .. }) => {
                // A real xterm answers cursor queries even when a query spans
                // multiple pipe reads. Include the previous three bytes, but do
                // not answer a query twice.
                let scan_from = output.len().saturating_sub(3);
                output.extend_from_slice(&bytes);
                for _ in output[scan_from..]
                    .windows(4)
                    .filter(|sequence| *sequence == b"\x1b[6n")
                {
                    manager
                        .write(TERMINAL_WEBVIEW, &session, b"\x1b[1;1R")
                        .unwrap();
                }
                if !submitted
                    && output
                        .windows(prompt.len())
                        .any(|sequence| sequence == prompt.as_bytes())
                {
                    manager
                        .write(TERMINAL_WEBVIEW, &session, command.as_bytes())
                        .expect("write user command after the shell prompt");
                    submitted = true;
                }
            }
            Ok(TerminalEvent::Exited {
                exit_code: Some(0), ..
            }) => {
                break if String::from_utf8_lossy(&output).contains("SHELL_42") {
                    Ok(())
                } else {
                    Err("PowerShell did not evaluate the user command".to_owned())
                };
            }
            event => break Err(format!("PowerShell failed: {event:?}")),
        }
    };
    manager.close_owner(TERMINAL_WEBVIEW);
    assert!(
        result.is_ok(),
        "{result:?}; command submitted: {submitted}; output: {}",
        String::from_utf8_lossy(&output)
    );
    assert!(!manager.has_owner_sessions(TERMINAL_WEBVIEW).unwrap());
}
