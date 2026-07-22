use super::*;

impl Runtime {
    /// Canonical workspace used by repository and model-facing tools.
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Projection position, lag, and readiness for every built-in reducer.
    pub fn projection_status(&self) -> Result<Vec<ProjectionStatus>, RuntimeError> {
        self.projections.status().map_err(Into::into)
    }

    /// Catch all built-in projections up to the current journal head.
    pub fn drain_projections(&self) -> Result<ProjectionRunReport, RuntimeError> {
        self.projections.drain(256, 16_384).map_err(Into::into)
    }

    /// Delete and replay one projection, or every projection when omitted.
    pub fn rebuild_projection(
        &self,
        name: Option<&str>,
    ) -> Result<ProjectionRunReport, RuntimeError> {
        name.map_or_else(
            || self.projections.rebuild_all(),
            |projection| self.projections.rebuild(projection),
        )
        .map_err(Into::into)
    }

    /// Bounded local storage health report without decrypted event payloads.
    pub fn state_doctor(&self) -> Result<Value, RuntimeError> {
        let (journal_head, record_hash) = self.journal.head()?;
        let writer_lease = self.writer_lease.as_ref().map_or_else(
            || json!({"held": false, "reason": "database-coordinated"}),
            |lease| json!({"held": true, "path": lease.path()}),
        );
        let projection_adapter = self
            .storage_diagnostic
            .get("adapter")
            .cloned()
            .unwrap_or_else(|| Value::String("unknown".into()));
        Ok(json!({
            "recovery_mode": self.journal.is_recovery_mode(),
            "recovery_reason": self.recovery_reason,
            "journal_head": journal_head,
            "record_hash": record_hash,
            "storage": self.storage_diagnostic,
            "writer_lease": writer_lease,
            "projection_store": {
                "adapter": projection_adapter,
                "positions": self.projection_status()?,
            },
            "repository_adapters": {
                "sessions": "event-journal:sessions-v1+messages-v1",
                "work": "event-journal:tasks-v1+decisions-v1",
                "work_projection": "redb-projection:work-v1",
                "memory": "event-journal:memory-v1",
                "memory_projection": "redb-projection:memory-v1",
                "memory_index": "tantivy-or-degraded",
                "research": "event-journal:research-runs-v1+sources-v1+claims-v1",
                "telemetry": "derived:journal-envelopes+typed-safe-counters",
                "workflows": "event-journal+redb-projection:workflows-v1",
            }
        }))
    }

    /// Policy readiness and decision-log safety status.
    pub async fn policy_doctor(&self) -> Result<Value, RuntimeError> {
        self.policy
            .doctor()
            .await
            .map_err(GatewayError::from)
            .map_err(Into::into)
    }

    /// Native/OCI helper readiness and configured fallback status.
    pub fn sandbox_doctor(&self) -> SandboxDoctorReport {
        let mut report = sandbox_doctor(&self.sandbox_executor_config);
        report.canonical_workspace = Some(self.workspace.clone());
        report.sandbox_profile = self.sandbox_profile.clone();
        report.protected_path_exclusions_supported = match self.sandbox_backend.as_str() {
            "native" => report.protected_path_exclusions_supported,
            "windows_job" => cfg!(target_os = "windows"),
            "oci" => true,
            _ => false,
        };
        report.protected_paths = self.development_sandbox.protected_filesystem.clone();
        report.resolved_shell = self.development_sandbox.shell.clone();
        report.development_actor_scope =
            "terminal users, main agents, and child agents without workflow lineage".into();
        report.sanitized_command_roots =
            std::env::split_paths(&std::ffi::OsString::from(&self.development_sandbox.path))
                .collect();
        report.explicit_filesystem = self.sandbox_filesystem.clone();
        report.derived_filesystem = self.development_sandbox.filesystem.clone();
        report.explicit_executables = self.sandbox_executables.clone();
        report.derived_executables = self.development_sandbox.executables.clone();
        report.network_destinations = self.sandbox_network_destinations.clone();
        report.public_network_wildcard = self
            .sandbox_network_destinations
            .iter()
            .any(|destination| destination == "*")
            .then(|| {
                "public HTTP(S) only; private, loopback, link-local, and metadata origins require exact entries"
                    .into()
            });
        report
    }

