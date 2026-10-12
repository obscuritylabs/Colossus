//! Exercise two real authenticated channels and the worker relay's frame path.
use super::*;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};

struct NativeFrameHost {
    lease: Lease,
    request: Configure,
    guard: Mutex<Option<LeaseGuard>>,
    sequence: AtomicU64,
    delay: Duration,
    revoked: AtomicBool,
}

#[async_trait]
impl PresentationHost for NativeFrameHost {
    async fn dispatch(
        &self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError> {
        match command {
            PresentationCommand::Configure(request)
                if request.session == self.request.session
                    && request.target == self.request.target
                    && request.accepts(self.lease) =>
            {
                *self.guard.lock().unwrap() = Some(LeaseGuard::new(
                    self.lease,
                    Instant::now(),
                    Duration::from_millis(1500),
                )?);
                Ok(PresentationReply::Configured(self.lease))
            }
            PresentationCommand::Renew { lease, lease_ms } => {
                self.guard
                    .lock()
                    .unwrap()
                    .as_mut()
                    .ok_or(PresentationError::Hidden)?
                    .renew(
                        lease,
                        Instant::now(),
                        Duration::from_millis(u64::from(lease_ms)),
                    )?;
                Ok(PresentationReply::Ack)
            }
            _ => Err(PresentationError::Invalid),
        }
    }

    async fn frame(&self, lease: Lease) -> Result<Option<Frame>, PresentationError> {
        tokio::time::sleep(self.delay).await;
        let guard = self.guard.lock().unwrap();
        let guard = guard.as_ref().ok_or(PresentationError::Hidden)?;
        if guard.lease() != lease {
            return Err(PresentationError::Stale);
        }
        if !guard.visible(Instant::now()) {
            return Err(PresentationError::Hidden);
        }
        Ok(Some(Frame {
            lease,
            sequence: self.sequence.fetch_add(1, Ordering::AcqRel) + 1,
            stride: lease.pixel_width * 4,
            pixels: vec![0x69; lease.pixel_width as usize * lease.pixel_height as usize * 4],
        }))
    }

    fn revoke_now(&self) {
        self.revoked.store(true, Ordering::Release);
    }

    async fn revoke(&self) {
        self.revoke_now();
    }
}

// The production worker NativeRelay owns a native PresentationClient and
// decodes its frame before the GUI serve_presentation re-encodes it. Keep this
// transport regression independent of Runtime admission and CEF.
struct Relay {
    native: PresentationClient,
    codec: Mutex<Option<FrameCodec>>,
    revoked: AtomicBool,
    enrollment: [u8; 32],
}

#[async_trait]
impl PresentationHost for Relay {
    async fn dispatch(
        &self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError> {
        match command {
            PresentationCommand::Configure(request) => {
                let lease = self.native.configure(request).await?;
                *self.codec.lock().unwrap() = Some(FrameCodec::new(
                    self.native.surface_key(),
                    self.enrollment,
                    lease,
                )?);
                Ok(PresentationReply::Configured(lease))
            }
            PresentationCommand::Renew { lease, lease_ms } => {
                self.native.renew(lease, lease_ms).await?;
                Ok(PresentationReply::Ack)
            }
            _ => Err(PresentationError::Invalid),
        }
    }

    async fn frame(&self, lease: Lease) -> Result<Option<Frame>, PresentationError> {
        let Some(bytes) = self.native.next_frame(lease).await? else {
            return Ok(None);
        };
        let mut codec = self.codec.lock().unwrap();
        Ok(Some(
            codec
                .as_mut()
                .ok_or(PresentationError::Stale)?
                .decode(bytes)?,
        ))
    }

    fn revoke_now(&self) {
        self.revoked.store(true, Ordering::Release);
        self.native.disconnect();
    }

    async fn revoke(&self) {
        self.revoke_now();
    }
}

async fn connect(
    host: Arc<dyn PresentationHost>,
    key: [u8; 32],
    enrollment: [u8; 32],
) -> (
    PresentationClient,
    tokio::task::JoinHandle<Result<(), PresentationError>>,
) {
    let (client, server) = tokio::io::duplex(128 * 1024);
    let (client_read, client_write) = tokio::io::split(client);
    let (server_read, server_write) = tokio::io::split(server);
    let serving = tokio::spawn(serve_presentation(
        PresentationChannel::new(server_read, server_write),
        Zeroizing::new(key),
        enrollment,
        host,
    ));
    let client = PresentationClient::connect(
        PresentationChannel::new(client_read, client_write),
        Zeroizing::new(key),
        enrollment,
    )
    .await
    .unwrap();
    (client, serving)
}

fn retina() -> (Configure, Lease) {
    let mut request = configure();
    request.width = 832;
    request.height = 520;
    request.scale_milli = 2000;
    let mut lease = lease();
    lease.pixel_width = 1664;
    lease.pixel_height = 1040;
    (request, lease)
}

async fn two_hops(
    request: Configure,
    lease: Lease,
    delay: Duration,
) -> (
    PresentationClient,
    Arc<NativeFrameHost>,
    Arc<Relay>,
    tokio::task::JoinHandle<Result<(), PresentationError>>,
    tokio::task::JoinHandle<Result<(), PresentationError>>,
) {
    let native = Arc::new(NativeFrameHost {
        lease,
        request,
        guard: Mutex::new(None),
        sequence: AtomicU64::new(0),
        delay,
        revoked: AtomicBool::new(false),
    });
    let (native_client, native_serve) = connect(native.clone(), [4; 32], [5; 32]).await;
    let relay = Arc::new(Relay {
        native: native_client,
        codec: Mutex::new(None),
        revoked: AtomicBool::new(false),
        enrollment: [5; 32],
    });
    let (gui_client, gui_serve) = connect(relay.clone(), [6; 32], [7; 32]).await;
    (gui_client, native, relay, gui_serve, native_serve)
}

#[tokio::test]
async fn retina_frame_crosses_two_authenticated_hops_and_expires_without_renewal() {
    let (request, expected) = retina();
    let (gui, native, relay, gui_serve, native_serve) =
        two_hops(request.clone(), expected, Duration::ZERO).await;
    let admitted = gui.configure(request).await.unwrap();
    assert_eq!(admitted, expected);
    let bytes = gui.next_frame(admitted).await.unwrap().unwrap();
    let mut invented = bytes.clone();
    invented[HEADER_BYTES + 32] ^= 1;
    assert!(matches!(
        FrameCodec::new(gui.surface_key(), [7; 32], admitted)
            .unwrap()
            .decode(invented),
        Err(PresentationError::Unauthenticated)
    ));
    let frame = FrameCodec::new(gui.surface_key(), [7; 32], admitted)
        .unwrap()
        .decode(bytes)
        .unwrap();
    assert_eq!(frame.pixels.len(), 1664 * 1040 * 4);
    assert!(frame.pixels.iter().all(|value| *value == 0x69));
    let mut foreign = admitted;
    foreign.document_generation += 1;
    assert_eq!(gui.next_frame(foreign).await, Err(PresentationError::Stale));
    tokio::time::sleep(Duration::from_millis(1550)).await;
    assert_eq!(
        gui.next_frame(admitted).await,
        Err(PresentationError::Hidden)
    );
    gui.disconnect();
    assert!(gui_serve.await.unwrap().is_err());
    assert!(native_serve.await.unwrap().is_err());
    assert!(relay.revoked.load(Ordering::Acquire));
    assert!(native.revoked.load(Ordering::Acquire));
}

#[tokio::test]
async fn queued_renew_waits_for_slow_read_only_poll_but_expired_lease_stays_hidden() {
    let request = configure();
    let expected = lease();
    let (gui, native, relay, gui_serve, native_serve) =
        two_hops(request.clone(), expected, Duration::from_millis(850)).await;
    gui.configure(request).await.unwrap();
    let polling = {
        let gui = gui.clone();
        tokio::spawn(async move { gui.next_frame(expected).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    let renewing = {
        let gui = gui.clone();
        tokio::spawn(async move { gui.renew(expected, 1500).await })
    };
    let bytes = polling.await.unwrap().unwrap().unwrap();
    assert_eq!(
        FrameCodec::new(gui.surface_key(), [7; 32], expected)
            .unwrap()
            .decode(bytes)
            .unwrap()
            .pixels
            .len(),
        256
    );
    renewing.await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(1550)).await;
    assert_eq!(
        gui.next_frame(expected).await,
        Err(PresentationError::Hidden)
    );
    gui.disconnect();
    assert!(gui_serve.await.unwrap().is_err());
    assert!(native_serve.await.unwrap().is_err());
    assert!(relay.revoked.load(Ordering::Acquire));
    assert!(native.revoked.load(Ordering::Acquire));
}

#[tokio::test]
async fn blocked_effect_still_expires_on_short_deadline_and_revokes_both_hops() {
    struct Blocked;
    #[async_trait]
    impl PresentationHost for Blocked {
        async fn dispatch(
            &self,
            _: PresentationCommand,
        ) -> Result<PresentationReply, PresentationError> {
            std::future::pending().await
        }
        async fn frame(&self, _: Lease) -> Result<Option<Frame>, PresentationError> {
            Ok(None)
        }
        fn revoke_now(&self) {}
        async fn revoke(&self) {}
    }
    let (client, serving) = connect(Arc::new(Blocked), [8; 32], [9; 32]).await;
    let started = Instant::now();
    assert_eq!(
        client.focus(lease(), true).await,
        Err(PresentationError::OutcomeUnknown)
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(serving.await.unwrap().is_err());
    assert_eq!(
        client.next_frame(lease()).await,
        Err(PresentationError::OutcomeUnknown)
    );
}

#[tokio::test]
async fn read_only_poll_reports_corrupt_frame_authentication_and_closes_channel() {
    async fn read_command(
        reader: &mut (impl AsyncRead + Unpin),
        codec: &mut ControlCodec,
    ) -> PresentationCommand {
        let mut header = [0; CONTROL_HEADER_BYTES];
        reader.read_exact(&mut header).await.unwrap();
        let length = codec.payload_length(&header).unwrap();
        let mut bytes = vec![0; CONTROL_HEADER_BYTES + length];
        bytes[..CONTROL_HEADER_BYTES].copy_from_slice(&header);
        reader
            .read_exact(&mut bytes[CONTROL_HEADER_BYTES..])
            .await
            .unwrap();
        codec.decode(&bytes).unwrap()
    }

    let (client_stream, server_stream) = tokio::io::duplex(128 * 1024);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let (mut server_read, mut server_write) = tokio::io::split(server_stream);
    let server = tokio::spawn(async move {
        let mut incoming =
            ControlCodec::new(Zeroizing::new([11; 32]), [12; 32], Role::NativeToHost);
        let mut outgoing =
            ControlCodec::new(Zeroizing::new([11; 32]), [12; 32], Role::HostToNative);
        assert!(matches!(
            read_command(&mut server_read, &mut incoming).await,
            PresentationCommand::Ready
        ));
        server_write
            .write_all(&outgoing.encode(&PresentationReply::Ready).unwrap())
            .await
            .unwrap();
        assert!(matches!(
            read_command(&mut server_read, &mut incoming).await,
            PresentationCommand::Configure(_)
        ));
        server_write
            .write_all(
                &outgoing
                    .encode(&PresentationReply::Configured(lease()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(matches!(
            read_command(&mut server_read, &mut incoming).await,
            PresentationCommand::Poll { .. }
        ));
        server_write
            .write_all(&outgoing.encode(&PresentationReply::Frame(lease())).unwrap())
            .await
            .unwrap();
        let mut invented = FrameCodec::new(Zeroizing::new([11; 32]), [12; 32], lease())
            .unwrap()
            .encode(&Frame {
                lease: lease(),
                sequence: 1,
                stride: 32,
                pixels: vec![0x69; 256],
            })
            .unwrap();
        invented[HEADER_BYTES + 1] ^= 1;
        server_write.write_all(&invented).await.unwrap();
    });
    let client = PresentationClient::connect(
        PresentationChannel::new(client_read, client_write),
        Zeroizing::new([11; 32]),
        [12; 32],
    )
    .await
    .unwrap();
    client.configure(configure()).await.unwrap();
    assert_eq!(
        client.next_frame(lease()).await,
        Err(PresentationError::Unauthenticated)
    );
    server.await.unwrap();
    assert_eq!(
        client.next_frame(lease()).await,
        Err(PresentationError::OutcomeUnknown)
    );
}
