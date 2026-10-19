use super::{
    effect::{StagedComputerReply, commit},
    execution::{ComputerToolExecutor, unknown},
    service::{ComputerRun, RuntimeComputerTools},
};
use crate::prelude::*;
use colossus_contracts::ComputerScreenshotDescriptor;
use colossus_ports::{ComputerArtifactPublication, ComputerScreenshotCapture};
use zeroize::Zeroizing;

pub(super) fn admit(
    service: &RuntimeComputerTools,
    context: &ExecutionContext,
) -> Result<(), ToolError> {
    if service.artifacts.lock().map_err(|_| unknown())?.is_none() {
        return Err(ToolError::Failed(
            "computer image publisher unavailable".into(),
        ));
    }
    if context
        .image_input_budget
        .as_ref()
        .is_some_and(|budget| budget.remaining_count == 0 || budget.remaining_bytes == 0)
    {
        return Err(ToolError::Failed(
            "computer screenshot exceeds remaining image context budget".into(),
        ));
    }
    Ok(())
}

pub(super) fn quarantine(
    service: &RuntimeComputerTools,
    run: &ComputerRun,
    capture: ComputerScreenshotCapture,
    bound: u64,
    staged: &StdMutex<Option<StagedComputerReply>>,
    observed_at: Instant,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    let invalid = || {
        ExecutionError::OutcomeUnknown(
            "computer capture evidence mismatch; automatic retry is prohibited".into(),
        )
    };
    capture
        .descriptor
        .validate(&service.limits)
        .map_err(|_| invalid())?;
    let verified = validate_image_bytes("computer-screenshot.png", Some("image/png"), &capture.png)
        .map_err(|_| invalid())?;
    let descriptor = &capture.descriptor;
    if verified.sha256 != descriptor.sha256
        || verified.size_bytes != u64::from(descriptor.size_bytes)
        || verified.width_pixels != descriptor.width
        || verified.height_pixels != descriptor.height
        || verified.size_bytes > bound
    {
        return Err(invalid());
    }
    let state = run.state.lock().map_err(|_| invalid())?;
    if state
        .session
        .as_ref()
        .is_none_or(|session| session.window_id != capture.root_window_id)
        || state
            .observation
            .as_ref()
            .is_some_and(|(id, _)| id == &descriptor.observation_id)
    {
        return Err(invalid());
    }
    drop(state);
    *staged.lock().map_err(|_| invalid())? = Some(StagedComputerReply::Screenshot(
        capture.descriptor,
        observed_at,
    ));
    Ok(QuarantinedEffectResult {
        bytes: capture.png,
        media_type: "image/png".into(),
        effect_succeeded: true,
    })
}

pub(super) async fn publish(
    executor: &ComputerToolExecutor,
    call: ToolCall,
    context: ExecutionContext,
    run: &ComputerRun,
    descriptor: ComputerScreenshotDescriptor,
    bytes: Vec<u8>,
    observed_at: Instant,
) -> Result<ToolResult, ToolError> {
    executor.computer.run(&context).map_err(|_| unknown())?;
    let publisher = executor
        .computer
        .artifacts
        .lock()
        .map_err(|_| unknown())?
        .clone()
        .ok_or_else(unknown)?;
    let image = publisher
        .publish(
            Arc::clone(&executor.journal),
            ComputerArtifactPublication {
                binding: run.binding.clone(),
                call_id: call.call_id.clone(),
                descriptor: descriptor.clone(),
                bytes: Zeroizing::new(bytes),
            },
        )
        .await
        .map_err(|_| unknown())?;
    if image.sha256 != descriptor.sha256
        || image.media_type != "image/png"
        || image.size_bytes != u64::from(descriptor.size_bytes)
        || image.width_pixels != descriptor.width
        || image.height_pixels != descriptor.height
    {
        return Err(unknown());
    }
    executor.computer.run(&context).map_err(|_| unknown())?;
    let output = serde_json::to_string(&json!({"session_id":descriptor.session_id,"control_generation":descriptor.control_generation,
        "observation_id":descriptor.observation_id,"window_id":descriptor.window_id,"artifact":image})).map_err(|_| unknown())?;
    commit(
        run,
        StagedComputerReply::Screenshot(descriptor, observed_at),
    )
    .map_err(|_| unknown())?;
    Ok(ToolResult {
        call_id: call.call_id,
        name: call.name,
        output,
        exit_code: 0,
        images: vec![image],
    })
}
