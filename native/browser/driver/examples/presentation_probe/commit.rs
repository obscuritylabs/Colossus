//! A held response distinguishes navigation admission from actual document commit.
use super::{exercise::document, fixture::Fixture, launch::Launched};
use colossus_browser_presentation::{Configure, Lease, PresentationError};
use colossus_contracts::{BrowserAction, BrowserObservation, BrowserSnapshotId};
use colossus_ports::{
    BrowserDriver as _, BrowserDriverCommand, BrowserDriverControl, BrowserDriverError,
};
use std::{sync::atomic::Ordering, time::Duration};

pub async fn exercise(
    launched: &Launched,
    mut read: BrowserDriverCommand,
    premature: &BrowserObservation,
    control: &BrowserDriverControl,
    fixture: &Fixture,
) -> Result<BrowserDriverCommand, &'static str> {
    read.target.document_id = premature.tab.document_id.clone();
    let lease = launched
        .presentation
        .configure(Configure {
            session: read.session_id.clone(),
            target: read.target.clone(),
            control_generation: read.control_generation,
            viewport_generation: 3,
            width: 800,
            height: 600,
            scale_milli: 1001,
            lease_ms: 1500,
        })
        .await
        .map_err(|_| "pending-document viewer configuration failed")?;
    let page = launched
        .presentation
        .observe(lease)
        .await
        .map_err(|_| "pending-document observation failed")?;
    if page.target != read.target || !page.loading {
        return Err("premature snapshot was not taken before held response commit");
    }
    fixture.recovery_released.store(true, Ordering::Release);
    committed(launched, lease, &read).await?;
    let mut stale = read.clone();
    stale.action = BrowserAction::Click {
        element: premature
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.nodes.iter().find(|node| node.role == "button"))
            .ok_or("pending-document snapshot lost its ordinary element")?
            .element
            .clone(),
    };
    stale.snapshot_id = None;
    stale.next_document_id = document('c');
    if launched.browser.execute(stale, control).await != Err(BrowserDriverError::Stale) {
        return Err("premature snapshot mutation survived actual document commit");
    }
    read.next_document_id = document('c');
    read.snapshot_id = Some(snapshot('d')?);
    let fresh = launched
        .browser
        .execute(read.clone(), control)
        .await
        .map_err(|_| "committed-document read recovery failed")?;
    if fresh.tab.document_id != read.next_document_id
        || fresh
            .snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.document_id != read.next_document_id)
    {
        return Err("committed-document snapshot did not adopt its exact next ticket");
    }
    read.target.document_id = fresh.tab.document_id;
    read.next_document_id = document('e');
    read.snapshot_id = Some(snapshot('f')?);
    Ok(read)
}

async fn committed(
    launched: &Launched,
    lease: Lease,
    read: &BrowserDriverCommand,
) -> Result<(), &'static str> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match launched.presentation.observe(lease).await {
            Err(PresentationError::Stale) => return Ok(()),
            Ok(page) if page.target == read.target && page.loading => {
                if tokio::time::Instant::now() >= deadline {
                    return Err("held navigation failed to commit its native document");
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            _ => return Err("actual document commit reused the premature snapshot epoch"),
        }
    }
}

fn snapshot(byte: char) -> Result<BrowserSnapshotId, &'static str> {
    BrowserSnapshotId::parse(format!("bn_{}", byte.to_string().repeat(32)))
        .map_err(|_| "committed-document snapshot identity invalid")
}
