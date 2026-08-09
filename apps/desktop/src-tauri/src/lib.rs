mod bundle;
mod commands;
mod connection;
mod desktop_commands;
mod desktop_dto;
mod desktop_settings;
mod diagnostics;
mod dto;
mod managed_runtime;
mod provider_enrollment;
mod state;
mod terminal;
mod terminal_commands;
mod terminal_process;
mod terminal_protocol;
mod updates;
mod workspace_files;

use commands::{
    cancel_run, choose_run_attachment, create_run, get_run, list_runs, read_artifact_content,
    respond_interaction, watch_run,
};
use desktop_commands::{
    add_external_target, apply_managed_model_configuration, choose_workspace,
    configure_managed_runtime, connect_colossus, connection_status, desktop_release_channel,
    desktop_status, import_ca_bundle, initialize_desktop, remove_ca_bundle, remove_external_target,
    restart_managed_runtime, run_managed_self_test, select_target, set_approval_mode,
    set_terminal_enabled,
};
use diagnostics::{desktop_release_metadata, export_diagnostics};
use terminal_commands::{
    close_terminal, open_terminal, resize_terminal, show_terminal_window, signal_terminal,
    terminal_context, write_terminal,
};
use updates::{check_desktop_update, install_desktop_update};
use workspace_files::{list_workspace_directory, read_workspace_file};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Start the native Colossus desktop application.
///
/// # Panics
///
/// Panics when Tauri cannot initialize or run the application event loop.
pub fn run() {
    let application = tauri::Builder::default()
        .register_uri_scheme_protocol(terminal_protocol::SCHEME, |context, request| {
            terminal_protocol::respond(&context, &request)
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            desktop_release_channel,
            desktop_release_metadata,
            check_desktop_update,
            install_desktop_update,
            export_diagnostics,
            initialize_desktop,
            desktop_status,
            import_ca_bundle,
            remove_ca_bundle,
            add_external_target,
            remove_external_target,
            choose_workspace,
            configure_managed_runtime,
            apply_managed_model_configuration,
            restart_managed_runtime,
            run_managed_self_test,
            select_target,
            set_approval_mode,
            set_terminal_enabled,
            connect_colossus,
            connection_status,
            create_run,
            choose_run_attachment,
            read_artifact_content,
            get_run,
            list_runs,
            watch_run,
            cancel_run,
            respond_interaction,
            list_workspace_directory,
            read_workspace_file,
            show_terminal_window,
            terminal_context,
            open_terminal,
            write_terminal,
            resize_terminal,
            signal_terminal,
            close_terminal,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the Colossus desktop application");
    application.run(|app, event| {
        if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
            use tauri::Manager as _;

            tauri::async_runtime::block_on(app.state::<state::AppState>().close_all());
        }
    });
}
