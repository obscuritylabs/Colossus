//! Actual application artifact bytes pass policy before native file input.
use super::{
    arguments::BrowserInvocation,
    service::{BrowserRun, RuntimeBrowserTools},
};
use crate::prelude::*;
use colossus_contracts::{BrowserAction, BrowserControlLease, BrowserTarget};
use colossus_ports::{BrowserUploadArtifact, BrowserUploadArtifactRequest};
use sha2::Digest as _;
use zeroize::Zeroizing;

pub(super) async fn prepare_upload(
    browser: &RuntimeBrowserTools,
    journal: Arc<dyn EventJournal>,
    invocation: &BrowserInvocation,
    context: &ExecutionContext,
) -> Result<Option<BrowserUploadArtifact>, ToolError> {
    let BrowserInvocation::Action {
        action: BrowserAction::Upload { artifact_id, .. },
        ..
    } = invocation
    else {
        return Ok(None);
    };
    let run = browser.run(context)?;
    browser.authority(&run, invocation)?;
    let publisher = browser
        .artifacts
        .lock()
        .map_err(|_| unavailable())?
        .clone()
        .ok_or_else(unavailable)?;
    let artifact = publisher
        .resolve_upload(
            journal,
            BrowserUploadArtifactRequest {
                binding: run.actor.binding.clone(),
                run_id: run.actor.run_id.clone(),
                artifact_id: artifact_id.clone(),
            },
        )
        .await
        .map_err(|_| {
            ToolError::Denied(
                "browser upload artifact is unavailable to this initiating owner".into(),
            )
        })?;
    if artifact.descriptor.artifact_id != *artifact_id
        || artifact.bytes.is_empty()
        || artifact.bytes.len() > colossus_ports::MAX_BROWSER_TRANSFER_BYTES as usize
        || artifact.bytes.len() != artifact.descriptor.size_bytes as usize
        || hex::encode(sha2::Sha256::digest(&*artifact.bytes)) != artifact.descriptor.sha256
    {
        return Err(ToolError::Denied(
            "browser upload artifact custody is invalid".into(),
        ));
    }
    Ok(Some(artifact))
}

pub(super) fn policy_content(
    invocation: &BrowserInvocation,
    upload: Option<&BrowserUploadArtifact>,
) -> Result<Value, ToolError> {
    let value = serde_json::to_value(invocation)
        .map_err(|_| ToolError::Failed("browser request encoding failed".into()))?;
    Ok(match upload {
        None => value,
        Some(upload) => {
            json!({"browser":value,"artifact":upload.descriptor,"content_base64":BASE64.encode(&*upload.bytes),"size":upload.bytes.len(),"media_type":"application/octet-stream"})
        }
    })
}
pub(super) fn invocation(request: &EffectRequest) -> Result<BrowserInvocation, ExecutionError> {
    let content = if request.action == "browser.upload" {
        let object = request.content.as_object().ok_or_else(invalid)?;
        if object.len() != 5
            || ![
                "browser",
                "artifact",
                "content_base64",
                "size",
                "media_type",
            ]
            .iter()
            .all(|key| object.contains_key(*key))
        {
            return Err(invalid());
        }
        &object["browser"]
    } else {
        &request.content
    };
    serde_json::from_value(content.clone()).map_err(|_| invalid())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn upload(
    browser: &RuntimeBrowserTools,
    run: &BrowserRun,
    lease: &BrowserControlLease,
    target: &BrowserTarget,
    element: colossus_contracts::BrowserElementRef,
    artifact_id: &str,
    request: &EffectRequest,
    prepared: &StdMutex<Option<BrowserUploadArtifact>>,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    let artifact = prepared
        .lock()
        .map_err(|_| invalid())?
        .take()
        .ok_or_else(invalid)?;
    let expected = serde_json::to_value(&artifact.descriptor).map_err(|_| invalid())?;
    if artifact.descriptor.artifact_id != artifact_id
        || request.content["artifact"] != expected
        || request.content["size"].as_u64() != Some(artifact.bytes.len() as u64)
        || request.content["media_type"].as_str() != Some("application/octet-stream")
    {
        return Err(invalid());
    }
    let encoded = request.content["content_base64"]
        .as_str()
        .ok_or_else(invalid)?;
    let actual = Zeroizing::new(BASE64.decode(encoded).map_err(|_| invalid())?);
    if *actual != *artifact.bytes || *Zeroizing::new(BASE64.encode(&*actual)) != encoded {
        return Err(invalid());
    }
    let observation = browser
        .coordinator
        .upload(&run.actor, lease, target, element, artifact, &run.control)
        .await
        .map_err(super::service::execution_error)?;
    let bytes = serde_json::to_vec(&observation).map_err(|_| invalid())?;
    Ok(QuarantinedEffectResult {
        bytes,
        media_type: "application/json".into(),
        effect_succeeded: true,
    })
}
fn invalid() -> ExecutionError {
    ExecutionError::Failed(
        "prepared browser upload does not match actual authorized artifact bytes".into(),
    )
}
pub(super) fn unavailable() -> ToolError {
    ToolError::Failed("browser artifact custody adapter unavailable".into())
}
