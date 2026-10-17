use super::*;

pub(super) async fn prepare(
    server: WorkerServer,
    native: Option<&colossus_sidecar_protocol::NativeBrowserBootstrap>,
    application_id: &str,
    workspace_identity: &BootstrapWorkspaceIdentity,
    instance: (&Path, Uuid),
    credential_id: &str,
    credentials: &Arc<colossus_worker::PublicApiCredentialManager>,
) -> Result<WorkerServer, FailureCode> {
    let (instance_dir, instance_id) = instance;
    let Some(native) = native else {
        return Ok(server);
    };
    let enrollment = colossus_browser_presentation::native_admission::NativeBrowserEnrollment {
        generation: *Uuid::parse_str(&native.generation)
            .map_err(|_| FailureCode::InvalidBootstrap)?
            .as_bytes(),
        instance: *instance_id.as_bytes(),
        application_id: application_id.to_owned(),
        workspace_version: workspace_identity.version,
        workspace_digest: hex::decode(&workspace_identity.sha256)
            .map_err(|_| FailureCode::InvalidWorkspace)?
            .try_into()
            .map_err(|_| FailureCode::InvalidWorkspace)?,
        parent_process_id: native.parent_process_id,
    };
    let native_options = colossus_worker::NativeBrowserServerConfig::new(
        instance_dir,
        enrollment,
        decode_worker_authentication(&native.authentication)
            .map_err(|_| FailureCode::InvalidBootstrap)?,
        credential_id.to_owned(),
        Arc::clone(credentials),
    )
    .map_err(|_| FailureCode::InvalidBootstrap)?;
    server
        .with_native_browser_api(native_options)
        .await
        .map_err(|_| FailureCode::RuntimeFailed)
}
