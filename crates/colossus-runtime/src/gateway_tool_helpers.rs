use super::*;

pub(super) struct GatewayBoundEffects {
    pub(super) identity: workspace_lease::WorkspaceIdentity,
    pub(super) pack_process: Arc<dyn EffectExecutor>,
    pub(super) integration: Arc<dyn EffectExecutor>,
    pub(super) mcp: Arc<dyn EffectExecutor>,
}

pub(super) struct GatewayToolExecutor {
    pub(super) gateway: Arc<EffectGateway>,
    pub(super) filesystem: Arc<dyn EffectExecutor>,
    pub(super) process: Option<Arc<dyn EffectExecutor>>,
    pub(super) http: Arc<HttpExecutor>,
    pub(super) work: Option<Arc<WorkEffectExecutor>>,
    pub(super) memory: Option<Arc<MemoryEffectExecutor>>,
    pub(super) skills: Option<Arc<dyn EffectExecutor>>,
    pub(super) pack_processes: Option<Arc<PackProcessExecutor>>,
    pub(super) integrations: Option<Arc<IntegrationExecutor>>,
    pub(super) mcp: Option<Arc<McpExecutor>>,
    pub(super) bound_effects: Option<GatewayBoundEffects>,
    pub(super) search: Option<Arc<dyn SearchProvider>>,
    pub(super) workspace: PathBuf,
    pub(super) repository_id: String,
    pub(super) executables: Vec<PathBuf>,
}

impl GatewayToolExecutor {
    pub(super) fn current_session(context: &ExecutionContext) -> Result<String, ToolError> {
        context
            .session_id
            .clone()
            .ok_or_else(|| ToolError::Denied("durable state tools require a session".into()))
    }

    pub(super) async fn execute_work_tool(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
        operation: WorkOperation,
    ) -> Result<String, ToolError> {
        let action = operation.action().to_owned();
        let resource = operation.resource().to_owned();
        let mut request = effect_request(
            model_actor(call, &context),
            &action,
            resource,
            serde_json::to_value(&operation)
                .map_err(|error| ToolError::Failed(error.to_string()))?,
        );
        request.capabilities = vec![action];
        request.context = context;
        let result = self
            .gateway
            .execute(
                request,
                self.work
                    .as_deref()
                    .ok_or_else(|| ToolError::Failed("work adapter is unavailable".into()))?,
            )
            .await
            .map_err(tool_gateway_error)?;
        let output = String::from_utf8(result.bytes)
            .map_err(|_| ToolError::Failed("work result returned non-UTF-8".into()))?;
        serde_json::from_str::<Value>(&output)
            .map_err(|error| ToolError::Failed(format!("invalid work result: {error}")))?;
        Ok(bounded_tool_text(&output, 1024 * 1024))
    }

    pub(super) async fn execute_memory_tool(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
        operation: MemoryOperation,
    ) -> Result<String, ToolError> {
        let action = operation.action().to_owned();
        let resource = operation.resource();
        let mut request = effect_request(
            model_actor(call, &context),
            &action,
            resource,
            serde_json::to_value(operation)
                .map_err(|error| ToolError::Failed(error.to_string()))?,
        );
        request.capabilities = vec![action];
        request.context = context;
        let result = self
            .gateway
            .execute(
                request,
                self.memory
                    .as_deref()
                    .ok_or_else(|| ToolError::Failed("memory adapter is unavailable".into()))?,
            )
            .await
            .map_err(tool_gateway_error)?;
        let output = String::from_utf8(result.bytes)
            .map_err(|_| ToolError::Failed("memory result returned non-UTF-8".into()))?;
        serde_json::from_str::<Value>(&output)
            .map_err(|error| ToolError::Failed(format!("invalid memory result: {error}")))?;
        Ok(bounded_tool_text(&output, 1024 * 1024))
    }

    pub(super) async fn execute_skill_tool(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
        operation: SkillOperation,
    ) -> Result<String, ToolError> {
        let action = operation.action().to_owned();
        let mut request = effect_request(
            model_actor(call, &context),
            &action,
            operation.resource(),
            serde_json::to_value(operation)
                .map_err(|error| ToolError::Failed(error.to_string()))?,
        );
        request.capabilities = vec![action];
        request.context = context;
        let result = self
            .gateway
            .execute(
                request,
                self.skills
                    .as_deref()
                    .ok_or_else(|| ToolError::Failed("skill adapter is unavailable".into()))?,
            )
            .await
            .map_err(tool_gateway_error)?;
        let output = String::from_utf8(result.bytes)
            .map_err(|_| ToolError::Failed("skill resource returned non-UTF-8".into()))?;
        serde_json::from_str::<Value>(&output)
            .map_err(|error| ToolError::Failed(format!("invalid skill result: {error}")))?;
        Ok(bounded_tool_text(&output, 256 * 1024))
    }

