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
    pub fn extension_repository(&self) -> Arc<dyn ExtensionRepository> {
        Arc::clone(&self.extensions)
    }

    pub(super) async fn execute_pack_operation(
        &self,
        operation: PackOperation,
    ) -> Result<Value, RuntimeError> {
        let mut request = effect_request(
            terminal_actor(),
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![operation.action().into()];
        if let PackOperation::BundleBuild {
            signing_key_reference,
            ..
        }
        | PackOperation::CollectionBuild {
            signing_key_reference,
            ..
        }
        | PackOperation::BundleKeyInfo {
            signing_key_reference,
        } = &operation
        {
            request.credential_references = vec![CredentialReference {
                reference: signing_key_reference.clone(),
                value_hash: None,
            }];
        }
        if let PackOperation::RegistryPull {
            credential_reference,
            ..
        }
        | PackOperation::RegistryPush {
            credential_reference,
            ..
        } = &operation
        {
            request.credential_references = credential_reference
                .iter()
                .map(|reference| CredentialReference {
                    reference: reference.clone(),
                    value_hash: None,
                })
                .collect();
        }
        let released = self
            .gateway
            .execute(request, self.pack_executor.as_ref())
            .await?;
        serde_json::from_slice(&released.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// List canonical capability-pack lifecycles.
    pub fn list_packs(&self, limit: usize) -> Result<Vec<PackInstallation>, RuntimeError> {
        self.packs.list(limit).map_err(Into::into)
    }

    /// Reconstruct one canonical capability-pack lifecycle.
    pub fn get_pack(&self, name: &str) -> Result<Option<PackInstallation>, RuntimeError> {
        self.packs.get(name).map_err(Into::into)
    }

    /// Verify a local capability pack through policy and post-effect release.
    pub async fn verify_pack(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<PackVerification, RuntimeError> {
        let path = extension_path(&self.workspace, path.as_ref());
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::Verify { path })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Install a verified pack through approval, audit, and one-use permit enforcement.
    pub async fn install_pack(
        &self,
        path: impl AsRef<Path>,
        allow_untrusted: bool,
    ) -> Result<PackInstallation, RuntimeError> {
        let path = extension_path(&self.workspace, path.as_ref());
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::Install {
                path,
                allow_untrusted,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Reverify and enable one installed pack through approval and audit.
    pub async fn enable_pack(&self, name: &str) -> Result<PackInstallation, RuntimeError> {
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::Enable { name: name.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Disable one installed pack through approval and audit.
    pub async fn disable_pack(&self, name: &str) -> Result<PackInstallation, RuntimeError> {
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::Disable { name: name.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Uninstall one pack through approval and audit while retaining lifecycle history.
    pub async fn uninstall_pack(&self, name: &str) -> Result<PackInstallation, RuntimeError> {
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::Uninstall { name: name.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Add one publisher/key trust binding through approval and audit.
    pub async fn add_pack_trust(
        &self,
        publisher: &str,
        public_key: &str,
    ) -> Result<PublisherTrust, RuntimeError> {
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::TrustAdd {
                publisher: publisher.into(),
                public_key: public_key.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// List canonical publisher/key trust bindings.
    pub fn list_pack_trust(&self, limit: usize) -> Result<Vec<PublisherTrust>, RuntimeError> {
        self.packs.list_trust(limit).map_err(Into::into)
    }

    /// Verify a signed multi-pack and skill collection through policy.
    pub async fn verify_collection(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<CollectionVerification, RuntimeError> {
        let path = extension_path(&self.workspace, path.as_ref());
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::CollectionVerify { path })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Build and sign a deterministic offline collection through policy.
    #[allow(clippy::too_many_arguments)]
    pub async fn build_collection(
        &self,
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        name: &str,
        version: &str,
        publisher: &str,
        created_at: &str,
        signing_key_reference: &str,
    ) -> Result<CollectionMaterialization, RuntimeError> {
        let source = extension_path(&self.workspace, source.as_ref());
        let destination = extension_path(&self.workspace, destination.as_ref());
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::CollectionBuild {
                source,
                destination,
                name: name.into(),
                version: version.into(),
                publisher: publisher.into(),
                created_at: created_at.into(),
                signing_key_reference: signing_key_reference.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Install every trusted collection artifact without replacing existing bytes.
    pub async fn install_collection(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<CollectionInstallation, RuntimeError> {
        let path = extension_path(&self.workspace, path.as_ref());
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::CollectionInstall { path })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Pull one authenticated collection transport into a clean local directory.
    pub async fn pull_registry_collection(
        &self,
        url: &str,
        destination: impl AsRef<Path>,
        credential_reference: Option<&str>,
    ) -> Result<RegistryPullResult, RuntimeError> {
        let destination = extension_path(&self.workspace, destination.as_ref());
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::RegistryPull {
                url: url.into(),
                destination,
                credential_reference: credential_reference.map(str::to_owned),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Push one verified collection through a create-only authenticated registry request.
    pub async fn push_registry_collection(
        &self,
        path: impl AsRef<Path>,
        url: &str,
        credential_reference: Option<&str>,
    ) -> Result<RegistryPushResult, RuntimeError> {
        let path = extension_path(&self.workspace, path.as_ref());
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::RegistryPush {
                path,
                url: url.into(),
                credential_reference: credential_reference.map(str::to_owned),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Invoke one active verified pack tool through approval, sandboxing, and audit.
    pub async fn call_pack_tool(&self, tool: &str) -> Result<Value, RuntimeError> {
        let (declaration, input) = self
            .pack_process_executor
            .invocation(tool)
            .ok_or_else(|| RuntimeError::Config(format!("active pack tool not found: {tool}")))?;
        let mut request = effect_request(
            terminal_actor(),
            &declaration.action,
            declaration.executable.display().to_string(),
            serde_json::to_value(input).map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![declaration.action];
        request.credential_references = declaration
            .environment
            .values()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|reference| CredentialReference {
                reference,
                value_hash: None,
            })
            .collect();
        let released = self
            .gateway
            .execute(request, self.pack_process_effect_executor.as_ref())
            .await?;
        let process: Value = serde_json::from_slice(&released.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))?;
        let decode = |field: &str| -> Result<String, RuntimeError> {
            let encoded = process
                .get(field)
                .and_then(Value::as_str)
                .ok_or_else(|| RuntimeError::Config(format!("pack output lacks {field}")))?;
            let bytes = BASE64
                .decode(encoded)
                .map_err(|error| RuntimeError::Config(error.to_string()))?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        };
        Ok(json!({
            "pack": declaration.pack,
            "tool": declaration.tool,
            "stdout": decode("stdout_base64")?,
            "stderr": decode("stderr_base64")?,
            "exit_code": process.get("exit_code").and_then(Value::as_i64),
            "truncated": process.get("truncated").and_then(Value::as_bool).unwrap_or(false),
        }))
    }

    /// Verify a signed offline release bundle through policy and post-effect release.
    pub async fn verify_bundle(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<colossus_contracts::BundleVerification, RuntimeError> {
        let path = extension_path(&self.workspace, path.as_ref());
        serde_json::from_value(
            self.execute_pack_operation(PackOperation::BundleVerify { path })
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
            self.execute_pack_operation(PackOperation::BundleBuild {
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
            self.execute_pack_operation(PackOperation::BundleInstall { path, prefix })
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
            self.execute_pack_operation(PackOperation::BundleKeyInfo {
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
