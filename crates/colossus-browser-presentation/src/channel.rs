//! One private bounded presentation channel; automation cancellation is independent.
use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _},
    sync::{mpsc, oneshot},
    task::AbortHandle,
};
use zeroize::Zeroizing;

#[cfg(all(windows, debug_assertions))]
#[path = "channel_diagnostics.rs"]
mod diagnostics;

use crate::{
    CONTROL_HEADER_BYTES, ClientIdentityChallenge, ClientIdentityKey, ClientIdentityPolicy,
    Configure, ControlCodec, Frame, FrameCodec, HEADER_BYTES, HumanFenceReceipt, Input, Lease,
    PageState, PresentationCommand, PresentationError, PresentationReply, Role,
};

// A Retina BGRA paint can cross the native host and worker relay before the
// presenter receives its authenticated copy. Read-only metadata also waits for
// the CEF-owning thread during page loading. Keep effects on the short deadline;
// only Poll and Observe use the bounded read budget. Native leases still expire
// independently and a completed read cannot authorize input or renew visibility.
const CONTROL_OPERATION_TIMEOUT: Duration = Duration::from_millis(750);
const FRAME_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
// Callers may queue behind one in-flight frame. The worker still applies the
// short per-effect deadline after dispatch, so this wait grants no extra
// execution time or retry to a human command.
const CALL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(6);

fn operation_timeout(command: &PresentationCommand) -> Duration {
    if matches!(
        command,
        PresentationCommand::Poll { .. } | PresentationCommand::Observe { .. }
    ) {
        FRAME_OPERATION_TIMEOUT
    } else {
        CONTROL_OPERATION_TIMEOUT
    }
}

trait Reader: AsyncRead + Unpin + Send {}
impl<T: AsyncRead + Unpin + Send> Reader for T {}
trait Writer: AsyncWrite + Unpin + Send {}
impl<T: AsyncWrite + Unpin + Send> Writer for T {}

/// Exclusively owned, supervisor-created native channel; it names no public endpoint.
pub struct PresentationChannel {
    reader: Box<dyn Reader>,
    writer: Box<dyn Writer>,
}
impl PresentationChannel {
    /// Move positively verified inherited channel halves into their sole owner.
    pub fn new(
        reader: impl AsyncRead + Unpin + Send + 'static,
        writer: impl AsyncWrite + Unpin + Send + 'static,
    ) -> Self {
        Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
        }
    }
}

/// Native host effects run on its CEF-owning thread through a bounded adapter queue.
#[async_trait]
pub trait PresentationHost: Send + Sync + 'static {
    /// Repeat immutable enrollment, opaque ownership and current lease checks.
    async fn dispatch(
        &self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError>;
    /// Obtain at most one latest bounded native paint, under the exact current lease.
    async fn frame(&self, lease: Lease) -> Result<Option<Frame>, PresentationError>;
    /// Immediately fence pending native human effects, including dropped serve futures.
    fn revoke_now(&self);
    /// Revoke human visibility, input and pending commands after private channel loss.
    /// This does not prove browser/process cleanup or cancel automation authority.
    async fn revoke(&self);
}

struct RevokeOnDrop(Arc<dyn PresentationHost>);
impl Drop for RevokeOnDrop {
    fn drop(&mut self) {
        self.0.revoke_now();
    }
}

async fn read_control<T: for<'de> serde::Deserialize<'de>>(
    reader: &mut dyn Reader,
    codec: &mut ControlCodec,
) -> Result<T, PresentationError> {
    let mut header = [0_u8; CONTROL_HEADER_BYTES];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|_| PresentationError::OutcomeUnknown)?;
    let length = codec.payload_length(&header)?;
    let mut bytes = vec![0; CONTROL_HEADER_BYTES + length];
    bytes[..CONTROL_HEADER_BYTES].copy_from_slice(&header);
    reader
        .read_exact(&mut bytes[CONTROL_HEADER_BYTES..])
        .await
        .map_err(|_| PresentationError::OutcomeUnknown)?;
    codec.decode(&bytes)
}
async fn write_control(
    writer: &mut dyn Writer,
    codec: &mut ControlCodec,
    reply: &PresentationReply,
) -> Result<(), PresentationError> {
    writer
        .write_all(&codec.encode(reply)?)
        .await
        .map_err(|_| PresentationError::OutcomeUnknown)
}