    /// Provider profile readiness without performing network effects.
    pub fn provider_profiles(&self) -> Vec<ProviderReadiness> {
        self.providers.profiles()
    }

    /// Role-to-profile routing with specialized-role fallback handled by the registry.
    pub fn provider_routes(&self) -> Value {
        json!(self.providers.routes())
    }

    /// Resolve one role to bounded provider metadata without making a network request.
    pub fn provider_route(&self, role: &str) -> Result<ProviderRoute, RuntimeError> {
        let provider = self.providers.resolve(role)?;
        Ok(ProviderRoute {
            role: role.into(),
            profile: provider.profile().name.clone(),
            provider: provider.profile().kind.as_str().into(),
            model: provider.profile().model.clone(),
        })
    }

    /// Safe configured search profile metadata without resolving credentials.
    pub fn search_profiles(&self) -> Vec<SearchProfileSummary> {
        self.search.profiles()
    }

    /// Resolve one exact search role without performing a network request.
    pub fn search_route(&self, role: &str) -> Result<SearchRoute, RuntimeError> {
        self.search.route(role).map_err(Into::into)
    }

    /// Run one explicit operator search through the same provider-neutral path as agents.
    pub async fn search(
        &self,
        role: &str,
        query: &str,
        limit: usize,
    ) -> Result<SearchResponse, RuntimeError> {
        self.search
            .search(
                role,
                Actor {
                    actor_type: ActorType::User,
                    id: "terminal-user".into(),
                },
                SearchRequest {
                    query: query.into(),
                    limit,
                },
                ExecutionContext::default(),
            )
            .await
            .map_err(Into::into)
    }

    /// Stable active model-visible tool catalog.
    pub fn tool_specs(&self) -> Vec<ToolSpec> {
        self.tools.list_specs()
    }

    /// Active tool catalog with resolved access metadata and existing schema fields.
    pub fn tool_catalog(&self) -> Vec<Value> {
        let metadata = self
            .access
            .tools
            .iter()
            .map(|tool| (tool.name.as_str(), tool))
            .collect::<BTreeMap<_, _>>();
        self.tools
            .list_specs()
            .into_iter()
            .map(|spec| {
                let access = metadata
                    .get(spec.name.as_str())
                    .expect("active tool must have access metadata");
                json!({
                    "name": spec.name,
                    "description": spec.description,
                    "input_schema": spec.input_schema,
                    "effect_action": spec.effect_action,
                    "capability": spec.capability,
                    "max_output_bytes": spec.max_output_bytes,
                    "profile": self.access.profile,
                    "family": access.family,
                    "source": access.source,
                    "action_class": access.action_class,
                    "decision": access.decision,
                    "selection_reason": access.reason,
                    "canonical_workspace": self.workspace,
                    "sandbox_profile": self.sandbox_profile,
                    "development_grant_scope": "terminal users, main agents, and child agents without workflow lineage",
                })
            })
            .collect()
    }

    /// Credential-free effective tool and action profile report.
    pub fn effective_access(&self) -> Value {
        let mut value = serde_json::to_value(&self.access).unwrap_or_else(|_| json!({}));
        if let Some(report) = value.as_object_mut() {
            report.insert("canonical_workspace".into(), json!(self.workspace));
            report.insert(
                "sandbox".into(),
                json!({
                    "profile": self.sandbox_profile,
                    "development_actor_scope": "terminal users, main agents, and child agents without workflow lineage",
                    "resolved_shell": self.development_sandbox.shell,
                    "protected_paths": self.development_sandbox.protected_filesystem,
                    "sanitized_command_roots": std::env::split_paths(
                        &std::ffi::OsString::from(&self.development_sandbox.path)
                    ).collect::<Vec<_>>(),
                    "filesystem": {
                        "explicit": self.sandbox_filesystem,
                        "derived": self.development_sandbox.filesystem,
                    },
                    "executables": {
                        "explicit": self.sandbox_executables,
                        "derived": self.development_sandbox.executables,
                    },
                    "network_destinations": self.sandbox_network_destinations,
                    "public_wildcard": self.sandbox_network_destinations.iter().any(|destination| destination == "*")
                        .then_some("public HTTP(S) only; exact entries remain required for non-public origins"),
                }),
            );
        }
        value
    }

