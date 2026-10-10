use crate::*;
use async_trait::async_trait;
use colossus_contracts::{BrowserDocumentId, BrowserSessionId, BrowserTabId, BrowserTarget};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use zeroize::Zeroizing;

fn lease() -> Lease {
    Lease {
        tab: 1,
        session_generation: 1,
        control_generation: 0,
        viewport_generation: 1,
        document_generation: 1,
        pixel_width: 8,
        pixel_height: 8,
    }
}
fn configure() -> Configure {
    Configure {
        session: BrowserSessionId::parse(format!("bs_{}", "1".repeat(32))).unwrap(),
        target: BrowserTarget {
            tab_id: BrowserTabId::parse(format!("bt_{}", "2".repeat(32))).unwrap(),
            document_id: BrowserDocumentId::parse(format!("bd_{}", "3".repeat(32))).unwrap(),
        },
        control_generation: 0,
        viewport_generation: 1,
        width: 8,
        height: 8,
        scale_milli: 1000,
        lease_ms: 1000,
    }
}
fn codec(role: Role) -> ControlCodec {
    ControlCodec::new(Zeroizing::new([1; 32]), [2; 32], role)
}

#[test]
fn control_authenticates_direction_enrollment_sequence_and_bytes() {
    let mut sender = codec(Role::NativeToHost);
    let bytes = sender
        .encode(&PresentationCommand::Configure(configure()))
        .unwrap();
    let mut reader = codec(Role::NativeToHost);
    let value: PresentationCommand = reader.decode(&bytes).unwrap();
    assert!(matches!(value, PresentationCommand::Configure(_)));
    assert_eq!(
        reader.decode::<PresentationCommand>(&bytes).unwrap_err(),
        PresentationError::Stale
    );
    assert_eq!(
        codec(Role::HostToNative)
            .decode::<PresentationCommand>(&bytes)
            .unwrap_err(),
        PresentationError::Unauthenticated
    );
    for offset in [8, 40, 48, 52, CONTROL_HEADER_BYTES] {
        let mut changed = bytes.clone();
        changed[offset] ^= 1;
        assert!(
            codec(Role::NativeToHost)
                .decode::<PresentationCommand>(&changed)
                .is_err()
        );
    }
    let mut wrong = ControlCodec::new(Zeroizing::new([3; 32]), [2; 32], Role::NativeToHost);
    assert_eq!(
        wrong.decode::<PresentationCommand>(&bytes).unwrap_err(),
        PresentationError::Unauthenticated
    );
}
#[test]
fn control_prefix_and_effect_shape_bound_allocation() {
    let mut sender = codec(Role::NativeToHost);
    let bytes = sender.encode(&PresentationCommand::Ready).unwrap();
    let mut header: [u8; CONTROL_HEADER_BYTES] = bytes[..CONTROL_HEADER_BYTES].try_into().unwrap();
    header[48..52].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        codec(Role::NativeToHost).payload_length(&header),
        Err(PresentationError::LimitExceeded)
    );
    let mut invalid = lease();
    invalid.document_generation = 0;
    assert_eq!(invalid.validate(), Err(PresentationError::Invalid));
    assert!(
        lease().validate().is_ok(),
        "human0 is separately authorized, not a native identity"
    );
    assert_eq!(
        PresentationCommand::Input {
            lease: lease(),
            input: Input::MouseMove { x: 8, y: 0 },
            modifiers: 0
        }
        .validate(),
        Err(PresentationError::Invalid)
    );
    assert_eq!(
        PresentationCommand::Input {
            lease: lease(),
            input: Input::ImeCancel,
            modifiers: u32::MAX
        }
        .validate(),
        Err(PresentationError::Invalid)
    );
}
#[test]
fn fractional_scale_uses_exact_native_ceil_and_placement_binding() {
    for scale in [1001, 1250, 1500] {
        let mut request = configure();
        request.width = 3;
        request.height = 5;
        request.scale_milli = scale;
        request.validate().unwrap();
        let mut answer = lease();
        answer.pixel_width = (3_u32 * scale).div_ceil(1000);
        answer.pixel_height = (5_u32 * scale).div_ceil(1000);
        assert!(request.accepts(answer));
        answer.control_generation = 1;
        assert!(!request.accepts(answer));
        answer.control_generation = 0;
        answer.pixel_width -= 1;
        assert!(!request.accepts(answer));
    }
    let mut request = configure();
    request.width = 4096;
    request.height = 4096;
    request.scale_milli = 4000;
    assert_eq!(request.validate(), Err(PresentationError::Invalid));
}

