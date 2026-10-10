//! A delayed page-script commit is read-recoverable while old mutations stay fenced.
use super::{exercise::document, fixture::Fixture, launch::Launched};
use colossus_contracts::{BrowserAction, BrowserObservation, BrowserSnapshotId};
use colossus_ports::{
    BrowserDriver as _, BrowserDriverCommand, BrowserDriverControl, BrowserDriverError,
};

pub async fn exercise(
    launched: &Launched,
    mut read: BrowserDriverCommand,
    snapshot: &BrowserObservation,
    control: &BrowserDriverControl,
    mut configure: colossus_browser_presentation::Configure,
    fixture: &Fixture,
    held_commit: bool,
) -> Result<Option<BrowserDriverCommand>, &'static str> {
    let element = snapshot
        .snapshot
        .as_ref()
        .and_then(|value| {
            value
                .nodes
                .iter()
                .find(|node| node.role == "button" && node.name == "Native button")
        })
        .ok_or("synthetic delayed-navigation button unavailable")?
        .element
        .clone();
    let mut click = read.clone();
    click.action = BrowserAction::Click { element };
    click.snapshot_id = None;
    click.next_document_id = document('9');
    let result = launched
        .browser
        .execute(click.clone(), control)
        .await
        .map_err(|_| "synthetic delayed-navigation click failed")?;
    if result.tab.document_id != read.target.document_id {
        return Err("synthetic navigation committed before the delayed recovery fixture");
    }
    // The fixture's fixed page timer commits after Click's native response. Wait
    // for its fixed-origin receipt before testing the read-only observer fence.
    // Agent actions hide the previous pixel lease. Keep this read-only viewer
    // visible so Chromium need not defer the fixture timer as a background page.
    configure.viewport_generation += 1;
    let recovery_lease = launched
        .presentation
        .configure(configure)
        .await
        .map_err(|_| "recovery read-only viewer configure failed")?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while fixture
        .recovery_requests
        .load(std::sync::atomic::Ordering::Acquire)
        == 0
    {
        if tokio::time::Instant::now() >= deadline {
            return Err("synthetic delayed navigation did not reach its fixed-origin fixture");
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    loop {
        match launched.presentation.observe(recovery_lease).await {
            Err(colossus_browser_presentation::PresentationError::Stale) => break,
            Ok(page) if page.target == read.target => {
                if tokio::time::Instant::now() >= deadline {
                    return Err("synthetic delayed navigation did not commit its document");
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            _ => return Err("read-only observer stole the agent document ledger"),
        }
    }
    if launched.browser.execute(click, control).await != Err(BrowserDriverError::Stale) {
        return Err("old mutation escaped native document fence");
    }
    read.next_document_id = document('a');
    read.snapshot_id = Some(
        BrowserSnapshotId::parse(format!("bn_{}", "b".repeat(32)))
            .map_err(|_| "recovery snapshot identity invalid")?,
    );
    let future = launched.browser.execute(read.clone(), control);
    tokio::pin!(future);
    let recovered = if held_commit {
        tokio::select! {
            result = &mut future => result.map_err(|_| "pending-document snapshot failed")?,
            _ = tokio::time::sleep(std::time::Duration::from_millis(400)) => {
                fixture.recovery_released.store(true, std::sync::atomic::Ordering::Release);
                if future.await != Err(BrowserDriverError::OutcomeUnknown) {
                    return Err("snapshot spanning actual document commit was not fenced");
                }
                if launched.browser.capabilities().available
                    || launched.browser.execute(read, control).await != Err(BrowserDriverError::Unavailable)
                {
                    return Err("uncertain pending snapshot retained dispatch authority");
                }
                return Ok(None);
            }
        }
    } else {
        future
            .await
            .map_err(|_| "read-only native document recovery failed")?
    };
    if recovered.tab.document_id != read.next_document_id
        || recovered
            .snapshot
            .as_ref()
            .is_none_or(|value| value.document_id != read.next_document_id)
    {
        return Err("read recovery did not atomically commit the coordinator document");
    }
    if held_commit {
        super::commit::exercise(launched, read, &recovered, control, fixture)
            .await
            .map(Some)
    } else {
        read.target.document_id = recovered.tab.document_id;
        read.next_document_id = document('c');
        read.snapshot_id = Some(
            BrowserSnapshotId::parse(format!("bn_{}", "d".repeat(32)))
                .map_err(|_| "detached snapshot identity invalid")?,
        );
        Ok(Some(read))
    }
}
