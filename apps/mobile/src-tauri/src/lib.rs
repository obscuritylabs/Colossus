//! Native human-session custody and closed control-plane operations. No execution runtime.
mod commands;

/// Launch the mobile client and its local-only renderer.
///
/// # Panics
/// Panics when the platform cannot initialize the application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(colossus_control_plane_client::ControlPlaneClient::default())
        .invoke_handler(tauri::generate_handler![
            commands::mobile_connect,
            commands::mobile_login,
            commands::mobile_request,
            commands::mobile_logout,
            commands::mobile_open_browser,
        ])
        .setup(|app| {
            let development_origin = app.config().build.dev_url.as_ref().map(|url| url.origin());
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Colossus")
            .inner_size(420.0, 900.0)
            .on_navigation(move |url| {
                url.scheme() == "tauri"
                    || url.host_str() == Some("tauri.localhost")
                    || (cfg!(debug_assertions)
                        && development_origin.as_ref() == Some(&url.origin()))
            })
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Colossus Mobile could not start");
}
