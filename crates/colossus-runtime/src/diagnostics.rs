use super::*;

enum ProviderModelsProbe {
    Models(Vec<ProviderModelInfo>),
    HttpError(ProviderResponseDiagnostic),
}

const MAX_PROVIDER_DIAGNOSTIC_DISPLAY_CHARS: usize = 64 * 1024;

/// Render explicitly released provider evidence for a trusted local diagnostic surface.
///
/// Response evidence and offered tool names are placed before the potentially large request so
/// the most useful failure details survive the display bound.
pub fn format_provider_response_diagnostic(diagnostic: &ProviderResponseDiagnostic) -> String {
    let tool_names = diagnostic
        .request_body
        .as_ref()
        .and_then(|body| body.get("tools"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .or_else(|| tool.pointer("/function/name").and_then(Value::as_str))
        })
        .collect::<Vec<_>>();
    let content_type = diagnostic.content_type.as_deref().unwrap_or("unknown");
    let truncation = if diagnostic.body_truncated {
        " (truncated at 16 KiB)"
    } else {
        ""
    };
    let request_body = diagnostic
        .request_body
        .as_ref()
        .map(|body| {
            serde_json::to_string_pretty(body)
                .unwrap_or_else(|_| "<request body could not be rendered>".into())
        })
        .unwrap_or_else(|| "<none>".into());
    let offered_tools = if tool_names.is_empty() {
        "<none>".into()
    } else {
        tool_names.join(", ")
    };
    let mut output = format!(
        "Provider response diagnostics (explicit local opt-in; not written to durable run history)\n\
         Response: HTTP {} ({content_type}, {}){truncation}\n\
         Response body:\n{}\n\n\
         Offered tool names ({}): {offered_tools}\n\n\
         Request: {} {}\n\
         Request body:\n{request_body}\n\n\
         Warning: the request can contain user, session, and tool-result data. Review before sharing.",
        diagnostic.status,
        diagnostic.body_encoding,
        diagnostic.body,
        tool_names.len(),
        diagnostic.request_method,
        diagnostic.request_url,
    );
    if output.chars().count() > MAX_PROVIDER_DIAGNOSTIC_DISPLAY_CHARS {
        output = output
            .chars()
            .take(MAX_PROVIDER_DIAGNOSTIC_DISPLAY_CHARS)
            .collect();
        output.push_str("\n[diagnostic display truncated]");
    }
    output
}

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
        self.drain_projections_bounded(256, 16_384)
    }

    /// Replay bounded projection batches without requiring a moving journal head to
    /// become current. Long-running hosts use this to keep background maintenance
    /// fair; explicit operator drains and shutdown checkpoints use [`Self::drain_projections`].
    pub fn drain_projections_bounded(
        &self,
        batch_limit: usize,
        max_rounds: usize,
    ) -> Result<ProjectionRunReport, RuntimeError> {
        self.projections
            .drain(batch_limit, max_rounds)
            .map_err(Into::into)
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
        Ok(self.providers.resolve(role)?.route())
    }

    /// Safe configured model profiles with explicit provider, limits, and capabilities.
    pub fn model_profiles(&self) -> Vec<ModelRoute> {
        self.providers.models()
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
        match self.provider_models_probe(profile, false).await? {
            ProviderModelsProbe::Models(models) => Ok(models),
            ProviderModelsProbe::HttpError(_) => Err(RuntimeError::Config(
                "provider returned diagnostics without an explicit request".into(),
            )),
        }
    }

    async fn provider_models_probe(
        &self,
        profile: Option<&str>,
        include_response_diagnostics: bool,
    ) -> Result<ProviderModelsProbe, RuntimeError> {
        let provider = match profile {
            Some(profile) => self.providers.profile(profile)?,
            None => self.providers.resolve("primary")?.provider().clone(),
        };
        if provider.profile().kind == ProviderKind::Echo {
            return Ok(ProviderModelsProbe::Models(
                self.providers
                    .models_for_provider(&provider.profile().name)
                    .into_iter()
                    .map(|model| ProviderModelInfo {
                        id: model.model,
                        object: Some("model".into()),
                        owned_by: Some("colossus".into()),
                    })
                    .collect(),
            ));
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
                provider_profile: provider.profile().name.clone(),
                model_profile: None,
                model: None,
                max_output_tokens: None,
                request: None,
                include_response_diagnostics,
            })
            .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec!["provider.call".into()];
        request.credential_references = provider.credential_reference().into_iter().collect();
        let result = self.gateway.execute(request, provider.as_ref()).await?;
        if let Ok(models) = serde_json::from_slice(&result.bytes) {
            return Ok(ProviderModelsProbe::Models(models));
        }
        if include_response_diagnostics
            && let Ok(diagnostic) = serde_json::from_slice(&result.bytes)
        {
            return Ok(ProviderModelsProbe::HttpError(diagnostic));
        }
        Err(RuntimeError::Config(
            "released provider catalog output violated its normalized contract".into(),
        ))
    }

    /// Check a provider connection profile by exercising its catalog endpoint through policy.
    pub async fn provider_doctor(
        &self,
        profile: Option<&str>,
    ) -> Result<ProviderReadiness, RuntimeError> {
        self.provider_doctor_with_diagnostics(profile, false).await
    }

    /// Check a provider profile with optional bounded non-success response diagnostics.
    pub async fn provider_doctor_with_diagnostics(
        &self,
        profile: Option<&str>,
        include_response_diagnostics: bool,
    ) -> Result<ProviderReadiness, RuntimeError> {
        let provider = match profile {
            Some(profile) => self.providers.profile(profile)?,
            None => self.providers.resolve("primary")?.provider().clone(),
        };
        let mut readiness = provider.static_readiness();
        if provider.profile().kind == ProviderKind::Echo {
            return Ok(readiness);
        }
        let mut checks = Vec::new();
        match self
            .provider_models_probe(Some(&provider.profile().name), include_response_diagnostics)
            .await
        {
            Ok(ProviderModelsProbe::Models(models)) => {
                checks.push(ProviderReadinessCheck {
                    name: "models_endpoint".into(),
                    status: "pass".into(),
                    detail: format!(
                        "Reached the configured models endpoint and normalized {} model records.",
                        models.len()
                    ),
                    provider_response: None,
                });
            }
            Ok(ProviderModelsProbe::HttpError(diagnostic)) => {
                checks.push(ProviderReadinessCheck {
                    name: "models_endpoint".into(),
                    status: "fail".into(),
                    detail: format!("provider endpoint returned HTTP {}", diagnostic.status),
                    provider_response: Some(diagnostic),
                });
            }
            Err(error) => {
                checks.push(ProviderReadinessCheck {
                    name: "models_endpoint".into(),
                    status: "fail".into(),
                    detail: error.to_string(),
                    provider_response: None,
                });
            }
        }
        readiness.ready = checks.iter().all(|check| check.status == "pass");
        readiness.checks = checks;
        Ok(readiness)
    }

    /// Check one explicit model profile by exercising a bounded generation through policy.
    pub async fn model_doctor(&self, profile: Option<&str>) -> Result<Value, RuntimeError> {
        self.model_doctor_with_diagnostics(profile, false).await
    }

    /// Check a model profile with optional bounded non-success response diagnostics.
    pub async fn model_doctor_with_diagnostics(
        &self,
        profile: Option<&str>,
        include_response_diagnostics: bool,
    ) -> Result<Value, RuntimeError> {
        let resolved = match profile {
            Some(profile) => self.providers.model(profile)?,
            None => self.providers.resolve("primary")?,
        };
        let route = resolved.route();
        let provider = resolved.provider();
        let endpoint = provider.profile().generation_endpoint()?;
        let output_limit = route.limits.max_output_tokens.min(32);
        let tools = if route.capabilities.tool_calls {
            vec![ModelToolDefinition {
                name: "colossus.readiness".into(),
                description: "Representative tool-schema compatibility probe.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "paths": {
                            "type": "array",
                            "maxItems": 4,
                            "items": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 4096
                            }
                        }
                    },
                    "required": ["paths"],
                    "additionalProperties": false
                }),
            }]
        } else {
            Vec::new()
        };
        let mut request = effect_request(
            system_actor("model-diagnostics"),
            provider.profile().kind.generation_action(),
            endpoint,
            serde_json::to_value(ProviderEffectInput {
                provider_profile: route.provider_profile.clone(),
                model_profile: Some(route.model_profile.clone()),
                model: Some(route.model.clone()),
                max_output_tokens: Some(output_limit),
                request: Some(ModelRequest {
                    instructions: "This is a model readiness probe. Reply with exactly: ok".into(),
                    messages: vec![ModelMessage {
                        role: ModelMessageRole::User,
                        content: "Reply with exactly: ok".into(),
                        tool_call_id: None,
                        tool_calls: Vec::new(),
                    }],
                    tools,
                    max_output_tokens: Some(output_limit),
                }),
                include_response_diagnostics,
            })
            .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec!["provider.call".into()];
        request.credential_references = provider.credential_reference().into_iter().collect();
        let generation = match self.gateway.execute(request, provider.as_ref()).await {
            Ok(result) => {
                if include_response_diagnostics
                    && let Ok(diagnostic) =
                        serde_json::from_slice::<ProviderResponseDiagnostic>(&result.bytes)
                {
                    ProviderReadinessCheck {
                        name: "generation".into(),
                        status: "fail".into(),
                        detail: format!("provider endpoint returned HTTP {}", diagnostic.status),
                        provider_response: Some(diagnostic),
                    }
                } else {
                    match serde_json::from_slice::<ProviderTurn>(&result.bytes) {
                        Ok(turn)
                            if turn.model_profile == route.model_profile
                                && turn.provider_profile == route.provider_profile =>
                        {
                            ProviderReadinessCheck {
                                name: "generation".into(),
                                status: "pass".into(),
                                detail: if route.capabilities.tool_calls {
                                    "Completed and normalized one bounded model generation with a representative tool schema."
                                        .into()
                                } else {
                                    "Completed and normalized one bounded text-only model generation."
                                        .into()
                                },
                                provider_response: None,
                            }
                        }
                        Ok(_) | Err(_) => ProviderReadinessCheck {
                            name: "generation".into(),
                            status: "fail".into(),
                            detail:
                                "Released generation metadata violated the selected model route."
                                    .into(),
                            provider_response: None,
                        },
                    }
                }
            }
            Err(error) => ProviderReadinessCheck {
                name: "generation".into(),
                status: "fail".into(),
                detail: error.to_string(),
                provider_response: None,
            },
        };
        Ok(json!({
            "ready": generation.status == "pass",
            "route": route,
            "checks": [
                {
                    "name": "metadata",
                    "status": "pass",
                    "detail": "Explicit limits and capabilities are valid."
                },
                generation,
            ],
        }))
    }
}
