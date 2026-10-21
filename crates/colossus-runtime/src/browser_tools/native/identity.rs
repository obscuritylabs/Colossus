//! Native certificate review before initial navigation has produced a viewer or tab.
use super::types::{RuntimeNativeBrowserAuthority, RuntimeNativeBrowserError, authority};
use crate::{Runtime, browser_tools::RuntimeBrowserTools};
use colossus_browser_presentation::{
    ClientIdentityChallenge, ClientIdentityKey, ClientIdentityPolicy, PresentationClient,
    native_admission::NativeClientIdentityReview,
};
use colossus_contracts::{BrowserControlState, BrowserLifecycle, BrowserMode, BrowserSessionId};
use colossus_ports::{BrowserDriverOpenRequest, RunControl};
use std::{sync::atomic::Ordering, time::Duration};

const MAX_REVIEW_CONTEXTS: usize = 64;
const LOOKUP_TIMEOUT: Duration = Duration::from_millis(500);
const SELECTION_TIMEOUT: Duration = Duration::from_secs(5);

struct OwnedRequest {
    request: BrowserDriverOpenRequest,
    control: RunControl,
    generation: u64,
    run_id: Option<String>,
}

fn requests(
    browser: &RuntimeBrowserTools,
    role: &RuntimeNativeBrowserAuthority,
) -> Result<Vec<OwnedRequest>, RuntimeNativeBrowserError> {
    authority(browser, role)?;
    if browser.native.stopped.load(Ordering::Acquire) {
        return Err(RuntimeNativeBrowserError::Unavailable);
    }
    let entries = browser
        .native
        .entries
        .lock()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
        .values()
        .filter(|entry| entry.instance == role.instance && !entry.control.is_cancelled())
        .cloned()
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for entry in entries {
        let request = entry
            .request
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        let view = entry
            .view
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        if !view.closed
            && !view.retired
            && let Some(request) = request.as_ref().filter(|request| request.run_id.is_none())
        {
            // Human-origin allocations stay here after handoff. Their current
            // coordinator writer, rather than the original control, owns review.
            candidates.push((
                request.clone(),
                (!view.fenced).then(|| entry.control.clone()),
            ));
        }
    }
    candidates.extend(
        browser
            .native
            .agent_requests
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
            .values()
            .cloned()
            .map(|request| (request, None)),
    );
    let runs = browser
        .runs
        .lock()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let mut owned = Vec::new();
    for (request, human_control) in candidates {
        if request.binding.application_id != role.application_id
            || request.binding.runtime_id != browser.runtime_id
            || request.binding.workspace_id != browser.workspace_id
            || request.options.mode != BrowserMode::Embedded
        {
            continue;
        }
        let Ok(summary) = browser
            .coordinator
            .get(&request.binding, &request.session_id)
        else {
            continue;
        };
        let current = if summary.control == BrowserControlState::Agent {
            runs.iter().find_map(|run| {
                if run.control.is_cancelled() || run.actor.binding != request.binding {
                    return None;
                }
                let registered = run.sessions.get(&request.session_id)?;
                // A same-conversation run can retain a stale ledger entry. Only
                // the exact current coordinator lease establishes a writer.
                let lease = browser
                    .coordinator
                    .lease(
                        &run.actor,
                        &request.session_id,
                        registered.lease.control_generation,
                    )
                    .ok()?;
                (lease == registered.lease).then(|| {
                    (
                        run.control.clone(),
                        lease.control_generation,
                        Some(run.actor.run_id.clone()),
                    )
                })
            })
        } else if summary.control == BrowserControlState::Human
            && summary.control_generation == 0
            && matches!(
                summary.lifecycle,
                BrowserLifecycle::Allocating | BrowserLifecycle::Ready
            )
        {
            // Registrar records this exact opening run before native navigation,
            // which necessarily precedes the first writer lease.
            match request.run_id.as_ref() {
                Some(run_id) => runs
                    .iter()
                    .find(|run| {
                        &run.actor.run_id == run_id
                            && run.actor.binding == request.binding
                            && !run.control.is_cancelled()
                    })
                    .map(|run| (run.control.clone(), 0, Some(run_id.clone()))),
                None => human_control
                    .filter(|control| !control.is_cancelled())
                    .map(|control| (control, 0, None)),
            }
        } else {
            None
        };
        if let Some((control, generation, run_id)) = current {
            owned.push(OwnedRequest {
                request,
                control,
                generation,
                run_id,
            });
        }
    }
    if owned.len() > MAX_REVIEW_CONTEXTS {
        return Err(RuntimeNativeBrowserError::LimitExceeded);
    }
    Ok(owned)
}