    pub(super) async fn execute_integration_tool(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
    ) -> Result<Option<String>, ToolError> {
        let executor = self
            .integrations
            .as_deref()
            .ok_or_else(|| ToolError::Failed("integration adapter is unavailable".into()))?;
        let Some((operation, credentials)) = executor
            .invocation(&call.name, call.arguments.clone())
            .map_err(|error| ToolError::Failed(error.to_string()))?
        else {
            return Ok(None);
        };
        let mut request = effect_request(
            model_actor(call, &context),
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation)
                .map_err(|error| ToolError::Failed(error.to_string()))?,
        );
        request.capabilities = vec!["integration.invoke".into()];
        request.credential_references = credentials;
        request.context = context;
        let effect = self
            .bound_effects
            .as_ref()
            .map_or(executor as &dyn EffectExecutor, |effects| {
                effects.integration.as_ref()
            });
        let result = self
            .gateway
            .execute(request, effect)
            .await
            .map_err(tool_gateway_error)?;
        let output = String::from_utf8(result.bytes)
            .map_err(|_| ToolError::Failed("integration result returned non-UTF-8".into()))?;
        serde_json::from_str::<Value>(&output)
            .map_err(|error| ToolError::Failed(format!("invalid integration result: {error}")))?;
        Ok(Some(bounded_tool_text(&output, 1024 * 1024)))
    }

    pub(super) async fn execute_pack_tool(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
    ) -> Result<Option<(String, i32)>, ToolError> {
        let executor = self
            .pack_processes
            .as_deref()
            .ok_or_else(|| ToolError::Failed("pack process adapter is unavailable".into()))?;
        let effect = self
            .bound_effects
            .as_ref()
            .map_or(executor as &dyn EffectExecutor, |effects| {
                effects.pack_process.as_ref()
            });
        let Some((declaration, input)) = executor.invocation(&call.name) else {
            return Ok(None);
        };
        if !call
            .arguments
            .as_object()
            .is_some_and(serde_json::Map::is_empty)
        {
            return Err(ToolError::InvalidArguments {
                tool: call.name.clone(),
                message: "verified pack tool accepts no dynamic arguments".into(),
            });
        }
        let mut request = effect_request(
            model_actor(call, &context),
            &declaration.action,
            declaration.executable.display().to_string(),
            serde_json::to_value(input).map_err(|error| ToolError::Failed(error.to_string()))?,
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
        request.context = context;
        let result = self
            .gateway
            .execute(request, effect)
            .await
            .map_err(tool_gateway_error)?;
        let value: Value = serde_json::from_slice(&result.bytes)
            .map_err(|error| ToolError::Failed(format!("invalid pack process result: {error}")))?;
        let decode = |field: &str| -> Result<String, ToolError> {
            let encoded = value.get(field).and_then(Value::as_str).ok_or_else(|| {
                ToolError::Failed(format!("pack process result field {field} is absent"))
            })?;
            let bytes = BASE64
                .decode(encoded)
                .map_err(|error| ToolError::Failed(format!("invalid pack output: {error}")))?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        };
        let exit_code = value
            .get("exit_code")
            .and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok())
            .ok_or_else(|| ToolError::Failed("pack process exit_code is absent".into()))?;
        let output = serde_json::to_string(&json!({
            "pack": declaration.pack,
            "tool": declaration.tool,
            "stdout": decode("stdout_base64")?,
            "stderr": decode("stderr_base64")?,
            "exit_code": exit_code,
            "truncated": value.get("truncated").and_then(Value::as_bool).unwrap_or(false),
        }))
        .map_err(|error| ToolError::Failed(error.to_string()))?;
        Ok(Some((bounded_tool_text(&output, 1024 * 1024), exit_code)))
    }

    pub(super) async fn discover_mcp_tool_output(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
        server: Option<&str>,
    ) -> Result<String, ToolError> {
        let executor = self
            .mcp
            .as_deref()
            .ok_or_else(|| ToolError::Failed("MCP adapter is unavailable".into()))?;
        let effect = self
            .bound_effects
            .as_ref()
            .map_or(executor as &dyn EffectExecutor, |effects| {
                effects.mcp.as_ref()
            });
        let tools = discover_mcp_tools(
            self.gateway.as_ref(),
            executor,
            effect,
            model_actor(call, &context),
            context,
            server,
        )
        .await
        .map_err(mcp_runtime_tool_error)?;
        serde_json::to_string(&tools)
            .map(|output| bounded_tool_text(&output, 1024 * 1024))
            .map_err(|error| ToolError::Failed(error.to_string()))
    }

    pub(super) async fn execute_mcp_tool(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<String, ToolError> {
        let executor = self
            .mcp
            .as_deref()
            .ok_or_else(|| ToolError::Failed("MCP adapter is unavailable".into()))?;
        let effect = self
            .bound_effects
            .as_ref()
            .map_or(executor as &dyn EffectExecutor, |effects| {
                effects.mcp.as_ref()
            });
        let output = invoke_mcp_tool(
            self.gateway.as_ref(),
            executor,
            effect,
            model_actor(call, &context),
            context,
            server,
            tool,
            arguments,
        )
        .await
        .map_err(mcp_runtime_tool_error)?;
        serde_json::to_string(&output)
            .map(|output| bounded_tool_text(&output, 1024 * 1024))
            .map_err(|error| ToolError::Failed(error.to_string()))
    }

    pub(super) async fn execute_repository_tool(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
        operation: RepositoryOperation,
    ) -> Result<String, ToolError> {
        let action = operation.action().to_owned();
        let resource =
            fs::canonicalize(model_workspace_path(&self.workspace, operation.resource())?)
                .map_err(|error| ToolError::Failed(error.to_string()))?
                .display()
                .to_string();
        let mut request = effect_request(
            model_actor(call, &context),
            &action,
            resource,
            serde_json::to_value(operation)
                .map_err(|error| ToolError::Failed(error.to_string()))?,
        );
        request.capabilities = vec![action];
        request.context = context;
        let raw_repository = Arc::new(RepositoryEffectExecutor {
            workspace: self.workspace.clone(),
        });
        let repository: Arc<dyn EffectExecutor> = self.bound_effects.as_ref().map_or_else(
            || Arc::clone(&raw_repository) as Arc<dyn EffectExecutor>,
            |effects| {
                Arc::new(WorkspaceBoundEffectExecutor::new(
                    effects.identity.clone(),
                    Arc::clone(&raw_repository),
                )) as Arc<dyn EffectExecutor>
            },
        );
        let result = self
            .gateway
            .execute(request, repository.as_ref())
            .await
            .map_err(tool_gateway_error)?;
        let output = String::from_utf8(result.bytes)
            .map_err(|_| ToolError::Failed("repository result returned non-UTF-8".into()))?;
        serde_json::from_str::<Value>(&output)
            .map_err(|error| ToolError::Failed(format!("invalid repository result: {error}")))?;
        Ok(bounded_tool_text(&output, 1024 * 1024))
    }

    pub(super) async fn execute_filesystem_mutation(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
        path: PathBuf,
        content: Value,
    ) -> Result<String, ToolError> {
        let mut request = effect_request(
            model_actor(call, &context),
            "filesystem.write",
            path.display().to_string(),
            content,
        );
        request.capabilities = vec!["filesystem.write".into()];
        request.context = context;
        let result = self
            .gateway
            .execute(request, self.filesystem.as_ref())
            .await
            .map_err(tool_gateway_error)?;
        let output = String::from_utf8(result.bytes)
            .map_err(|_| ToolError::Failed("filesystem mutation returned non-UTF-8".into()))?;
        serde_json::from_str::<Value>(&output)
            .map_err(|error| ToolError::Failed(format!("invalid mutation result: {error}")))?;
        Ok(bounded_tool_text(&output, 1024 * 1024))
    }

    pub(super) async fn execute_patch_tool(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
    ) -> Result<String, ToolError> {
        let path = model_workspace_path(&self.workspace, required_tool_string(call, "path")?)?;
        let display_path = workspace_relative(&self.workspace, &path)?;
        let (old, new) = if call.name == "patch.reverse" {
            (
                required_tool_string(call, "new")?,
                required_tool_string(call, "old")?,
            )
        } else {
            (
                required_tool_string(call, "old")?,
                required_tool_string(call, "new")?,
            )
        };
        let mut request = effect_request(
            model_actor(call, &context),
            &call.name,
            path.display().to_string(),
            json!({
                "operation": "replace",
                "display_path": display_path,
                "old": old,
                "new": new,
                "replace_all": optional_tool_bool(call, "replace_all")?.unwrap_or(false),
            }),
        );
        request.capabilities = vec![call.name.clone()];
        request.context = context;
        let result = self
            .gateway
            .execute(request, self.filesystem.as_ref())
            .await
            .map_err(tool_gateway_error)?;
        let output = String::from_utf8(result.bytes)
            .map_err(|_| ToolError::Failed("patch result returned non-UTF-8".into()))?;
        serde_json::from_str::<Value>(&output)
            .map_err(|error| ToolError::Failed(format!("invalid patch result: {error}")))?;
        Ok(bounded_tool_text(&output, 1024 * 1024))
    }

    pub(super) fn resolve_executable(&self, requested: &str) -> Result<PathBuf, ToolError> {
        if requested.is_empty() || requested.contains('\0') {
            return Err(ToolError::InvalidArguments {
                tool: "shell.run".into(),
                message: "argv[0] must name one configured executable".into(),
            });
        }
        let requested_path = Path::new(requested);
        let matches = self
            .executables
            .iter()
            .filter(|candidate| {
                candidate == &requested_path
                    || candidate
                        .file_name()
                        .is_some_and(|name| name == requested_path.as_os_str())
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [executable] => Ok((*executable).clone()),
            [] => Err(ToolError::Denied(format!(
                "executable {requested} is not explicitly configured"
            ))),
            _ => Err(ToolError::Denied(format!(
                "executable name {requested} is ambiguous; use its configured absolute path"
            ))),
        }
    }

    pub(super) fn git_executable(&self) -> Result<PathBuf, ToolError> {
        let matches = self
            .executables
            .iter()
            .filter(|candidate| {
                candidate
                    .file_stem()
                    .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("git"))
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [executable] => Ok((*executable).clone()),
            [] => Err(ToolError::Denied(
                "Git tools require one explicitly configured git executable".into(),
            )),
            _ => Err(ToolError::Denied(
                "multiple git executables are configured; keep one exact identity".into(),
            )),
        }
    }

    pub(super) fn shell_executable(&self) -> Result<PathBuf, ToolError> {
        let matches = self
            .executables
            .iter()
            .filter(|candidate| candidate.to_str().is_some_and(is_shell_wrapper))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [executable] => Ok((*executable).clone()),
            [] => Err(ToolError::Denied(
                "command mode requires workspace-development or one explicit shell executable"
                    .into(),
            )),
            _ => Err(ToolError::Denied(
                "multiple shell executables are configured; keep one exact shell identity".into(),
            )),
        }
    }

    pub(super) fn sanitized_command_path(&self) -> Result<String, ToolError> {
        const MAX_ROOTS: usize = 64;
        let mut roots = std::env::var_os("PATH")
            .map(|path| {
                std::env::split_paths(&path)
                    .filter(|path| path.is_absolute())
                    .filter_map(|path| fs::canonicalize(path).ok())
                    .filter(|path| path.is_dir() && !path.starts_with(&self.workspace))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        roots.extend(
            self.executables
                .iter()
                .filter_map(|path| path.parent())
                .filter_map(|path| fs::canonicalize(path).ok()),
        );
        roots.sort();
        roots.dedup();
        roots.truncate(MAX_ROOTS);
        std::env::join_paths(roots)
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(|error| ToolError::Failed(format!("cannot construct sanitized PATH: {error}")))
    }

    pub(super) async fn execute_process_tool(
        &self,
        call: &ToolCall,
        context: ExecutionContext,
        action: &str,
        executable: PathBuf,
        spec: ProcessSpec,
    ) -> Result<ProcessToolOutput, ToolError> {
        let cwd = spec.cwd.clone();
        let args = spec.args.clone();
        let mut request = effect_request(
            model_actor(call, &context),
            action,
            executable.display().to_string(),
            serde_json::to_value(spec).map_err(|error| ToolError::Failed(error.to_string()))?,
        );
        request.capabilities = vec![action.into()];
        request.context = context;
        let result = self
            .gateway
            .execute(
                request,
                self.process
                    .as_deref()
                    .ok_or_else(|| ToolError::Failed("process adapter is unavailable".into()))?,
            )
            .await
            .map_err(tool_gateway_error)?;
        let value: Value = serde_json::from_slice(&result.bytes)
            .map_err(|error| ToolError::Failed(format!("invalid process result: {error}")))?;
        let decode = |field: &str| -> Result<String, ToolError> {
            let encoded = value.get(field).and_then(Value::as_str).ok_or_else(|| {
                ToolError::Failed(format!("process result field {field} is absent"))
            })?;
            let bytes = BASE64
                .decode(encoded)
                .map_err(|error| ToolError::Failed(format!("invalid process output: {error}")))?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        };
        let exit_code = value
            .get("exit_code")
            .and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok())
            .unwrap_or(-1);
        Ok(ProcessToolOutput {
            executable,
            cwd,
            args,
            stdout: decode("stdout_base64")?,
            stderr: decode("stderr_base64")?,
            exit_code,
            truncated: value
                .get("output_truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            observed_origins: value
                .get("observed_origins")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(64)
                .map(str::to_owned)
                .collect(),
        })
    }
}
