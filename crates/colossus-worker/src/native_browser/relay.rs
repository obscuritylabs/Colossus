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
        }
    }
    fn active(&self) -> Result<(), PresentationError> {
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
        let lease = self.client.configure(value.clone()).await?;
        // Only the authenticated private native ACK is passed to this non-wire Runtime seam.
        self.session
            .confirm_presentation_lease(&value, lease)
            .map_err(|_| PresentationError::OutcomeUnknown)?;
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
                let state = self.client.observe(lease).await?;
                self.session
                    .confirm_presentation_state(lease, &state)
                    .map_err(|_| PresentationError::OutcomeUnknown)?;
                self.acknowledged()?;
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
        let Some(bytes) = self.client.next_frame(lease).await? else {
            return Ok(None);
        };
        self.acknowledged()?;
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
        let _ = self.service.finish(self.session.session_id(), false).await;
    }
}
