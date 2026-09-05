//! Operator plugin lifecycle through the normal effect, approval, and audit boundary.

use super::*;
use colossus_contracts::{PluginInstallSource, PluginManagementRequest, PluginOrigin};

struct PluginManagementExecutor {
    store: Option<Arc<PluginStore>>,
    configuration: Arc<PluginsConfig>,
}

impl PluginManagementExecutor {
    fn store(&self) -> Result<&PluginStore, ExecutionError> {
        self.store
            .as_deref()
            .ok_or_else(|| failed("plugin lifecycle requires an explicit Colossus home"))
    }

    fn profile(&self, name: &str) -> Result<&PluginTrustProfile, ExecutionError> {
        self.configuration
            .trust_profiles
            .get(name)
            .ok_or_else(|| failed(format!("plugin trust profile not found: {name}")))
    }

    fn execute_local(
        &self,
        operation: PluginManagementRequest,
        actor: Actor,
        request: &EffectRequest,
    ) -> Result<Value, ExecutionError> {
        use PluginManagementRequest as Op;
        let value = match operation {
            Op::Inventory => serde_json::to_value(super::plugin_catalog::narrow_plugin_inventory(
                self.store()?.inventory().map_err(failed)?,
                &self.configuration,
            )),
            Op::Show { name } => serde_json::to_value(
                super::plugin_catalog::narrow_plugin_inventory(
                    self.store()?.inventory().map_err(failed)?,
                    &self.configuration,
                )
                .into_iter()
                .filter(|plugin| plugin.manifest.name == name)
                .collect::<Vec<_>>(),
            ),
            Op::SkillRead { skill_id, digest }
            | Op::ResourceList { skill_id, digest }
            | Op::ResourceRead {
                skill_id, digest, ..
            } => {
                let name = skill_id
                    .split_once('/')
                    .map(|(name, _)| name)
                    .ok_or_else(|| failed("select a qualified plugin/skill identifier"))?;
                let (plugins, _lease) = self
                    .store()?
                    .snapshot_digests_with_lease(&BTreeMap::from([(name.into(), digest)]))
                    .map_err(failed)?;
                let skill = plugins
                    .iter()
                    .flat_map(|plugin| &plugin.skills)
                    .find(|skill| skill.id == skill_id)
                    .ok_or_else(|| failed("selected skill is unavailable"))?;
                match serde_json::from_value::<Op>(request.content.clone()).map_err(failed)? {
                    Op::SkillRead { .. } => serde_json::to_value(skill),
                    Op::ResourceList { .. } => {
                        serde_json::to_value(list_plugin_resources(skill).map_err(failed)?)
                    }
                    Op::ResourceRead { path, .. } => {
                        serde_json::to_value(read_plugin_resource(skill, &path).map_err(failed)?)
                    }
                    _ => return Err(failed("invalid plugin preview operation")),
                }
            }
            Op::VerifyInstalled { name, digest } => {
                let (plugins, _lease) = self
                    .store()?
                    .snapshot_digests_with_lease(&BTreeMap::from([(name, digest.clone())]))
                    .map_err(failed)?;
                let plugin = plugins
                    .first()
                    .ok_or_else(|| failed("plugin is not installed"))?;
                let trust = if plugin.installation.origin == PluginOrigin::Bundled {
                    let embedded = colossus_bundled_plugins::core_artifact().map_err(failed)?;
                    if embedded.manifest_digest != digest {
                        return Err(failed(
                            "bundled content is managed by another Colossus version; restart this executable",
                        ));
                    }
                    plugin.installation.trust.clone()
                } else {
                    let layout = self.store()?.root().join("layouts/sha256").join(
                        digest
                            .strip_prefix("sha256:")
                            .ok_or_else(|| failed("invalid plugin digest"))?,
                    );
                    let artifact = colossus_plugins::verify_plugin_layout(&layout, Some(&digest))
                        .map_err(failed)?;
                    let bundles = colossus_plugins::sigstore_bundles_for_subject(&layout, &digest)
                        .map_err(failed)?;
                    let profile = plugin
                        .installation
                        .trust
                        .profile
                        .as_deref()
                        .unwrap_or("default");
                    colossus_plugins::verify_plugin_trust(
                        profile,
                        self.profile(profile)?,
                        &artifact.manifest,
                        &bundles,
                    )
                    .map_err(failed)?
                };
                Ok(
                    json!({"digest": digest, "integrity": "verified", "trust": trust, "origin": plugin.installation.origin}),
                )
            }
            Op::Validate { path } => serde_json::to_value(
                colossus_plugins::validate_plugin(Path::new(&path)).map_err(failed)?,
            ),
            Op::Verify {
                path,
                digest,
                trust_profile,
            } => {
                let path = Path::new(&path);
                let temporary = tempfile::tempdir().map_err(failed)?;
                let layout = if path.is_file() {
                    let layout = temporary.path().join("layout");
                    colossus_plugins::import_layout_archive(path, &layout).map_err(failed)?;
                    layout
                } else {
                    path.to_owned()
                };
                let artifact = colossus_plugins::verify_plugin_layout(&layout, digest.as_deref())
                    .map_err(failed)?;
                let bundles = colossus_plugins::sigstore_bundles_for_subject(
                    &layout,
                    &artifact.manifest_digest,
                )
                .map_err(failed)?;
                let trust = colossus_plugins::verify_plugin_trust(
                    &trust_profile,
                    self.profile(&trust_profile)?,
                    &artifact.manifest,
                    &bundles,
                )
                .map_err(failed)?;
                Ok(
                    json!({"digest": artifact.manifest_digest, "trust": trust, "manifest": artifact.parsed_manifest}),
                )
            }
            Op::Install {
                source,
                trust_profile,
            } => {
                let profile = self.profile(&trust_profile)?;
                let installation = match source {
                    PluginInstallSource::Directory { path } => {
                        self.store()?.install_directory_with_trust(
                            Path::new(&path),
                            &trust_profile,
                            profile,
                            actor,
                        )
                    }
                    PluginInstallSource::Layout { path, digest } => {
                        self.store()?.install_layout_with_trust(
                            Path::new(&path),
                            digest.as_deref(),
                            &trust_profile,
                            profile,
                            actor,
                        )
                    }
                    PluginInstallSource::Archive { path, digest } => {
                        self.store()?.install_archive_with_trust(
                            Path::new(&path),
                            digest.as_deref(),
                            &trust_profile,
                            profile,
                            actor,
                        )
                    }
                    PluginInstallSource::Reference { .. } => {
                        return Err(failed(
                            "registry installation requires the registry effect boundary",
                        ));
                    }
                }
                .map_err(failed)?;
                serde_json::to_value(installation)
            }
            Op::Enable {
                name,
                digest,
                allow_untrusted,
            } => {
                let installation = self
                    .store()?
                    .list(10_000)
                    .map_err(failed)?
                    .into_iter()
                    .find(|entry| entry.manifest.name == name && entry.digest == digest)
                    .ok_or_else(|| failed("selected plugin digest is not installed"))?;
                if installation.origin != PluginOrigin::Bundled
                    && !installation.trust.trusted
                    && (!allow_untrusted || request.approval.is_none())
                {
                    return Err(failed(
                        "untrusted enablement requires a request-bound approval, not only allow_untrusted",
                    ));
                }
                serde_json::to_value(
                    self.store()?
                        .enable(&name, &digest, allow_untrusted, actor)
                        .map_err(failed)?,
                )
            }
            Op::Disable { name } => {
                self.store()?.disable(&name, actor).map_err(failed)?;
                Ok(json!({"plugin": name, "active": false}))
            }
            Op::Uninstall {
                name,
                digest,
                purge_data,
            } => serde_json::to_value(
                self.store()?
                    .uninstall(&name, &digest, purge_data, actor)
                    .map_err(failed)?,
            ),
            Op::Gc => serde_json::to_value(self.store()?.gc().map_err(failed)?),
            Op::Package { directory, output } => {
                let artifact = colossus_plugins::package_plugin_to_layout(
                    Path::new(&directory),
                    Path::new(&output),
                    None,
                )
                .map_err(failed)?;
                Ok(
                    json!({"digest": artifact.manifest_digest, "output": output, "manifest": artifact.parsed_manifest}),
                )
            }
            Op::Export { name, output } => {
                let digest = self
                    .store()?
                    .export_active(&name, Path::new(&output))
                    .map_err(failed)?;
                Ok(json!({"plugin": name, "digest": digest, "output": output}))
            }
            Op::Pull { .. } | Op::Push { .. } | Op::Update { .. } => {
                return Err(failed(
                    "registry operation requires the registry effect boundary",
                ));
            }
        };
        value.map_err(failed)
    }
}

