use super::*;

/// Permit-bearing adapter for release-bundle effects.
pub struct BundleExecutor {
    service: Arc<BundleService>,
}

impl BundleExecutor {
    /// Construct a gateway executor for release-bundle operations.
    #[must_use]
    pub fn new(service: Arc<BundleService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl EffectExecutor for BundleExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: BundleOperation = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        if request.action != operation.action() || request.resource != operation.resource() {
            return Err(ExecutionError::Failed(
                "bundle request does not match its authorized action and resource".into(),
            ));
        }
        if let Some(path) = source_path(&operation) {
            enforce_read(path, &permit)?;
        }
        if let Some(path) = destination_path(&operation) {
            enforce_write(path, &permit)?;
        }
        let value = match operation {
            BundleOperation::Verify { path } => {
                serde_json::to_value(self.service.verify(Path::new(&path)).map_err(failed)?)
            }
            BundleOperation::Build {
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
                    .build(
                        Path::new(&source),
                        Path::new(&destination),
                        &name,
                        &version,
                        &publisher,
                        &created_at,
                        source_revision,
                        resolve_signing_seed(&signing_key_reference).map_err(failed)?,
                    )
                    .map_err(failed)?,
            ),
            BundleOperation::Install { path, prefix } => serde_json::to_value(
                self.service
                    .install(Path::new(&path), Path::new(&prefix))
                    .map_err(failed)?,
            ),
            BundleOperation::KeyInfo {
                signing_key_reference,
            } => serde_json::to_value(bundle_signing_key_info(
                resolve_signing_seed(&signing_key_reference).map_err(failed)?,
            )),
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

fn source_path(operation: &BundleOperation) -> Option<&Path> {
    match operation {
        BundleOperation::Verify { path } | BundleOperation::Install { path, .. } => {
            Some(Path::new(path))
        }
        BundleOperation::Build { source, .. } => Some(Path::new(source)),
        BundleOperation::KeyInfo { .. } => None,
    }
}

fn destination_path(operation: &BundleOperation) -> Option<&Path> {
    match operation {
        BundleOperation::Build { destination, .. } => Some(Path::new(destination)),
        BundleOperation::Install { prefix, .. } => Some(Path::new(prefix)),
        BundleOperation::Verify { .. } | BundleOperation::KeyInfo { .. } => None,
    }
}

fn enforce_read(path: &Path, permit: &ExecutionPermit) -> Result<(), ExecutionError> {
    let canonical = fs::canonicalize(path).map_err(failed)?;
    if permit.obligations().resource_authority == ResourceAuthority::Ambient
        || permit.obligations().filesystem.iter().any(|grant| {
            matches!(grant.mode.as_str(), "read" | "write")
                && fs::canonicalize(&grant.root).is_ok_and(|root| canonical.starts_with(root))
        })
    {
        Ok(())
    } else {
        Err(ExecutionError::Failed(format!(
            "bundle source {} is outside authorized roots",
            canonical.display()
        )))
    }
}

fn enforce_write(path: &Path, permit: &ExecutionPermit) -> Result<(), ExecutionError> {
    let mut existing = path;
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or_else(|| ExecutionError::Failed("bundle destination has no parent".into()))?;
    }
    let canonical = fs::canonicalize(existing).map_err(failed)?;
    if permit.obligations().resource_authority == ResourceAuthority::Ambient
        || permit.obligations().filesystem.iter().any(|grant| {
            grant.mode == "write"
                && fs::canonicalize(&grant.root).is_ok_and(|root| canonical.starts_with(root))
        })
    {
        Ok(())
    } else {
        Err(ExecutionError::Failed(format!(
            "bundle destination {} is outside authorized roots",
            path.display()
        )))
    }
}

fn resolve_signing_seed(reference: &str) -> Result<[u8; 32], BundleError> {
    let variable = reference.strip_prefix("env:").ok_or_else(|| {
        BundleError::Invalid("bundle signing keys require env:VARIABLE references".into())
    })?;
    if variable.is_empty()
        || !variable.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
    {
        return Err(BundleError::Invalid(
            "bundle signing keys require env:VARIABLE references".into(),
        ));
    }
    let encoded = std::env::var(variable).map_err(|_| {
        BundleError::Invalid(format!("bundle credential {variable} is unavailable"))
    })?;
    let bytes = hex::decode(&encoded)
        .or_else(|_| BASE64.decode(&encoded))
        .map_err(|_| BundleError::Invalid("bundle signing seed is not hex or base64".into()))?;
    bytes
        .try_into()
        .map_err(|_| BundleError::Invalid("bundle signing seed must contain 32 bytes".into()))
}

fn failed(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}
