mod bundle;
mod codex_auth;
mod commands;
mod connection;
mod desktop_commands;
mod desktop_dto;
mod desktop_settings;
mod diagnostics;
mod dto;
mod managed_runtime;
mod provider_enrollment;
mod run_list;
mod space_search;
mod state;
mod terminal;
mod terminal_commands;
mod terminal_process;
mod terminal_protocol;
mod updates;
mod workspace_files;

use codex_auth::{codex_auth_login, codex_auth_logout, codex_auth_status};
use commands::{
    archive_thread, cancel_run, choose_run_attachment, create_run, get_run, list_asides, list_runs,
    read_artifact_content, respond_interaction, restore_thread, watch_run,
};
use desktop_commands::{
    add_external_target, apply_managed_model_configuration, archive_space, choose_workspace,
    configure_managed_runtime, connect_colossus, connection_status, create_space,
    desktop_release_channel, desktop_status, get_session_map, get_thread_delegate,
    import_ca_bundle, initialize_desktop, list_spaces, remove_ca_bundle, remove_external_target,
    rename_space, restart_managed_runtime, restore_space, run_managed_self_test,
    search_space_threads, select_space, select_target, set_approval_mode, set_terminal_enabled,
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
    if let Err(error) = desktop_settings::SettingsStore::open_application() {
        eprintln!("Colossus Desktop could not start: {}", error.message);
        std::process::exit(1);
    }
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
            codex_auth_status,
            codex_auth_login,
            codex_auth_logout,
            import_ca_bundle,
            remove_ca_bundle,
            add_external_target,
            remove_external_target,
            choose_workspace,
            create_space,
            list_spaces,
            select_space,
            rename_space,
            archive_space,
            restore_space,
            search_space_threads,
            configure_managed_runtime,
            apply_managed_model_configuration,
            restart_managed_runtime,
            run_managed_self_test,
            get_thread_delegate,
            get_session_map,
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
            list_asides,
            watch_run,
            cancel_run,
            archive_thread,
            restore_thread,
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
