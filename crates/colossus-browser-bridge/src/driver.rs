use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use colossus_contracts::{
    BrowserCapabilities, BrowserObservation, BrowserSessionId, BrowserTabSummary,
};
use colossus_ports::{
    BrowserCancelDisposition, BrowserDownloadDescriptor, BrowserDownloadReadRequest, BrowserDriver,
    BrowserDriverCommand, BrowserDriverControl, BrowserDriverError, BrowserDriverOpenRequest,
    BrowserNativeHandoffRequest, BrowserScreenshotChunk, BrowserScreenshotDescriptor,
    BrowserScreenshotReadRequest, BrowserUploadCommitRequest, BrowserUploadPrepareRequest,
    BrowserUploadReceipt, BrowserUploadWriteRequest,
};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, interval},
};

use crate::{
    BrowserBridgeEnrollment, BrowserBridgeKey, InheritedBrowserChannel,
    framing::AuthenticatedChannel,
    transport::{self, Dispatch},
    wire::{Request, Response},
};

const POLL: Duration = Duration::from_millis(10);

mod capture;
mod transfer;

struct Session {
    generation: u64,
    closing: bool,
}

/// Real inherited-channel `BrowserDriver`; construction never spawns or accepts a listener.
/// The supervisor remains responsible for verified launch, containment and process reaping.
pub struct BrowserBridgeDriver {
    enrollment: BrowserBridgeEnrollment,
    available: Arc<AtomicBool>,
    data: mpsc::Sender<Dispatch>,
    control: mpsc::Sender<Dispatch>,
    channels: [tokio::task::AbortHandle; 2],
    sessions: Mutex<BTreeMap<BrowserSessionId, Session>>,
}

impl BrowserBridgeDriver {
    /// Bind two independently authenticated inherited channels to accepted native evidence.
    /// Requires an active Tokio runtime. Child self-report cannot supply this enrollment.
    pub async fn connect(
        data: InheritedBrowserChannel,
        control: InheritedBrowserChannel,
        enrollment: BrowserBridgeEnrollment,
        key: BrowserBridgeKey,
    ) -> Result<Self, BrowserDriverError> {
        enrollment.validate()?;
        tokio::runtime::Handle::try_current().map_err(|_| BrowserDriverError::Unavailable)?;
        let available = Arc::new(AtomicBool::new(false));
        let key = Arc::new(key);
        let capacity = usize::from(enrollment.capabilities.limits.max_concurrent_actions);
        let (data, data_owner) = transport::spawn_channel(
            AuthenticatedChannel::new(data, Arc::clone(&key), &enrollment, b"data")?,
            Arc::clone(&available),
            capacity,
        );
        let (control, control_owner) = transport::spawn_channel(
            AuthenticatedChannel::new(control, key, &enrollment, b"control")?,
            Arc::clone(&available),
            usize::from(enrollment.capabilities.limits.max_sessions) * 2,
        );
        let driver = Self {
            enrollment,
            available,
            data,
            control,
            channels: [data_owner, control_owner],
            sessions: Mutex::new(BTreeMap::new()),
        };
        for channel in [&driver.data, &driver.control] {
            let response =
                transport::send(channel, Request::Ready {}, None, driver.cleanup_deadline())
                    .await?;
            match response
                .await
                .map_err(|_| BrowserDriverError::Unavailable)?
            {
                Ok(Response::Ready {}) => {}
                _ => return Err(BrowserDriverError::Unavailable),
            }
        }
        driver.available.store(true, Ordering::Release);
        Ok(driver)
    }

    /// Exact outstanding native cleanup obligations, including failed/unknown allocations.
    pub fn owned_session_count(&self) -> Result<usize, BrowserDriverError> {
        Ok(self
            .sessions
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .len())
    }

    /// Fence dispatch and close both exact native channel owners to request host exit.
    ///
    /// Pending or queued effects become unknown and are never replayed. Channel closure
    /// is not a native shutdown acknowledgement: the supervisor retains every session
    /// until proxy/socket drain, `CefShutdown`/process exit and profile cleanup are proved.
    pub fn disconnect_for_shutdown(&self) {
        self.available.store(false, Ordering::Release);
        for channel in &self.channels {
            channel.abort();
        }
    }

