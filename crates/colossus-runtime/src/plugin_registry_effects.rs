use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum PluginRegistryOperation {
    Pull { reference: String, output: String },
    Push { layout: String, reference: String },
}

impl PluginRegistryOperation {
    pub(super) fn action(&self) -> &'static str {
        match self {
            Self::Pull { .. } => "plugin.pull",
            Self::Push { .. } => "plugin.push",
        }
    }
}

pub(super) struct PluginRegistryEffectExecutor {
    profile: PluginRegistryProfile,
    credentials: Arc<dyn CredentialResolver>,
    gateway: Arc<EffectGateway>,
    process: Arc<dyn EffectExecutor>,
    workspace: PathBuf,
    oci: bool,
}

impl PluginRegistryEffectExecutor {
    pub(super) fn new(
        profile: PluginRegistryProfile,
        credentials: Arc<dyn CredentialResolver>,
        gateway: Arc<EffectGateway>,
        process: Arc<dyn EffectExecutor>,
        workspace: PathBuf,
        oci: bool,
    ) -> Self {
        Self {
            profile,
            credentials,
            gateway,
            process,
            workspace,
            oci,
        }
    }

    async fn credential(
        &self,
        parent: &EffectRequest,
    ) -> Result<RegistryCredential, ExecutionError> {
        // This method is entered only after the transfer permit and all file grants
        // have been checked. Parse Docker configuration exactly once inside that effect.
        match resolve_registry_credential_source(&self.profile, self.credentials.as_ref())
            .map_err(failed)?
        {
            RegistryCredentialResolution::Ready(credential) => Ok(credential),
            RegistryCredentialResolution::DockerHelper { executable, server } => {
                let executable = if self.oci {
                    executable
                } else {
                    fs::canonicalize(executable).map_err(failed)?
                };
                let spec = ProcessSpec {
                    cwd: self.workspace.clone(),
                    args: vec!["get".into()],
                    environment: BTreeMap::new(),
                    stdin_base64: Some(BASE64.encode(format!("{server}\n"))),
                    stdin_completion: None,
                    timeout_ms: None,
                    max_output_bytes: None,
                };
                let action = "plugin.registry.credential_helper";
                let mut request = effect_request(
                    parent.actor.clone(),
                    action,
                    executable.display().to_string(),
                    serde_json::to_value(spec).map_err(failed)?,
                );
                request.context = parent.context.clone();
                request.capabilities = vec![action.into()];
                let executor = DockerCredentialHelperExecutor::new(Arc::clone(&self.process));
                let released = self
                    .gateway
                    .execute(request, &executor)
                    .await
                    .map_err(failed)?;
                let value: Value = serde_json::from_slice(&released.bytes).map_err(failed)?;
                let handle = value
                    .get("handle")
                    .and_then(Value::as_str)
                    .ok_or_else(|| failed("credential helper returned no opaque handle"))?;
                executor.take(handle).map_err(failed)
            }
        }
    }
}

#[async_trait]
impl EffectExecutor for PluginRegistryEffectExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: PluginRegistryOperation =
            serde_json::from_value(request.content.clone()).map_err(failed)?;
        if request.action != operation.action() || request.resource != self.profile.origin {
            return Err(failed(
                "plugin registry request does not match its authorized effect",
            ));
        }
        enforce_registry_network(&self.profile, &permit)?;
        enforce_registry_filesystem(&operation, &permit)?;
        enforce_registry_ca_files(&self.profile, &permit)?;
        if let RegistryAuthConfig::Docker { config_path, .. } = &self.profile.auth {
            let path =
                colossus_plugins::docker_config_path(config_path.as_deref()).map_err(failed)?;
            plugin_management::enforce_management_path(&path, false, &permit)?;
        }
        let credential = self.credential(request).await?;
        let client = PluginRegistryClient::new(self.profile.clone(), credential).map_err(failed)?;
        let transfer = match operation {
            PluginRegistryOperation::Pull { reference, output } => client
                .pull(&reference, Path::new(&output))
                .await
                .map_err(failed)?,
            PluginRegistryOperation::Push { layout, reference } => client
                .push(Path::new(&layout), &reference)
                .await
                .map_err(failed)?,
        };
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&transfer).map_err(failed)?,
            effect_succeeded: true,
        })
    }
}

