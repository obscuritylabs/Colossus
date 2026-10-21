//! Native operational logging. Renderer errors never supply log text or paths.

use colossus_home::ConfinedRoot;
use colossus_observability::{LocalLogLevel, LocalLogSink};
use serde::Deserialize;

use crate::desktop_settings::SettingsStore;

pub(crate) fn initialize(
    store: &SettingsStore,
    package_info: &tauri::PackageInfo,
) -> std::io::Result<LocalLogSink> {
    let root = ConfinedRoot::bind(store.application_root())
        .map_err(|_| std::io::Error::other("Desktop log storage is unavailable"))?;
    let logs =
        LocalLogSink::open(&root, "desktop", LocalLogLevel::from_environment())?.install()?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        tracing::error!(target: "colossus.desktop", event_name = "desktop.panicked", error_type = "panic");
        previous(panic);
    }));
    started(package_info);
    Ok(logs)
}

fn started(package_info: &tauri::PackageInfo) {
    tracing::info!(
        target: "colossus.desktop",
        event_name = "desktop.started",
        application_version = %package_info.version,
        platform = std::env::consts::OS,
        process_id = std::process::id(),
    );
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RendererFailure {
    Render,
    Script,
    UnhandledPromise,
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri injects an owned caller Webview.
pub(crate) fn report_renderer_failure(
    caller: tauri::Webview,
    failure: RendererFailure,
) -> Result<(), crate::dto::CommandErrorDto> {
    crate::browser::commands::require_controller(&caller)?;
    // Only fixed categories cross this boundary. Even a compromised renderer
    // cannot write strings, native paths or error payloads into operational logs.
    let category = match failure {
        RendererFailure::Render => "renderer_render",
        RendererFailure::Script => "renderer_script",
        RendererFailure::UnhandledPromise => "renderer_unhandled_promise",
    };
    self::failure(category, false, false);
    Ok(())
}

pub(crate) fn failure(code: &str, retryable: bool, outcome_unknown: bool) {
    let code = if code.len() <= 64
        && !code.is_empty()
        && code
            .bytes()
            .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'_')
    {
        code
    } else {
        "unclassified"
    };
    tracing::warn!(
        target: "colossus.desktop",
        event_name = "desktop.command_failed",
        error_type = code,
        retryable,
        outcome_unknown,
    );
}

#[cfg(test)]
mod tests;
