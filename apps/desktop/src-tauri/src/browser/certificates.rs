//! Native PKI setup. Renderer requests actions/preferences, never secret material.

#[cfg(feature = "embedded-chromium-preview")]
mod review;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, Webview};

use super::{commands::require_controller, manager::error};
use crate::{dto::CommandErrorDto, provider_enrollment::DialogAppearanceInput, state::AppState};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CertificateAction {
    Status,
    ImportCa,
    ImportClientIdentity,
    ReviewClientIdentity,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CertificateRequest {
    generation: u64,
    action: CertificateAction,
    tab_id: Option<String>,
    appearance: Option<DialogAppearanceInput>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // Independent native capability metadata.
pub(crate) struct CertificateStatusDto {
    scope: &'static str,
    ca_import_available: bool,
    pfx_import_available: bool,
    client_identity_review_available: bool,
    client_identity_selection_ready: bool,
    acceptance_pending: bool,
    message: String,
    fingerprints_sha256: Vec<String>,
}

impl CertificateStatusDto {
    fn current() -> Self {
        #[cfg(feature = "embedded-chromium-preview")]
        {
            let status = colossus_native_browser::pki::status();
            Self {
                scope: status.scope,
                ca_import_available: status.ca_import_available,
                pfx_import_available: status.pfx_import_available,
                client_identity_review_available: status.client_identity_review_available,
                client_identity_selection_ready: status.client_identity_selection_ready,
                acceptance_pending: status.acceptance_pending,
                message: status.message.into(),
                fingerprints_sha256: Vec::new(),
            }
        }
        #[cfg(not(feature = "embedded-chromium-preview"))]
        Self { scope: "operating_system_user", ca_import_available: false, pfx_import_available: false, client_identity_review_available: false, client_identity_selection_ready: false, acceptance_pending: false, message: "The current browser uses native operating-system trust. Browser certificate import is available only in the embedded Chromium preview. Colossus network CA bundles and client identities do not configure this browser.".into(), fingerprints_sha256: Vec::new() }
    }
}

#[tauri::command]
pub(crate) async fn browser_certificates(
    app: AppHandle,
    caller: Webview,
    state: State<'_, AppState>,
    request: CertificateRequest,
) -> Result<CertificateStatusDto, CommandErrorDto> {
    require_controller(&caller)?;
    let scope = state.browser.validate(request.generation)?;
    if !matches!(request.action, CertificateAction::ReviewClientIdentity)
        && request.tab_id.is_some()
    {
        return Err(error(
            "A tab can be selected only for native identity review.",
        ));
    }
    if matches!(request.action, CertificateAction::Status) {
        return Ok(CertificateStatusDto::current());
    }
    #[cfg(not(feature = "embedded-chromium-preview"))]
    {
        let _ = (app, scope, request.appearance);
        Err(error(
            "Browser certificate import is unavailable in this build.",
        ))
    }
    #[cfg(feature = "embedded-chromium-preview")]
    {
        if matches!(request.action, CertificateAction::ReviewClientIdentity) {
            review::review(app, &caller, &state, request, &scope).await
        } else {
            import(app, &caller, &state, request, &scope).await
        }
    }
}

#[cfg(feature = "embedded-chromium-preview")]
async fn import(
    app: AppHandle,
    caller: &Webview,
    state: &AppState,
    request: CertificateRequest,
    scope: &str,
) -> Result<CertificateStatusDto, CommandErrorDto> {
    use colossus_native_browser::pki;
    use sha2::Digest as _;
    use tauri::Manager as _;
    use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};
    let pki = pki::status();
    let client = matches!(request.action, CertificateAction::ImportClientIdentity);
    if (client && !pki.pfx_import_available) || (!client && !pki.ca_import_available) {
        return Err(error(
            "Native browser certificate import is unavailable on this platform.",
        ));
    }
    state.browser.hide_all();
    // The native confirmation identifies the actual trust scope. Browser profile
    // isolation must never be implied for user-store mutations.
    let dialog = app.dialog().file();
    let selected = if client {
        dialog
            .add_filter("Password-protected PKCS#12 identity", &["pfx", "p12"])
            .blocking_pick_file()
    } else {
        dialog
            .add_filter(
                "CA trust anchor (PEM or DER)",
                &["pem", "crt", "cer", "der"],
            )
            .blocking_pick_file()
    };
    let Some(selected) = selected else {
        return Ok(CertificateStatusDto::current());
    };
    let path = selected
        .into_path()
        .map_err(|_| error("The native certificate file could not be opened."))?;
    // Existing no-follow bounded reader works for native-selected binary input;
    // the bytes never enter a command result or the renderer.
    let bytes = zeroize::Zeroizing::new(
        crate::desktop_settings::read_ca_bundle_source(&path)
            .map_err(|_| error("Choose a regular certificate file no larger than 4 MiB."))?,
    );
    let password = if client {
        let parent = app
            .get_window("main")
            .ok_or_else(|| error("The Desktop window has closed."))?;
        Some(
            colossus_native_credential_ui::prompt_password(
                parent,
                request.appearance.map(Into::into).unwrap_or_default(),
            )
            .await
            .map_err(|failure| error(&failure.to_string()))?,
        )
    } else {
        None
    };
    let fingerprint = if client {
        hex::encode(sha2::Sha256::digest(&bytes))
    } else {
        pki::ca_fingerprint(&bytes).map_err(|failure| error(&failure.to_string()))?
    };
    let description = if client {
        "PKCS#12 package SHA-256"
    } else {
        "CA certificate DER SHA-256"
    };
    let agreed = app.dialog().message(format!("Import this selected native certificate input?\n\n{description}: {fingerprint}\n\nThis changes the operating-system user's certificate store and can affect other applications. Browser profiles do not isolate this trust. All Colossus browser tabs will close before import. Native Chromium client identity requests require a separate exact-origin fingerprint review. Installed key-use acceptance remains pending. Manage removal through the operating system's certificate manager.")).title("Review browser certificate import (preview)").kind(MessageDialogKind::Warning).buttons(MessageDialogButtons::OkCancel).blocking_show();
    if !agreed {
        return Ok(CertificateStatusDto::current());
    }
    require_controller(caller)?;
    if state.browser.validate(request.generation)? != scope {
        return Err(error(
            "The selected workspace changed before certificate import.",
        ));
    }
    let (generation, operation) = state
        .browser
        .close_all_authorized(request.generation, scope)
        .await?;
    // Recheck the selected controller after teardown and before native mutation.
    require_controller(caller)?;
    let lifecycle = state.browser.lock()?;
    if lifecycle.scope.as_deref() != Some(scope)
        || lifecycle.generation != generation
        || !lifecycle.tabs.is_empty()
    {
        return Err(error(
            "The selected workspace changed before certificate import.",
        ));
    }
    let receipt = if let Some(password) = password {
        pki::import_pfx(&bytes, password.expose())
    } else {
        pki::import_ca(&bytes)
    }
    .map_err(|failure| error(&failure.to_string()))?;
    drop(lifecycle);
    drop(operation);
    let mut status = CertificateStatusDto::current();
    status.fingerprints_sha256 = receipt.fingerprints_sha256;
    status.message = "Imported into the operating-system user store. Review the exact origin and certificate fingerprint when Chromium requests a client identity. Installed trust/key-use acceptance remains pending. Manage removal through the operating system's certificate manager.".into();
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn certificate_requests_have_no_path_password_or_key_fields() {
        for secret in [
            "path",
            "password",
            "privateKey",
            "keyBytes",
            "origin",
            "fingerprint",
        ] {
            let value = serde_json::json!({"generation": 1, "action": "import_client_identity", "appearance": null, (secret): "must not cross IPC"});
            assert!(serde_json::from_value::<CertificateRequest>(value).is_err());
        }
    }
}
