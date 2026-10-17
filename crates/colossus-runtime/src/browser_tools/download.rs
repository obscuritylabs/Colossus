use super::{
    execution::BrowserToolExecutor,
    service::{BrowserRun, RuntimeBrowserTools},
};
use crate::prelude::*;
use colossus_contracts::{
    BrowserControlLease, BrowserElementRef, BrowserSessionBinding, BrowserTarget,
};
use colossus_ports::{BrowserDownloadDescriptor, BrowserDownloadPublication};
use sha2::Digest as _;
use zeroize::Zeroizing;
pub(super) struct Receipt {
    binding: BrowserSessionBinding,
    run_id: String,
    descriptor: BrowserDownloadDescriptor,
}

pub(super) async fn capture(
    browser: &RuntimeBrowserTools,
    run: &BrowserRun,
    lease: &BrowserControlLease,
    target: &BrowserTarget,
    element: BrowserElementRef,
    policy_bound: u64,
    receipt: &StdMutex<Option<Receipt>>,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    if policy_bound < u64::from(colossus_ports::MAX_BROWSER_TRANSFER_BYTES) {
        return Err(ExecutionError::Failed(
            "browser download exceeds policy output ceiling".into(),
        ));
    }
    let mut captured = browser
        .coordinator
        .download(&run.actor, lease, target, element, &run.control)
        .await
        .map_err(super::service::execution_error)?;
    if captured.bytes.len() != captured.descriptor.size_bytes as usize
        || hex::encode(sha2::Sha256::digest(&*captured.bytes)) != captured.descriptor.sha256
    {
        return Err(ExecutionError::OutcomeUnknown(
            "browser download evidence mismatch; automatic retry is prohibited".into(),
        ));
    }
    *receipt.lock().map_err(|_| {
        ExecutionError::OutcomeUnknown("browser download receipt unavailable".into())
    })? = Some(Receipt {
        binding: run.actor.binding.clone(),
        run_id: run.actor.run_id.clone(),
        descriptor: captured.descriptor,
    });
    Ok(QuarantinedEffectResult {
        bytes: std::mem::take(&mut *captured.bytes),
        media_type: "application/octet-stream".into(),
        effect_succeeded: true,
    })
}
pub(super) async fn publish(
    executor: &BrowserToolExecutor,
    call: ToolCall,
    context: &ExecutionContext,
    receipt: Receipt,
    bytes: Vec<u8>,
) -> Result<ToolResult, ToolError> {
    let bytes = Zeroizing::new(bytes);
    executor.browser.run(context).map_err(|_| unknown())?;
    let publisher = executor
        .browser
        .artifacts
        .lock()
        .map_err(|_| unknown())?
        .clone()
        .ok_or_else(unknown)?;
    let session_id = receipt.descriptor.session_id.clone();
    let target = receipt.descriptor.target.clone();
    let control_generation = receipt.descriptor.control_generation;
    let artifact = publisher
        .publish_download(
            Arc::clone(&executor.journal),
            BrowserDownloadPublication {
                binding: receipt.binding,
                run_id: receipt.run_id,
                call_id: call.call_id.clone(),
                descriptor: receipt.descriptor,
                bytes,
            },
        )
        .await
        .map_err(|_| unknown())?;
    let output=serde_json::to_string(&json!({"session_id":session_id,"target":target,"control_generation":control_generation,"artifact":artifact})).map_err(|_| unknown())?;
    Ok(ToolResult {
        call_id: call.call_id,
        name: call.name,
        output,
        exit_code: 0,
        images: Vec::new(),
    })
}
fn unknown() -> ToolError {
    ToolError::OutcomeUnknown(
        "browser download publication did not positively complete; automatic retry is prohibited"
            .into(),
    )
}
