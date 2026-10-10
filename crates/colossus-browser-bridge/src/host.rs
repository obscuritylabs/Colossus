use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use colossus_contracts::{BrowserAction, BrowserSessionId, BrowserTabSummary};
use colossus_ports::{
    BrowserCancelDisposition, BrowserDriver, BrowserDriverCommand, BrowserDriverControl,
    BrowserDriverError, BrowserDriverOpenRequest, RunControl,
};
use tokio::{sync::Mutex as AsyncMutex, time::timeout};

use crate::{
    BrowserBridgeEnrollment, BrowserBridgeKey, InheritedBrowserChannel,
    framing::AuthenticatedChannel,
    wire::{Request, Response},
};

mod capture;
mod dispatch;
mod handoff;
mod transfer;

struct Session {
    state: Mutex<SessionState>,
    operation: Arc<AsyncMutex<()>>,
}

struct SessionState {
    open: Option<BrowserDriverOpenRequest>,
    tabs: BTreeMap<colossus_contracts::BrowserTabId, BrowserTabSummary>,
    generation: u64,
    writer_run: Option<String>,
    cancelled_through: Option<u64>,
    quiescing: bool,
    closing: bool,
    authority: RunControl,
    transfer: Option<capture::Transfer>,
    handoff_confirmed: bool,
    upload: Option<transfer::Upload>,
    download: Option<transfer::Download>,
}

struct Host {
    enrollment: BrowserBridgeEnrollment,
    driver: Arc<dyn BrowserDriver>,
    sessions: Mutex<BTreeMap<BrowserSessionId, Arc<Session>>>,
}

/// Serve a contained browser driver on two supervisor-owned inherited channels.
///
/// The native driver must independently enforce the full process/network envelope.
/// Close acknowledgements require native quiescence and context close; the supervisor
/// additionally waits for proxy revocation, `CefShutdown` and process-tree reaping.
/// Any channel failure fences admissions, cancels all sessions and attempts bounded
/// cleanup. Failed cleanup returns unknown so the supervisor retains its obligation.
pub async fn serve_browser_host(
    data: InheritedBrowserChannel,
    control: InheritedBrowserChannel,
    enrollment: BrowserBridgeEnrollment,
    key: BrowserBridgeKey,
    driver: Arc<dyn BrowserDriver>,
) -> Result<(), BrowserDriverError> {
    enrollment.validate()?;
    if enrollment.cancellation_closes_context
        != (driver.cancellation_disposition() == BrowserCancelDisposition::ClosesContext)
    {
        return Err(BrowserDriverError::Unavailable);
    }
    let key = Arc::new(key);
    let data = AuthenticatedChannel::new(data, Arc::clone(&key), &enrollment, b"data")?;
    let control = AuthenticatedChannel::new(control, key, &enrollment, b"control")?;
    let host = Arc::new(Host {
        enrollment,
        driver,
        sessions: Mutex::new(BTreeMap::new()),
    });
    let mut data_task = tokio::spawn(channel_loop(data, Arc::clone(&host), false));
    let mut control_task = tokio::spawn(channel_loop(control, Arc::clone(&host), true));
    let _guard = ChannelOwners {
        host: Arc::clone(&host),
        data: data_task.abort_handle(),
        control: control_task.abort_handle(),
    };
    tokio::select! {
        _ = &mut data_task => { control_task.abort(); let _ = control_task.await; }
        _ = &mut control_task => { data_task.abort(); let _ = data_task.await; }
    }
    host.drain().await
}

struct ChannelOwners {
    host: Arc<Host>,
    data: tokio::task::AbortHandle,
    control: tokio::task::AbortHandle,
}

impl Drop for ChannelOwners {
    fn drop(&mut self) {
        // Cancelling the serve future cannot detach channel owners. Native cleanup
        // evidence still belongs to the enclosing process-tree supervisor.
        if let Ok(sessions) = self.host.sessions.lock() {
            for session in sessions.values() {
                if let Ok(mut state) = session.state.lock() {
                    state.closing = true;
                    state.authority.cancel();
                }
            }
        }
        self.data.abort();
        self.control.abort();
    }
}

