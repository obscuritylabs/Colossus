use super::*;
use colossus_contracts::{PluginInstallSource, PluginManagementRequest};

impl Runtime {
    /// Authoritative event journal for bounded audit views.
    pub fn journal(&self) -> Arc<dyn EventJournal> {
        Arc::clone(&self.journal)
    }

    /// Read bounded ciphertext-free evidence from the authoritative journal.
    pub fn audit_evidence(
        &self,
        from: u64,
        limit: usize,
    ) -> Result<Vec<AuditEvidence>, RuntimeError> {
        self.journal
            .read_global(from.max(1), limit.clamp(1, 10_000))
            .map(|events| events.iter().map(evidence).collect())
            .map_err(Into::into)
    }

    /// Durable audit-export consumer readiness and retry state.
    pub fn audit_export_status(&self) -> Result<AuditExportStatus, RuntimeError> {
        self.audit_exports.status().map_err(Into::into)
    }

    /// Drain configured external audit evidence work.
    pub async fn drain_audit_exports(&self) -> Result<AuditExportReport, RuntimeError> {
        self.audit_exports
            .drain(256, 16_384)
            .await
            .map_err(Into::into)
    }

    /// Operator-authorized replay of all configured audit evidence.
    pub fn reset_audit_exports(&self) -> Result<AuditExportStatus, RuntimeError> {
        self.audit_exports.reset().map_err(Into::into)
    }