#[async_trait]
impl EffectExecutor for PluginManagementExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let operation: PluginManagementRequest =
            serde_json::from_value(request.content.clone()).map_err(failed)?;
        if request.action != operation.action()
            || request.resource != operation.resource()
            || request.actor.actor_type != ActorType::User
        {
            return Err(failed(
                "operator plugin request does not match its authorized effect",
            ));
        }
        for (path, write) in management_paths(&operation) {
            enforce_management_path(path, write, &permit)?;
        }
        let value = self.execute_local(operation, request.actor.clone(), request)?;
        Ok(QuarantinedEffectResult {
            media_type: "application/json".into(),
            bytes: serde_json::to_vec(&value).map_err(failed)?,
            effect_succeeded: true,
        })
    }
}

pub(super) fn management_paths(operation: &PluginManagementRequest) -> Vec<(&str, bool)> {
    use PluginManagementRequest as Op;
    match operation {
        Op::Validate { path } | Op::Verify { path, .. } => vec![(path, false)],
        Op::Install {
            source:
                PluginInstallSource::Directory { path }
                | PluginInstallSource::Layout { path, .. }
                | PluginInstallSource::Archive { path, .. },
            ..
        } => vec![(path, false)],
        Op::Package { directory, output } => vec![(directory, false), (output, true)],
        Op::Export { output, .. } | Op::Pull { output, .. } => vec![(output, true)],
        Op::Push { layout, .. } => vec![(layout, false)],
        _ => Vec::new(),
    }
}