async fn channel_loop(
    mut channel: AuthenticatedChannel,
    host: Arc<Host>,
    control: bool,
) -> Result<(), BrowserDriverError> {
    let mut sequence = 0_u64;
    loop {
        sequence = sequence.checked_add(1).ok_or(BrowserDriverError::Denied)?;
        // Idle inherited channels can remain open; each accepted operation is bounded.
        let request = channel.read::<Request>(b"request", sequence).await?;
        let correct_channel = matches!(&request, Request::Ready {})
            || matches!(&request, Request::Cancel { .. } | Request::Close { .. }) == control;
        if !correct_channel {
            return Err(BrowserDriverError::Denied);
        }
        let deadline = Duration::from_millis(u64::from(
            host.enrollment.capabilities.limits.navigation_timeout_ms,
        ));
        let session = request.session().cloned();
        let response = timeout(deadline, host.dispatch(request))
            .await
            .unwrap_or_else(|_| Response::rejected(BrowserDriverError::OutcomeUnknown));
        if matches!(
            response,
            Response::Rejected {
                code: crate::wire::ErrorCode::OutcomeUnknown
            }
        ) && let Some(id) = session
        {
            host.fence_unknown(&id);
        }
        timeout(deadline, channel.write(b"response", sequence, &response))
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)??;
    }
}

