mod commands;
mod connection;
mod dto;
mod state;

use commands::{
    cancel_run, connect_colossus, connection_status, create_run, get_run, list_runs,
    respond_interaction, watch_run,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Start the native Colossus desktop application.
///
/// # Panics
///
/// Panics when Tauri cannot initialize or run the application event loop.
pub fn run() {
    tauri::Builder::default()
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            connect_colossus,
            connection_status,
            create_run,
            get_run,
            list_runs,
            watch_run,
            cancel_run,
            respond_interaction,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the Colossus desktop application");
}