    async fn call(
        &self,
        request: Request,
        session: BrowserSessionId,
        generation: u64,
        control: &BrowserDriverControl,
        deadline: Duration,
    ) -> Result<Response, BrowserDriverError> {
        if control.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        let mut receiver =
            transport::send(&self.data, request, Some(control.clone()), deadline).await?;
        // A dispatched frame has an independent owner. Dropping this caller cannot
        // drop a half-read frame or bypass the authenticated quiescence channel.
        let mut cleanup = CancelOnDrop {
            channel: self.control.clone(),
            available: Arc::clone(&self.available),
            request: Some(Request::Cancel {
                session_id: session,
                through_generation: generation,
            }),
            deadline: self.cleanup_deadline(),
        };
        let end = Instant::now() + deadline;
        let mut poll = interval(POLL);
        loop {
            tokio::select! {
                response = &mut receiver => {
                    let response = response.map_err(|_| BrowserDriverError::OutcomeUnknown)?
                        .and_then(transport::checked_response);
                    if !matches!(response, Err(BrowserDriverError::OutcomeUnknown)) { cleanup.request = None; }
                    return response;
                }
                _ = poll.tick() => {
                    if control.is_cancelled() || Instant::now() >= end {
                        return Err(BrowserDriverError::OutcomeUnknown);
                    }
                }
            }
        }
    }

    fn cleanup_deadline(&self) -> Duration {
        Duration::from_millis(u64::from(
            self.enrollment.capabilities.limits.navigation_timeout_ms,
        ))
    }

    async fn control(&self, request: Request) -> Result<(), BrowserDriverError> {
        let receiver =
            transport::send(&self.control, request, None, self.cleanup_deadline()).await?;
        let response = receiver
            .await
            .map_err(|_| BrowserDriverError::OutcomeUnknown)??;
        if matches!(
            transport::checked_response(response)?,
            Response::Acknowledged {}
        ) {
            Ok(())
        } else {
            self.available.store(false, Ordering::Release);
            Err(BrowserDriverError::OutcomeUnknown)
        }
    }
}

impl Drop for BrowserBridgeDriver {
    fn drop(&mut self) {
        self.disconnect_for_shutdown();
    }
}

struct CancelOnDrop {
    channel: mpsc::Sender<Dispatch>,
    available: Arc<AtomicBool>,
    request: Option<Request>,
    deadline: Duration,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(request) = self.request.take() {
            self.available.store(false, Ordering::Release);
            let (response, _) = oneshot::channel();
            if self
                .channel
                .try_send(Dispatch {
                    request,
                    control: None,
                    deadline: self.deadline,
                    response,
                })
                .is_err()
            {
                self.available.store(false, Ordering::Release);
            }
        }
    }
}

#[async_trait]
impl BrowserDriver for BrowserBridgeDriver {
    fn cancellation_disposition(&self) -> BrowserCancelDisposition {
        if self.enrollment.cancellation_closes_context {
            BrowserCancelDisposition::ClosesContext
        } else {
            BrowserCancelDisposition::RetainsContext
        }
    }

    fn capabilities(&self) -> BrowserCapabilities {
        if self.available.load(Ordering::Acquire) {
            self.enrollment.capabilities.clone()
        } else {
            BrowserCapabilities::unavailable()
        }
    }

