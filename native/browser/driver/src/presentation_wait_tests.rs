use super::*;
use colossus_browser_presentation::Configure;
use colossus_contracts::{BrowserDocumentId, BrowserSessionId, BrowserTabId, BrowserTarget};

#[derive(Default)]
struct MemoryHost {
    controls: Vec<PresentationCommand>,
    frames: usize,
    revoked: usize,
}
impl Target for MemoryHost {
    fn control(
        &mut self,
        command: PresentationCommand,
    ) -> Result<PresentationReply, PresentationError> {
        self.controls.push(command);
        Ok(PresentationReply::Ack)
    }
    fn frame(&mut self, _: Lease) -> Result<Option<Frame>, PresentationError> {
        self.frames += 1;
        Ok(None)
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
async fn slow_data_wait_settles_readers_without_interleaving_page_or_input_changes() {
    let (sender, receiver) = mpsc::channel(8);
    let mut lane = Lane::new(receiver, Arc::new(AtomicBool::new(false)));
    let mut host = MemoryHost::default();
    let mut results = Vec::new();
    for command in [
        PresentationCommand::Observe { lease: lease() },
        PresentationCommand::Configure(configuration()),
        PresentationCommand::Human {
            lease: lease(),
            command: colossus_browser_presentation::HumanCommand::Reload,
        },
        PresentationCommand::Focus {
            lease: lease(),
            focused: true,
        },
        PresentationCommand::FenceHuman { lease: lease() },
    ] {
        let (reply, result) = oneshot::channel();
        sender.send(Command::Control(command, reply)).await.unwrap();
        results.push(result);
    }
    let (reply, frame) = oneshot::channel();
    sender.send(Command::Frame(lease(), reply)).await.unwrap();
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
    assert!(matches!(result.await.unwrap(), Ok(PresentationReply::Ack)));
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
    assert!(host.revoked > 0);
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
    lane.service(&mut host, false);
    assert_eq!(host.controls.len(), 16);
    lane.service(&mut host, false);
    assert_eq!(host.controls.len(), 32);
    for reply in replies {
        assert!(matches!(reply.await.unwrap(), Ok(PresentationReply::Ack)));
    }
}
