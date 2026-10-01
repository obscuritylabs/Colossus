//! Real second-process activation, isolated from the installed app and its data.
use std::{
    os::windows::process::CommandExt as _,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{Manager as _, WebviewUrl, WebviewWindowBuilder};

const FIXTURE_ID: &str = "COLOSSUS_INSTANCE_ACCEPTANCE_ID";

#[test]
#[ignore = "subprocess entry point for native_single_instance_restores_original_window"]
fn secondary_launch() {
    let id = std::env::var(FIXTURE_ID).expect("launched only by the acceptance test");
    let guard = colossus_windows_native::DesktopLaunchGuard::acquire(&id).unwrap();
    let mut context = crate::app_context::create();
    context.config_mut().identifier = id;
    context.config_mut().app.windows.clear();
    let _app = tauri::Builder::default()
        .any_thread()
        .plugin(super::plugin())
        .build(context)
        .unwrap();
    drop(guard);
    panic!("a second Desktop instance reached application setup");
}

#[test]
#[ignore = "requires Windows WebView2 and an interactive desktop"]
fn native_single_instance_restores_original_window() {
    let profile = tempfile::tempdir().unwrap();
    let id = format!("com.colossus.instance-test.{}", uuid::Uuid::now_v7());
    let guard = colossus_windows_native::DesktopLaunchGuard::acquire(&id).unwrap();
    let mut context = crate::app_context::create();
    context.config_mut().identifier.clone_from(&id);
    context.config_mut().app.windows.clear();
    let result = Arc::new(Mutex::new(None));
    let outcome = result.clone();
    let directory = profile.path().to_owned();
    let app = tauri::Builder::default()
        .any_thread()
        .plugin(super::plugin())
        .setup(move |app| {
            let window = WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External("about:blank".parse()?),
            )
            .data_directory(directory)
            .title("Colossus instance acceptance")
            .inner_size(360.0, 160.0)
            .visible(false)
            .build()?;
            let app = app.handle().clone();
            std::thread::spawn(move || {
                let checked = check_activation(&id, &window);
                *outcome.lock().unwrap() = Some(checked);
                app.exit(0);
            });
            Ok(())
        })
        .build(context)
        .unwrap();
    drop(guard);
    app.run_return(|_, _| {});
    result
        .lock()
        .unwrap()
        .take()
        .expect("activation test completed")
        .unwrap();
}

fn check_activation(id: &str, window: &tauri::WebviewWindow) -> Result<(), String> {
    let timeout = Duration::from_secs(20);
    for minimized in [false, true] {
        if minimized {
            window.minimize().map_err(|e| e.to_string())?;
        } else {
            window.hide().map_err(|e| e.to_string())?;
        }
        let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .args([
                "--exact",
                "desktop_instance::native_tests::secondary_launch",
                "--ignored",
                "--nocapture",
            ])
            .env(FIXTURE_ID, id)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW: never create a console for the helper.
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                if !status.success() {
                    return Err(format!("second launch did not exit cleanly: {status}"));
                }
                break;
            }
            if start.elapsed() >= timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err("second launch did not hand off to the original process".into());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        while !window.is_visible().map_err(|e| e.to_string())?
            || window.is_minimized().map_err(|e| e.to_string())?
        {
            if start.elapsed() >= timeout {
                return Err("original window was not restored".into());
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        if window.app_handle().webview_windows().len() != 1 {
            return Err("activation created another main window".into());
        }
    }
    Ok(())
}
