//! Read-only native attachment to a browser already allocated by an authenticated run.
use super::types::*;
use crate::Runtime;
use colossus_browser_presentation::Configure;
use colossus_contracts::{BrowserMode, BrowserScope, BrowserTarget};
use colossus_ports::BrowserDriverError;
use std::sync::Arc;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

impl Runtime {
    /// Attach native pixels to an existing Embedded page of this exact enrolled
    /// application and conversation. This grants no browser writer or new host.
    pub async fn watch_native_browser(
        &self,
        role: &RuntimeNativeBrowserAuthority,
        run_id: &str,
        conversation_id: &str,
    ) -> Result<RuntimeNativeBrowserSession, RuntimeNativeBrowserError> {
        let browser = self
            .browser
            .as_ref()
            .ok_or(RuntimeNativeBrowserError::Unavailable)?;
        authority(browser, role)?;
        if !self.native_browser_available(role) {
            return Err(RuntimeNativeBrowserError::Unavailable);
        }
        let run = browser
            .runs
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
            .get(run_id)
            .filter(|run| {
                !run.control.is_cancelled()
                    && run.actor.binding.application_id == role.application_id
                    && run.actor.binding.scope
                        == BrowserScope::Conversation {
                            id: conversation_id.to_owned(),
                        }
            })
            .cloned()
            .ok_or(RuntimeNativeBrowserError::Denied)?;
        #[cfg(windows)]
        if let Some(session) = super::windows_watch::resume(browser, role, &run).await? {
            return Ok(session);
        }
        let original = {
            let requests = browser
                .native
                .agent_requests
                .lock()
                .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
            run.sessions
                .keys()
                .find_map(|id| {
                    requests
                        .get(id)
                        .filter(|request| {
                            request.binding == run.actor.binding
                                && request.run_id.as_deref() == Some(run_id)
                                && request.options.mode == BrowserMode::Embedded
                        })
                        .cloned()
                })
                .ok_or(RuntimeNativeBrowserError::Unavailable)?
        };
        let owned = run
            .sessions
            .get(&original.session_id)
            .ok_or(RuntimeNativeBrowserError::Denied)?;
        // The first real gateway-authorized action establishes the native writer
        // generation. A viewer cannot promote it or race the initial open grant.
        if browser
            .native
            .agent_generations
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
            .get(&original.session_id)
            != Some(&owned.lease.control_generation)
        {
            return Err(RuntimeNativeBrowserError::Unavailable);
        }
        browser.coordinator.lease(
            &run.actor,
            &original.session_id,
            owned.lease.control_generation,
        )?;
        let summary = browser
            .coordinator
            .get(&run.actor.binding, &original.session_id)?;
        let selected = summary
            .tabs
            .iter()
            .find(|tab| Some(&tab.tab_id) == summary.selected_tab_id.as_ref())
            .ok_or(RuntimeNativeBrowserError::Stale)?;
        let target = BrowserTarget {
            tab_id: selected.tab_id.clone(),
            document_id: selected.document_id.clone(),
        };
        let configure = Configure {
            session: original.session_id.clone(),
            target: target.clone(),
            control_generation: owned.lease.control_generation,
            viewport_generation: 1,
            width: 800,
            height: 600,
            scale_milli: 1000,
            lease_ms: 1500,
        };
        configure.validate()?;
        let entry = Arc::new(Entry {
            id: Uuid::now_v7(),
            instance: role.instance,
            binding: run.actor.binding.clone(),
            conversation_id: conversation_id.to_owned(),
            request: std::sync::Mutex::new(Some(original.clone())),
            control: Default::default(),
            operation: AsyncMutex::new(()),
            view: std::sync::Mutex::new(View {
                configure: Some(configure),
                agent_generation: Some(owned.lease.control_generation),
                agent_target: Some(target),
                fenced: true,
                ..View::default()
            }),
        });
        {
            let mut entries = browser
                .native
                .entries
                .lock()
                .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
            if entries.len() >= usize::from(browser.coordinator.capabilities().limits.max_sessions)
            {
                return Err(RuntimeNativeBrowserError::LimitExceeded);
            }
            if entries.values().any(|e| {
                e.request.lock().map_or(true, |request| {
                    request
                        .as_ref()
                        .is_some_and(|r| r.session_id == original.session_id)
                })
            }) {
                return Err(RuntimeNativeBrowserError::Busy);
            }
            entries.insert(entry.id, entry.clone());
        }
        let provider = browser
            .native
            .presenter
            .as_ref()
            .ok_or(RuntimeNativeBrowserError::Unavailable)?;
        let acquired = async {
            let (client, digest) = provider.acquire(&original).await?;
            let deadline = provider.expires_at(&original).await?;
            Ok::<_, BrowserDriverError>((client, digest, deadline))
        }
        .await;
        let (client, digest, deadline) = match acquired {
            Ok(value) => value,
            Err(error) => {
                // The run keeps the browser's cleanup owner; no failed viewer may revoke its writer.
                browser
                    .native
                    .entries
                    .lock()
                    .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
                    .remove(&entry.id);
                return Err(error.into());
            }
        };
        if run.control.is_cancelled()
            || browser
                .native
                .stopped
                .load(std::sync::atomic::Ordering::Acquire)
        {
            client.disconnect();
            return Err(RuntimeNativeBrowserError::Stale);
        }
        browser.coordinator.lease(
            &run.actor,
            &original.session_id,
            owned.lease.control_generation,
        )?;
        {
            let mut view = entry
                .view
                .lock()
                .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
            view.client = Some(client.clone());
            view.digest = Some(digest);
        }
        super::expiry::track(browser, &entry, deadline)?;
        let session = RuntimeNativeBrowserSession {
            entry,
            client,
            digest,
            session_id: original.session_id,
        };
        // As with human handoff, the outer worker relay redeems one real private
        // Configure ACK instead of replaying it against the current document.
        let configure = {
            let mut view = session
                .entry
                .view
                .lock()
                .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?;
            let target = view
                .agent_target
                .clone()
                .ok_or(RuntimeNativeBrowserError::Stale)?;
            let config = view
                .configure
                .as_mut()
                .ok_or(RuntimeNativeBrowserError::Stale)?;
            config.target = target;
            config.clone()
        };
        let lease = match session.client.configure(configure.clone()).await {
            Ok(lease) => lease,
            Err(colossus_browser_presentation::PresentationError::Stale) => {
                // A concurrent Core navigation is a known rejected viewer plan.
                // Keep the original authenticated host channel and let the next
                // native refresh obtain the new Core target; never replay effects.
                browser
                    .native
                    .entries
                    .lock()
                    .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
                    .remove(&session.entry.id);
                return Err(RuntimeNativeBrowserError::Stale);
            }
            Err(error) => return Err(error.into()),
        };
        session.confirm_presentation_lease(&configure, lease)?;
        session
            .entry
            .view
            .lock()
            .map_err(|_| RuntimeNativeBrowserError::OutcomeUnknown)?
            .handoff_relay_lease = Some((configure, lease));
        Ok(session)
    }
}
