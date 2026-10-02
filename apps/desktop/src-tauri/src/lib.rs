mod app_context;
mod approval_adapter;
mod browser;
mod bundle;
mod codex_auth;
mod command_review;
mod command_review_protocol;
mod commands;
mod process_session_commands;
use process_session_commands::{list_shell_sessions, read_shell_session, stop_shell_session};
mod configuration_import;
mod connection;
mod desktop_commands;
mod desktop_credentials;
mod desktop_dto;
mod desktop_instance;
mod desktop_settings;
mod diagnostics;
mod dto;
mod managed_configuration;
mod managed_configuration_commands;
mod managed_diagnostics;
mod managed_runtime;
mod mcp_health;
mod plugin_adapter;
mod plugin_commands;
mod plugin_selection;
mod provider_catalog;
mod provider_enrollment;
mod remembered_approvals;
mod run_list;
mod setup_package;
mod space_search;
mod state;
mod status_bar;
mod terminal;
mod terminal_commands;
mod terminal_process;
mod terminal_protocol;
#[cfg(windows)]
mod uninstall;
mod updates;
mod workspace_files;
mod workspace_git;
mod workspace_search;

/// Run the opt-in native browser acceptance harness.
#[cfg(feature = "browser-test-bridge")]
#[must_use]
pub fn run_browser_acceptance() -> i32 {
    browser::acceptance::run()
}

