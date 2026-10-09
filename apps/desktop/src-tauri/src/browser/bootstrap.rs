//! Entry-point CEF installation and nonblocking UI-thread message pumping.

#[cfg(feature = "embedded-chromium-preview")]
use colossus_native_browser::chromium;

/// Own the preview's private cache for the entire native application lifecycle.
pub(crate) fn initialize() -> Option<tempfile::TempDir> {
    #[cfg(feature = "embedded-chromium-preview")]
    {
        let store = crate::desktop_settings::SettingsStore::open_application().ok()?;
        let cache = tempfile::Builder::new()
            .prefix("browser-session-cef-")
            .tempdir_in(store.application_root())
            .ok()?;
        let executable = std::env::current_exe().ok()?;
        let helper = executable.parent()?.join(if cfg!(windows) {
            "colossus-browser-helper.exe"
        } else {
            "colossus-browser-helper"
        });
        match chromium::bootstrap(cache.path(), &helper) {
            chromium::Bootstrap::SubprocessExit(code) => std::process::exit(code),
            chromium::Bootstrap::Ready => Some(cache),
            chromium::Bootstrap::Unavailable(_) => None,
        }
    }
    #[cfg(not(feature = "embedded-chromium-preview"))]
    None
}

pub(crate) fn start_pump(app: &tauri::AppHandle) {
    #[cfg(feature = "embedded-chromium-preview")]
    if chromium::readiness().is_ok() {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let (sender, receive) = tokio::sync::oneshot::channel();
                if app
                    .run_on_main_thread(move || {
                        chromium::pump();
                        let _ = sender.send(());
                    })
                    .is_err()
                {
                    break;
                }
                // At most one main-thread pump is queued. A stalled Tauri loop
                // cannot accumulate unbounded timer callbacks.
                if receive.await.is_err() {
                    break;
                }
            }
        });
    }
    #[cfg(not(feature = "embedded-chromium-preview"))]
    let _ = app;
}

pub(crate) fn shutdown() {
    #[cfg(feature = "embedded-chromium-preview")]
    chromium::shutdown();
}
