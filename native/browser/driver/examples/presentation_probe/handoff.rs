//! Real manual navigation must be confirmed by the same irrevocably fenced native host.
use super::{
    exercise::{painted, session},
    fixture::ORIGIN,
    launch::Launched,
};
use colossus_browser_presentation::{Configure, HumanCommand, Lease, PresentationCommand};
use colossus_contracts::{BrowserSessionBinding, BrowserTarget};
use colossus_ports::{
    BrowserDriver as _, BrowserDriverControl, BrowserDriverError, BrowserNativeHandoffRequest,
};
use std::time::{Duration, Instant};

pub async fn navigate(
    launched: &Launched,
    lease: Lease,
    configure: &mut Configure,
) -> Result<Lease, &'static str> {
    launched
        .presentation
        .command(PresentationCommand::Human {
            lease,
            command: HumanCommand::Navigate {
                url: format!("{ORIGIN}/fixture.html?handoff=1"),
            },
        })
        .await
        .map_err(|_| "manual native navigation failed")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = launched
            .presentation
            .observe(lease)
            .await
            .map_err(|_| "manual native document observation failed")?;
        if !state.loading && state.target.document_id != configure.target.document_id {
            configure.target = state.target;
            configure.viewport_generation += 1;
            let lease = launched
                .presentation
                .configure(configure.clone())
                .await
                .map_err(|_| "manual native document presentation failed")?;
            painted(launched, lease).await?;
            return Ok(lease);
        }
        if Instant::now() >= deadline {
            return Err("manual native navigation did not commit a fresh document");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

pub async fn confirm(
    launched: &Launched,
    binding: BrowserSessionBinding,
    original: BrowserTarget,
    receipt: &colossus_browser_presentation::HumanFenceReceipt,
    control: &BrowserDriverControl,
) -> Result<(), &'static str> {
    if receipt.state.target.document_id == original.document_id {
        return Err("manual document did not change before native handoff");
    }
    let request = BrowserNativeHandoffRequest {
        binding,
        session_id: session(),
        expected_target: original,
        confirmed_target: receipt.state.target.clone(),
        native_document_generation: receipt.native_document_generation,
    };
    let tab = launched
        .browser
        .confirm_native_handoff(request.clone(), control)
        .await
        .map_err(|_| "native fenced document confirmation failed")?;
    if tab.tab_id != request.confirmed_target.tab_id
        || tab.document_id != request.confirmed_target.document_id
    {
        return Err("native confirmation changed its exact fenced target");
    }
    if launched
        .browser
        .confirm_native_handoff(request, control)
        .await
        != Err(BrowserDriverError::Stale)
    {
        return Err("native handoff confirmation was replayable");
    }
    Ok(())
}