struct Host {
    revoked: AtomicBool,
    sequence: AtomicU64,
    blocked: AtomicBool,
    entered: AtomicBool,
    bad_ack: AtomicBool,
    unknown: AtomicBool,
    effects: AtomicU64,
    fenced: AtomicBool,
    bad_fence: AtomicBool,
}
#[async_trait]
impl PresentationHost for Host {
    async fn dispatch(
        &self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError> {
        if self.blocked.load(Ordering::Acquire) {
            self.entered.store(true, Ordering::Release);
            std::future::pending::<()>().await;
        }
        match command {
            PresentationCommand::Configure(request) => {
                if request.control_generation != 0
                    || request.session != configure().session
                    || request.target != configure().target
                {
                    return Err(PresentationError::Stale);
                }
                Ok(PresentationReply::Configured(lease()))
            }
            PresentationCommand::Input { lease: value, .. }
            | PresentationCommand::Focus { lease: value, .. } => {
                if value != lease() {
                    return Err(PresentationError::Stale);
                }
                if self.fenced.load(Ordering::Acquire) {
                    return Err(PresentationError::Hidden);
                }
                self.effects.fetch_add(1, Ordering::AcqRel);
                if self.unknown.load(Ordering::Acquire) {
                    return Err(PresentationError::OutcomeUnknown);
                }
                if self.bad_ack.load(Ordering::Acquire) {
                    Ok(PresentationReply::Ready)
                } else {
                    Ok(PresentationReply::Ack)
                }
            }
            PresentationCommand::FenceHuman { lease: value } => {
                if value != lease() {
                    return Err(PresentationError::Stale);
                }
                self.fenced.store(true, Ordering::Release);
                self.effects.fetch_add(1, Ordering::AcqRel);
                let mut prior_lease = value;
                if self.bad_fence.load(Ordering::Acquire) {
                    prior_lease.viewport_generation += 1;
                }
                let mut target = configure().target;
                target.document_id =
                    BrowserDocumentId::parse(format!("bd_{}", "4".repeat(32))).unwrap();
                Ok(PresentationReply::Fenced(HumanFenceReceipt {
                    prior_lease,
                    native_document_generation: 2,
                    state: PageState {
                        target,
                        url: "https://fixture.invalid/".into(),
                        title: "Fixture".into(),
                        loading: false,
                        can_go_back: false,
                        can_go_forward: false,
                    },
                }))
            }
            _ => Err(PresentationError::Invalid),
        }
    }
    async fn frame(&self, value: Lease) -> Result<Option<Frame>, PresentationError> {
        if value != lease() {
            return Err(PresentationError::Stale);
        }
        Ok(Some(Frame {
            lease: value,
            sequence: self.sequence.fetch_add(1, Ordering::AcqRel) + 1,
            stride: 32,
            pixels: vec![6; 256],
        }))
    }
    fn revoke_now(&self) {
        self.revoked.store(true, Ordering::Release);
    }
    async fn revoke(&self) {
        self.revoke_now();
    }
}
fn host() -> Arc<Host> {
    Arc::new(Host {
        revoked: AtomicBool::new(false),
        sequence: AtomicU64::new(0),
        blocked: AtomicBool::new(false),
        entered: AtomicBool::new(false),
        bad_ack: AtomicBool::new(false),
        unknown: AtomicBool::new(false),
        effects: AtomicU64::new(0),
        fenced: AtomicBool::new(false),
        bad_fence: AtomicBool::new(false),
    })
}
async fn pair(
    host: Arc<Host>,
) -> (
    PresentationClient,
    tokio::task::JoinHandle<Result<(), PresentationError>>,
) {
    let (client, server) = tokio::io::duplex(1024);
    let (client_read, client_write) = tokio::io::split(client);
    let (server_read, server_write) = tokio::io::split(server);
    let serving = tokio::spawn(serve_presentation(
        PresentationChannel::new(server_read, server_write),
        Zeroizing::new([1; 32]),
        [2; 32],
        host,
    ));
    let client = PresentationClient::connect(
        PresentationChannel::new(client_read, client_write),
        Zeroizing::new([1; 32]),
        [2; 32],
    )
    .await
    .unwrap();
    (client, serving)
}
#[tokio::test]
async fn real_private_channel_authenticates_frame_and_rejects_foreign_controller() {
    let host = host();
    let (client, serving) = pair(Arc::clone(&host)).await;
    let configured = client.configure(configure()).await.unwrap();
    let bytes = client.next_frame(configured).await.unwrap().unwrap();
    let frame = FrameCodec::new(client.surface_key(), [2; 32], configured)
        .unwrap()
        .decode(bytes)
        .unwrap();
    assert_eq!(frame.pixels, vec![6; 256]);
    let mut foreign = configured;
    foreign.control_generation = 1;
    assert_eq!(
        client.input(foreign, Input::ImeCancel, 0).await,
        Err(PresentationError::Stale)
    );
    let mut wrong = configure();
    wrong.session = BrowserSessionId::parse(format!("bs_{}", "9".repeat(32))).unwrap();
    assert_eq!(client.configure(wrong).await, Err(PresentationError::Stale));
    client.disconnect();
    assert!(serving.await.unwrap().is_err());
    assert!(host.revoked.load(Ordering::Acquire));
}
#[tokio::test]
async fn ack_only_method_rejects_queries_before_native_dispatch() {
    let host = host();
    let (client, serving) = pair(Arc::clone(&host)).await;
    host.blocked.store(true, Ordering::Release);
    for command in [
        PresentationCommand::Ready,
        PresentationCommand::Configure(configure()),
        PresentationCommand::Observe { lease: lease() },
        PresentationCommand::FenceHuman { lease: lease() },
        PresentationCommand::Poll { lease: lease() },
    ] {
        assert_eq!(
            client.command(command).await,
            Err(PresentationError::Invalid)
        );
    }
    assert!(!host.entered.load(Ordering::Acquire));
    assert!(!host.fenced.load(Ordering::Acquire));
    assert_eq!(host.effects.load(Ordering::Acquire), 0);
    host.blocked.store(false, Ordering::Release);
    client.configure(configure()).await.unwrap();
    client.focus(lease(), true).await.unwrap();
    assert_eq!(host.effects.load(Ordering::Acquire), 1);
    client.disconnect();
    assert!(serving.await.unwrap().is_err());
}

#[tokio::test]
async fn dropped_dispatched_caller_revokes_private_channel_without_retry() {
    let host = host();
    let (client, serving) = pair(Arc::clone(&host)).await;
    client.configure(configure()).await.unwrap();
    host.blocked.store(true, Ordering::Release);
    let action_client = client.clone();
    let action = tokio::spawn(async move { action_client.focus(lease(), true).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !host.entered.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    action.abort();
    let _ = action.await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), serving)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(host.revoked.load(Ordering::Acquire));
    assert_eq!(
        client.input(lease(), Input::ImeCancel, 0).await,
        Err(PresentationError::OutcomeUnknown)
    );
}

#[tokio::test]
async fn dropped_server_future_immediately_fences_native_input() {
    let host = host();
    let (client, serving) = pair(Arc::clone(&host)).await;
    client.configure(configure()).await.unwrap();
    serving.abort();
    let _ = serving.await;
    assert!(host.revoked.load(Ordering::Acquire));
    assert_eq!(
        client.input(lease(), Input::ImeCancel, 0).await,
        Err(PresentationError::OutcomeUnknown)
    );
}

#[tokio::test]
async fn wrong_authenticated_mutation_ack_is_unknown_and_revokes_channel() {
    let host = host();
    let (client, serving) = pair(Arc::clone(&host)).await;
    client.configure(configure()).await.unwrap();
    host.bad_ack.store(true, Ordering::Release);
    assert_eq!(
        client.focus(lease(), true).await,
        Err(PresentationError::OutcomeUnknown)
    );
    assert!(serving.await.unwrap().is_err());
    assert!(host.revoked.load(Ordering::Acquire));
    assert_eq!(
        client.focus(lease(), true).await,
        Err(PresentationError::OutcomeUnknown)
    );
}

#[tokio::test]
async fn authenticated_unknown_effect_is_terminal_without_mutation_retry() {
    let host = host();
    let (client, serving) = pair(Arc::clone(&host)).await;
    client.configure(configure()).await.unwrap();
    host.unknown.store(true, Ordering::Release);
    assert_eq!(
        client.focus(lease(), true).await,
        Err(PresentationError::OutcomeUnknown)
    );
    assert!(serving.await.unwrap().is_err());
    assert!(host.revoked.load(Ordering::Acquire));
    assert_eq!(
        client.focus(lease(), true).await,
        Err(PresentationError::OutcomeUnknown)
    );
    assert_eq!(host.effects.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn human_fence_returns_exact_new_document_and_never_reenables_input() {
    let host = host();
    let (client, serving) = pair(Arc::clone(&host)).await;
    client.configure(configure()).await.unwrap();
    let mut foreign = lease();
    foreign.document_generation += 1;
    assert!(matches!(
        client.fence_human(foreign).await,
        Err(PresentationError::Stale)
    ));
    assert!(!host.fenced.load(Ordering::Acquire));
    let receipt = client.fence_human(lease()).await.unwrap();
    assert_eq!(receipt.prior_lease, lease());
    assert_eq!(receipt.native_document_generation, 2);
    assert_ne!(
        receipt.state.target.document_id,
        configure().target.document_id
    );
    client.configure(configure()).await.unwrap();
    assert_eq!(
        client.input(lease(), Input::ImeCancel, 0).await,
        Err(PresentationError::Hidden)
    );
    assert_eq!(host.effects.load(Ordering::Acquire), 1);
    client.disconnect();
    assert!(serving.await.unwrap().is_err());
}

#[tokio::test]
async fn wrong_fence_receipt_keeps_native_authority_revoked_and_is_unknown() {
    let host = host();
    let (client, serving) = pair(Arc::clone(&host)).await;
    client.configure(configure()).await.unwrap();
    host.bad_fence.store(true, Ordering::Release);
    assert!(matches!(
        client.fence_human(lease()).await,
        Err(PresentationError::OutcomeUnknown)
    ));
    assert!(serving.await.unwrap().is_err());
    assert!(host.fenced.load(Ordering::Acquire));
    assert!(host.revoked.load(Ordering::Acquire));
    assert_eq!(host.effects.load(Ordering::Acquire), 1);
}
