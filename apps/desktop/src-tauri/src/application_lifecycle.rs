//! Controlled application shutdown while native close callbacks keep pumping.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

use tauri::Manager as _;

pub(crate) fn callback() -> (
    impl FnMut(&tauri::AppHandle, tauri::RunEvent),
    Arc<AtomicBool>,
) {
    let shutdown = Arc::new(AtomicU8::new(0));
    let acknowledged = Arc::new(AtomicBool::new(false));
    let shutdown_result = acknowledged.clone();
    let callback = move |app: &tauri::AppHandle, event| {
        #[cfg(target_os = "macos")]
        if matches!(
            event,
            tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            }
        ) {
            crate::status_bar::show_main_window(app);
        }
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            #[cfg(feature = "embedded-chromium-preview")]
            eprintln!(
                "Desktop ExitRequested received (shutdown phase {})",
                shutdown.load(Ordering::Acquire)
            );
            if shutdown.load(Ordering::Acquire) < 2 {
                api.prevent_exit();
                if shutdown
                    .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    app.state::<crate::dictation::DictationState>().cancel();
                    let app = app.clone();
                    let shutdown = shutdown.clone();
                    tauri::async_runtime::spawn(async move {
                        // Keep Tauri's UI thread alive for actual native close
                        // acknowledgements; blocking it would deadlock CEF.
                        match app
                            .state::<crate::state::AppState>()
                            .close_all_settled()
                            .await
                        {
                            Ok(()) => {
                                #[cfg(feature = "embedded-chromium-preview")]
                                eprintln!("Desktop native browser close callbacks settled");
                                shutdown.store(2, Ordering::Release);
                                app.exit(0);
                            }
                            Err(error) => {
                                eprintln!("Desktop shutdown did not settle: {}", error.message);
                                // The app and event pump remain live. A later
                                // explicit Quit can reconcile delayed closes.
                                shutdown.store(0, Ordering::Release);
                            }
                        }
                    });
                }
            }
        } else if matches!(event, tauri::RunEvent::Exit) {
            #[cfg(feature = "embedded-chromium-preview")]
            eprintln!("Desktop Exit received; settling native browser lifecycle");
            match crate::browser::bootstrap::shutdown() {
                Ok(()) => {
                    shutdown_result.store(true, Ordering::Release);
                    #[cfg(feature = "embedded-chromium-preview")]
                    println!("Desktop native browser lifecycle settled");
                }
                Err(error) => eprintln!("native browser shutdown failed: {error}"),
            }
        }
    };
    (callback, acknowledged)
}
