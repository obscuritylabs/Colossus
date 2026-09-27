//! Shared Tauri context for the application and its native acceptance harness.

pub(crate) fn create() -> tauri::Context<tauri::Wry> {
    // On macOS this macro embeds a process-wide Info.plist symbol. Expand it
    // once even when the application and acceptance entry points are linked.
    tauri::generate_context!()
}
