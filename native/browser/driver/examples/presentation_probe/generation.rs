//! Real navigation rotates opaque targets and independently fences old native leases.
use super::{
    exercise::{document, painted, session},
    fixture::ORIGIN,
    launch::Launched,
};
use colossus_browser_presentation::{Configure, PresentationError};
use colossus_contracts::{
    BrowserAction, BrowserSessionBinding, BrowserSnapshotId, BrowserTarget, BrowserUrl,
};
use colossus_ports::{
    BrowserDriver as _, BrowserDriverCommand, BrowserDriverControl, BrowserDriverError,
};

pub async fn exercise(
    launched: &Launched,
    binding: BrowserSessionBinding,
    target: BrowserTarget,
    mut configure: Configure,
    control: &BrowserDriverControl,
) -> Result<(), &'static str> {
    configure.control_generation = 1;
    configure.viewport_generation += 1;
    configure.target = target.clone();
    let lease = launched
        .presentation
        .configure(configure.clone())
        .await
        .map_err(|_| "agent viewer configure failed")?;
    let mut command = BrowserDriverCommand {
        binding,
        run_id: "native-presentation-run".into(),
        session_id: session(),
        target: target.clone(),
        control_generation: 1,
        action: BrowserAction::Navigate {
            url: BrowserUrl::parse(format!("{ORIGIN}/fixture.html"))
                .map_err(|_| "fixture navigation URL invalid")?,
        },
        next_document_id: document('6'),
        snapshot_id: None,
        new_tab: None,
    };
    let observed = launched
        .browser
        .execute(command.clone(), control)
        .await
        .map_err(|_| "native navigation did not complete")?;
    if observed.tab.document_id == target.document_id {
        return Err("native navigation reused its opaque document");
    }
    command.action = BrowserAction::Snapshot { max_nodes: 32 };
    command.snapshot_id = Some(
        BrowserSnapshotId::parse(format!("bn_{}", "7".repeat(32)))
            .map_err(|_| "fixture stale snapshot invalid")?,
    );
    if !matches!(
        launched.browser.execute(command, control).await,
        Err(BrowserDriverError::Stale)
    ) {
        return Err("old opaque target survived native navigation");
    }
    configure.viewport_generation += 1;
    if launched.presentation.configure(configure.clone()).await != Err(PresentationError::Stale) {
        return Err("old opaque document configured a new native page");
    }
    if launched.presentation.next_frame(lease).await != Err(PresentationError::Stale) {
        return Err("old native document lease produced new page pixels");
    }
    let state = launched
        .presentation
        .observe(lease)
        .await
        .map_err(|_| "native document recovery metadata failed")?;
    if state.target.document_id != observed.tab.document_id {
        return Err("private recovery target disagreed with native action commit");
    }
    configure.target = state.target;
    let lease = launched
        .presentation
        .configure(configure)
        .await
        .map_err(|_| "new native document lease failed")?;
    painted(launched, lease).await
}
