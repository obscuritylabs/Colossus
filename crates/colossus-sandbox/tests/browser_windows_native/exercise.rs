//! Real private frame/native-input receipt, then read-only agent view and detach.
use colossus_browser_presentation::{
    Configure, FrameCodec, HumanCommand, Input, Lease, PresentationCommand, PresentationError,
};
use colossus_contracts::{BrowserAction, BrowserDocumentId, BrowserSnapshotId, BrowserTarget};
use colossus_ports::{
    BrowserDriver, BrowserDriverCommand, BrowserDriverControl, BrowserDriverOpenRequest, RunControl,
};
use colossus_sandbox::{WindowsBrowserPresentation, WindowsBrowserSupervisor};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub async fn run(
    supervisor: Arc<WindowsBrowserSupervisor>,
    request: BrowserDriverOpenRequest,
    denied: String,
    origin: String,
    uploaded: Arc<std::sync::atomic::AtomicUsize>,
) {
    let control = BrowserDriverControl::new(RunControl::default(), RunControl::default());
    use colossus_browser_bridge::BrowserHostFactory as _;
    let driver = supervisor.launch(&request, &control).await.expect(
        "native Windows launch unavailable; retain state and inspect platform prerequisites",
    );
    let tab = driver
        .open_session(request.clone(), &control)
        .await
        .expect("native owned session readiness");
    let attachment = supervisor
        .take_presentation(&request)
        .await
        .expect("exact owned presentation admission");
    assert!(
        supervisor.take_presentation(&request).await.is_err(),
        "attachment cannot be copied or replayed"
    );
    let mut configure = Configure {
        session: request.session_id.clone(),
        target: BrowserTarget {
            tab_id: tab.tab_id,
            document_id: tab.document_id,
        },
        control_generation: 0,
        viewport_generation: 1,
        width: 800,
        height: 600,
        scale_milli: 1250,
        lease_ms: 1500,
    };
    let lease = attachment
        .client
        .configure(configure.clone())
        .await
        .expect("actual native viewport");
    assert_eq!((lease.pixel_width, lease.pixel_height), (1000, 750));
    painted(&attachment, lease).await;
    attachment
        .client
        .focus(lease, true)
        .await
        .expect("native focus ack");
    for pressed in [true, false] {
        attachment
            .client
            .input(
                lease,
                Input::MouseButton {
                    x: 125,
                    y: 125,
                    button: 0,
                    pressed,
                },
                0,
            )
            .await
            .expect("native mouse input");
    }
    for text in ["A", "b", "3"] {
        attachment
            .client
            .input(lease, Input::Character { text: text.into() }, 0)
            .await
            .expect("native key input");
    }
    attachment
        .client
        .input(
            lease,
            Input::ImeCommit {
                text: "Ω😀".into()
            },
            0,
        )
        .await
        .expect("native IME commit");
    let mut stale = lease;
    stale.document_generation += 1;
    assert_eq!(
        attachment.client.input(stale, Input::ImeCancel, 0).await,
        Err(PresentationError::Stale)
    );
    assert!(
        attachment
            .client
            .command(PresentationCommand::Human {
                lease,
                command: HumanCommand::Navigate {
                    url: format!("{denied}/outside")
                }
            })
            .await
            .is_err(),
        "foreign loopback origin must be denied"
    );
    assert!(
        attachment
            .client
            .command(PresentationCommand::Human {
                lease,
                command: HumanCommand::Navigate {
                    url: "http://unreviewed-dns.invalid/outside".into()
                }
            })
            .await
            .is_err(),
        "foreign DNS origin must be denied"
    );
    let mut found = false;
    let receipt = attachment
        .client
        .fence_human(lease)
        .await
        .expect("explicit native human-to-agent handoff barrier");
    assert_eq!(
        receipt.prior_lease, lease,
        "handoff must acknowledge exact prior viewport"
    );
    assert!(
        receipt.native_document_generation >= lease.document_generation,
        "handoff carries current native document evidence"
    );
    configure.target = receipt.state.target;
    assert_eq!(
        attachment.client.input(lease, Input::ImeCancel, 0).await,
        Err(PresentationError::Hidden),
        "human input must be fenced before any agent snapshot"
    );
    for index in 1..=25 {
        let observed = snapshot(&*driver, &request, &configure.target, &control, index).await;
        configure.target.document_id = observed.tab.document_id.clone();
        let bytes = serde_json::to_vec(&observed).expect("bounded synthetic observation");
        let text = std::str::from_utf8(&bytes).expect("synthetic UTF-8 observation");
        assert!(
            !text.contains("synthetic-protected-value"),
            "password value must stay redacted"
        );
        if text.contains("Ab3Ω😀") {
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        found,
        "actual native input must change the ordinary DOM field"
    );
    super::capture::check(&*driver, &request, &configure.target, &control).await;
    super::transfer::check(
        &*driver,
        &request,
        &mut configure.target,
        &control,
        &origin,
        &uploaded,
    )
    .await;
    assert!(
        attachment
            .client
            .input(lease, Input::Character { text: "X".into() }, 0)
            .await
            .is_err(),
        "agent admission revokes old human writer"
    );
    configure.control_generation = 1;
    configure.viewport_generation = 2;
    let viewer = attachment
        .client
        .configure(configure.clone())
        .await
        .expect("read-only current agent viewport");
    painted(&attachment, viewer).await;
    assert_eq!(
        attachment.client.focus(viewer, true).await,
        Err(PresentationError::Hidden)
    );
    assert_eq!(
        attachment.client.input(viewer, Input::ImeCancel, 0).await,
        Err(PresentationError::Hidden)
    );
    attachment
        .client
        .hide(viewer)
        .await
        .expect("native viewport hide ack");
    assert!(
        attachment.client.renew(viewer, 1500).await.is_err(),
        "hidden lease cannot revive"
    );
    attachment.client.disconnect();
    let _ = snapshot(&*driver, &request, &configure.target, &control, 50).await;
    driver
        .close_session(&request.session_id)
        .await
        .expect("native close and full cleanup receipt");
    driver
        .close_session(&request.session_id)
        .await
        .expect("native close retry is idempotent");
    assert!(
        !driver.capabilities().available,
        "terminal cleanup fences capabilities"
    );
}
pub(super) async fn snapshot(
    driver: &dyn BrowserDriver,
    request: &BrowserDriverOpenRequest,
    target: &BrowserTarget,
    control: &BrowserDriverControl,
    index: u32,
) -> colossus_contracts::BrowserObservation {
    driver
        .execute(
            command(
                request,
                target,
                BrowserAction::Snapshot { max_nodes: 128 },
                index,
            ),
            control,
        )
        .await
        .expect("actual agent semantic observation after native input or viewer detach")
}
pub(super) fn command(
    request: &BrowserDriverOpenRequest,
    target: &BrowserTarget,
    action: BrowserAction,
    index: u32,
) -> BrowserDriverCommand {
    let snapshot_id = matches!(action, BrowserAction::Snapshot { .. })
        .then(|| BrowserSnapshotId::parse(format!("bn_{index:032x}")).unwrap());
    BrowserDriverCommand {
        binding: request.binding.clone(),
        run_id: "native-windows-acceptance-run".into(),
        session_id: request.session_id.clone(),
        target: target.clone(),
        control_generation: 1,
        action,
        next_document_id: BrowserDocumentId::parse(format!("bd_{index:032x}")).unwrap(),
        snapshot_id,
        new_tab: None,
    }
}
async fn painted(attachment: &WindowsBrowserPresentation, lease: Lease) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        attachment
            .client
            .renew(lease, 1500)
            .await
            .expect("native visibility heartbeat");
        if let Some(bytes) = attachment
            .client
            .next_frame(lease)
            .await
            .expect("actual native frame")
        {
            let frame = FrameCodec::new(
                attachment.client.surface_key(),
                attachment.enrollment_digest,
                lease,
            )
            .unwrap()
            .decode(bytes)
            .expect("native frame full enrollment authentication");
            assert!(
                frame
                    .pixels
                    .chunks_exact(4)
                    .any(|p| p[0] != p[1] || p[1] != p[2]),
                "actual colored fixture BGRA required"
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "actual native BGRA paint deadline"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