    /// List safe metadata for explicitly configured MCP servers.
    pub fn mcp_servers(&self) -> Vec<McpServerSummary> {
        self.mcp_executor.servers()
    }

    /// Discover all allowlisted MCP tools through separately authorized pages.
    pub async fn mcp_tools(
        &self,
        server: Option<&str>,
    ) -> Result<Vec<McpToolSummary>, RuntimeError> {
        discover_mcp_tools(
            self.gateway.as_ref(),
            self.mcp_executor.as_ref(),
            self.mcp_effect_executor.as_ref(),
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            ExecutionContext::default(),
            server,
        )
        .await
    }

    /// Discover, validate, and invoke one allowlisted MCP tool through the gateway.
    pub async fn mcp_call(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<McpCallOutput, RuntimeError> {
        invoke_mcp_tool(
            self.gateway.as_ref(),
            self.mcp_executor.as_ref(),
            self.mcp_effect_executor.as_ref(),
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            ExecutionContext::default(),
            server,
            tool,
            arguments,
        )
        .await
    }

    /// List models for a profile through the universal effect boundary.
    pub async fn provider_models(
        &self,
        profile: Option<&str>,
    ) -> Result<Vec<ProviderModelInfo>, RuntimeError> {
        let provider = profile.map_or_else(
            || self.providers.resolve("primary"),
            |profile| self.providers.profile(profile),
        )?;
        if provider.profile().kind == ProviderKind::Echo {
            return Ok(vec![ProviderModelInfo {
                id: provider.profile().model.clone(),
                object: Some("model".into()),
                owned_by: Some("colossus".into()),
            }]);
        }
        let endpoint = provider
            .profile()
            .models_endpoint()?
            .ok_or_else(|| RuntimeError::Config("provider has no models endpoint".into()))?;
        let mut request = effect_request(
            system_actor("provider-diagnostics"),
            "provider.models",
            endpoint,
            serde_json::to_value(ProviderEffectInput {
                profile: provider.profile().name.clone(),
                request: None,
            })
            .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec!["provider.call".into()];
        request.credential_references = provider.credential_reference().into_iter().collect();
        let result = self.gateway.execute(request, provider.as_ref()).await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Check a provider profile by exercising its models endpoint through policy.
    pub async fn provider_doctor(
        &self,
        profile: Option<&str>,
    ) -> Result<ProviderReadiness, RuntimeError> {
        let provider = profile.map_or_else(
            || self.providers.resolve("primary"),
            |profile| self.providers.profile(profile),
        )?;
        let mut readiness = provider.static_readiness();
        if provider.profile().kind == ProviderKind::Echo {
            return Ok(readiness);
        }
        match self.provider_models(Some(&provider.profile().name)).await {
            Ok(models) => {
                readiness.ready = true;
                readiness.checks = vec![ProviderReadinessCheck {
                    name: "models_endpoint".into(),
                    status: "pass".into(),
                    detail: format!(
                        "Reached the configured models endpoint and normalized {} model records.",
                        models.len()
                    ),
                }];
            }
            Err(error) => {
                readiness.ready = false;
                readiness.checks = vec![ProviderReadinessCheck {
                    name: "models_endpoint".into(),
                    status: "fail".into(),
                    detail: error.to_string(),
                }];
            }
        }
        Ok(readiness)
    }
}