fn current_owner(
    browser: &RuntimeBrowserTools,
    role: &RuntimeNativeBrowserAuthority,
    original: &OwnedRequest,
    challenge: &ClientIdentityChallenge,
) -> Result<OwnedRequest, RuntimeNativeBrowserError> {
    if original.control.is_cancelled() {
        return Err(RuntimeNativeBrowserError::Stale);
    }
    requests(browser, role)?
        .into_iter()
        .find(|current| {
            current.request == original.request
                && current.run_id == original.run_id
                && current.generation == original.generation
                && current.generation == challenge.key.control_generation
                && !current.control.is_cancelled()
                && current
                    .request
                    .options
                    .allowed_origins
                    .contains(&challenge.origin)
        })
        .ok_or(RuntimeNativeBrowserError::Stale)
}

fn attempted(
    browser: &RuntimeBrowserTools,
    session: &BrowserSessionId,
    key: ClientIdentityKey,
) -> Result<bool, RuntimeNativeBrowserError> {
    Ok(browser
        .native
        .identity_attempts
        .lock()
        .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
        .get(session)
        .is_some_and(|prior| key.request_id <= prior.request_id))
}

async fn select(
    browser: &RuntimeBrowserTools,
    role: &RuntimeNativeBrowserAuthority,
    original: &OwnedRequest,
    client: &PresentationClient,
    challenge: &ClientIdentityChallenge,
    fingerprint: Option<String>,
) -> Result<(), RuntimeNativeBrowserError> {
    let current = current_owner(browser, role, original, challenge)?;
    {
        let mut attempts = browser
            .native
            .identity_attempts
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
        if attempts
            .get(&current.request.session_id)
            .is_some_and(|prior| challenge.key.request_id <= prior.request_id)
        {
            return Err(RuntimeNativeBrowserError::Stale);
        }
        if attempts.len() >= MAX_REVIEW_CONTEXTS
            && !attempts.contains_key(&current.request.session_id)
        {
            return Err(RuntimeNativeBrowserError::LimitExceeded);
        }
        // Native request IDs increase for this exact supervised process's
        // lifetime. Retain a bounded high-water mark before the first write,
        // including dropped futures and lost ACKs; never try another identity.
        attempts.insert(current.request.session_id.clone(), challenge.key);
    }
    if current.control.is_cancelled() {
        return Err(RuntimeNativeBrowserError::Stale);
    }
    tokio::time::timeout(
        SELECTION_TIMEOUT,
        client.select_client_identity(challenge.key, fingerprint),
    )
    .await
    .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)??;
    // A successful initial handshake can finish Open and grant this same run's
    // first writer before its selection ACK is read. Do not reject that ordinary
    // generation advance after the native host acknowledged the exact choice.
    if current.control.is_cancelled() {
        return Err(RuntimeNativeBrowserError::OutcomeUnknown);
    }
    authority(browser, role).map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)
}

async fn presenter(
    browser: &RuntimeBrowserTools,
    owned: &OwnedRequest,
) -> Result<PresentationClient, RuntimeNativeBrowserError> {
    let presenter = browser
        .native
        .presenter
        .as_ref()
        .ok_or(RuntimeNativeBrowserError::Unavailable)?;
    tokio::time::timeout(LOOKUP_TIMEOUT, presenter.acquire(&owned.request))
        .await
        .map_err(|_| RuntimeNativeBrowserError::Busy)?
        .map(|(client, _)| client)
        .map_err(|_| RuntimeNativeBrowserError::Stale)
}