fn enforce_registry_ca_files(
    profile: &PluginRegistryProfile,
    permit: &ExecutionPermit,
) -> Result<(), ExecutionError> {
    if permit.obligations().resource_authority == ResourceAuthority::Ambient {
        return Ok(());
    }
    for path in profile
        .ca_bundle_path
        .iter()
        .chain(profile.token_ca_bundle_paths.values())
        .chain(profile.blob_redirect_ca_bundle_paths.values())
    {
        let canonical = fs::canonicalize(path).map_err(failed)?;
        if !permit.obligations().filesystem.iter().any(|grant| {
            matches!(grant.mode.as_str(), "read" | "write")
                && fs::canonicalize(&grant.root).is_ok_and(|root| canonical == root)
        }) {
            return Err(failed(format!(
                "plugin registry CA file is outside authorized roots: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn enforce_registry_network(
    profile: &PluginRegistryProfile,
    permit: &ExecutionPermit,
) -> Result<(), ExecutionError> {
    if permit.obligations().resource_authority == ResourceAuthority::Ambient {
        return Ok(());
    }
    let destinations = &permit.obligations().network_destinations;
    for origin in std::iter::once(&profile.origin)
        .chain(profile.token_origins.iter())
        .chain(profile.blob_redirect_origins.iter())
    {
        if !matches!(
            network_destination_match(destinations, origin).map_err(failed)?,
            Some(colossus_policy::NetworkDestinationMatch::Exact)
        ) {
            return Err(failed(format!(
                "plugin registry origin is outside the authorized exact destinations: {origin}"
            )));
        }
    }
    Ok(())
}

fn enforce_registry_filesystem(
    operation: &PluginRegistryOperation,
    permit: &ExecutionPermit,
) -> Result<(), ExecutionError> {
    if permit.obligations().resource_authority == ResourceAuthority::Ambient {
        return Ok(());
    }
    let (path, mode) = match operation {
        PluginRegistryOperation::Pull { output, .. } => (Path::new(output), "write"),
        PluginRegistryOperation::Push { layout, .. } => (Path::new(layout), "read"),
    };
    let canonical = if path.exists() {
        fs::canonicalize(path).map_err(failed)?
    } else {
        let mut ancestor = path;
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .ok_or_else(|| failed("plugin registry output has no existing ancestor"))?;
        }
        fs::canonicalize(ancestor).map_err(failed)?
    };
    let allowed = permit.obligations().filesystem.iter().any(|grant| {
        (grant.mode == mode || (mode == "read" && grant.mode == "write"))
            && fs::canonicalize(&grant.root).is_ok_and(|root| canonical.starts_with(root))
    });
    if allowed {
        Ok(())
    } else {
        Err(failed(format!(
            "plugin registry {mode} path is outside authorized roots: {}",
            path.display()
        )))
    }
}

pub(super) struct DockerCredentialHelperExecutor {
    process: Arc<dyn EffectExecutor>,
    credentials: StdMutex<BTreeMap<String, RegistryCredential>>,
}

impl DockerCredentialHelperExecutor {
    pub(super) fn new(process: Arc<dyn EffectExecutor>) -> Self {
        Self {
            process,
            credentials: StdMutex::new(BTreeMap::new()),
        }
    }

    pub(super) fn take(&self, handle: &str) -> Result<RegistryCredential, RuntimeError> {
        self.credentials
            .lock()
            .map_err(|_| RuntimeError::Config("credential helper vault lock is poisoned".into()))?
            .remove(handle)
            .ok_or_else(|| RuntimeError::Config("credential helper handle is invalid".into()))
    }
}

#[async_trait]
impl EffectExecutor for DockerCredentialHelperExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let result = self.process.execute(request, permit).await?;
        let value: Value = serde_json::from_slice(&result.bytes).map_err(failed)?;
        if value.get("success").and_then(Value::as_bool) != Some(true)
            || value.get("exit_code").and_then(Value::as_i64) != Some(0)
            || value.get("output_truncated").and_then(Value::as_bool) != Some(false)
        {
            return Err(failed("Docker credential helper failed"));
        }
        let stdout = value
            .get("stdout_base64")
            .and_then(Value::as_str)
            .ok_or_else(|| failed("Docker credential helper returned no output"))?;
        let stdout = BASE64
            .decode(stdout)
            .map_err(|_| failed("Docker credential helper output was not valid base64"))?;
        let credential = registry_credential_from_helper_output(&stdout).map_err(failed)?;
        let handle = uuid::Uuid::now_v7().to_string();
        let mut credentials = self
            .credentials
            .lock()
            .map_err(|_| failed("credential helper vault lock is poisoned"))?;
        if credentials.len() >= 8 {
            return Err(failed("credential helper vault capacity is exhausted"));
        }
        credentials.insert(handle.clone(), credential);
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&json!({"handle": handle})).map_err(failed)?,
            effect_succeeded: true,
        })
    }
}

fn failed(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}