/// Serve a separately authenticated native presentation role. Disconnect revokes input.
/// The caller still owns whole-host cleanup; successful replies are no containment proof.
pub async fn serve_presentation(
    mut channel: PresentationChannel,
    key: Zeroizing<[u8; 32]>,
    enrollment: [u8; 32],
    host: Arc<dyn PresentationHost>,
) -> Result<(), PresentationError> {
    let _revoke_on_drop = RevokeOnDrop(Arc::clone(&host));
    #[cfg(all(windows, debug_assertions))]
    let diagnostics = diagnostics::Diagnostics::new("server");
    let mut incoming = ControlCodec::new(Zeroizing::new(*key), enrollment, Role::NativeToHost);
    let mut outgoing = ControlCodec::new(Zeroizing::new(*key), enrollment, Role::HostToNative);
    let mut frames: Option<FrameCodec> = None;
    let result = async {
        let ready: PresentationCommand = tokio::time::timeout(
            Duration::from_secs(5),
            read_control(channel.reader.as_mut(), &mut incoming),
        )
        .await
        .map_err(|_| PresentationError::OutcomeUnknown)??;
        if !matches!(ready, PresentationCommand::Ready) {
            return Err(PresentationError::Unauthenticated);
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            write_control(
                channel.writer.as_mut(),
                &mut outgoing,
                &PresentationReply::Ready,
            ),
        )
        .await
        .map_err(|_| PresentationError::OutcomeUnknown)??;
        loop {
            let command: PresentationCommand =
                read_control(channel.reader.as_mut(), &mut incoming).await?;
            command.validate()?;
            if matches!(command, PresentationCommand::Ready) {
                return Err(PresentationError::Stale);
            }
            let budget = operation_timeout(&command);
            #[cfg(all(windows, debug_assertions))]
            let trace = diagnostics.begin(&command);
            #[cfg(all(windows, debug_assertions))]
            let mut read_trace = diagnostics.begin_read(&command);
            #[cfg(all(windows, debug_assertions))]
            let mut read_error = None;
            let result = tokio::time::timeout(budget, async {
                if let PresentationCommand::Poll { lease } = command {
                    match host.frame(lease).await {
                        Ok(Some(frame)) => {
                            if frame.lease != lease {
                                return Err(PresentationError::Stale);
                            }
                            let codec = frames.as_mut().ok_or(PresentationError::Stale)?;
                            let bytes = codec.encode(&frame)?;
                            write_control(
                                channel.writer.as_mut(),
                                &mut outgoing,
                                &PresentationReply::Frame(lease),
                            )
                            .await?;
                            channel
                                .writer
                                .write_all(&bytes)
                                .await
                                .map_err(|_| PresentationError::OutcomeUnknown)?;
                        }
                        Ok(None) => {
                            write_control(
                                channel.writer.as_mut(),
                                &mut outgoing,
                                &PresentationReply::Empty,
                            )
                            .await?
                        }
                        Err(error) => {
                            #[cfg(all(windows, debug_assertions))]
                            {
                                read_error = Some(error);
                            }
                            write_control(
                                channel.writer.as_mut(),
                                &mut outgoing,
                                &PresentationReply::Error(error),
                            )
                            .await?;
                            if error == PresentationError::OutcomeUnknown {
                                return Err(error);
                            }
                        }
                    }
                } else {
                    #[cfg(all(windows, debug_assertions))]
                    diagnostics::emit(trace, "host_dispatch", None);
                    let reply = match host.dispatch(command).await {
                        Ok(reply) => reply,
                        Err(error) => PresentationReply::Error(error),
                    };
                    #[cfg(all(windows, debug_assertions))]
                    if let PresentationReply::Error(error) = &reply {
                        read_error = Some(*error);
                    }
                    #[cfg(all(windows, debug_assertions))]
                    diagnostics::emit(
                        trace,
                        "host_return",
                        match &reply {
                            PresentationReply::Error(error) => Some(*error),
                            _ => None,
                        },
                    );
                    if let PresentationReply::Configured(lease) = &reply {
                        lease.validate()?;
                        frames = Some(FrameCodec::new(Zeroizing::new(*key), enrollment, *lease)?);
                    }
                    if let PresentationReply::State(state) = &reply {
                        state.validate()?;
                    }
                    if let PresentationReply::Fenced(receipt) = &reply {
                        receipt.validate()?;
                    }
                    if let PresentationReply::ClientIdentity {
                        challenge: Some(challenge),
                    } = &reply
                    {
                        challenge.validate()?;
                    }
                    write_control(channel.writer.as_mut(), &mut outgoing, &reply).await?;
                    if matches!(
                        reply,
                        PresentationReply::Error(PresentationError::OutcomeUnknown)
                    ) {
                        return Err(PresentationError::OutcomeUnknown);
                    }
                }
                #[cfg(all(windows, debug_assertions))]
                diagnostics::read_stage(&mut read_trace, "response_write_complete");
                Ok(())
            })
            .await;
            #[cfg(all(windows, debug_assertions))]
            diagnostics.finish_read(
                read_trace,
                match &result {
                    Ok(Ok(())) => read_error,
                    Ok(Err(error)) => Some(*error),
                    Err(_) => Some(PresentationError::OutcomeUnknown),
                },
            );
            let result = result.map_err(|_| {
                #[cfg(all(windows, debug_assertions))]
                diagnostics::emit(
                    trace,
                    "serve_timeout",
                    Some(PresentationError::OutcomeUnknown),
                );
                PresentationError::OutcomeUnknown
            })?;
            result?;
        }
    }
    .await;
    host.revoke_now();
    let _ = tokio::time::timeout(CONTROL_OPERATION_TIMEOUT, host.revoke()).await;
    result
}