    /// List recent metadata-only run telemetry.
    pub fn telemetry_runs(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RunTelemetrySummary>, RuntimeError> {
        self.telemetry
            .list_runs(session_id, limit)
            .map_err(Into::into)
    }

    /// Inspect a full or uniquely prefixed run without exposing event payloads.
    pub fn telemetry_run(
        &self,
        id_or_prefix: &str,
        limit: usize,
    ) -> Result<RunTelemetryDetail, RuntimeError> {
        self.telemetry
            .get_run(id_or_prefix, limit)
            .map_err(Into::into)
    }

    /// Aggregate metadata-only counters over recent runs.
    pub fn telemetry_metrics(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<TelemetryMetrics, RuntimeError> {
        self.telemetry
            .metrics(session_id, limit)
            .map_err(Into::into)
    }

    /// Discover the current workspace-filtered active set, or the scoped run snapshot.
    pub fn list_plugins(&self) -> Result<Vec<AgentPluginRecord>, RuntimeError> {
        Ok(self.plugin_catalog.capture()?.records.clone())
    }

    /// Live management inventory, including disabled installations and workspace exclusions.
    pub fn plugin_inventory(&self) -> Result<Vec<PluginInventoryEntry>, RuntimeError> {
        let inventory = self
            .plugin_store
            .as_ref()
            .map(|store| store.inventory())
            .transpose()?
            .unwrap_or_default();
        Ok(super::plugin_catalog::narrow_plugin_inventory(
            inventory,
            &self.plugin_configuration,
        ))
    }

    /// Return every machine-scoped plugin installation, including inactive digests.
    pub fn plugin_installations(&self) -> Result<Vec<PluginInstallation>, RuntimeError> {
        self.plugin_store
            .as_ref()
            .ok_or_else(|| {
                RuntimeError::Config("plugin lifecycle requires a Colossus home".into())
            })?
            .list(10_000)
            .map_err(Into::into)
    }

    /// Preview deterministic Agent Skill composition without executing a model turn.
    pub fn compose_plugin_skills(
        &self,
        instructions: &str,
        explicit: &[String],
        sticky: &[String],
    ) -> Result<PluginComposition, RuntimeError> {
        let catalog = self.plugin_catalog.capture()?;
        compose_plugins(
            &catalog.records,
            instructions,
            explicit,
            sticky,
            self.plugins_enabled,
        )
        .map_err(Into::into)
    }

    pub(super) async fn execute_plugin_operation(
        &self,
        operation: PluginOperation,
    ) -> Result<Value, RuntimeError> {
        self.read_plugin_as(operation, None, terminal_actor()).await
    }

    /// Execute a read-only plugin operation with authenticated caller provenance.
    /// A supplied digest pins the read; lifecycle changes never substitute new content.
    pub async fn read_plugin_as(
        &self,
        operation: PluginOperation,
        digest: Option<&str>,
        actor: Actor,
    ) -> Result<Value, RuntimeError> {
        let catalog = self.plugin_catalog.capture()?;
        if let Some(digest) = digest {
            let name = match &operation {
                PluginOperation::List => {
                    return Err(RuntimeError::Config(
                        "plugin list does not accept a digest".into(),
                    ));
                }
                PluginOperation::Inspect { plugin_name } => plugin_name.as_str(),
                PluginOperation::SkillRead { skill_id }
                | PluginOperation::ListResources { skill_id }
                | PluginOperation::ReadResource { skill_id, .. } => skill_id
                    .split_once('/')
                    .map(|(name, _)| name)
                    .ok_or_else(|| {
                        RuntimeError::Config(
                            "a qualified plugin/skill identifier is required".into(),
                        )
                    })?,
            };
            if !catalog.records.iter().any(|record| {
                record.installation.manifest.name == name && record.installation.digest == digest
            }) {
                return Err(RuntimeError::Config(
                    "selected plugin digest is unavailable; refresh the plugin inventory".into(),
                ));
            }
        }
        let mut request = effect_request(
            actor,
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![operation.action().into()];
        let released = scope_plugin_catalog(
            catalog,
            self.gateway.execute(request, self.plugin_executor.as_ref()),
        )
        .await?;
        serde_json::from_slice(&released.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Load one qualified Agent Skill through the plugin permission boundary.
    pub async fn read_plugin_skill(
        &self,
        skill_id: &str,
    ) -> Result<colossus_contracts::PluginSkillRecord, RuntimeError> {
        serde_json::from_value(
            self.execute_plugin_operation(PluginOperation::SkillRead {
                skill_id: skill_id.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// List resources for a qualified Agent Skill through the permission boundary.
    pub async fn plugin_skill_resources(
        &self,
        skill_id: &str,
    ) -> Result<Vec<PluginResourceEntry>, RuntimeError> {
        serde_json::from_value(
            self.execute_plugin_operation(PluginOperation::ListResources {
                skill_id: skill_id.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Read one bounded text resource for a qualified Agent Skill through policy.
    pub async fn read_plugin_resource(
        &self,
        skill_id: &str,
        path: &str,
    ) -> Result<PluginResourceRead, RuntimeError> {
        serde_json::from_value(
            self.execute_plugin_operation(PluginOperation::ReadResource {
                skill_id: skill_id.into(),
                path: path.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Install one local Agent Plugin directory globally, initially disabled.
    pub async fn install_plugin_directory(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<PluginInstallation, RuntimeError> {
        return self
            .install_plugin_directory_with_trust(path, "default")
            .await;
    }

    /// Install a local directory under an explicit configured trust profile.
    pub async fn install_plugin_directory_with_trust(
        &self,
        path: impl AsRef<Path>,
        trust_profile: &str,
    ) -> Result<PluginInstallation, RuntimeError> {
        let value = self
            .manage_plugin(PluginManagementRequest::Install {
                source: PluginInstallSource::Directory {
                    path: path.as_ref().display().to_string(),
                },
                trust_profile: trust_profile.into(),
            })
            .await?;
        serde_json::from_value(value).map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Install one verified OCI layout candidate globally, initially disabled.
    pub async fn install_plugin_layout(
        &self,
        path: impl AsRef<Path>,
        digest: Option<&str>,
    ) -> Result<PluginInstallation, RuntimeError> {
        return self
            .install_plugin_layout_with_trust(path, digest, "default")
            .await;
    }

    /// Trust-verify and install one OCI layout candidate as disabled.
    pub async fn install_plugin_layout_with_trust(
        &self,
        path: impl AsRef<Path>,
        digest: Option<&str>,
        trust_profile: &str,
    ) -> Result<PluginInstallation, RuntimeError> {
        let value = self
            .manage_plugin(PluginManagementRequest::Install {
                source: PluginInstallSource::Layout {
                    path: path.as_ref().display().to_string(),
                    digest: digest.map(str::to_owned),
                },
                trust_profile: trust_profile.into(),
            })
            .await?;
        serde_json::from_value(value).map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Install one deterministic OCI layout tar globally, initially disabled.
    pub async fn install_plugin_archive(
        &self,
        path: impl AsRef<Path>,
        digest: Option<&str>,
    ) -> Result<PluginInstallation, RuntimeError> {
        return self
            .install_plugin_archive_with_trust(path, digest, "default")
            .await;
    }

    /// Trust-verify and install one OCI layout tar as disabled.
    pub async fn install_plugin_archive_with_trust(
        &self,
        path: impl AsRef<Path>,
        digest: Option<&str>,
        trust_profile: &str,
    ) -> Result<PluginInstallation, RuntimeError> {
        let value = self
            .manage_plugin(PluginManagementRequest::Install {
                source: PluginInstallSource::Archive {
                    path: path.as_ref().display().to_string(),
                    digest: digest.map(str::to_owned),
                },
                trust_profile: trust_profile.into(),
            })
            .await?;
        serde_json::from_value(value).map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Pull a plugin and its OCI 1.1 referrers into a fresh local OCI layout.
    pub async fn pull_plugin(
        &self,
        registry: &str,
        reference: &str,
        output: impl AsRef<Path>,
    ) -> Result<PluginRegistryTransfer, RuntimeError> {
        let profile = self.plugin_registry_profile(registry)?.clone();
        let credential = self.resolve_plugin_registry_credential(&profile).await?;
        let operation = PluginRegistryOperation::Pull {
            reference: reference.into(),
            output: workspace_absolute_path(&self.workspace, output.as_ref())
                .display()
                .to_string(),
        };
        self.execute_plugin_registry_operation(profile, credential, operation)
            .await
    }

    /// Push a plugin OCI layout and its retained referrers to an exact registry profile.
    pub async fn push_plugin(
        &self,
        registry: &str,
        layout: impl AsRef<Path>,
        reference: &str,
    ) -> Result<PluginRegistryTransfer, RuntimeError> {
        let profile = self.plugin_registry_profile(registry)?.clone();
        let credential = self.resolve_plugin_registry_credential(&profile).await?;
        let operation = PluginRegistryOperation::Push {
            layout: workspace_absolute_path(&self.workspace, layout.as_ref())
                .display()
                .to_string(),
            reference: reference.into(),
        };
        self.execute_plugin_registry_operation(profile, credential, operation)
            .await
    }

    /// Pull, trust-verify, and install one registry reference as disabled.
    pub async fn install_plugin_reference(
        &self,
        registry: &str,
        reference: &str,
        expected_name: Option<&str>,
    ) -> Result<PluginInstallation, RuntimeError> {
        if expected_name == Some("colossus") {
            return Err(RuntimeError::Config(
                "colossus is bundled with Colossus; update the executable to change its version"
                    .into(),
            ));
        }
        let profile = self.plugin_registry_profile(registry)?.clone();
        let trust_profile_name = profile.trust_profile.clone();
        let temporary = tempfile::Builder::new()
            .prefix(".plugin-pull-")
            .tempdir_in(&self.workspace)?;
        let layout = temporary.path().join("layout");
        self.pull_plugin(registry, reference, &layout).await?;
        let artifact = colossus_plugins::verify_plugin_layout(&layout, None)?;
        let config: colossus_contracts::AgentPluginOciConfig =
            serde_json::from_slice(&artifact.config)
                .map_err(|error| RuntimeError::Config(error.to_string()))?;
        if let Some(expected) = expected_name
            && expected != config.name
        {
            return Err(RuntimeError::Config(format!(
                "registry update resolved plugin {}, expected {expected}",
                config.name
            )));
        }
        self.install_plugin_layout_with_trust(
            &layout,
            Some(&artifact.manifest_digest),
            &trust_profile_name,
        )
        .await
    }

    /// Export the active plugin plus retained signature/referrer material for air gaps.
    pub async fn export_plugin(
        &self,
        name: &str,
        output: impl AsRef<Path>,
    ) -> Result<String, RuntimeError> {
        let value = self
            .manage_plugin(PluginManagementRequest::Export {
                name: name.into(),
                output: output.as_ref().display().to_string(),
            })
            .await?;
        serde_json::from_value(value["digest"].clone())
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    pub(super) fn plugin_registry_profile(
        &self,
        name: &str,
    ) -> Result<&PluginRegistryProfile, RuntimeError> {
        self.plugin_configuration
            .registries
            .get(name)
            .ok_or_else(|| {
                RuntimeError::Config(format!("plugin registry profile not found: {name}"))
            })
    }

    async fn execute_plugin_registry_operation(
        &self,
        profile: PluginRegistryProfile,
        helper_credential: Option<RegistryCredential>,
        operation: PluginRegistryOperation,
    ) -> Result<PluginRegistryTransfer, RuntimeError> {
        let mut request = effect_request(
            terminal_actor(),
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![operation.action().into()];
        let executor = PluginRegistryEffectExecutor::new(
            profile,
            Arc::clone(&self.plugin_credentials),
            helper_credential,
        );
        let released = self.gateway.execute(request, &executor).await?;
        serde_json::from_slice(&released.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    async fn resolve_plugin_registry_credential(
        &self,
        profile: &PluginRegistryProfile,
    ) -> Result<Option<RegistryCredential>, RuntimeError> {
        match docker_credential_helper(profile)? {
            None => Ok(None),
            Some((executable, server)) => {
                let executable = if self.sandbox_backend == "oci" {
                    executable
                } else {
                    fs::canonicalize(executable)?
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
                    terminal_actor(),
                    action,
                    executable.display().to_string(),
                    serde_json::to_value(spec)
                        .map_err(|error| RuntimeError::Config(error.to_string()))?,
                );
                request.capabilities = vec![action.into()];
                let executor = Arc::new(DockerCredentialHelperExecutor::new(Arc::clone(
                    &self.process_executor,
                )));
                let released = self.gateway.execute(request, executor.as_ref()).await?;
                let value: Value = serde_json::from_slice(&released.bytes)
                    .map_err(|error| RuntimeError::Config(error.to_string()))?;
                let handle = value.get("handle").and_then(Value::as_str).ok_or_else(|| {
                    RuntimeError::Config("credential helper returned no opaque handle".into())
                })?;
                executor.take(handle).map(Some)
            }
        }
    }

    /// Select one exact installed manifest digest as globally active.
    pub async fn enable_plugin(
        &self,
        name: &str,
        digest: &str,
        allow_untrusted: bool,
    ) -> Result<PluginInstallation, RuntimeError> {
        let value = self
            .manage_plugin(PluginManagementRequest::Enable {
                name: name.into(),
                digest: digest.into(),
                allow_untrusted,
            })
            .await?;
        serde_json::from_value(value).map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Disable one globally active plugin name.
    pub async fn disable_plugin(&self, name: &str) -> Result<(), RuntimeError> {
        let value = self
            .manage_plugin(PluginManagementRequest::Disable { name: name.into() })
            .await?;
        let _ = value;
        Ok(())
    }

    /// Uninstall one exact plugin digest, preserving data unless explicitly purged.
    pub async fn uninstall_plugin(
        &self,
        name: &str,
        digest: &str,
        purge_data: bool,
    ) -> Result<PluginInstallation, RuntimeError> {
        let value = self
            .manage_plugin(PluginManagementRequest::Uninstall {
                name: name.into(),
                digest: digest.into(),
                purge_data,
            })
            .await?;
        serde_json::from_value(value).map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Remove inactive, unreferenced plugin content.
    pub async fn gc_plugins(&self) -> Result<Vec<String>, RuntimeError> {
        let value = self.manage_plugin(PluginManagementRequest::Gc).await?;
        serde_json::from_value(value).map_err(|error| RuntimeError::Config(error.to_string()))
    }
}