fn enforce_management_path(
    path: &str,
    write: bool,
    permit: &ExecutionPermit,
) -> Result<(), ExecutionError> {
    let path = Path::new(path);
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(failed(
            "plugin management paths must be absolute and normalized",
        ));
    }
    let canonical = if write && !path.exists() {
        let parent = path
            .parent()
            .ok_or_else(|| failed("plugin output needs an existing parent"))?;
        fs::canonicalize(parent).map_err(failed)?.join(
            path.file_name()
                .ok_or_else(|| failed("invalid plugin output"))?,
        )
    } else {
        fs::canonicalize(path).map_err(failed)?
    };
    if permit.obligations().resource_authority != ResourceAuthority::Ambient
        && !permit.obligations().filesystem.iter().any(|grant| {
            (grant.mode == "write" || (!write && grant.mode == "read"))
                && canonical.starts_with(&grant.root)
        })
    {
        return Err(failed(
            "plugin path is outside the permit's authorized filesystem roots",
        ));
    }
    Ok(())
}

fn failed(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

impl Runtime {
    /// Execute a typed operator request through the existing authorization boundaries.
    pub async fn manage_plugin(
        &self,
        mut operation: PluginManagementRequest,
    ) -> Result<Value, RuntimeError> {
        use PluginManagementRequest as Op;
        match &mut operation {
            Op::Validate { path } | Op::Verify { path, .. } => {
                *path = workspace_absolute_path(&self.workspace, Path::new(path))
                    .display()
                    .to_string()
            }
            Op::Install {
                source:
                    PluginInstallSource::Directory { path }
                    | PluginInstallSource::Layout { path, .. }
                    | PluginInstallSource::Archive { path, .. },
                ..
            } => {
                *path = workspace_absolute_path(&self.workspace, Path::new(path))
                    .display()
                    .to_string()
            }
            Op::Package { directory, output } => {
                *directory = workspace_absolute_path(&self.workspace, Path::new(directory))
                    .display()
                    .to_string();
                *output = workspace_absolute_path(&self.workspace, Path::new(output))
                    .display()
                    .to_string();
            }
            Op::Export { output, .. } => {
                *output = workspace_absolute_path(&self.workspace, Path::new(output))
                    .display()
                    .to_string()
            }
            Op::Pull {
                registry,
                reference,
                output,
            } => {
                return serde_json::to_value(
                    self.pull_plugin(registry, reference, Path::new(output))
                        .await?,
                )
                .map_err(|error| RuntimeError::Config(error.to_string()));
            }
            Op::Push {
                registry,
                reference,
                layout,
            } => {
                return serde_json::to_value(
                    self.push_plugin(registry, Path::new(layout), reference)
                        .await?,
                )
                .map_err(|error| RuntimeError::Config(error.to_string()));
            }
            Op::Install {
                source:
                    PluginInstallSource::Reference {
                        registry,
                        reference,
                    },
                trust_profile,
            } => {
                let enforced = &self.plugin_registry_profile(registry)?.trust_profile;
                if trust_profile != "default" && trust_profile != enforced {
                    return Err(RuntimeError::Config(format!(
                        "Registry {registry} enforces trust profile {enforced}; use that profile or explicitly reconfigure the registry. Offline trust overrides do not weaken registry policy."
                    )));
                }
                return serde_json::to_value(
                    Box::pin(self.install_plugin_reference(registry, reference, None)).await?,
                )
                .map_err(|error| RuntimeError::Config(error.to_string()));
            }
            Op::Update {
                name,
                registry,
                reference,
            } => {
                if name == "colossus" {
                    return Err(RuntimeError::Config("colossus is bundled with Colossus; update the executable to change its version".into()));
                }
                return serde_json::to_value(
                    Box::pin(self.install_plugin_reference(registry, reference, Some(name)))
                        .await?,
                )
                .map_err(|error| RuntimeError::Config(error.to_string()));
            }
            _ => {}
        }
        let mut request = effect_request(
            terminal_actor(),
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![operation.action().into()];
        let executor = WorkspaceBoundEffectExecutor::new(
            self._workspace_lease.identity(),
            Arc::new(PluginManagementExecutor {
                store: self.plugin_store.clone(),
                configuration: Arc::clone(&self.plugin_configuration),
            }),
        );
        let released = self.gateway.execute(request, &executor).await?;
        serde_json::from_slice(&released.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }
}
