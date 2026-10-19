//! Restore the owning Desktop process rather than starting another tray/runtime.

#[cfg(windows)]
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
#[cfg(windows)]
use tauri::Manager as _;

#[cfg(windows)]
struct InstanceState(Mutex<Option<colossus_windows_native::DesktopInstanceOwner>>);

#[cfg(windows)]
pub(crate) fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("desktop-instance")
        .setup(|app, _| {
            // Startup's native guard stays held until build() completes. The
            // callback only queues the existing restore operation: invocation
            // arguments, paths and renderer input never cross this boundary.
            let target = app.clone();
            let pending = Arc::new(AtomicBool::new(false));
            let instance = colossus_windows_native::DesktopInstance::install(
                &app.config().identifier,
                move || {
                    if pending.swap(true, Ordering::AcqRel) {
                        return true;
                    }
                    let window_app = target.clone();
                    let completed = Arc::clone(&pending);
                    if target
                        .run_on_main_thread(move || {
                            crate::status_bar::show_main_window(&window_app);
                            completed.store(false, Ordering::Release);
                        })
                        .is_err()
                    {
                        pending.store(false, Ordering::Release);
                        return false;
                    }
                    true
                },
            )?;
            match instance {
                colossus_windows_native::DesktopInstance::Primary(owner) => {
                    app.manage(InstanceState(Mutex::new(Some(owner))));
                }
                colossus_windows_native::DesktopInstance::Secondary => {
                    app.cleanup_before_exit();
                    std::process::exit(0);
                }
            }
            Ok(())
        })
        .on_event(|app, event| {
            if !matches!(event, tauri::RunEvent::Exit) {
                return;
            }
            let Some(state) = app.try_state::<InstanceState>() else {
                return;
            };
            let owner = state.0.lock().ok().and_then(|mut value| value.take());
            if let Some(mut owner) = owner
                && owner.stop().is_err()
            {
                // Keep custody after a failed stop; process exit ultimately
                // closes these own-process resources. Never claim an ACK.
                eprintln!("Colossus Desktop activation cleanup was not acknowledged.");
                if let Ok(mut value) = state.0.lock() {
                    *value = Some(owner);
                }
            }
        })
        .build()
}

#[cfg(all(test, windows))]
mod native_tests;
