//! GUI relay never accepts a human fence receipt or run authority from its caller.
use super::*;
use colossus_browser_presentation::{
    Configure, Frame, FrameCodec, Lease, PresentationClient, PresentationCommand,
    PresentationError, PresentationHost, PresentationReply,
};

pub(super) struct NativeRelay {
    service: Arc<NativeBrowserService>,
    session: Arc<RuntimeNativeBrowserSession>,
    client: PresentationClient,
    revoked: AtomicBool,
    frames: Mutex<Option<(Lease, FrameCodec)>>,
    #[cfg(all(windows, debug_assertions))]
    observe_diagnostics: std::sync::atomic::AtomicU8,
}
impl NativeRelay {
    pub(super) fn new(
        service: Arc<NativeBrowserService>,
        session: Arc<RuntimeNativeBrowserSession>,
    ) -> Self {
        Self {
            client: session.presentation(),
            service,
            session,
            revoked: AtomicBool::new(false),
            frames: Mutex::new(None),
            #[cfg(all(windows, debug_assertions))]
            observe_diagnostics: std::sync::atomic::AtomicU8::new(0),
        }
    }
    #[cfg(all(windows, debug_assertions))]
    fn trace_observe(&self, bit: u8, stage: &'static str) {
        if self.observe_diagnostics.fetch_or(bit, Ordering::AcqRel) & bit == 0 {
            // Fixed provenance categories only: no target, page or native payload.
            eprintln!("EVIDENCE native-relay-observe stage={stage}");
        }
    }
    fn active(&self) -> Result<(), PresentationError> {
        if self
            .session
            .is_expired()
            .map_err(|_| PresentationError::OutcomeUnknown)?
        {
            return Err(PresentationError::Expired);
        }
        if !self.revoked.load(Ordering::Acquire) && self.service.authorized() {
            Ok(())
        } else {
            Err(PresentationError::Hidden)
        }
    }
    fn acknowledged(&self) -> Result<(), PresentationError> {
        // Losing authority after dispatch cannot be reported as a no-effect rejection.
        self.active().map_err(|_| PresentationError::OutcomeUnknown)
    }
    fn read_error(&self, error: PresentationError) -> PresentationError {
        if self.session.is_expired().unwrap_or(false) {
            PresentationError::Expired
        } else {
            error
        }
    }
    async fn configure(&self, value: Configure) -> Result<PresentationReply, PresentationError> {
        let current = self
            .session
            .configure()
            .map_err(|_| PresentationError::Stale)?;
        if current.session != value.session
            || current.target != value.target
            || current.control_generation != value.control_generation
        {
            return Err(PresentationError::Stale);
        }
        if self
            .frames
            .lock()
            .map_err(|_| PresentationError::OutcomeUnknown)?
            .as_ref()
            .is_some_and(|(prior, _)| value.viewport_generation <= prior.viewport_generation)
        {
            return Err(PresentationError::Stale);
        }
        let lease = if value == current && value.control_generation > 0 {
            match self
                .session
                .take_handoff_presentation_lease(&value)
                .map_err(|_| PresentationError::Stale)?
            {
                Some(lease) => lease,
                None => return Err(PresentationError::Stale),
            }
        } else {
            let lease = self.client.configure(value.clone()).await?;
            // Only the authenticated private native ACK reaches Runtime.
            self.session
                .confirm_presentation_lease(&value, lease)
                .map_err(|_| PresentationError::OutcomeUnknown)?;
            lease
        };
        self.acknowledged()?;
        *self
            .frames
            .lock()
            .map_err(|_| PresentationError::OutcomeUnknown)? = Some((
            lease,
            FrameCodec::new(
                self.client.surface_key(),
                self.session.enrollment_digest(),
                lease,
            )?,
        ));
        Ok(PresentationReply::Configured(lease))
    }
}
#[async_trait]
impl PresentationHost for NativeRelay {
    async fn dispatch(
        &self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError> {
        self.active()?;
        match command {
            PresentationCommand::Configure(value) => self.configure(value).await,
            PresentationCommand::Observe { lease } => {
                let state = self.client.observe(lease).await.map_err(|error| {
                    #[cfg(all(windows, debug_assertions))]
                    self.trace_observe(1, "native_rejected");
                    self.read_error(error)
                })?;
                self.session
                    .confirm_presentation_state(lease, &state)
                    .map_err(|error| {
                        #[cfg(all(windows, debug_assertions))]
                        self.trace_observe(2, "runtime_rejected");
                        match error {
                            // A concurrent handoff, viewport or document transition
                            // can retire this read-only observation. Retire its view
                            // without classifying a known stale query as a lost effect.
                            colossus_runtime::RuntimeNativeBrowserError::Stale => {
                                PresentationError::Stale
                            }
                            _ => PresentationError::OutcomeUnknown,
                        }
                    })?;
                self.active()?;
                #[cfg(all(windows, debug_assertions))]
                self.trace_observe(4, "accepted");
                Ok(PresentationReply::State(state))
            }
            // Handoff is exclusively Runtime's gateway operation over its retained native client.
            PresentationCommand::FenceHuman { .. }
            | PresentationCommand::Ready
            | PresentationCommand::Poll { .. } => Err(PresentationError::Invalid),
            command => {
                self.client.command(command).await?;
                self.acknowledged()?;
                Ok(PresentationReply::Ack)
            }
        }
    }
    async fn frame(&self, lease: Lease) -> Result<Option<Frame>, PresentationError> {
        self.active()?;
        let Some(bytes) = self
            .client
            .next_frame(lease)
            .await
            .map_err(|error| self.read_error(error))?
        else {
            return Ok(None);
        };
        self.active()?;
        let mut frames = self
            .frames
            .lock()
            .map_err(|_| PresentationError::OutcomeUnknown)?;
        let (owned, codec) = frames.as_mut().ok_or(PresentationError::Stale)?;
        if *owned != lease {
            return Err(PresentationError::Stale);
        }
        Ok(Some(codec.decode(bytes)?))
    }
    fn revoke_now(&self) {
        self.revoked.store(true, Ordering::Release);
    }
    async fn revoke(&self) {
        self.revoke_now();
        // Detach hides the viewer but retains the run-owned host channel after transfer.
        #[cfg(windows)]
        let _ = self.service.finish_owned(&self.session).await;
        #[cfg(not(windows))]
        let _ = self.service.finish(self.session.session_id(), false).await;
    }
}