use browser::commands::{browser_command, browser_context, browser_viewport};
use codex_auth::{codex_auth_login, codex_auth_logout, codex_auth_status};
use command_review::{command_review_context, finish_command_review};
use commands::{
    archive_thread, cancel_run, choose_run_attachment, create_run, get_run, list_asides, list_runs,
    list_session_activity, read_artifact_content, respond_interaction, restore_thread, watch_run,
};
use configuration_import::{apply_repository_configuration, inspect_repository_configuration};
use desktop_commands::{
    add_external_target, apply_managed_model_configuration, archive_space, choose_workspace,
    configure_managed_runtime, connect_colossus, connection_status, create_space,
    desktop_release_channel, desktop_status, get_session_map, get_thread_delegate,
    import_ca_bundle, import_client_identity, initialize_desktop, list_spaces, remove_ca_bundle,
    remove_client_identity, remove_external_target, rename_space, restart_managed_runtime,
    restore_space, run_managed_self_test, search_space_threads, select_space, select_target,
    set_approval_mode, set_terminal_enabled,
};
use diagnostics::{desktop_release_metadata, export_diagnostics};
use managed_configuration_commands::catalog_deletion::{
    delete_global_model, delete_global_provider,
};
use managed_configuration_commands::updates::sync_managed_configuration;
use managed_configuration_commands::{
    apply_space_configuration, create_managed_credential, delete_global_mcp_server,
    delete_managed_credential, get_managed_configuration, reenter_managed_credential,
    rotate_managed_credential, save_global_defaults, save_space_configuration,
    upsert_global_mcp_server, upsert_global_model, upsert_global_provider,
    upsert_global_search_provider, upsert_global_telemetry_profile,
};
use managed_diagnostics::{
    begin_managed_mcp_oauth, complete_managed_mcp_oauth, diagnose_managed_mcp_server,
    diagnose_managed_model, diagnose_managed_provider, diagnose_managed_search,
    diagnose_managed_telemetry, get_managed_extension_inventory, logout_managed_mcp_oauth,
    managed_mcp_oauth_status,
};
use plugin_commands::{
    cancel_plugin_operation, get_plugin_inventory, manage_plugin, read_plugin_preview,
};
use plugin_selection::resolve_plugin_selection;
use provider_catalog::{discover_managed_provider_models, get_provider_presets};
use remembered_approvals::{clear_remembered_commands, remembered_command_count};
use setup_package::{
    apply_setup_package, cancel_setup_package_review, configure_setup_credential,
    export_setup_package, inspect_setup_package, list_setup_packages, open_setup_link,
    remove_setup_package, use_setup_model,
};
use terminal_commands::pane::{mount_terminal_pane, terminal_pane_viewport};
use terminal_commands::{
    close_terminal, open_terminal, resize_terminal, show_terminal_window, signal_terminal,
    terminal_context, write_terminal,
};
use updates::{check_desktop_update, install_desktop_update};
use workspace_files::{list_workspace_directory, read_workspace_file};
use workspace_git::commands::{
    get_workspace_git_commit, get_workspace_git_diff, get_workspace_git_status,
    list_workspace_git_commits,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Start the native Colossus desktop application.
///
/// # Panics
///
/// Panics when Tauri cannot initialize or run the application event loop.
// Composition-only registration list: native command implementations stay in modules.
#[allow(clippy::too_many_lines)]
pub fn run() {
    #[cfg(windows)]
    if let Some(code) = uninstall::run_if_requested() {
        std::process::exit(code);
    }
    let context = app_context::create();
    #[cfg(windows)]
    let launch_guard =
        colossus_windows_native::DesktopLaunchGuard::acquire(&context.config().identifier)
            .expect("another Colossus launch did not finish");
    if let Err(error) = desktop_settings::SettingsStore::open_application() {
        eprintln!("Colossus Desktop could not start: {}", error.message);
        std::process::exit(1);
    }
    let application = tauri::Builder::default();
    #[cfg(windows)]
    let application = application.plugin(desktop_instance::plugin());
    let application = application
        .register_uri_scheme_protocol(command_review_protocol::SCHEME, |context, request| {
            command_review_protocol::respond(&context, &request)
        })
        .register_uri_scheme_protocol(terminal_protocol::SCHEME, |context, request| {
            terminal_protocol::respond(&context, &request)
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(any(target_os = "macos", windows))]
    let application = application.plugin(tauri_plugin_notification::init());
    let application = application
        .manage(state::AppState::default())
        .manage(terminal_commands::pane::TerminalPaneState::default())
        .manage(status_bar::StatusBarState::default())
        .manage(setup_package::SetupReviewState::default())
        .manage(command_review::CommandReviewState::default())
        .manage(workspace_git::commands::GitState::default())
        .manage(workspace_search::SearchState::default())
        .setup(|app| {
            status_bar::setup(app)?;
            browser::start_watchdog(app.handle().clone());
            terminal_commands::pane::start_watchdog(app.handle().clone());
            Ok(())
        })
        .on_page_load(|view, payload| {
            if view.label() == "main"
                && matches!(payload.event(), tauri::webview::PageLoadEvent::Started)
            {
                use tauri::Manager as _;
                view.state::<state::AppState>().browser.controller_loading();
                terminal_commands::pane::hide(view.app_handle(), true);
            }
        })
        .on_window_event(|window, event| {
            browser::handle_window_event(window, event);
            status_bar::handle_window_event(window, event);
        })
        .invoke_handler(tauri::generate_handler![
            mount_terminal_pane,
            terminal_pane_viewport,
            browser_context,
            browser_command,
            browser_viewport,
            command_review_context,
            finish_command_review,
            remembered_command_count,
            clear_remembered_commands,
            get_plugin_inventory,
            resolve_plugin_selection,
            read_plugin_preview,
            manage_plugin,
            cancel_plugin_operation,
            desktop_release_channel,
            desktop_release_metadata,
            check_desktop_update,
            install_desktop_update,
            export_diagnostics,
            open_setup_link,
            list_setup_packages,
            inspect_setup_package,
            cancel_setup_package_review,
            apply_setup_package,
            configure_setup_credential,
            use_setup_model,
            remove_setup_package,
            export_setup_package,
            initialize_desktop,
            desktop_status,
            codex_auth_status,
            codex_auth_login,
            codex_auth_logout,
            import_ca_bundle,
            import_client_identity,
            remove_ca_bundle,
            remove_client_identity,
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
            get_managed_configuration,
            inspect_repository_configuration,
            apply_repository_configuration,
            save_global_defaults,
            upsert_global_mcp_server,
            delete_global_mcp_server,
            delete_global_model,
            delete_global_provider,
            diagnose_managed_mcp_server,
            managed_mcp_oauth_status,
            begin_managed_mcp_oauth,
            complete_managed_mcp_oauth,
            logout_managed_mcp_oauth,
            diagnose_managed_provider,
            diagnose_managed_model,
            diagnose_managed_search,
            diagnose_managed_telemetry,
            get_managed_extension_inventory,
            upsert_global_provider,
            upsert_global_model,
            upsert_global_search_provider,
            upsert_global_telemetry_profile,
            save_space_configuration,
            apply_space_configuration,
            sync_managed_configuration,
            create_managed_credential,
            rotate_managed_credential,
            reenter_managed_credential,
            delete_managed_credential,
            configure_managed_runtime,
            discover_managed_provider_models,
            get_provider_presets,
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
            list_shell_sessions,
            read_shell_session,
            stop_shell_session,
            list_session_activity,
            list_asides,
            watch_run,
            cancel_run,
            archive_thread,
            restore_thread,
            respond_interaction,
            list_workspace_directory,
            get_workspace_git_status,
            list_workspace_git_commits,
            get_workspace_git_commit,
            get_workspace_git_diff,
            workspace_search::search_workspace_files,
            read_workspace_file,
            show_terminal_window,
            terminal_context,
            open_terminal,
            write_terminal,
            resize_terminal,
            signal_terminal,
            close_terminal,
            status_bar::sync_status_bar_pins,
            status_bar::notify_background,
        ])
        .build(context)
        .expect("failed to build the Colossus desktop application");
    #[cfg(windows)]
    drop(launch_guard);
    application.run(|app, event| {
        #[cfg(target_os = "macos")]
        if matches!(
            event,
            tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            }
        ) {
            status_bar::show_main_window(app);
        }
        if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
            use tauri::Manager as _;

            tauri::async_runtime::block_on(app.state::<state::AppState>().close_all());
        }
    });
}
