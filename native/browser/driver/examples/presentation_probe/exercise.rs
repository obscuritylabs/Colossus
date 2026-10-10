//! Real private presentation/native input acceptance with finite cleanup on failure.
use super::{
    fixture::{Fixture, ORIGIN},
    launch::Launched,
};
use colossus_browser_bridge::BrowserBridgeEnrollment;
use colossus_browser_presentation::{
    Configure, FrameCodec, HumanCommand, Input, Lease, PresentationCommand, PresentationError,
};
use colossus_contracts::{
    BrowserAction, BrowserDocumentId, BrowserMode, BrowserOpenOptions, BrowserOrigin,
    BrowserSessionBinding, BrowserSessionId, BrowserSnapshotId, BrowserTabId, BrowserTarget,
    BrowserUrl,
};
use colossus_ports::{
    BrowserDriver as _, BrowserDriverCommand, BrowserDriverControl, BrowserDriverOpenRequest,
    RunControl,
};
use sha2::{Digest as _, Sha256};
use std::{
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

pub async fn run() -> Result<(), &'static str> {
    let host = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("supply the exact staged native host")?
        .canonicalize()
        .map_err(|_| "native host unavailable")?;
    let component = host.parent().ok_or("native component unavailable")?;
    let manifest = manifest(component)?;
    if host
        .file_name()
        .is_none_or(|name| name != "colossus-native-browser-host")
        || !manifest.is_file()
    {
        return Err("supply the inventoried native component host");
    }
    let inventory = std::fs::read(&manifest).map_err(|_| "native inventory unavailable")?;
    let before = Sha256::digest(&inventory);
    let fixture = Fixture::start().await?;
    let binding = BrowserSessionBinding {
        runtime_id: "native-presentation-runtime".into(),
        workspace_id: "native-presentation-workspace".into(),
        application_id: "native-presentation-app".into(),
        scope: colossus_contracts::BrowserScope::Conversation {
            id: "native-presentation-conversation".into(),
        },
    };
    // Synthetic developer acceptance ceiling; never installed/released as capabilities.
    let capabilities = serde_json::from_value(serde_json::json!({"available":true,"engine_version":"154.0.8037.98","modes":["embedded"],"actions":["navigate","snapshot","click"],"limits":{"max_sessions":1,"max_tabs":1,"max_concurrent_actions":1,"max_snapshot_nodes":128,"max_observation_bytes":65536,"action_timeout_ms":30000,"navigation_timeout_ms":60000,"max_lease_ms":1800000},"private_ca_trust":false,"client_identities":false,"restrictive_egress":true})).map_err(|_| "fixture capability shape invalid")?;
    let origin = BrowserOrigin::parse(ORIGIN).map_err(|_| "fixture origin invalid")?;
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| "fixture nonce unavailable")?;
    let enrollment = BrowserBridgeEnrollment {
        binding: binding.clone(),
        mode: BrowserMode::Embedded,
        profile: Default::default(),
        allowed_origins: vec![origin.clone()],
        instance_nonce: nonce,
        component_digest: before.into(),
        cancellation_closes_context: true,
        capabilities,
    };
    let mut viewer_enrollment = enrollment.clone();
    let mut commit_enrollment = enrollment.clone();
    getrandom::fill(&mut commit_enrollment.instance_nonce)
        .map_err(|_| "fixture commit nonce unavailable")?;
    getrandom::fill(&mut viewer_enrollment.instance_nonce)
        .map_err(|_| "fixture viewer nonce unavailable")?;
    let mut launched = Launched::start(&host, component, enrollment, fixture.port, true).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        exercise(&launched, binding.clone(), origin.clone()),
    )
    .await
    .map_err(|_| "native presentation acceptance timed out")
    .and_then(|value| value);
    let cleanup = launched.finish().await;
    cleanup?;
    result?;
    let mut viewer =
        Launched::start(&host, component, viewer_enrollment, fixture.port, false).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        super::readonly::exercise(&viewer, binding.clone(), origin.clone(), &fixture, false),
    )
    .await
    .map_err(|_| "native viewer acceptance timed out")
    .and_then(|value| value);
    let cleanup = viewer.finish().await;
    cleanup?;
    result?;
    let mut commit =
        Launched::start(&host, component, commit_enrollment, fixture.port, false).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        super::readonly::exercise(&commit, binding, origin, &fixture, true),
    )
    .await
    .map_err(|_| "native pending-document acceptance timed out")
    .and_then(|value| value);
    let cleanup = commit.finish().await;
    cleanup?;
    result?;
    if fixture.requests.load(Ordering::Acquire) == 0 {
        return Err("native fixture proxy was not authenticated");
    }
    if Sha256::digest(std::fs::read(&manifest).map_err(|_| "native inventory unavailable")?)
        != before
    {
        return Err("native component inventory changed");
    }
    Ok(())
}
fn manifest(component: &std::path::Path) -> Result<PathBuf, &'static str> {
    #[cfg(target_os = "macos")]
    {
        let contents = component
            .parent()
            .filter(|path| path.file_name().is_some_and(|name| name == "Contents"))
            .ok_or("native host bundle layout invalid")?;
        let app = contents.parent().ok_or("native host bundle unavailable")?;
        let name = app
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| name.ends_with(".app"))
            .ok_or("native host bundle layout invalid")?;
        Ok(app.with_file_name(format!("{name}.browser-component.json")))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(component.join("browser-component.json"))
    }
}
pub(super) fn session() -> BrowserSessionId {
    BrowserSessionId::parse(format!("bs_{}", "1".repeat(32))).expect("fixed fixture identity")
}
pub(super) fn document(suffix: char) -> BrowserDocumentId {
    BrowserDocumentId::parse(format!("bd_{}", suffix.to_string().repeat(32)))
        .expect("fixed fixture identity")
}
pub(super) async fn painted(launched: &Launched, lease: Lease) -> Result<(), &'static str> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        launched
            .presentation
            .renew(lease, 1500)
            .await
            .map_err(|_| "native frame heartbeat failed")?;
        if let Some(bytes) = launched
            .presentation
            .next_frame(lease)
            .await
            .map_err(|_| "native frame transport failed")?
        {
            let frame =
                FrameCodec::new(launched.presentation.surface_key(), launched.digest, lease)
                    .map_err(|_| "native frame lease invalid")?
                    .decode(bytes)
                    .map_err(|_| "native BGRA authentication failed")?;
            if frame
                .pixels
                .chunks_exact(4)
                .any(|pixel| pixel[0] != pixel[1] || pixel[1] != pixel[2])
            {
                return Ok(());
            }
            // Chromium can publish the initial blank surface before the page's
            // first paint. Keep the same finite deadline for the colored content.
        }
        if Instant::now() >= deadline {
            return Err("native page did not produce a frame");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn exercise(
    launched: &Launched,
    binding: BrowserSessionBinding,
    origin: BrowserOrigin,
) -> Result<(), &'static str> {
    let control = BrowserDriverControl::new(RunControl::default(), RunControl::default());
    let tab_id =
        BrowserTabId::parse(format!("bt_{}", "2".repeat(32))).map_err(|_| "fixture tab invalid")?;
    let mut target = BrowserTarget {
        tab_id: tab_id.clone(),
        document_id: document('3'),
    };
    let original_target = target.clone();
    let request = BrowserDriverOpenRequest {
        binding: binding.clone(),
        run_id: None,
        session_id: session(),
        tab_id,
        document_id: target.document_id.clone(),
        options: BrowserOpenOptions {
            profile: Default::default(),
            mode: BrowserMode::Embedded,
            allowed_origins: vec![origin],
            initial_url: Some(
                BrowserUrl::parse(format!("{ORIGIN}/fixture.html"))
                    .map_err(|_| "fixture URL invalid")?,
            ),
        },
    };
    launched
        .browser
        .open_session(request, &control)
        .await
        .map_err(|_| "native presentation admission failed")?;
    let mut configure = Configure {
        session: session(),
        target: target.clone(),
        control_generation: 0,
        viewport_generation: 1,
        width: 800,
        height: 600,
        scale_milli: 1250,
        lease_ms: 1500,
    };
    let mut lease = launched
        .presentation
        .configure(configure.clone())
        .await
        .map_err(|_| "native viewport configure failed")?;
    if lease.pixel_width != 1000 || lease.pixel_height != 750 {
        return Err("native fractional scale dimensions disagree");
    }
    painted(launched, lease).await?;
    lease = super::handoff::navigate(launched, lease, &mut configure).await?;
    launched
        .presentation
        .focus(lease, true)
        .await
        .map_err(|_| "native focus acknowledgment failed")?;
    for pressed in [true, false] {
        launched
            .presentation
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
            .map_err(|_| "native mouse input failed")?;
    }
    for text in ["A", "b", "3"] {
        launched
            .presentation
            .input(lease, Input::Character { text: text.into() }, 0)
            .await
            .map_err(|_| "native character input failed")?;
    }
    launched
        .presentation
        .input(
            lease,
            Input::ImeCommit {
                text: "Ω😀".into()
            },
            0,
        )
        .await
        .map_err(|_| "native IME input failed")?;
    let mut stale = lease;
    stale.document_generation += 1;
    if launched
        .presentation
        .input(stale, Input::ImeCancel, 0)
        .await
        != Err(PresentationError::Stale)
    {
        return Err("native stale document input was accepted");
    }
    if launched
        .presentation
        .command(PresentationCommand::Human {
            lease,
            command: HumanCommand::Navigate {
                url: "https://outside-envelope.invalid/".into(),
            },
        })
        .await
        != Err(PresentationError::Hidden)
    {
        return Err("native human origin escape was accepted");
    }
    launched
        .presentation
        .hide(lease)
        .await
        .map_err(|_| "native hide failed")?;
    if launched
        .presentation
        .command(PresentationCommand::Human {
            lease,
            command: HumanCommand::Reload,
        })
        .await
        != Err(PresentationError::Hidden)
    {
        return Err("native hidden navigation was accepted");
    }
    if launched
        .presentation
        .input(lease, Input::ImeCancel, 0)
        .await
        != Err(PresentationError::Hidden)
    {
        return Err("native hidden input was accepted");
    }
    configure.viewport_generation += 1;
    lease = launched
        .presentation
        .configure(configure.clone())
        .await
        .map_err(|_| "native fresh viewport configure failed")?;
    painted(launched, lease).await?;
    tokio::time::sleep(Duration::from_millis(1550)).await;
    if launched
        .presentation
        .command(PresentationCommand::Human {
            lease,
            command: HumanCommand::Reload,
        })
        .await
        != Err(PresentationError::Hidden)
    {
        return Err("native expired navigation was accepted");
    }
    configure.viewport_generation += 1;
    lease = launched
        .presentation
        .configure(configure.clone())
        .await
        .map_err(|_| "native expired viewport recovery failed")?;
    let receipt = launched
        .presentation
        .fence_human(lease)
        .await
        .map_err(|_| "native human handoff fence failed")?;
    if receipt.prior_lease != lease
        || receipt.native_document_generation < lease.document_generation
    {
        return Err("native handoff receipt changed its exact prior attachment");
    }
    super::handoff::confirm(
        launched,
        binding.clone(),
        original_target,
        &receipt,
        &control,
    )
    .await?;
    target = receipt.state.target;
    configure.target = target.clone();
    if launched
        .presentation
        .input(lease, Input::ImeCancel, 0)
        .await
        != Err(PresentationError::Hidden)
    {
        return Err("fenced human authority accepted another native input");
    }
    // Real semantic read verifies native input on the manually navigated document
    // after one-shot native confirmation updates the private bridge target ledger.
    let observed = launched
        .browser
        .execute(
            BrowserDriverCommand {
                binding: binding.clone(),
                run_id: "native-presentation-run".into(),
                session_id: session(),
                target: target.clone(),
                control_generation: 1,
                action: BrowserAction::Snapshot { max_nodes: 128 },
                next_document_id: document('4'),
                snapshot_id: Some(
                    BrowserSnapshotId::parse(format!("bn_{}", "5".repeat(32)))
                        .map_err(|_| "fixture snapshot invalid")?,
                ),
                new_tab: None,
            },
            &control,
        )
        .await
        .map_err(|_| "native input semantic verification failed")?;
    let observed = serde_json::to_vec(&observed).map_err(|_| "native input observation invalid")?;
    let observed =
        std::str::from_utf8(&observed).map_err(|_| "native input observation invalid")?;
    if !observed.contains("Ab3Ω😀") {
        return Err("actual native input did not change the ordinary DOM field");
    }
    if launched
        .presentation
        .input(lease, Input::Character { text: "X".into() }, 0)
        .await
        != Err(PresentationError::Stale)
    {
        return Err("old human writer survived agent execution");
    }
    super::generation::exercise(launched, binding, target, configure, &control).await?;
    launched
        .browser
        .close_session(&session())
        .await
        .map_err(|_| "native close did not acknowledge CEF shutdown")?;
    Ok(())
}