    async fn open_session(
        &self,
        request: BrowserDriverOpenRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        if !self.available.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Unavailable);
        }
        crate::validation::open(&request, &self.enrollment)?;
        if control.is_cancelled() {
            return Err(BrowserDriverError::Cancelled);
        }
        let id = request.session_id.clone();
        let tab_id = request.tab_id.clone();
        let document_id = request.document_id.clone();
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if sessions.contains_key(&id) {
                return Err(BrowserDriverError::Stale);
            }
            if sessions.len() >= usize::from(self.enrollment.capabilities.limits.max_sessions) {
                return Err(BrowserDriverError::LimitExceeded);
            }
            sessions.insert(
                id.clone(),
                Session {
                    generation: 0,
                    closing: false,
                },
            );
        }
        let result = self
            .call(
                Request::Open {
                    request: Box::new(request),
                },
                id.clone(),
                0,
                control,
                self.cleanup_deadline(),
            )
            .await;
        match result {
            Ok(Response::Opened { tab })
                if tab.tab_id == tab_id
                    && tab.document_id == document_id
                    && tab.title.len() <= 1024 =>
            {
                Ok(tab)
            }
            Ok(_) => {
                self.available.store(false, Ordering::Release);
                let _ = self.cancel_session(&id).await;
                Err(BrowserDriverError::OutcomeUnknown)
            }
            Err(error) => {
                // A categorical rejection proves no allocation except unknown outcome.
                // Uncertain sessions deliberately remain owned until close acknowledgement.
                if error != BrowserDriverError::OutcomeUnknown {
                    self.sessions
                        .lock()
                        .map_err(|_| BrowserDriverError::OutcomeUnknown)?
                        .remove(&id);
                }
                Err(error)
            }
        }
    }

    async fn execute(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        if !self.available.load(Ordering::Acquire) {
            return Err(BrowserDriverError::Unavailable);
        }
        if !self
            .enrollment
            .capabilities
            .actions
            .contains(&command.action.kind())
        {
            return Err(BrowserDriverError::Unsupported);
        }
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            let session = sessions
                .get_mut(&command.session_id)
                .ok_or(BrowserDriverError::Stale)?;
            if session.closing || command.control_generation < session.generation {
                return Err(BrowserDriverError::Stale);
            }
            session.generation = command.control_generation;
        }
        let session_id = command.session_id.clone();
        let generation = command.control_generation;
        let deadline = Duration::from_millis(u64::from(
            self.enrollment.capabilities.limits.action_timeout_ms,
        ));
        match self
            .call(
                Request::Execute {
                    command: Box::new(command),
                },
                session_id.clone(),
                generation,
                control,
                deadline,
            )
            .await?
        {
            Response::Observed { observation }
                if observation.session_id == session_id
                    && crate::validation::bounded_observation(
                        &observation,
                        self.enrollment.capabilities.limits.max_observation_bytes as usize,
                    )
                    .is_ok() =>
            {
                Ok(observation)
            }
            _ => {
                self.available.store(false, Ordering::Release);
                let _ = self.cancel_session(&session_id).await;
                Err(BrowserDriverError::OutcomeUnknown)
            }
        }
    }

    async fn cancel_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        let generation = self
            .sessions
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .get(session)
            .map_or(0, |session| session.generation);
        self.control(Request::Cancel {
            session_id: session.clone(),
            through_generation: generation,
        })
        .await
    }

    async fn capture(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotDescriptor, BrowserDriverError> {
        self.capture_private(command, control).await
    }
    async fn read_screenshot_chunk(
        &self,
        request: BrowserScreenshotReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        self.read_screenshot_private(request, control).await
    }
    async fn prepare_upload(
        &self,
        request: BrowserUploadPrepareRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.prepare_upload_private(request, control).await
    }
    async fn write_upload_chunk(
        &self,
        request: BrowserUploadWriteRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserUploadReceipt, BrowserDriverError> {
        self.write_upload_private(request, control).await
    }
    async fn commit_upload(
        &self,
        request: BrowserUploadCommitRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserObservation, BrowserDriverError> {
        self.commit_upload_private(request, control).await
    }
    async fn download(
        &self,
        command: BrowserDriverCommand,
        control: &BrowserDriverControl,
    ) -> Result<BrowserDownloadDescriptor, BrowserDriverError> {
        self.download_private(command, control).await
    }
    async fn read_download_chunk(
        &self,
        request: BrowserDownloadReadRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserScreenshotChunk, BrowserDriverError> {
        self.read_download_private(request, control).await
    }
    async fn confirm_native_handoff(
        &self,
        request: BrowserNativeHandoffRequest,
        control: &BrowserDriverControl,
    ) -> Result<BrowserTabSummary, BrowserDriverError> {
        self.confirm_handoff_private(request, control).await
    }

    async fn close_session(&self, session: &BrowserSessionId) -> Result<(), BrowserDriverError> {
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| BrowserDriverError::OutcomeUnknown)?;
            if let Some(session) = sessions.get_mut(session) {
                session.closing = true;
            } else {
                return Ok(());
            }
        }
        self.control(Request::Close {
            session_id: session.clone(),
        })
        .await?;
        self.sessions
            .lock()
            .map_err(|_| BrowserDriverError::OutcomeUnknown)?
            .remove(session);
        Ok(())
    }
}