impl Host {
    fn session(&self, id: &BrowserSessionId) -> Result<Arc<Session>, BrowserDriverError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if let Some(session) = sessions.get(id) {
            return Ok(Arc::clone(session));
        }
        if sessions.len() >= usize::from(self.enrollment.capabilities.limits.max_sessions) {
            return Err(BrowserDriverError::LimitExceeded);
        }
        let session = Arc::new(Session {
            state: Mutex::new(SessionState {
                open: None,
                tabs: BTreeMap::new(),
                generation: 0,
                writer_run: None,
                cancelled_through: None,
                quiescing: false,
                closing: false,
                authority: RunControl::default(),
                transfer: None,
                handoff_confirmed: false,
                upload: None,
                download: None,
            }),
            operation: Arc::new(AsyncMutex::new(())),
        });
        sessions.insert(id.clone(), Arc::clone(&session));
        Ok(session)
    }

    async fn open(
        &self,
        request: BrowserDriverOpenRequest,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        crate::validation::open(&request, &self.enrollment)?;
        let session = self.session(&request.session_id)?;
        let _operation = session.operation.lock().await;
        let authority = {
            let mut state = session
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if state.open.is_some() {
                return Err(BrowserDriverError::Stale);
            }
            if state.closing || state.quiescing || state.cancelled_through.is_some() {
                return Err(BrowserDriverError::Cancelled);
            }
            state.open = Some(request.clone());
            state.authority.clone()
        };
        let control = BrowserDriverControl::new(RunControl::default(), authority);
        let result = self.driver.open_session(request.clone(), &control).await;
        match result {
            Ok(tab)
                if tab.tab_id == request.tab_id
                    && tab.document_id == request.document_id
                    && tab.title.len() <= 1024
                    && tab
                        .origin
                        .as_ref()
                        .is_none_or(|origin| request.options.allowed_origins.contains(origin)) =>
            {
                let mut state = session
                    .state
                    .lock()
                    .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                if state.closing || state.authority.is_cancelled() {
                    return Err(BrowserDriverError::OutcomeUnknown);
                }
                state.tabs.insert(tab.tab_id.clone(), tab.clone());
                Ok(tab)
            }
            Ok(_) => Err(BrowserDriverError::OutcomeUnknown),
            Err(error) => {
                if error != BrowserDriverError::OutcomeUnknown {
                    let mut sessions = self
                        .sessions
                        .lock()
                        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                    let mut state = session
                        .state
                        .lock()
                        .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
                    // A categorical rejection proves no allocation, but cannot erase
                    // a control fence which already owns cleanup of this identity.
                    state.open = None;
                    if !state.closing && !state.quiescing && state.cancelled_through.is_none() {
                        sessions.remove(&request.session_id);
                    }
                }
                Err(error)
            }
        }
    }

    async fn execute(
        &self,
        command: BrowserDriverCommand,
    ) -> Result<colossus_contracts::BrowserObservation, BrowserDriverError> {
        if matches!(
            command.action,
            BrowserAction::Screenshot { .. }
                | BrowserAction::Upload { .. }
                | BrowserAction::Download { .. }
        ) {
            return Err(BrowserDriverError::Unsupported);
        }
        let session = self
            .sessions
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .get(&command.session_id)
            .cloned()
            .ok_or(BrowserDriverError::Stale)?;
        let _operation = session.operation.lock().await;
        let authority = self.begin_command(&session, &command)?;
        let control = BrowserDriverControl::new(RunControl::default(), authority);
        let observation = self.driver.execute(command.clone(), &control).await?;
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        if state.closing || state.authority.is_cancelled() {
            return Err(BrowserDriverError::OutcomeUnknown);
        }
        crate::validation::observation(
            &observation,
            &command,
            state
                .open
                .as_ref()
                .ok_or(BrowserDriverError::OutcomeUnknown)?,
            &self.enrollment,
        )?;
        if let BrowserAction::TabClose { tab_id } = &command.action {
            state.tabs.remove(tab_id);
        } else {
            state
                .tabs
                .insert(observation.tab.tab_id.clone(), observation.tab.clone());
        }
        Ok(observation)
    }

    fn begin_command(
        &self,
        session: &Session,
        command: &BrowserDriverCommand,
    ) -> Result<RunControl, BrowserDriverError> {
        let mut state = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
        let open = state.open.as_ref().ok_or(BrowserDriverError::Stale)?;
        crate::validation::command(command, open, &self.enrollment)?;
        if state.closing
            || state.quiescing
            || command.control_generation < state.generation
            || state
                .cancelled_through
                .is_some_and(|generation| command.control_generation <= generation)
            || (command.control_generation == state.generation
                && state
                    .writer_run
                    .as_ref()
                    .is_some_and(|run| run != &command.run_id))
            || (command.control_generation > state.generation
                && state.writer_run.is_some()
                && state
                    .cancelled_through
                    .is_none_or(|generation| generation < state.generation))
            || state
                .tabs
                .get(&command.target.tab_id)
                .is_none_or(|tab| tab.document_id != command.target.document_id)
        {
            return Err(BrowserDriverError::Stale);
        }
        if command.control_generation > state.generation {
            state.authority = RunControl::default();
            state.generation = command.control_generation;
            state.writer_run = Some(command.run_id.clone());
        }
        state.transfer = None;
        state.upload = None;
        state.download = None;
        Ok(state.authority.clone())
    }

    async fn cancel(
        &self,
        id: &BrowserSessionId,
        generation: u64,
    ) -> Result<(), BrowserDriverError> {
        let session = self.session(id)?;
        let active = {
            let mut state = session
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if state.generation > generation {
                return Ok(());
            }
            state.cancelled_through = Some(
                state
                    .cancelled_through
                    .map_or(generation, |value| value.max(generation)),
            );
            state.authority.cancel();
            state.transfer = None;
            state.upload = None;
            state.download = None;
            state.quiescing = true;
            state.closing |= self.enrollment.cancellation_closes_context;
            state.open.is_some()
        };
        if active {
            self.driver.cancel_session(id).await?;
        }
        let _operation = session.operation.lock().await;
        session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .quiescing = false;
        Ok(())
    }

    async fn close(&self, id: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        let session = self.session(id)?;
        let active = {
            let mut state = session
                .state
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            state.closing = true;
            state.authority.cancel();
            state.open.is_some()
        };
        // Cancellation is independent of the action lock; close acknowledgement waits
        // until the pending open/action owner has relinquished native dispatch.
        if active {
            self.driver.cancel_session(id).await?;
        }
        let _operation = session.operation.lock().await;
        let allocated = session
            .state
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .open
            .is_some();
        if allocated {
            self.driver.close_session(id).await?;
            self.sessions
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?
                .remove(id);
        }
        // A pre-admission close leaves a bounded tombstone so a delayed open cannot allocate.
        Ok(())
    }

    async fn drain(&self) -> Result<(), BrowserDriverError> {
        let ids = self
            .sessions
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let deadline = Duration::from_millis(u64::from(
            self.enrollment.capabilities.limits.navigation_timeout_ms,
        ));
        let mut failed = false;
        for id in ids {
            if !matches!(timeout(deadline, self.close(&id)).await, Ok(Ok(()))) {
                failed = true;
            }
        }
        if failed {
            Err(BrowserDriverError::OutcomeUnknown)
        } else {
            Ok(())
        }
    }
}
