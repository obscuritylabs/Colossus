//! Native-only review of one expiring Chromium client identity request.

use colossus_native_browser::pki::IdentityRequest;
use tauri::{AppHandle, Webview};

use super::{CertificateRequest, CertificateStatusDto};
use crate::{
    browser::{commands::require_controller, manager::error},
    client_identity_review::{self, ClientIdentityDisplay, ClientIdentityDisplayCandidate},
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
    let fingerprint = prompt(
        &app,
        &review.surface,
        &review.request,
        caller,
        state,
        request.generation,
        scope,
    )
    .await?;
    // No ownership locks remain held while the operator considers a native
    // review. At dispatch, prevent workspace mutation and shutdown/commands,
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
    surface: &colossus_native_browser::chromium::Surface,
    native_request: &IdentityRequest,
    caller: &Webview,
    state: &AppState,
    generation: u64,
    scope: &str,
) -> Result<Option<String>, CommandErrorDto> {
    require_controller(caller)?;
    if state.browser.validate(generation)? != scope || native_request.is_expired() {
        return Err(error(
            "The native identity request expired or its workspace changed.",
        ));
    }
    if let [candidate] = native_request.candidates.as_slice() {
        return Ok(Some(candidate.fingerprint_sha256.clone()));
    }
    // The retained native Surface owns this legacy handshake. Its UI needs the
    // native selection epoch, not a connected Runtime client's TargetHandle.
    let epoch = *state.subscribe_selection().borrow();
    if !state.selection_is_current(scope, epoch) {
        return Err(error(
            "The selected workspace changed before identity review.",
        ));
    }
    let display = ClientIdentityDisplay {
        origin: native_request.origin.clone(),
        candidates: native_request
            .candidates
            .iter()
            .map(|candidate| ClientIdentityDisplayCandidate {
                label: candidate.label.clone(),
                fingerprint_sha256: candidate.fingerprint_sha256.clone(),
            })
            .collect(),
        remaining_ms: native_request.remaining_ms(),
    };
    let invalidated = async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            if state.browser.validate(generation).is_err()
                || native_request.is_expired()
                || !surface
                    .pending_client_identity()
                    .ok()
                    .flatten()
                    .is_some_and(|current| native_request.same_request(&current))
            {
                return;
            }
        }
    };
    tokio::select! {
        biased;
        () = invalidated => Err(error("The native identity request is no longer available.")),
        result = client_identity_review::show(app, scope, epoch, display) => result,
    }
}
