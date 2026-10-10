use super::{
    execution::BrowserToolExecutor,
    service::{BrowserRun, RuntimeBrowserTools, execution_error},
};
use crate::prelude::*;
use colossus_contracts::{BrowserControlLease, BrowserSessionBinding, BrowserTarget};
use colossus_ports::{BrowserArtifactPublication, BrowserScreenshotDescriptor};
use zeroize::Zeroizing;

pub(super) struct Receipt {
    binding: BrowserSessionBinding,
    run_id: String,
    descriptor: BrowserScreenshotDescriptor,
}

pub(super) async fn capture(
    browser: &RuntimeBrowserTools,
    run: &BrowserRun,
    lease: &BrowserControlLease,
    target: &BrowserTarget,
    max_bytes: u32,
    policy_bound: u64,
    receipt: &StdMutex<Option<Receipt>>,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    if u64::from(max_bytes) > policy_bound {
        return Err(ExecutionError::Failed(
            "browser screenshot exceeds policy output ceiling".into(),
        ));
    }
    let mut captured = browser
        .coordinator
        .screenshot(&run.actor, lease, target, max_bytes, &run.control)
        .await
        .map_err(execution_error)?;
    let verified =
        validate_image_bytes("browser-screenshot.png", Some("image/png"), &captured.bytes)
            .map_err(|_| {
                ExecutionError::OutcomeUnknown(
                    "browser capture did not return a verified PNG; automatic retry is prohibited"
                        .into(),
                )
            })?;
    if verified.sha256 != captured.descriptor.sha256
        || verified.size_bytes != u64::from(captured.descriptor.size_bytes)
        || verified.width_pixels != captured.descriptor.width
        || verified.height_pixels != captured.descriptor.height
    {
        return Err(ExecutionError::OutcomeUnknown(
            "browser capture evidence mismatch; automatic retry is prohibited".into(),
        ));
    }
    *receipt.lock().map_err(|_| {
        ExecutionError::OutcomeUnknown("browser capture receipt unavailable".into())
    })? = Some(Receipt {
        binding: run.actor.binding.clone(),
        run_id: run.actor.run_id.clone(),
        descriptor: captured.descriptor,
    });
    // The gateway owns these exact bytes until mandatory post-effect policy decides release.
    Ok(QuarantinedEffectResult {
        bytes: std::mem::take(&mut *captured.bytes),
        media_type: "image/png".into(),
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
    let target = receipt.descriptor.target.clone();
    let session_id = receipt.descriptor.session_id.clone();
    let control_generation = receipt.descriptor.control_generation;
    let image = publisher
        .publish(
            Arc::clone(&executor.journal),
            BrowserArtifactPublication {
                binding: receipt.binding,
                run_id: receipt.run_id,
                call_id: call.call_id.clone(),
                descriptor: receipt.descriptor,
                bytes,
            },
        )
        .await
        .map_err(|_| unknown())?;
    let output = serde_json::to_string(&json!({"session_id":session_id,"target":target,"control_generation":control_generation,"artifact":image}))
        .map_err(|_| unknown())?;
    Ok(ToolResult {
        call_id: call.call_id,
        name: call.name,
        output,
        exit_code: 0,
        images: vec![image],
    })
}

fn unknown() -> ToolError {
    ToolError::OutcomeUnknown("browser screenshot publication did not positively complete; automatic capture retry is prohibited".into())
}
