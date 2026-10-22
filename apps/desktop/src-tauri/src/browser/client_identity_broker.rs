//! Review initial TLS independently of tab creation and the in-flight navigation.

use std::time::Duration;

use colossus_sdk::{
    ClientIdentityPolicy, NativeBrowserClient, NativeBrowserIncarnation, NativeClientIdentityReview,
};
use tauri::{AppHandle, Manager as _};

use crate::{
    client_identity_review::{self, ClientIdentityDisplay, ClientIdentityDisplayCandidate},
    state::AppState,
};

const POLL_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let mut selection = state.subscribe_selection();
        let mut failed = false;
        while app.get_window("main").is_some() {
            if let Some(target_id) = state.browser.scope_snapshot()
                && let Some(client) = state.managed_browser_for(&target_id).await
                && let Ok(incarnation) = client.incarnation()
            {
                let selected = state.try_selected_target(&target_id);
                let epoch = selected
                    .as_ref()
                    .map(crate::state::SelectedTargetLease::epoch);
                let policy = if selected.is_some() {
                    ClientIdentityPolicy::AutomaticIfUnique
                } else {
                    ClientIdentityPolicy::Review
                };
                // Hold the short native selection lease across automatic selection,
                // never across the human review. The policy is host-owned, not JS input.
                // A workspace writer can be waiting for initial TLS to finish. Do
                // not queue another reader behind it: decline that exact request
                // through the advisory scope instead, allowing navigation to finish.
                let result = client.pending_client_identity(policy).await;
                drop(selected);
                match result {
                    Ok(Some(review)) => {
                        failed = false;
                        if let Some(epoch) = epoch
                            && current_client(&state, &target_id, epoch, &incarnation).await
                        {
                            review_identity(&app, &target_id, epoch, &client, &incarnation, review)
                                .await;
                        } else {
                            retire(&app, &target_id, &client, &incarnation, &review).await;
                        }
                    }
                    Ok(None) => failed = false,
                    Err(_) if !failed => {
                        failed = true;
                        tracing::warn!(target: "colossus.desktop",
                            event_name = "desktop.client_identity.lookup_failed",
                            error_type = "native_channel");
                    }
                    Err(_) => {}
                }
            }
            tokio::select! {
                () = tokio::time::sleep(POLL_INTERVAL) => {},
                changed = selection.changed() => { if changed.is_err() { break; } },
            }
        }
    });
}

async fn current_client(
    state: &AppState,
    target_id: &str,
    epoch: u64,
    incarnation: &NativeBrowserIncarnation,
) -> bool {
    state.selection_is_current(target_id, epoch)
        && state
            .managed_browser_for(target_id)
            .await
            .is_some_and(|client| client.matches_incarnation(incarnation))
}

fn same_request(left: &NativeClientIdentityReview, right: &NativeClientIdentityReview) -> bool {
    left.session == right.session
        && left.challenge.key == right.challenge.key
        && left.challenge.origin == right.challenge.origin
        && left.challenge.candidates == right.challenge.candidates
}

async fn review_identity(
    app: &AppHandle,
    target_id: &str,
    epoch: u64,
    client: &NativeBrowserClient,
    incarnation: &NativeBrowserIncarnation,
    review: NativeClientIdentityReview,
) {
    let display = ClientIdentityDisplay {
        origin: review.challenge.origin.as_str().to_owned(),
        candidates: review
            .challenge
            .candidates
            .iter()
            .map(|candidate| ClientIdentityDisplayCandidate {
                label: candidate.label.clone(),
                fingerprint_sha256: candidate.fingerprint_sha256.clone(),
            })
            .collect(),
        remaining_ms: review.challenge.remaining_ms,
    };
    // Dropping show closes its child WebView. Monitoring cannot renew the native
    // deadline and never requests automatic selection while the choice is visible.
    let outcome = {
        let decision = client_identity_review::show(app, target_id, epoch, display);
        let invalidated = async {
            loop {
                tokio::time::sleep(POLL_INTERVAL).await;
                if app.get_window("main").is_none()
                    || !current_client(&app.state::<AppState>(), target_id, epoch, incarnation)
                        .await
                {
                    return;
                }
                match client
                    .pending_client_identity(ClientIdentityPolicy::Review)
                    .await
                {
                    Ok(Some(current)) if same_request(&review, &current) => {}
                    _ => return,
                }
            }
        };
        tokio::select! {
            biased;
            () = invalidated => None,
            result = decision => Some(result),
        }
    }; // Close the child before waiting for any native retirement acknowledgement.
    let Some(outcome) = outcome else {
        retire(app, target_id, client, incarnation, &review).await;
        return;
    };
    let choice = if let Ok(choice) = outcome {
        choice
    } else {
        tracing::warn!(target: "colossus.desktop",
            event_name = "desktop.client_identity.review_cancelled",
            error_type = "review_unavailable");
        None
    };
    let state = app.state::<AppState>();
    let Some(selected) = state.try_selected_target(target_id) else {
        retire(app, target_id, client, incarnation, &review).await;
        return;
    };
    if selected.epoch() != epoch || !current_client(&state, target_id, epoch, incarnation).await {
        retire(app, target_id, client, incarnation, &review).await;
        return;
    }
    // Exactly one submission. Runtime and CEF independently recheck the original
    // origin, operation generation, cancellation and expiring one-use request.
    if client
        .select_client_identity(&review, choice)
        .await
        .is_err()
    {
        tracing::warn!(target: "colossus.desktop",
            event_name = "desktop.client_identity.selection_unconfirmed",
            error_type = "native_channel");
    }
}

async fn retire(
    app: &AppHandle,
    target_id: &str,
    client: &NativeBrowserClient,
    incarnation: &NativeBrowserIncarnation,
    review: &NativeClientIdentityReview,
) {
    // A workspace switch may deny its original request, never choose an identity.
    // Do not send an old request through a restarted or replaced sidecar client.
    if client.matches_incarnation(incarnation)
        && app
            .state::<AppState>()
            .managed_browser_for(target_id)
            .await
            .is_some_and(|current| current.matches_incarnation(incarnation))
    {
        let _ = client.select_client_identity(review, None).await;
    }
}
