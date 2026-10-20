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
    #[cfg(all(windows, debug_assertions))]
    native_diagnostics: std::sync::atomic::AtomicU32,
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
            #[cfg(all(windows, debug_assertions))]
            native_diagnostics: std::sync::atomic::AtomicU32::new(0),
        }
    }
    #[cfg(all(windows, debug_assertions))]
    fn trace_observe(&self, bit: u8, stage: &'static str) {
        if self.observe_diagnostics.fetch_or(bit, Ordering::AcqRel) & bit == 0 {
            // Fixed provenance categories only: no target, page or native payload.
            eprintln!("EVIDENCE native-relay-observe stage={stage}");
        }
    }
    #[cfg(all(windows, debug_assertions))]
    fn trace_native(&self, stage: &'static str, error: Option<PresentationError>) {
        let offset = match stage {
            "observe_failed" => 0,
            "configure_dispatch" => 7,
            "configure_ack" => 8,
            "configure_failed" => 9,
            "hide_dispatch" => 16,
            "hide_ack" => 17,
            "hide_failed" => 18,
            _ => return,
        };
        let (category, failure) = match error {
            None => (0, "none"),
            Some(PresentationError::Expired) => (0, "expired"),
            Some(PresentationError::Invalid) => (1, "invalid"),
            Some(PresentationError::Stale) => (2, "stale"),
            Some(PresentationError::Unauthenticated) => (3, "unauthenticated"),
            Some(PresentationError::Hidden) => (4, "hidden"),
            Some(PresentationError::LimitExceeded) => (5, "limit_exceeded"),
            Some(PresentationError::OutcomeUnknown) => (6, "outcome_unknown"),
        };
        let bit = 1 << (offset + category);
        if self.native_diagnostics.fetch_or(bit, Ordering::AcqRel) & bit == 0 {
            // At most one fixed line per stage/error category for this relay.
            // A prior benign stale read cannot suppress a later channel failure.
            eprintln!("EVIDENCE native-relay-native stage={stage} failure={failure}");
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
            #[cfg(all(windows, debug_assertions))]
            self.trace_native("configure_dispatch", None);
            let configured = self.client.configure(value.clone()).await;
            #[cfg(all(windows, debug_assertions))]
            match configured {
                Ok(_) => self.trace_native("configure_ack", None),
                Err(error) => self.trace_native("configure_failed", Some(error)),
            }
            let lease = configured?;
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
                    {
                        self.trace_observe(1, "native_rejected");
                        self.trace_native("observe_failed", Some(error));
                    }
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
                #[cfg(all(windows, debug_assertions))]
                let hiding = matches!(command, PresentationCommand::Hide { .. });
                #[cfg(all(windows, debug_assertions))]
                if hiding {
                    self.trace_native("hide_dispatch", None);
                }
                let dispatched = self.client.command(command).await;
                #[cfg(all(windows, debug_assertions))]
                if hiding {
                    match dispatched {
                        Ok(()) => self.trace_native("hide_ack", None),
                        Err(error) => self.trace_native("hide_failed", Some(error)),
                    }
                }
                dispatched?;
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
