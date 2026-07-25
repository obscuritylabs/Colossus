use serde::Serialize;
use std::{
    fs::OpenOptions,
    io::Write as _,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt as _;

use crate::{
    bundle::VerifiedBundle,
    desktop_commands,
    desktop_dto::{DesktopReleaseChannelDto, DesktopStatusDto, RuntimeFailureCodeDto},
    dto::CommandErrorDto,
    state::AppState,
};

const MAX_DIAGNOSTICS_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum BundleIntegrityStatusDto {
    Verified,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CodeSigningStatusDto {
    Development,
    Verified,
    AdHoc,
    Unsigned,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReleaseMetadataDto {
    platform: &'static str,
    architecture: &'static str,
    channel: DesktopReleaseChannelDto,
    bundle_integrity: BundleIntegrityStatusDto,
    code_signing: CodeSigningStatusDto,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeHealthDto {
    connection_state: &'static str,
    managed_state: &'static str,
    target_count: usize,
    selected_target_kind: Option<&'static str>,
    additional_ca_certificates: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SanitizedDiagnosticError {
    component: &'static str,
    code: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsDocument {
    schema_version: u16,
    application_version: String,
    exported_at_unix_ms: u128,
    release: ReleaseMetadataDto,
    runtime: RuntimeHealthDto,
    recent_sanitized_errors: Vec<SanitizedDiagnosticError>,
    privacy: &'static str,
}

#[tauri::command]
pub(crate) fn desktop_release_metadata() -> ReleaseMetadataDto {
    release_metadata()
}

#[tauri::command]
pub(crate) async fn export_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, CommandErrorDto> {
    let status = desktop_commands::diagnostics_status(&state).await?;
    let document = diagnostics_document(&app, &status);
    let encoded = serde_json::to_vec_pretty(&document).map_err(|_| diagnostics_error())?;
    if encoded.is_empty() || encoded.len() > MAX_DIAGNOSTICS_BYTES {
        return Err(diagnostics_error());
    }
    let selected = app
        .dialog()
        .file()
        .add_filter("JSON diagnostics", &["json"])
        .set_file_name("colossus-diagnostics.json")
        .blocking_save_file();
    let Some(path) = selected else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|_| diagnostics_error())?;
    let mut destination = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(|_| diagnostics_error())?;
    destination
        .write_all(&encoded)
        .and_then(|()| destination.sync_all())
        .map_err(|_| diagnostics_error())?;
    Ok(true)
}

fn release_metadata() -> ReleaseMetadataDto {
    ReleaseMetadataDto {
        platform: platform(),
        architecture: std::env::consts::ARCH,
        channel: DesktopReleaseChannelDto::current(),
        bundle_integrity: if VerifiedBundle::load().is_ok() {
            BundleIntegrityStatusDto::Verified
        } else {
            BundleIntegrityStatusDto::Failed
        },
        code_signing: code_signing_status(),
    }
}

fn diagnostics_document(app: &AppHandle, status: &DesktopStatusDto) -> DiagnosticsDocument {
    let selected_target_kind = status
        .targets
        .iter()
        .find(|target| target.selected)
        .map(|target| match target.kind {
            crate::desktop_dto::RuntimeTargetKindDto::ManagedLocal => "managed_local",
            crate::desktop_dto::RuntimeTargetKindDto::ExternalDaemon => "external_daemon",
        });
    let mut recent_sanitized_errors = Vec::new();
    if let Some(code) = status
        .targets
        .iter()
        .find(|target| target.selected)
        .and_then(|target| target.failure_code)
    {
        recent_sanitized_errors.push(SanitizedDiagnosticError {
            component: "runtime",
            code: failure_code(code),
        });
    }
    DiagnosticsDocument {
        schema_version: 1,
        application_version: app.package_info().version.to_string(),
        exported_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis()),
        release: release_metadata(),
        runtime: RuntimeHealthDto {
            connection_state: connection_state(status),
            managed_state: managed_state(status),
            target_count: status.targets.len(),
            selected_target_kind,
            additional_ca_certificates: status.additional_ca_bundle.certificate_count,
        },
        recent_sanitized_errors,
        privacy: "Prompts, credentials, model output, headers, certificate paths, and filesystem paths are excluded.",
    }
}

fn connection_state(status: &DesktopStatusDto) -> &'static str {
    use crate::dto::ConnectionStateDto;
    match status.connection.state {
        ConnectionStateDto::Connected => "connected",
        ConnectionStateDto::Disconnected => "disconnected",
        ConnectionStateDto::NotConfigured => "not_configured",
        ConnectionStateDto::Starting => "starting",
        ConnectionStateDto::Restarting => "restarting",
        ConnectionStateDto::Stopping => "stopping",
        ConnectionStateDto::Failed => "failed",
    }
}

fn managed_state(status: &DesktopStatusDto) -> &'static str {
    use crate::desktop_dto::ManagedRuntimeStateDto;
    match status.managed_state {
        ManagedRuntimeStateDto::NeedsWorkspace => "needs_workspace",
        ManagedRuntimeStateDto::NeedsProvider => "needs_provider",
        ManagedRuntimeStateDto::Starting => "starting",
        ManagedRuntimeStateDto::Ready => "ready",
        ManagedRuntimeStateDto::Restarting => "restarting",
        ManagedRuntimeStateDto::Stopping => "stopping",
        ManagedRuntimeStateDto::Failed => "failed",
    }
}

fn failure_code(code: RuntimeFailureCodeDto) -> &'static str {
    match code {
        RuntimeFailureCodeDto::Integrity => "integrity",
        RuntimeFailureCodeDto::Permission => "permission",
        RuntimeFailureCodeDto::WorkspaceBusy => "workspace_busy",
        RuntimeFailureCodeDto::Configuration => "configuration",
        RuntimeFailureCodeDto::Authentication => "authentication",
        RuntimeFailureCodeDto::Provider => "provider",
        RuntimeFailureCodeDto::CrashLoop => "crash_loop",
        RuntimeFailureCodeDto::Transport => "transport",
        RuntimeFailureCodeDto::Internal => "internal",
    }
}

const fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unsupported"
    }
}

fn code_signing_status() -> CodeSigningStatusDto {
    match env!("COLOSSUS_DESKTOP_CODE_SIGNING_STATUS") {
        "development" => CodeSigningStatusDto::Development,
        "verified" => CodeSigningStatusDto::Verified,
        "ad_hoc" => CodeSigningStatusDto::AdHoc,
        "unsigned" => CodeSigningStatusDto::Unsigned,
        "unsupported" => CodeSigningStatusDto::Unsupported,
        _ => unreachable!("the desktop build script validates code-signing status"),
    }
}

fn diagnostics_error() -> CommandErrorDto {
    CommandErrorDto::local_sanitized(
        "diagnostics_export_failed",
        "The local diagnostics file could not be exported.",
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_metadata_is_path_and_secret_free() {
        let encoded = serde_json::to_string(&release_metadata()).expect("metadata");
        assert!(encoded.contains(std::env::consts::ARCH));
        assert!(!encoded.contains("Users"));
        assert!(!encoded.contains("credential"));
    }
}
