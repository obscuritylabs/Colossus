use super::*;
use colossus_browser_presentation::Configure;
use colossus_contracts::{BrowserDocumentId, BrowserSessionId, BrowserTabId, BrowserTarget};

#[derive(Default)]
struct MemoryHost {
    controls: Vec<PresentationCommand>,
    frames: usize,
    revoked: usize,
    read_rejection: Option<PresentationError>,
}
impl Target for MemoryHost {
    fn control(
        &mut self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError> {
        let observation = matches!(command, PresentationCommand::Observe { .. });
        let identity = matches!(command, PresentationCommand::PendingClientIdentity { .. });
        self.controls.push(command);
        if identity {
            return Ok(PresentationReply::ClientIdentity { challenge: None });
        }
        if observation {
            if let Some(error) = self.read_rejection {
                return Err(error);
            }
            return Ok(PresentationReply::State(
                colossus_browser_presentation::PageState {
                    target: configuration().target,
                    url: "http://127.0.0.1/fixture".into(),
                    title: "Unchanged native document".into(),
                    loading: false,
                    can_go_back: false,
                    can_go_forward: false,
                },
            ));
        }
        Ok(PresentationReply::Ack)
    }
    fn frame(&mut self, lease: Lease) -> Result<Option<Frame>, PresentationError> {
        self.frames += 1;
        if let Some(error) = self.read_rejection {
            return Err(error);
        }
        Ok(Some(Frame {
            lease,
            sequence: self.frames as u64,
            stride: lease.pixel_width * 4,
            pixels: vec![0x22; lease.pixel_width as usize * lease.pixel_height as usize * 4],
        }))
    }
    fn revoke(&mut self) {
        self.revoked += 1;
    }
}
fn lease() -> Lease {
    Lease {
        tab: 1,
        session_generation: 1,
        control_generation: 1,
        viewport_generation: 2,
        document_generation: 1,
        pixel_width: 2,
        pixel_height: 2,
    }
}
fn configuration() -> Configure {
    Configure {
        session: BrowserSessionId::parse(format!("bs_{}", "1".repeat(32))).unwrap(),
        target: BrowserTarget {
            tab_id: BrowserTabId::parse(format!("bt_{}", "2".repeat(32))).unwrap(),
            document_id: BrowserDocumentId::parse(format!("bd_{}", "3".repeat(32))).unwrap(),
        },
        control_generation: 1,
        viewport_generation: 3,
        width: 2,
        height: 2,
        scale_milli: 1000,
        lease_ms: 1500,
    }
}

#[tokio::test]
async fn identity_review_is_serviced_before_attachment_during_initial_navigation() {
    use colossus_browser_presentation::{ClientIdentityKey, ClientIdentityPolicy};
    let (sender, receiver) = mpsc::channel(8);
    let revoked = Arc::new(AtomicBool::new(false));
    let mut lane = Lane::new(receiver, Arc::clone(&revoked));
    let mut host = MemoryHost::default();
    let mut replies = Vec::new();
    for command in [
        PresentationCommand::PendingClientIdentity {
            policy: ClientIdentityPolicy::AutomaticIfUnique,
        },
        PresentationCommand::SelectClientIdentity {
            key: ClientIdentityKey {
                tab: 7,
                generation: 1,
                request_id: 11,
                control_generation: 0,
            },
            fingerprint_sha256: None,
        },
    ] {
        let (reply, receive) = oneshot::channel();
        sender.send(Command::Control(command, reply)).await.unwrap();
        replies.push(receive);
    }
    lane.service(&mut host, true);
    assert!(matches!(
        replies.remove(0).await.unwrap(),
        Ok(PresentationReply::ClientIdentity { challenge: None })
    ));
    assert!(matches!(
        replies.remove(0).await.unwrap(),
        Ok(PresentationReply::Ack)
    ));
    assert_eq!(
        host.controls.len(),
        2,
        "no Configure or viewport lease was required"
    );
    assert_eq!(host.frames, 0);
    revoked.store(true, Ordering::Release);
    let (reply, receive) = oneshot::channel();
    sender
        .send(Command::Control(
            PresentationCommand::PendingClientIdentity {
                policy: ClientIdentityPolicy::Review,
            },
            reply,
        ))
        .await
        .unwrap();
    lane.service(&mut host, true);
    assert!(matches!(
        receive.await.unwrap(),
        Err(PresentationError::Hidden)
    ));
    assert_eq!(
        host.controls.len(),
        2,
        "revoked private owner cannot queue key use"
    );
    assert_eq!(host.revoked, 1);
}

#[tokio::test]
async fn slow_data_wait_rejects_human_reads_and_mutations_before_dispatch() {
    let (sender, receiver) = mpsc::channel(8);
    let mut lane = Lane::new(receiver, Arc::new(AtomicBool::new(false)));
    let mut host = MemoryHost::default();
    let mut results = Vec::new();
    let human = Lease {
        control_generation: 0,
        ..lease()
    };
    for command in [
        PresentationCommand::Observe { lease: human },
        PresentationCommand::Configure(configuration()),
        PresentationCommand::Human {
            lease: human,
            command: colossus_browser_presentation::HumanCommand::Reload,
        },
        PresentationCommand::Focus {
            lease: human,
            focused: true,
        },
        PresentationCommand::FenceHuman { lease: human },
        PresentationCommand::Input {
            lease: human,
            input: colossus_browser_presentation::Input::MouseMove { x: 1, y: 1 },
            modifiers: 0,
        },
    ] {
        let (reply, result) = oneshot::channel();
        sender.send(Command::Control(command, reply)).await.unwrap();
        results.push(result);
    }
    let (reply, frame) = oneshot::channel();
    sender.send(Command::Frame(human, reply)).await.unwrap();
    lane.service(&mut host, true);
    for result in results {
        assert!(matches!(
            result.await.unwrap(),
            Err(PresentationError::Stale)
        ));
    }
    assert!(matches!(
        frame.await.unwrap(),
        Err(PresentationError::Stale)
    ));
    assert!(host.controls.is_empty());
    assert_eq!(host.frames, 0);
    // The same queue remains usable after the finite operation completes.
    let (reply, result) = oneshot::channel();
    sender
        .send(Command::Control(
            PresentationCommand::Observe { lease: lease() },
            reply,
        ))
        .await
        .unwrap();
    lane.service(&mut host, false);
    assert!(matches!(
        result.await.unwrap(),
        Ok(PresentationReply::State(_))
    ));
}

#[tokio::test]
async fn agent_observation_and_frames_continue_during_an_unchanged_document_wait() {
    let (sender, receiver) = mpsc::channel(8);
    let mut lane = Lane::new(receiver, Arc::new(AtomicBool::new(false)));
    let mut host = MemoryHost::default();
    for expected_sequence in 1..=2 {
        let (reply, observation) = oneshot::channel();
        sender
            .send(Command::Control(
                PresentationCommand::Observe { lease: lease() },
                reply,
            ))
            .await
            .unwrap();
        let (reply, frame) = oneshot::channel();
        sender.send(Command::Frame(lease(), reply)).await.unwrap();
        lane.service(&mut host, true);
        assert!(matches!(
            observation.await.unwrap(),
            Ok(PresentationReply::State(page))
                if page.target == configuration().target && !page.loading
        ));
        let frame = frame.await.unwrap().unwrap().unwrap();
        frame.validate().unwrap();
        assert_eq!(frame.lease, lease());
        assert_eq!(frame.sequence, expected_sequence);
        assert_eq!(frame.pixels, [0x22; 16]);
    }
    assert_eq!(host.controls.len(), 2);
    assert_eq!(host.frames, 2);
}

#[tokio::test]
async fn native_document_rejection_is_preserved_for_agent_reads_during_wait() {
    let (sender, receiver) = mpsc::channel(8);
    let mut lane = Lane::new(receiver, Arc::new(AtomicBool::new(false)));
    // The native Host, not Lane, owns current_document and document-adoption
    // fences. A document transition must retain its exact rejection here.
    let mut host = MemoryHost {
        read_rejection: Some(PresentationError::Stale),
        ..MemoryHost::default()
    };
    let (reply, observation) = oneshot::channel();
    sender
        .send(Command::Control(
            PresentationCommand::Observe { lease: lease() },
            reply,
        ))
        .await
        .unwrap();
    let (reply, frame) = oneshot::channel();
    sender.send(Command::Frame(lease(), reply)).await.unwrap();
    lane.service(&mut host, true);
    assert!(matches!(
        observation.await.unwrap(),
        Err(PresentationError::Stale)
    ));
    assert!(matches!(
        frame.await.unwrap(),
        Err(PresentationError::Stale)
    ));
    assert_eq!(host.controls.len(), 1);
    assert_eq!(host.frames, 1);
}

#[tokio::test]
async fn exact_hide_and_revocation_settle_during_wait_and_revoked_lane_cannot_renew() {
    let (sender, receiver) = mpsc::channel(8);
    let revoked = Arc::new(AtomicBool::new(false));
    let mut lane = Lane::new(receiver, revoked.clone());
    let mut host = MemoryHost::default();
    let (reply, hidden) = oneshot::channel();
    sender
        .send(Command::Control(
            PresentationCommand::Hide { lease: lease() },
            reply,
        ))
        .await
        .unwrap();
    let (reply, renewed) = oneshot::channel();
    sender
        .send(Command::Control(
            PresentationCommand::Renew {
                lease: lease(),
                lease_ms: 1000,
            },
            reply,
        ))
        .await
        .unwrap();
    lane.service(&mut host, true);
    assert!(matches!(hidden.await.unwrap(), Ok(PresentationReply::Ack)));
    assert!(matches!(renewed.await.unwrap(), Ok(PresentationReply::Ack)));
    revoked.store(true, Ordering::Release);
    let (reply, hidden) = oneshot::channel();
    sender
        .send(Command::Control(
            PresentationCommand::Hide { lease: lease() },
            reply,
        ))
        .await
        .unwrap();
    let (reply, renewed) = oneshot::channel();
    sender
        .send(Command::Control(
            PresentationCommand::Renew {
                lease: lease(),
                lease_ms: 1000,
            },
            reply,
        ))
        .await
        .unwrap();
    lane.service(&mut host, true);
    assert!(matches!(hidden.await.unwrap(), Ok(PresentationReply::Ack)));
    assert!(matches!(
        renewed.await.unwrap(),
        Err(PresentationError::Hidden)
    ));
    let (reply, observation) = oneshot::channel();
    sender
        .send(Command::Control(
            PresentationCommand::Observe { lease: lease() },
            reply,
        ))
        .await
        .unwrap();
    let (reply, frame) = oneshot::channel();
    sender.send(Command::Frame(lease(), reply)).await.unwrap();
    lane.service(&mut host, true);
    assert!(matches!(
        observation.await.unwrap(),
        Err(PresentationError::Hidden)
    ));
    assert!(matches!(
        frame.await.unwrap(),
        Err(PresentationError::Hidden)
    ));
    assert_eq!(host.frames, 0);
    assert!(
        host.controls
            .iter()
            .all(|command| !matches!(command, PresentationCommand::Observe { .. }))
    );
    assert!(host.revoked > 0);
}

#[tokio::test]
async fn terminal_revocation_applies_once_while_future_pumps_and_commands_stay_fenced() {
    let (sender, receiver) = mpsc::channel(32);
    let revoked = Arc::new(AtomicBool::new(true));
    let mut lane = Lane::new(receiver, revoked.clone());
    let mut host = MemoryHost::default();
    let human = Lease {
        control_generation: 0,
        ..lease()
    };
    // Adapter::revoke sets the atomic fence before queuing its notification.
    // Neither duplicate notifications nor later CEF pumps may re-hide the page.
    for waiting in [true, false, true, false] {
        sender.send(Command::Revoke).await.unwrap();
        sender.send(Command::Revoke).await.unwrap();
        let mut replies = Vec::new();
        for command in [
            PresentationCommand::Configure(configuration()),
            PresentationCommand::Renew {
                lease: lease(),
                lease_ms: 1000,
            },
            PresentationCommand::Observe { lease: lease() },
            PresentationCommand::Observe { lease: human },
            PresentationCommand::Human {
                lease: human,
                command: colossus_browser_presentation::HumanCommand::Reload,
            },
            PresentationCommand::Focus {
                lease: human,
                focused: true,
            },
            PresentationCommand::FenceHuman { lease: human },
            PresentationCommand::Input {
                lease: human,
                input: colossus_browser_presentation::Input::MouseMove { x: 1, y: 1 },
                modifiers: 0,
            },
        ] {
            let (reply, result) = oneshot::channel();
            sender.send(Command::Control(command, reply)).await.unwrap();
            replies.push(result);
        }
        let mut frames = Vec::new();
        for lease in [lease(), human] {
            let (reply, result) = oneshot::channel();
            sender.send(Command::Frame(lease, reply)).await.unwrap();
            frames.push(result);
        }
        let mut closing_replies = Vec::new();
        for command in [
            PresentationCommand::Hide { lease: lease() },
            PresentationCommand::Focus {
                lease: human,
                focused: false,
            },
        ] {
            let (reply, result) = oneshot::channel();
            sender.send(Command::Control(command, reply)).await.unwrap();
            closing_replies.push(result);
        }
        lane.service(&mut host, waiting);
        for reply in replies {
            assert!(matches!(
                reply.await.unwrap(),
                Err(PresentationError::Hidden)
            ));
        }
        for frame in frames {
            assert!(matches!(
                frame.await.unwrap(),
                Err(PresentationError::Hidden)
            ));
        }
        for reply in closing_replies {
            assert!(matches!(reply.await.unwrap(), Ok(PresentationReply::Ack)));
        }
        assert!(revoked.load(Ordering::Acquire));
    }
    // Empty pumps must not defocus agent input after it activates the page.
    lane.service(&mut host, true);
    lane.service(&mut host, false);
    assert_eq!(host.revoked, 1);
    assert_eq!(host.frames, 0);
    assert_eq!(host.controls.len(), 8);
    assert!(host.controls.iter().all(|command| matches!(
        command,
        PresentationCommand::Hide { .. } | PresentationCommand::Focus { focused: false, .. }
    )));
}

#[tokio::test]
async fn queued_revocation_latches_the_terminal_fence_before_later_commands() {
    let (sender, receiver) = mpsc::channel(8);
    let revoked = Arc::new(AtomicBool::new(false));
    let mut lane = Lane::new(receiver, revoked.clone());
    let mut host = MemoryHost::default();
    sender.send(Command::Revoke).await.unwrap();
    let (reply, observation) = oneshot::channel();
    sender
        .send(Command::Control(
            PresentationCommand::Observe { lease: lease() },
            reply,
        ))
        .await
        .unwrap();
    sender.send(Command::Revoke).await.unwrap();
    lane.service(&mut host, false);
    assert!(matches!(
        observation.await.unwrap(),
        Err(PresentationError::Hidden)
    ));
    assert!(revoked.load(Ordering::Acquire));
    lane.service(&mut host, true);
    assert_eq!(host.revoked, 1);
    assert!(host.controls.is_empty());
}

#[tokio::test]
async fn continuous_queue_traffic_has_a_fixed_per_pump_bound() {
    let (sender, receiver) = mpsc::channel(32);
    let mut lane = Lane::new(receiver, Arc::new(AtomicBool::new(false)));
    let mut host = MemoryHost::default();
    let mut replies = Vec::new();
    for _ in 0..32 {
        let (reply, result) = oneshot::channel();
        sender
            .send(Command::Control(
                PresentationCommand::Observe { lease: lease() },
                reply,
            ))
            .await
            .unwrap();
        replies.push(result);
    }
    lane.service(&mut host, true);
    assert_eq!(host.controls.len(), 16);
    lane.service(&mut host, true);
    assert_eq!(host.controls.len(), 32);
    for reply in replies {
        assert!(matches!(
            reply.await.unwrap(),
            Ok(PresentationReply::State(_))
        ));
    }
}
