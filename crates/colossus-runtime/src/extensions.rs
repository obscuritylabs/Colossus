use super::*;

pub(super) fn extension_path(workspace: &Path, path: &Path) -> String {
    workspace_absolute_path(workspace, path)
        .display()
        .to_string()
}

impl Runtime {
    pub(super) async fn execute_integration_operation(
        &self,
        operation: IntegrationRequest,
    ) -> Result<Value, RuntimeError> {
        let mut request = effect_request(
            terminal_actor(),
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![operation.action().into()];
        let references = match &operation {
            IntegrationRequest::ImportOpenApi {
                credential_reference,
                ..
            } => credential_reference.iter().cloned().collect::<Vec<_>>(),
            IntegrationRequest::ConnectNative {
                credential_reference,
                ..
            } => credential_reference.iter().cloned().collect(),
            _ => Vec::new(),
        };
        request.credential_references = references
            .into_iter()
            .map(|reference| colossus_contracts::CredentialReference {
                reference,
                value_hash: None,
            })
            .collect();
        let released = self
            .gateway
            .execute(request, self.integration_effect_executor.as_ref())
            .await?;
        serde_json::from_slice(&released.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// List safe persisted integration summaries.
    pub fn list_integrations(&self, limit: usize) -> Result<Vec<IntegrationSummary>, RuntimeError> {
        self.integration_executor
            .summaries(limit)
            .map_err(Into::into)
    }

    /// Reconstruct one persisted integration connection without resolving credentials.
    pub fn get_integration(
        &self,
        name: &str,
    ) -> Result<Option<IntegrationConnection>, RuntimeError> {
        self.integration_executor
            .get_connection(name)
            .map_err(Into::into)
    }

    /// Canonical extension repository for embedded application surfaces.
    pub fn integration_repository(&self) -> Arc<dyn IntegrationRepository> {
        Arc::clone(&self.integrations)
    }

    pub(super) async fn execute_bundle_operation(
        &self,
        operation: BundleOperation,
    ) -> Result<Value, RuntimeError> {
        let mut request = effect_request(
            terminal_actor(),
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![operation.action().into()];
        if let BundleOperation::Build {
            signing_key_reference,
            ..
        }
        | BundleOperation::KeyInfo {
            signing_key_reference,
        } = &operation
        {
            request.credential_references = vec![CredentialReference {
                reference: signing_key_reference.clone(),
                value_hash: None,
            }];
        }
        let released = self
            .gateway
            .execute(request, self.bundle_executor.as_ref())
            .await?;
        serde_json::from_slice(&released.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Verify a signed offline release bundle through policy and post-effect release.
    pub async fn verify_bundle(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<colossus_contracts::BundleVerification, RuntimeError> {
        let path = extension_path(&self.workspace, path.as_ref());
        serde_json::from_value(
            self.execute_bundle_operation(BundleOperation::Verify { path })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Build and sign a deterministic offline release bundle through policy.
    #[allow(clippy::too_many_arguments)]
    pub async fn build_bundle(
        &self,
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        name: &str,
        version: &str,
        publisher: &str,
        created_at: &str,
        source_revision: Option<&str>,
        signing_key_reference: &str,
    ) -> Result<BundleMaterialization, RuntimeError> {
        let source = extension_path(&self.workspace, source.as_ref());
        let destination = extension_path(&self.workspace, destination.as_ref());
        serde_json::from_value(
            self.execute_bundle_operation(BundleOperation::Build {
                source,
                destination,
                name: name.into(),
                version: version.into(),
                publisher: publisher.into(),
                created_at: created_at.into(),
                source_revision: source_revision.map(Into::into),
                signing_key_reference: signing_key_reference.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Verify and install the current-target native executable into a clean prefix.
    pub async fn install_bundle(
        &self,
        path: impl AsRef<Path>,
        prefix: impl AsRef<Path>,
    ) -> Result<BundleInstallation, RuntimeError> {
        let path = extension_path(&self.workspace, path.as_ref());
        let prefix = extension_path(&self.workspace, prefix.as_ref());
        serde_json::from_value(
            self.execute_bundle_operation(BundleOperation::Install { path, prefix })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Derive the public identity of a referenced bundle signing seed through policy.
    pub async fn bundle_signing_key_info(
        &self,
        signing_key_reference: &str,
    ) -> Result<BundleSigningKeyInfo, RuntimeError> {
        serde_json::from_value(
            self.execute_bundle_operation(BundleOperation::KeyInfo {
                signing_key_reference: signing_key_reference.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Compile and persist a JSON OpenAPI connection through policy and approval.
    pub async fn import_openapi_integration(
        &self,
        name: &str,
        document: Value,
        base_url: Option<&str>,
        auth: IntegrationAuth,
        credential_reference: Option<&str>,
        scopes: &[String],
    ) -> Result<IntegrationConnection, RuntimeError> {
        serde_json::from_value(
            self.execute_integration_operation(IntegrationRequest::ImportOpenApi {
                name: name.into(),
                document,
                base_url: base_url.map(Into::into),
                auth,
                credential_reference: credential_reference.map(Into::into),
                scopes: scopes.to_vec(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Connect one first-party native integration through policy and approval.
    #[allow(clippy::too_many_arguments)]
    pub async fn connect_native_integration(
        &self,
        name: &str,
        base_url: Option<&str>,
        auth: IntegrationAuth,
        credential_reference: Option<&str>,
        scopes: &[String],
    ) -> Result<IntegrationConnection, RuntimeError> {
        serde_json::from_value(
            self.execute_integration_operation(IntegrationRequest::ConnectNative {
                name: name.into(),
                base_url: base_url.map(Into::into),
                auth,
                credential_reference: credential_reference.map(Into::into),
                scopes: scopes.to_vec(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Disconnect a persisted integration through policy and approval.
    pub async fn disconnect_integration(
        &self,
        name: &str,
    ) -> Result<IntegrationConnection, RuntimeError> {
        serde_json::from_value(
            self.execute_integration_operation(IntegrationRequest::Disconnect {
                name: name.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Invoke one connected dynamic integration tool from an application/terminal caller.
    pub async fn call_integration_tool(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, RuntimeError> {
        let (operation, credentials) = self
            .integration_executor
            .invocation(tool_name, arguments)?
            .ok_or_else(|| {
                RuntimeError::Config(format!("integration tool not found: {tool_name}"))
            })?;
        let mut request = effect_request(
            terminal_actor(),
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec!["integration.invoke".into()];
        request.credential_references = credentials;
        let released = self
            .gateway
            .execute(request, self.integration_effect_executor.as_ref())
            .await?;
        serde_json::from_slice(&released.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }
}
