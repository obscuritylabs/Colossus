//! Read-only trusted native viewer of a real nonzero agent writer generation.
use super::{
    exercise::{document, painted, session},
    fixture::{Fixture, ORIGIN},
    launch::Launched,
};
use colossus_browser_presentation::{Configure, Input, PresentationError};
use colossus_contracts::{
    BrowserAction, BrowserMode, BrowserOpenOptions, BrowserOrigin, BrowserSessionBinding,
    BrowserSnapshotId, BrowserTabId, BrowserTarget, BrowserUrl,
};
use colossus_ports::{
    BrowserDriver as _, BrowserDriverCommand, BrowserDriverControl, BrowserDriverOpenRequest,
    RunControl,
};

pub async fn exercise(
    launched: &Launched,
    binding: BrowserSessionBinding,
    origin: BrowserOrigin,
    fixture: &Fixture,
    held_commit: bool,
) -> Result<(), &'static str> {
    fixture
        .recovery_requests
        .store(0, std::sync::atomic::Ordering::Release);
    fixture
        .recovery_released
        .store(!held_commit, std::sync::atomic::Ordering::Release);
    let control = BrowserDriverControl::new(RunControl::default(), RunControl::default());
    let tab_id = BrowserTabId::parse(format!("bt_{}", "2".repeat(32)))
        .map_err(|_| "fixture viewer tab invalid")?;
    let target = BrowserTarget {
        tab_id: tab_id.clone(),
        document_id: document('3'),
    };
    launched
        .browser
        .open_session(
            BrowserDriverOpenRequest {
                binding: binding.clone(),
                run_id: Some("native-viewer-run".into()),
                session_id: session(),
                tab_id,
                document_id: target.document_id.clone(),
                options: BrowserOpenOptions {
                    profile: Default::default(),
                    mode: BrowserMode::Embedded,
                    allowed_origins: vec![origin],
                    initial_url: Some(
                        BrowserUrl::parse(format!("{ORIGIN}/fixture.html"))
                            .map_err(|_| "fixture viewer URL invalid")?,
                    ),
                },
            },
            &control,
        )
        .await
        .map_err(|_| "native viewer agent admission failed")?;
    let command = BrowserDriverCommand {
        binding,
        run_id: "native-viewer-run".into(),
        session_id: session(),
        target,
        control_generation: 1,
        action: BrowserAction::Snapshot { max_nodes: 128 },
        next_document_id: document('4'),
        snapshot_id: Some(
            BrowserSnapshotId::parse(format!("bn_{}", "5".repeat(32)))
                .map_err(|_| "fixture viewer snapshot invalid")?,
        ),
        new_tab: None,
    };
    let observed = launched
        .browser
        .execute(command.clone(), &control)
        .await
        .map_err(|_| "native viewer agent action failed")?;
    let configure = Configure {
        session: session(),
        target: BrowserTarget {
            tab_id: observed.tab.tab_id.clone(),
            document_id: observed.tab.document_id.clone(),
        },
        control_generation: 1,
        viewport_generation: 1,
        width: 800,
        height: 600,
        scale_milli: 1001,
        lease_ms: 1500,
    };
    let lease = launched
        .presentation
        .configure(configure.clone())
        .await
        .map_err(|_| "native nonzero viewer configure failed")?;
    if lease.control_generation != 1 || lease.pixel_width != 801 || lease.pixel_height != 601 {
        return Err("native viewer invented a controller or scale");
    }
    painted(launched, lease).await?;
    if launched
        .presentation
        .input(lease, Input::Character { text: "X".into() }, 0)
        .await
        != Err(PresentationError::Hidden)
    {
        return Err("read-only native viewer gained human input");
    }
    if launched.presentation.focus(lease, true).await != Err(PresentationError::Hidden) {
        return Err("read-only native viewer gained focus");
    }
    let mut wrong = configure.clone();
    wrong.control_generation = 0;
    wrong.viewport_generation = 2;
    if launched.presentation.configure(wrong).await != Err(PresentationError::Stale) {
        return Err("read-only viewer accepted invented human epoch");
    }
    let command = super::recovery::exercise(
        launched,
        command,
        &observed,
        &control,
        configure,
        fixture,
        held_commit,
    )
    .await?;
    if let Some(command) = command {
        launched.presentation.disconnect();
        launched
            .browser
            .execute(command, &control)
            .await
            .map_err(|_| "detaching viewer revoked independent agent authority")?;
    }
    launched
        .browser
        .close_session(&session())
        .await
        .map_err(|_| "native viewer close did not acknowledge CEF shutdown")?;
    Ok(())
}
