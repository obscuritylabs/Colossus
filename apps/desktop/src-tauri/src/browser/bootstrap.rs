//! Entry-point CEF installation and nonblocking UI-thread message pumping.

#[cfg(feature = "embedded-chromium-preview")]
use colossus_native_browser::chromium;
#[cfg(any(feature = "embedded-chromium-preview", test))]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "embedded-chromium-preview")]
static INITIALIZED: AtomicBool = AtomicBool::new(false);

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
        let helper = helper_path(&executable)?;
        match chromium::bootstrap(cache.path(), &helper) {
            chromium::Bootstrap::SubprocessExit(code) => std::process::exit(code),
            chromium::Bootstrap::Ready => {
                INITIALIZED.store(true, Ordering::Release);
                Some(cache)
            }
            chromium::Bootstrap::Unavailable(_) => None,
        }
    }
    #[cfg(not(feature = "embedded-chromium-preview"))]
    None
}

#[cfg(any(feature = "embedded-chromium-preview", all(test, target_os = "macos")))]
fn helper_path(executable: &std::path::Path) -> Option<std::path::PathBuf> {
    let directory = executable.parent()?;
    #[cfg(target_os = "macos")]
    {
        // CEF requires a signed helper app inside the application bundle. A
        // sibling command-line helper cannot establish that sandbox topology.
        if directory.file_name()? != "MacOS" {
            return None;
        }
        let contents = directory.parent()?;
        if contents.file_name()? != "Contents" {
            return None;
        }
        Some(
            contents.join(
                "Frameworks/Colossus Browser Helper.app/Contents/MacOS/Colossus Browser Helper",
            ),
        )
    }
    #[cfg(not(target_os = "macos"))]
    Some(directory.join(if cfg!(windows) {
        "colossus-browser-helper.exe"
    } else {
        "colossus-browser-helper"
    }))
}

#[cfg_attr(
    not(feature = "embedded-chromium-preview"),
    allow(
        clippy::unnecessary_wraps,
        reason = "Ordinary builds preserve the fallible preview lifecycle API."
    )
)]
pub(crate) fn start_pump(
    app: &tauri::AppHandle,
) -> Result<(), colossus_native_browser::BrowserError> {
    #[cfg(feature = "embedded-chromium-preview")]
    if chromium::readiness().is_ok() {
        let quit = app.clone();
        chromium::bind_quit_handler(move || {
            eprintln!("Desktop native Quit handler received AppKit request");
            let quit = quit.clone();
            // AppKit invokes terminate: from its main-thread event dispatch.
            // Queue the Tauri exit request from a worker so ExitRequested can
            // settle native browser closes before the event loop exits.
            tauri::async_runtime::spawn(async move {
                eprintln!("Desktop native Quit worker requests Tauri exit");
                quit.exit(0);
            });
        })?;
        eprintln!("Desktop native Quit handler bound");
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
    Ok(())
}

#[cfg_attr(
    not(feature = "embedded-chromium-preview"),
    allow(
        clippy::unnecessary_wraps,
        reason = "Ordinary builds preserve the fallible preview lifecycle API."
    )
)]
pub(crate) fn shutdown() -> Result<(), colossus_native_browser::BrowserError> {
    #[cfg(feature = "embedded-chromium-preview")]
    return shutdown_initialized(&INITIALIZED, chromium::shutdown);
    #[cfg(not(feature = "embedded-chromium-preview"))]
    Ok(())
}

#[cfg(any(feature = "embedded-chromium-preview", test))]
fn shutdown_initialized(
    initialized: &AtomicBool,
    shutdown: impl FnOnce() -> Result<(), colossus_native_browser::BrowserError>,
) -> Result<(), colossus_native_browser::BrowserError> {
    if !initialized.load(Ordering::Acquire) {
        // An unavailable preview owns no initialized engine or native cache.
        return Ok(());
    }
    // Preserve the obligation on any failure: an initialized engine must never
    // be reclassified as a no-op merely because its teardown was rejected.
    shutdown()?;
    initialized.store(false, Ordering::Release);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::helper_path;
    use super::{AtomicBool, Ordering, shutdown_initialized};
    use colossus_native_browser::BrowserError;
    #[cfg(target_os = "macos")]
    use std::path::Path;

    #[test]
    fn absent_browser_bootstrap_requires_no_native_shutdown() {
        let initialized = AtomicBool::new(false);
        assert_eq!(
            shutdown_initialized(&initialized, || panic!("no engine exists")),
            Ok(())
        );
    }

    #[test]
    fn failed_native_shutdown_preserves_the_acknowledgement_obligation() {
        let initialized = AtomicBool::new(true);
        assert_eq!(
            shutdown_initialized(&initialized, || Err(BrowserError::Unavailable)),
            Err(BrowserError::Unavailable)
        );
        assert!(initialized.load(Ordering::Acquire));
        assert_eq!(shutdown_initialized(&initialized, || Ok(())), Ok(()));
        assert!(!initialized.load(Ordering::Acquire));
        assert_eq!(
            shutdown_initialized(&initialized, || panic!("already shut down")),
            Ok(())
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn helper_is_discovered_in_the_signed_frameworks_bundle() {
        assert_eq!(
            helper_path(Path::new("/private/Colossus Preview.app/Contents/MacOS/colossus-desktop")),
            Some(Path::new("/private/Colossus Preview.app/Contents/Frameworks/Colossus Browser Helper.app/Contents/MacOS/Colossus Browser Helper").to_path_buf())
        );
        assert!(helper_path(Path::new("/private/target/debug/colossus-desktop")).is_none());
        assert!(helper_path(Path::new("/private/Other/MacOS/colossus-desktop")).is_none());
    }
}