impl Runtime {
    /// Discover handshakes using this separately enrolled native application.
    /// Explicit singleton policy can submit one exact choice after current-owner
    /// validation; uncertain writes are reported and never automatically replayed.
    pub async fn pending_native_browser_client_identity(
        &self,
        role: &RuntimeNativeBrowserAuthority,
        policy: ClientIdentityPolicy,
    ) -> Result<Option<NativeClientIdentityReview>, RuntimeNativeBrowserError> {
        let browser = self
            .browser
            .as_ref()
            .ok_or(RuntimeNativeBrowserError::Unavailable)?;
        if !self.native_browser_available(role) {
            return Err(RuntimeNativeBrowserError::Unavailable);
        }
        let deadline = tokio::time::Instant::now() + LOOKUP_TIMEOUT;
        for owned in requests(browser, role)? {
            // Discovery is always read-only. Auto here could use a key before
            // Runtime checked origin, cancellation or a changed writer.
            let result = tokio::time::timeout_at(deadline, async {
                let client = presenter(browser, &owned).await?;
                let challenge = client
                    .pending_client_identity(ClientIdentityPolicy::Review)
                    .await?;
                Ok::<_, RuntimeNativeBrowserError>((client, challenge))
            })
            .await;
            let (client, challenge) = match result {
                Ok(Ok((client, Some(challenge)))) => (client, challenge),
                Ok(
                    Ok((_, None))
                    | Err(
                        RuntimeNativeBrowserError::Stale
                        | RuntimeNativeBrowserError::Busy
                        | RuntimeNativeBrowserError::Unavailable,
                    ),
                ) => continue,
                Ok(Err(error)) => return Err(error),
                Err(_) => break,
            };
            challenge.validate()?;
            if current_owner(browser, role, &owned, &challenge).is_err()
                || attempted(browser, &owned.request.session_id, challenge.key)?
            {
                continue;
            }
            if policy == ClientIdentityPolicy::AutomaticIfUnique && challenge.candidates.len() == 1
            {
                select(
                    browser,
                    role,
                    &owned,
                    &client,
                    &challenge,
                    Some(challenge.candidates[0].fingerprint_sha256.clone()),
                )
                .await?;
                tracing::info!(target: "colossus.browser", event = "client_identity_automatic_submitted",
                    "Native browser submitted its sole eligible certificate; server authentication remains pending");
                return Ok(None);
            }
            return Ok(Some(NativeClientIdentityReview {
                session: owned.request.session_id,
                challenge,
            }));
        }
        Ok(None)
    }

    /// Submit one choice for the original owned request. Uncertain writes are not retried.
    pub async fn select_native_browser_client_identity(
        &self,
        role: &RuntimeNativeBrowserAuthority,
        session: &BrowserSessionId,
        key: ClientIdentityKey,
        fingerprint_sha256: Option<String>,
    ) -> Result<(), RuntimeNativeBrowserError> {
        key.validate()?;
        let browser = self
            .browser
            .as_ref()
            .ok_or(RuntimeNativeBrowserError::Unavailable)?;
        let owned = requests(browser, role)?
            .into_iter()
            .find(|owned| &owned.request.session_id == session)
            .ok_or(RuntimeNativeBrowserError::Denied)?;
        let client = presenter(browser, &owned).await?;
        let current = client
            .pending_client_identity(ClientIdentityPolicy::Review)
            .await?
            .ok_or(RuntimeNativeBrowserError::Stale)?;
        current.validate()?;
        if current.key != key
            || fingerprint_sha256.as_ref().is_some_and(|fingerprint| {
                !current
                    .candidates
                    .iter()
                    .any(|candidate| &candidate.fingerprint_sha256 == fingerprint)
            })
        {
            return Err(RuntimeNativeBrowserError::Stale);
        }
        select(browser, role, &owned, &client, &current, fingerprint_sha256).await
    }
}
