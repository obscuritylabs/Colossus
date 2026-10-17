//! Native-only review of one expiring Chromium client identity request.

use colossus_native_browser::pki::IdentityRequest;
use tauri::{AppHandle, Manager as _, Webview, Window};
use tauri_plugin_dialog::{
    DialogExt as _, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

use super::{CertificateRequest, CertificateStatusDto};
use crate::{
    browser::{commands::require_controller, manager::error},
    dto::CommandErrorDto,
    state::AppState,
};

pub(super) async fn review(
    app: AppHandle,
    caller: &Webview,
    state: &AppState,
    request: CertificateRequest,
    scope: &str,
) -> Result<CertificateStatusDto, CommandErrorDto> {
    use colossus_native_browser::{BrowserView, chromium::Surface, pki::IdentityRequest};
    struct PendingReview {
        surface: Surface,
        request: IdentityRequest,
    }
    impl Drop for PendingReview {
        fn drop(&mut self) {
            self.surface.cancel_client_identity(&self.request);
        }
    }
    let _review_operation = state
        .browser
        .identity_review
        .try_lock()
        .map_err(|_| error("Another native client identity review is already open."))?;
    if !colossus_native_browser::pki::status().client_identity_review_available {
        return Err(error(
            "Native client identity review is unavailable on this platform.",
        ));
    }
    let id = request
        .tab_id
        .filter(|id| !id.is_empty() && id.len() <= 128)
        .ok_or_else(|| error("Select a browser tab with a pending client certificate request."))?;
    let (BrowserView::Chromium(surface), _) = state.browser.tab(&id, scope)? else {
        return Err(error(
            "Client identity review requires the actual Chromium guest.",
        ));
    };
    let native_request = surface.pending_client_identity().map_err(|failure| error(&failure.to_string()))?
        .ok_or_else(|| error("This tab has no pending client certificate request. Reload the website to request one."))?;
    let review = PendingReview {
        surface,
        request: native_request,
    };
    state.browser.hide_all();
    let parent = app
        .get_window("main")
        .ok_or_else(|| error("The Desktop window has closed."))?;
    let fingerprint = prompt(
        &app,
        &parent,
        &review.request,
        caller,
        state,
        request.generation,
        scope,
    )
    .await?;
    // No ownership locks remain held while the operator considers a native
    // dialog. At dispatch, prevent workspace mutation and shutdown/commands,
    // then recheck the original controller generation and exact native request.
    let _operation = state.browser.operation.lock().await;
    let _selection = state.browser_selection().await;
    require_controller(caller)?;
    if state.browser.validate(request.generation)? != scope {
        return Err(error(
            "The selected workspace changed before identity selection.",
        ));
    }
    review
        .surface
        .select_client_identity(&review.request, fingerprint.as_deref())
        .await
        .map_err(|failure| error(&failure.to_string()))?;
    let mut status = CertificateStatusDto::current();
    if let Some(fingerprint) = fingerprint {
        status.fingerprints_sha256.push(fingerprint);
        status.message = "Selected the reviewed native certificate for the exact pending HTTPS handshake. Chromium may reuse this choice for the exact origin in this temporary session. The server determines authentication success. Installed key-use acceptance remains pending.".into();
    } else {
        status.message = "No client identity was selected for the pending handshake.".into();
    }
    Ok(status)
}

async fn prompt(
    app: &AppHandle,
    parent: &Window,
    native_request: &IdentityRequest,
    caller: &Webview,
    state: &AppState,
    generation: u64,
    scope: &str,
) -> Result<Option<String>, CommandErrorDto> {
    for (index, candidate) in native_request.candidates.iter().enumerate() {
        require_controller(caller)?;
        if state.browser.validate(generation)? != scope || native_request.is_expired() {
            return Err(error(
                "The native identity request expired or its workspace changed.",
            ));
        }
        let (sender, receive) = tokio::sync::oneshot::channel();
        app.dialog().message(format!("Use this native client identity for this exact HTTPS origin?\n\nOrigin: {}\nCertificate DER SHA-256: {}\nCandidate {} of {}\n\nThe operating system keeps the private key. Chromium may reuse this choice for this exact HTTPS origin within this temporary browser session. Close all browser tabs to end the session. This does not grant agent authority. Select Next identity to review another candidate, or Cancel to send no identity. Native key-use acceptance for this preview is pending.", native_request.origin, candidate.fingerprint_sha256, index + 1, native_request.candidates.len()))
            .parent(&parent).title("Review browser client identity (preview)")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::YesNoCancelCustom("Use identity".into(), "Next identity".into(), "Cancel".into()))
            .show_with_result(move |result| { let _ = sender.send(result); });
        let response = tokio::time::timeout(std::time::Duration::from_mins(2), receive)
            .await
            .map_err(|_| error("The native identity request expired."))?
            .map_err(|_| error("The native identity dialog has closed."))?;
        match response {
            MessageDialogResult::Yes => {
                return Ok(Some(candidate.fingerprint_sha256.clone()));
            }
            MessageDialogResult::Custom(label) if label == "Use identity" => {
                return Ok(Some(candidate.fingerprint_sha256.clone()));
            }
            MessageDialogResult::No => {}
            MessageDialogResult::Custom(label) if label == "Next identity" => {}
            _ => return Ok(None),
        }
    }
    Ok(None)
}