type Response = Result<(PresentationReply, Option<Vec<u8>>), PresentationError>;
fn valid_reply(command: &PresentationCommand, reply: &PresentationReply) -> bool {
    if matches!(reply, PresentationReply::Error(_)) {
        return true;
    }
    match (command, reply) {
        (PresentationCommand::Configure(value), PresentationReply::Configured(lease)) => {
            value.accepts(*lease)
        }
        (PresentationCommand::Observe { .. }, PresentationReply::State(_)) => true,
        (
            PresentationCommand::PendingClientIdentity { .. },
            PresentationReply::ClientIdentity { challenge },
        ) => challenge
            .as_ref()
            .is_none_or(|value| value.validate().is_ok()),
        (PresentationCommand::FenceHuman { lease }, PresentationReply::Fenced(receipt)) => {
            lease == &receipt.prior_lease && receipt.validate().is_ok()
        }
        (PresentationCommand::Poll { lease }, PresentationReply::Frame(actual)) => lease == actual,
        (PresentationCommand::Poll { .. }, PresentationReply::Empty) => true,
        (
            PresentationCommand::Input { .. }
            | PresentationCommand::Focus { .. }
            | PresentationCommand::Hide { .. }
            | PresentationCommand::Renew { .. }
            | PresentationCommand::Human { .. }
            | PresentationCommand::SelectClientIdentity { .. },
            PresentationReply::Ack,
        ) => true,
        _ => false,
    }
}
struct Call {
    command: PresentationCommand,
    response: oneshot::Sender<Response>,
    #[cfg(all(windows, debug_assertions))]
    trace: Option<diagnostics::Trace>,
}
struct ClientOwner {
    sender: mpsc::Sender<Call>,
    worker: AbortHandle,
    key: Zeroizing<[u8; 32]>,
    #[cfg(all(windows, debug_assertions))]
    diagnostics: Arc<diagnostics::Diagnostics>,
}
impl Drop for ClientOwner {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

/// Native-only private client. Dropped effects revoke input; abandoned reads drain
/// their authenticated replies within the original deadline to preserve framing.
#[derive(Clone)]
pub struct PresentationClient(Arc<ClientOwner>);
impl PresentationClient {
    /// Authenticate the exact channel and enrollment before creating a native surface.
    pub async fn connect(
        mut channel: PresentationChannel,
        key: Zeroizing<[u8; 32]>,
        enrollment: [u8; 32],
    ) -> Result<Self, PresentationError> {
        let mut outgoing = ControlCodec::new(Zeroizing::new(*key), enrollment, Role::NativeToHost);
        let mut incoming = ControlCodec::new(Zeroizing::new(*key), enrollment, Role::HostToNative);
        tokio::time::timeout(
            Duration::from_secs(5),
            channel
                .writer
                .write_all(&outgoing.encode(&PresentationCommand::Ready)?),
        )
        .await
        .map_err(|_| PresentationError::OutcomeUnknown)?
        .map_err(|_| PresentationError::OutcomeUnknown)?;
        let ready: PresentationReply = tokio::time::timeout(
            Duration::from_secs(5),
            read_control(channel.reader.as_mut(), &mut incoming),
        )
        .await
        .map_err(|_| PresentationError::OutcomeUnknown)??;
        if !matches!(ready, PresentationReply::Ready) {
            return Err(PresentationError::Unauthenticated);
        }
        let (sender, mut receiver) = mpsc::channel::<Call>(16);
        let frame_key = Zeroizing::new(*key);
        #[cfg(all(windows, debug_assertions))]
        let diagnostics = Arc::new(diagnostics::Diagnostics::new("client"));
        #[cfg(all(windows, debug_assertions))]
        let worker_diagnostics = Arc::clone(&diagnostics);
        let worker = tokio::spawn(async move {
            let mut frames: Option<FrameCodec> = None;
            while let Some(mut call) = receiver.recv().await {
                if call.response.is_closed() {
                    #[cfg(all(windows, debug_assertions))]
                    diagnostics::emit(call.trace, "cancel_before_dispatch", None);
                    continue;
                }
                let read_only_frame = matches!(&call.command, PresentationCommand::Poll { .. });
                let read_only = read_only_frame
                    || matches!(
                        &call.command,
                        PresentationCommand::Observe { .. }
                            | PresentationCommand::PendingClientIdentity {
                                policy: ClientIdentityPolicy::Review,
                            }
                    );
                let budget = operation_timeout(&call.command);
                #[cfg(all(windows, debug_assertions))]
                let mut read_trace = worker_diagnostics.begin_read(&call.command);
                let operation = async {
                    #[cfg(all(windows, debug_assertions))]
                    diagnostics::emit(call.trace, "dispatch", None);
                    channel
                        .writer
                        .write_all(&outgoing.encode(&call.command)?)
                        .await
                        .map_err(|_| PresentationError::OutcomeUnknown)?;
                    #[cfg(all(windows, debug_assertions))]
                    {
                        diagnostics::emit(call.trace, "wire_write_complete", None);
                        diagnostics::read_stage(&mut read_trace, "wire_write_complete");
                    }
                    let reply: PresentationReply =
                        read_control(channel.reader.as_mut(), &mut incoming).await?;
                    #[cfg(all(windows, debug_assertions))]
                    {
                        diagnostics::emit(call.trace, "decoded_reply", None);
                        diagnostics::read_stage(&mut read_trace, "decoded_reply");
                    }
                    if !valid_reply(&call.command, &reply) {
                        return Err(PresentationError::OutcomeUnknown);
                    }
                    let frame = if let PresentationReply::Frame(lease) = &reply {
                        if !matches!(&call.command, PresentationCommand::Poll { lease: expected } if expected == lease)
                        {
                            return Err(PresentationError::Stale);
                        }
                        let codec = frames.as_mut().ok_or(PresentationError::Stale)?;
                        let mut header = [0_u8; HEADER_BYTES];
                        channel
                            .reader
                            .read_exact(&mut header)
                            .await
                            .map_err(|_| PresentationError::OutcomeUnknown)?;
                        let length = codec.payload_length(&header)?;
                        let mut bytes = vec![0; HEADER_BYTES + length];
                        bytes[..HEADER_BYTES].copy_from_slice(&header);
                        channel
                            .reader
                            .read_exact(&mut bytes[HEADER_BYTES..])
                            .await
                            .map_err(|_| PresentationError::OutcomeUnknown)?;
                        let decoded = codec.decode(bytes.clone())?;
                        if decoded.lease != *lease {
                            return Err(PresentationError::Stale);
                        }
                        Some(bytes)
                    } else {
                        None
                    };
                    if let PresentationReply::Configured(lease) = &reply {
                        if !matches!(&call.command, PresentationCommand::Configure(value) if value.accepts(*lease))
                        {
                            return Err(PresentationError::Stale);
                        }
                        lease.validate()?;
                        frames = Some(FrameCodec::new(
                            Zeroizing::new(*frame_key),
                            enrollment,
                            *lease,
                        )?);
                    }
                    if let PresentationReply::State(state) = &reply {
                        state.validate()?;
                    }
                    if let PresentationReply::Fenced(receipt) = &reply {
                        receipt.validate()?;
                    }
                    if let PresentationReply::ClientIdentity {
                        challenge: Some(challenge),
                    } = &reply
                    {
                        challenge.validate()?;
                    }
                    #[cfg(all(windows, debug_assertions))]
                    diagnostics::read_stage(&mut read_trace, "read_complete");
                    if let PresentationReply::Error(error) = reply {
                        if error == PresentationError::OutcomeUnknown {
                            return Err(error);
                        }
                        return Ok((PresentationReply::Error(error), None));
                    }
                    Ok((reply, frame))
                };
                let result = tokio::select! {
                    // A UI snapshot or native Review lookup can expire after
                    // dispatch, or during a frame transfer. These reads cannot
                    // select a key or change authority. Drain their bounded replies
                    // before the next call to preserve authenticated framing.
                    // AutomaticIfUnique remains effectful and closes on abandon.
                    _ = call.response.closed(), if !read_only => {
                        #[cfg(all(windows, debug_assertions))]
                        diagnostics::emit(call.trace, "response_dropped", None);
                        break;
                    },
                    result = tokio::time::timeout(budget, operation) => match result {
                        Ok(Ok(reply)) => {
                            #[cfg(all(windows, debug_assertions))]
                            diagnostics::emit(call.trace, "reply", match &reply.0 {
                                PresentationReply::Error(error) => Some(*error),
                                _ => None,
                            });
                            Ok(reply)
                        },
                        // Poll has no human effect. Preserve a verified parser's
                        // categorical rejection for diagnostics, but terminate
                        // this channel below just as an uncertain effect does.
                        Ok(Err(error)) => {
                            #[cfg(all(windows, debug_assertions))]
                            diagnostics::emit(call.trace, "operation_error", Some(error));
                            Err(if read_only_frame { error } else { PresentationError::OutcomeUnknown })
                        },
                        Err(_) => {
                            #[cfg(all(windows, debug_assertions))]
                            diagnostics::emit(call.trace, "operation_timeout", Some(PresentationError::OutcomeUnknown));
                            Err(PresentationError::OutcomeUnknown)
                        },
                    },
                };
                #[cfg(all(windows, debug_assertions))]
                worker_diagnostics.finish_read(
                    read_trace,
                    match &result {
                        Ok((PresentationReply::Error(error), _)) | Err(error) => Some(*error),
                        _ => None,
                    },
                );
                let disconnected = result.is_err();
                let _ = call.response.send(result);
                if disconnected {
                    break;
                }
            }
        });
        Ok(Self(Arc::new(ClientOwner {
            sender,
            worker: worker.abort_handle(),
            key,
            #[cfg(all(windows, debug_assertions))]
            diagnostics,
        })))
    }
    /// Transfer a zeroizing native-only frame credential copy to the trusted pixel surface.
    pub fn surface_key(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(*self.0.key)
    }
    /// Close the private channel now; native host revokes presentation authority.
    pub fn disconnect(&self) {
        self.0.worker.abort();
    }
    async fn call(
        &self,
        command: PresentationCommand,
    ) -> Result<(PresentationReply, Option<Vec<u8>>), PresentationError> {
        command.validate()?;
        #[cfg(all(windows, debug_assertions))]
        let trace = self.0.diagnostics.begin(&command);
        #[cfg(all(windows, debug_assertions))]
        diagnostics::emit(trace, "queue_enter", None);
        let (response, receiver) = oneshot::channel();
        tokio::time::timeout(
            CONTROL_OPERATION_TIMEOUT,
            self.0.sender.send(Call {
                command,
                response,
                #[cfg(all(windows, debug_assertions))]
                trace,
            }),
        )
        .await
        .map_err(|_| {
            #[cfg(all(windows, debug_assertions))]
            diagnostics::emit(
                trace,
                "queue_timeout",
                Some(PresentationError::OutcomeUnknown),
            );
            PresentationError::OutcomeUnknown
        })?
        .map_err(|_| {
            #[cfg(all(windows, debug_assertions))]
            diagnostics::emit(
                trace,
                "queue_closed",
                Some(PresentationError::OutcomeUnknown),
            );
            PresentationError::OutcomeUnknown
        })?;
        #[cfg(all(windows, debug_assertions))]
        diagnostics::emit(trace, "queued", None);
        let result = tokio::time::timeout(CALL_RESPONSE_TIMEOUT, receiver)
            .await
            .map_err(|_| {
                #[cfg(all(windows, debug_assertions))]
                diagnostics::emit(
                    trace,
                    "response_timeout",
                    Some(PresentationError::OutcomeUnknown),
                );
                PresentationError::OutcomeUnknown
            })?
            .map_err(|_| {
                #[cfg(all(windows, debug_assertions))]
                diagnostics::emit(
                    trace,
                    "response_closed",
                    Some(PresentationError::OutcomeUnknown),
                );
                PresentationError::OutcomeUnknown
            })??;
        if let PresentationReply::Error(error) = result.0 {
            return Err(error);
        }
        Ok(result)
    }
    /// Configure a native-owned viewport and return its exact frame lease.
    pub async fn configure(&self, value: Configure) -> Result<Lease, PresentationError> {
        match self.call(PresentationCommand::Configure(value)).await?.0 {
            PresentationReply::Configured(lease) => Ok(lease),
            _ => {
                self.disconnect();
                Err(PresentationError::OutcomeUnknown)
            }
        }
    }
    /// Obtain public metadata for the current native TLS request before view attachment.
    /// An automatic policy may queue key use; a lost reply must never be retried blindly.
    pub async fn pending_client_identity(
        &self,
        policy: ClientIdentityPolicy,
    ) -> Result<Option<ClientIdentityChallenge>, PresentationError> {
        match self
            .call(PresentationCommand::PendingClientIdentity { policy })
            .await?
            .0
        {
            PresentationReply::ClientIdentity { challenge } => Ok(challenge),
            _ => {
                self.disconnect();
                Err(PresentationError::OutcomeUnknown)
            }
        }
    }
    /// Submit an offered fingerprint or cancel the exact native one-use request.
    /// The host validates again on its CEF thread; ACK is not authentication proof.
    pub async fn select_client_identity(
        &self,
        key: ClientIdentityKey,
        fingerprint_sha256: Option<String>,
    ) -> Result<(), PresentationError> {
        match self
            .call(PresentationCommand::SelectClientIdentity {
                key,
                fingerprint_sha256,
            })
            .await?
            .0
        {
            PresentationReply::Ack => Ok(()),
            _ => {
                self.disconnect();
                Err(PresentationError::OutcomeUnknown)
            }
        }
    }
    /// Acknowledge an exact active native lease effect.
    pub async fn command(&self, value: PresentationCommand) -> Result<(), PresentationError> {
        if !matches!(
            value,
            PresentationCommand::Input { .. }
                | PresentationCommand::Focus { .. }
                | PresentationCommand::Hide { .. }
                | PresentationCommand::Renew { .. }
                | PresentationCommand::Human { .. }
        ) {
            return Err(PresentationError::Invalid);
        }
        match self.call(value).await?.0 {
            PresentationReply::Ack => Ok(()),
            _ => {
                self.disconnect();
                Err(PresentationError::OutcomeUnknown)
            }
        }
    }
    /// Send one authenticated native input, without any uncertain-effect retry.
    pub async fn input(
        &self,
        lease: Lease,
        input: Input,
        modifiers: u32,
    ) -> Result<(), PresentationError> {
        self.command(PresentationCommand::Input {
            lease,
            input,
            modifiers,
        })
        .await
    }
    /// Set focus on the exact active native lease.
    pub async fn focus(&self, lease: Lease, focused: bool) -> Result<(), PresentationError> {
        self.command(PresentationCommand::Focus { lease, focused })
            .await
    }
    /// Renew unchanged live native visibility.
    pub async fn renew(&self, lease: Lease, lease_ms: u16) -> Result<(), PresentationError> {
        self.command(PresentationCommand::Renew { lease, lease_ms })
            .await
    }
    /// Hide and revoke native human input.
    pub async fn hide(&self, lease: Lease) -> Result<(), PresentationError> {
        self.command(PresentationCommand::Hide { lease }).await
    }
    /// Read bounded native metadata and the observed document target.
    pub async fn observe(&self, lease: Lease) -> Result<PageState, PresentationError> {
        match self.call(PresentationCommand::Observe { lease }).await?.0 {
            PresentationReply::State(state) => Ok(state),
            _ => Err(PresentationError::Stale),
        }
    }
    /// Irrevocably fence the exact initial human owner and return native document evidence.
    /// An uncertain receipt closes presentation; no failure re-enables human authority.
    pub async fn fence_human(&self, lease: Lease) -> Result<HumanFenceReceipt, PresentationError> {
        match self
            .call(PresentationCommand::FenceHuman { lease })
            .await?
            .0
        {
            PresentationReply::Fenced(receipt) => Ok(receipt),
            _ => {
                self.disconnect();
                Err(PresentationError::OutcomeUnknown)
            }
        }
    }
    /// Obtain an authenticated latest frame for the trusted native pixel surface.
    pub async fn next_frame(&self, lease: Lease) -> Result<Option<Vec<u8>>, PresentationError> {
        let (reply, bytes) = self.call(PresentationCommand::Poll { lease }).await?;
        match reply {
            PresentationReply::Empty | PresentationReply::Frame(_) => Ok(bytes),
            _ => Err(PresentationError::Stale),
        }
    }
}
