use super::*;

/// Permit-bearing adapter for pack and bundle effects.
pub struct PackExecutor {
    service: Arc<PackService>,
}

impl PackExecutor {
    /// Construct the adapter around one lifecycle service.
    pub fn new(service: Arc<PackService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl EffectExecutor for PackExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: PackOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if request.action != operation.action() || request.resource != operation.resource() {
            return Err(ExecutionError::Failed(
                "pack request does not match its authorized action and resource".into(),
            ));
        }
        if let Some(path) = source_path(&operation) {
            enforce_read_grant(path, &permit)?;
        }
        if let Some(path) = destination_path(&operation) {
            enforce_write_grant(path, &permit)?;
        }
        if matches!(
            operation,
            PackOperation::RegistryPull { .. } | PackOperation::RegistryPush { .. }
        ) {
            enforce_registry_credentials(&operation, request)?;
        }
        let value = match operation {
            PackOperation::Verify { path } => {
                serde_json::to_value(self.service.verify(Path::new(&path)).map_err(execution)?)
            }
            PackOperation::Install {
                path,
                allow_untrusted,
            } => serde_json::to_value(
                self.service
                    .install(Path::new(&path), allow_untrusted, request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::Enable { name } => serde_json::to_value(
                self.service
                    .enable(&name, request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::Disable { name } => serde_json::to_value(
                self.service
                    .disable(&name, request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::Uninstall { name } => serde_json::to_value(
                self.service
                    .uninstall(&name, request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::TrustAdd {
                publisher,
                public_key,
            } => serde_json::to_value(
                self.service
                    .add_trust(&publisher, &public_key, request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::BundleVerify { path } => serde_json::to_value(
                self.service
                    .verify_bundle(Path::new(&path))
                    .map_err(execution)?,
            ),
            PackOperation::BundleBuild {
                source,
                destination,
                name,
                version,
                publisher,
                created_at,
                source_revision,
                signing_key_reference,
            } => serde_json::to_value(
                self.service
                    .build_bundle(
                        Path::new(&source),
                        Path::new(&destination),
                        &name,
                        &version,
                        &publisher,
                        &created_at,
                        source_revision,
                        resolve_signing_seed(&signing_key_reference).map_err(execution)?,
                    )
                    .map_err(execution)?,
            ),
            PackOperation::BundleInstall { path, prefix } => serde_json::to_value(
                self.service
                    .install_bundle(Path::new(&path), Path::new(&prefix))
                    .map_err(execution)?,
            ),
            PackOperation::BundleKeyInfo {
                signing_key_reference,
            } => serde_json::to_value(signing_key_info(
                resolve_signing_seed(&signing_key_reference).map_err(execution)?,
            )),
            PackOperation::CollectionVerify { path } => serde_json::to_value(
                self.service
                    .verify_collection(Path::new(&path))
                    .map_err(execution)?,
            ),
            PackOperation::CollectionBuild {
                source,
                destination,
                name,
                version,
                publisher,
                created_at,
                signing_key_reference,
            } => serde_json::to_value(
                self.service
                    .build_collection(
                        Path::new(&source),
                        Path::new(&destination),
                        &name,
                        &version,
                        &publisher,
                        &created_at,
                        resolve_signing_seed(&signing_key_reference).map_err(execution)?,
                    )
                    .map_err(execution)?,
            ),
            PackOperation::CollectionInstall { path } => serde_json::to_value(
                self.service
                    .install_collection(Path::new(&path), request.actor.clone())
                    .map_err(execution)?,
            ),
            PackOperation::RegistryPull {
                url,
                destination,
                credential_reference,
            } => serde_json::to_value(
                self.service
                    .registry_pull(
                        &url,
                        Path::new(&destination),
                        credential_reference.as_deref(),
                        &permit,
                    )
                    .await
                    .map_err(pack_execution)?,
            ),
            PackOperation::RegistryPush {
                path,
                url,
                credential_reference,
            } => serde_json::to_value(
                self.service
                    .registry_push(
                        Path::new(&path),
                        &url,
                        credential_reference.as_deref(),
                        &permit,
                    )
                    .await
                    .map_err(pack_execution)?,
            ),
        }
        .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&value)
                .map_err(|error| ExecutionError::Failed(error.to_string()))?,
            effect_succeeded: true,
        })
    }
}
