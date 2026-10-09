//! Restore the owning Desktop process rather than starting another tray/runtime.
#[cfg(windows)]
pub(crate) fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    // Invocation arguments never become commands or renderer input.
    tauri_plugin_single_instance::init(|app, _args, _cwd| {
        crate::status_bar::show_main_window(app);
    })
}

#[cfg(all(test, windows))]
mod native_tests;
