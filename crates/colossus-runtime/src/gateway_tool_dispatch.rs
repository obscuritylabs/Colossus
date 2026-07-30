use super::*;

#[async_trait]
impl ToolExecutor for GatewayToolExecutor {
    async fn execute(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let mut exit_code = 0;
        let output = match call.name.as_str() {
            "echo" => bounded_tool_text(required_tool_string(&call, "text")?, 32_768),
            "filesystem.list" => {
                let input = optional_tool_string(&call, "path")?.unwrap_or(".");
                let path = model_workspace_path(&self.workspace, input)?;
                let mut request = effect_request(
                    model_actor(&call, &context),
                    "filesystem.list",
                    path.display().to_string(),
                    json!({}),
                );
                request.capabilities = vec!["filesystem.list".into()];
                request.context = context;
                let result = self
                    .gateway
                    .execute(request, self.filesystem.as_ref())
                    .await
                    .map_err(tool_gateway_error)?;
                let value: Value = serde_json::from_slice(&result.bytes)
                    .map_err(|error| ToolError::Failed(error.to_string()))?;
                let entries = value
                    .get("entries")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        ToolError::Failed("filesystem.list returned invalid JSON".into())
                    })?;
                let entries = entries
                    .iter()
                    .filter(|entry| {
                        !entry
                            .get("name")
                            .and_then(Value::as_str)
                            .is_some_and(|name| matches!(name, ".colossus" | ".git"))
                    })
                    .map(|entry| {
                        let mut entry = entry.clone();
                        let name = entry.get("name").and_then(Value::as_str).ok_or_else(|| {
                            ToolError::Failed("filesystem.list entry name is absent".into())
                        })?;
                        entry["path"] =
                            Value::String(workspace_relative(&self.workspace, &path.join(name))?);
                        Ok(entry)
                    })
                    .collect::<Result<Vec<_>, ToolError>>()?;
                serde_json::to_string(&json!({
                    "root": workspace_relative(&self.workspace, &path)?,
                    "entries": entries,
                }))
                .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "filesystem.read" => {
                let path =
                    model_workspace_path(&self.workspace, required_tool_string(&call, "path")?)?;
                let mut request = effect_request(
                    model_actor(&call, &context),
                    "filesystem.read",
                    path.display().to_string(),
                    json!({"path": path}),
                );
                request.capabilities = vec!["filesystem.read".into()];
                request.context = context;
                let result = self
                    .gateway
                    .execute(request, self.filesystem.as_ref())
                    .await
                    .map_err(tool_gateway_error)?;
                bounded_tool_text(
                    &String::from_utf8(result.bytes).map_err(|_| {
                        ToolError::Failed("filesystem.read returned non-UTF-8".into())
                    })?,
                    1024 * 1024,
                )
            }
            "filesystem.search" => {
                let input = optional_tool_string(&call, "path")?.unwrap_or(".");
                let path = model_workspace_path(&self.workspace, input)?;
                let content = json!({
                    "pattern": required_tool_string(&call, "pattern")?,
                    "glob": optional_tool_string(&call, "glob")?,
                    "regex": optional_tool_bool(&call, "regex")?.unwrap_or(true),
                    "case_sensitive": optional_tool_bool(&call, "case_sensitive")?.unwrap_or(true),
                    "max_matches": optional_tool_u64(&call, "max_matches")?.unwrap_or(100),
                });
                let mut request = effect_request(
                    model_actor(&call, &context),
                    "filesystem.search",
                    path.display().to_string(),
                    content,
                );
                request.capabilities = vec!["filesystem.search".into()];
                request.context = context;
                let result = self
                    .gateway
                    .execute(request, self.filesystem.as_ref())
                    .await
                    .map_err(tool_gateway_error)?;
                let mut value: Value = serde_json::from_slice(&result.bytes)
                    .map_err(|error| ToolError::Failed(error.to_string()))?;
                let matches = value
                    .get_mut("matches")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| {
                        ToolError::Failed("filesystem.search returned invalid JSON".into())
                    })?;
                for matched in matches {
                    let relative = matched
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| ToolError::Failed("search match path is absent".into()))?;
                    matched["path"] =
                        Value::String(workspace_relative(&self.workspace, &path.join(relative))?);
                }
                serde_json::to_string(&value)
                    .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "filesystem.write" => {
                let path =
                    model_workspace_path(&self.workspace, required_tool_string(&call, "path")?)?;
                let display_path = workspace_relative(&self.workspace, &path)?;
                self.execute_filesystem_mutation(
                    &call,
                    context,
                    path,
                    json!({
                        "operation": "write",
                        "display_path": display_path,
                        "text": required_tool_string(&call, "content")?,
                        "mode": required_tool_string(&call, "mode")?,
                    }),
                )
                .await?
            }
            "filesystem.replace" => {
                let path =
                    model_workspace_path(&self.workspace, required_tool_string(&call, "path")?)?;
                let display_path = workspace_relative(&self.workspace, &path)?;
                self.execute_filesystem_mutation(
                    &call,
                    context,
                    path,
                    json!({
                        "operation": "replace",
                        "display_path": display_path,
                        "old": required_tool_string(&call, "old")?,
                        "new": required_tool_string(&call, "new")?,
                        "replace_all": optional_tool_bool(&call, "replace_all")?.unwrap_or(false),
                    }),
                )
                .await?
            }
            "git.status" => {
                let process = self
                    .execute_process_tool(
                        &call,
                        context,
                        "git.status",
                        self.git_executable()?,
                        tool_process_spec(
                            self.workspace.clone(),
                            vec!["status".into(), "--porcelain=v1".into()],
                            BTreeMap::new(),
                            None,
                            Some(64 * 1024),
                        ),
                    )
                    .await?;
                exit_code = process.exit_code;
                let entries = process
                    .stdout
                    .lines()
                    .filter(|line| !line.is_empty())
                    .map(|line| {
                        json!({
                            "status": line.get(..2).unwrap_or(line),
                            "path": line.get(3..).unwrap_or_default(),
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::to_string(&json!({
                    "entries": entries,
                    "raw": process.stdout,
                    "stderr": process.stderr,
                    "exit_code": process.exit_code,
                    "truncated": process.truncated,
                }))
                .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "git.diff" => {
                let paths = optional_tool_string_array(&call, "paths")?.unwrap_or_default();
                let mut args = vec![
                    "diff".into(),
                    "--no-ext-diff".into(),
                    "--no-textconv".into(),
                ];
                if !paths.is_empty() {
                    args.push("--".into());
                    args.extend(
                        paths
                            .iter()
                            .map(|path| safe_git_path(path))
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                }
                let process = self
                    .execute_process_tool(
                        &call,
                        context,
                        "git.diff",
                        self.git_executable()?,
                        tool_process_spec(
                            self.workspace.clone(),
                            args,
                            BTreeMap::new(),
                            None,
                            Some(64 * 1024),
                        ),
                    )
                    .await?;
                exit_code = process.exit_code;
                serde_json::to_string(&json!({
                    "diff": process.stdout,
                    "stderr": process.stderr,
                    "exit_code": process.exit_code,
                    "truncated": process.truncated,
                }))
                .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "git.show" => {
                let revision = optional_tool_string(&call, "rev")?.unwrap_or("HEAD");
                validate_git_revision(revision)?;
                let mut args = vec![
                    "show".into(),
                    "--no-ext-diff".into(),
                    "--no-textconv".into(),
                    "--stat".into(),
                    "--patch".into(),
                    revision.into(),
                ];
                if let Some(path) = optional_tool_string(&call, "path")? {
                    args.push("--".into());
                    args.push(safe_git_path(path)?);
                }
                let process = self
                    .execute_process_tool(
                        &call,
                        context,
                        "git.show",
                        self.git_executable()?,
                        tool_process_spec(
                            self.workspace.clone(),
                            args,
                            BTreeMap::new(),
                            None,
                            Some(64 * 1024),
                        ),
                    )
                    .await?;
                exit_code = process.exit_code;
                serde_json::to_string(&json!({
                    "output": process.stdout,
                    "stderr": process.stderr,
                    "exit_code": process.exit_code,
                    "truncated": process.truncated,
                }))
                .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "repo.map" => {
                self.execute_repository_tool(
                    &call,
                    context,
                    RepositoryOperation::Map {
                        path: optional_tool_string(&call, "path")?.unwrap_or(".").into(),
                        max_files: usize::try_from(
                            optional_tool_u64(&call, "max_files")?.unwrap_or(200),
                        )
                        .unwrap_or(1_000),
                    },
                )
                .await?
            }
            "repo.symbol_search" => {
                self.execute_repository_tool(
                    &call,
                    context,
                    RepositoryOperation::SymbolSearch {
                        pattern: required_tool_string(&call, "pattern")?.into(),
                        path: optional_tool_string(&call, "path")?.unwrap_or(".").into(),
                        max_results: usize::try_from(
                            optional_tool_u64(&call, "max_results")?.unwrap_or(100),
                        )
                        .unwrap_or(500),
                    },
                )
                .await?
            }
            "repo.references" => {
                self.execute_repository_tool(
                    &call,
                    context,
                    RepositoryOperation::References {
                        symbol: required_tool_string(&call, "symbol")?.into(),
                        path: optional_tool_string(&call, "path")?.unwrap_or(".").into(),
                        max_results: usize::try_from(
                            optional_tool_u64(&call, "max_results")?.unwrap_or(100),
                        )
                        .unwrap_or(500),
                    },
                )
                .await?
            }
            "repo.file_summary" => {
                self.execute_repository_tool(
                    &call,
                    context,
                    RepositoryOperation::FileSummary {
                        path: required_tool_string(&call, "path")?.into(),
                        max_lines: usize::try_from(
                            optional_tool_u64(&call, "max_lines")?.unwrap_or(120),
                        )
                        .unwrap_or(500),
                    },
                )
                .await?
            }
            "patch.preview" | "patch.apply" | "patch.reverse" => {
                self.execute_patch_tool(&call, context).await?
            }
            "shell.run" => {
                let command = optional_tool_string(&call, "command")?;
                let argv = optional_tool_string_array(&call, "argv")?;
                if command.is_some() == argv.is_some() {
                    return Err(ToolError::InvalidArguments {
                        tool: call.name.clone(),
                        message: "exactly one of command or argv is required".into(),
                    });
                }
                let (executable, args, invocation) = if let Some(command) = command {
                    let executable = self.shell_executable()?;
                    let args = shell_command_arguments(&executable, command)?;
                    (executable, args, json!({"command": command}))
                } else {
                    let argv = argv.expect("validated argv presence");
                    let requested = argv.first().ok_or_else(|| ToolError::InvalidArguments {
                        tool: call.name.clone(),
                        message: "argv must not be empty".into(),
                    })?;
                    if is_shell_wrapper(requested) {
                        reject_shell_startup_profiles(&call, &argv[1..])?;
                    }
                    let executable = self.resolve_executable(requested)?;
                    (
                        executable,
                        argv.iter().skip(1).cloned().collect(),
                        json!({"argv": argv}),
                    )
                };
                let cwd = model_workspace_path(
                    &self.workspace,
                    optional_tool_string(&call, "cwd")?.unwrap_or("."),
                )?;
                let mut environment = optional_tool_environment(&call, "env")?;
                reject_reserved_shell_environment(&call, &environment)?;
                let isolated = tempfile::Builder::new()
                    .prefix(".colossus-shell-")
                    .tempdir_in(&self.workspace)
                    .map_err(|error| {
                        ToolError::Failed(format!(
                            "cannot create isolated shell directory: {error}"
                        ))
                    })?;
                configure_shell_environment(
                    &mut environment,
                    isolated.path(),
                    &self.sanitized_command_path()?,
                );
                let process = self
                    .execute_process_tool(
                        &call,
                        context,
                        "shell.run",
                        executable,
                        tool_process_spec(
                            cwd,
                            args,
                            environment,
                            optional_tool_u64(&call, "timeout_ms")?,
                            optional_tool_u64(&call, "max_output_bytes")?,
                        ),
                    )
                    .await?;
                exit_code = process.exit_code;
                let mut command = vec![process.executable.display().to_string()];
                command.extend(process.args.clone());
                serde_json::to_string(&json!({
                    "invocation": invocation,
                    "resolved_argv": command,
                    "exit_code": process.exit_code,
                    "stdout": process.stdout,
                    "stderr": process.stderr,
                    "cwd": workspace_relative(&self.workspace, &process.cwd)?,
                    "truncated": process.truncated,
                    "observed_origins": process.observed_origins,
                }))
                .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "task.create" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::TaskCreate {
                        session_id,
                        title: required_tool_string(&call, "title")?.into(),
                        description: optional_tool_string(&call, "description")?
                            .unwrap_or_default()
                            .into(),
                        status: optional_tool_value(&call, "status")?
                            .unwrap_or(TaskStatus::Pending),
                    },
                )
                .await?
            }
            "task.update" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::TaskUpdate {
                        id: required_tool_string(&call, "id")?.into(),
                        title: optional_tool_string(&call, "title")?.map(str::to_owned),
                        description: optional_tool_string(&call, "description")?.map(str::to_owned),
                        status: optional_tool_value(&call, "status")?,
                    },
                )
                .await?
            }
            "task.list" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::TaskList {
                        session_id,
                        status: optional_tool_value(&call, "status")?,
                        limit: tool_limit(&call, 100)?,
                    },
                )
                .await?
            }
            "decision.create" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::DecisionCreate {
                        session_id,
                        title: required_tool_string(&call, "title")?.into(),
                        decision: required_tool_string(&call, "decision")?.into(),
                        source: DecisionSource::Agent,
                        priority: optional_tool_value(&call, "priority")?
                            .unwrap_or(DecisionPriority::Normal),
                        intent: optional_tool_string(&call, "intent")?
                            .unwrap_or_default()
                            .into(),
                        applies_when: optional_tool_string(&call, "applies_when")?
                            .unwrap_or_default()
                            .into(),
                        rationale: optional_tool_string(&call, "rationale")?
                            .unwrap_or_default()
                            .into(),
                        source_excerpt: optional_tool_string(&call, "source_excerpt")?
                            .unwrap_or_default()
                            .into(),
                    },
                )
                .await?
            }
            "decision.update" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::DecisionUpdate {
                        id: required_tool_string(&call, "id")?.into(),
                        title: optional_tool_string(&call, "title")?.map(str::to_owned),
                        decision: optional_tool_string(&call, "decision")?.map(str::to_owned),
                        priority: optional_tool_value(&call, "priority")?,
                        intent: optional_tool_string(&call, "intent")?.map(str::to_owned),
                        applies_when: optional_tool_string(&call, "applies_when")?
                            .map(str::to_owned),
                        rationale: optional_tool_string(&call, "rationale")?.map(str::to_owned),
                        source_excerpt: optional_tool_string(&call, "source_excerpt")?
                            .map(str::to_owned),
                    },
                )
                .await?
            }
            "decision.list" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::DecisionList {
                        session_id,
                        status: optional_tool_value(&call, "status")?,
                        limit: tool_limit(&call, 100)?,
                    },
                )
                .await?
            }
            "decision.archive" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::DecisionArchive {
                        id: required_tool_string(&call, "id")?.into(),
                    },
                )
                .await?
            }
            "decision.supersede" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::DecisionSupersede {
                        id: required_tool_string(&call, "id")?.into(),
                        title: required_tool_string(&call, "title")?.into(),
                        decision: required_tool_string(&call, "decision")?.into(),
                        source: DecisionSource::Agent,
                        priority: optional_tool_value(&call, "priority")?
                            .unwrap_or(DecisionPriority::Normal),
                        intent: optional_tool_string(&call, "intent")?
                            .unwrap_or_default()
                            .into(),
                        applies_when: optional_tool_string(&call, "applies_when")?
                            .unwrap_or_default()
                            .into(),
                        rationale: optional_tool_string(&call, "rationale")?
                            .unwrap_or_default()
                            .into(),
                        source_excerpt: optional_tool_string(&call, "source_excerpt")?
                            .unwrap_or_default()
                            .into(),
                    },
                )
                .await?
            }
            "agent.delegate" => {
                if context.subagent_id.is_some() {
                    return Err(ToolError::Denied(
                        "subagents cannot delegate recursively".into(),
                    ));
                }
                let session_id = Self::current_session(&context)?;
                let parent_run_id = context.run_id.clone().ok_or_else(|| {
                    ToolError::Denied("agent.delegate requires a parent run".into())
                })?;
                let allowed_tools = context.offered_tools.clone();
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::SubagentCreate {
                        session_id,
                        parent_run_id,
                        parent_call_id: call.call_id.clone(),
                        task: required_tool_string(&call, "task")?.into(),
                        role: "subagent_default".into(),
                        allowed_tools: Some(allowed_tools),
                    },
                )
                .await?
            }
            "agent.result" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::SubagentRead {
                        id: required_tool_string(&call, "id")?.into(),
                    },
                )
                .await?
            }
            "agent.list" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::SubagentList {
                        session_id,
                        status: optional_tool_value(&call, "status")?,
                        limit: tool_limit(&call, 100)?,
                    },
                )
                .await?
            }
            "goal.show" => {
                let id = context.goal_id.clone().ok_or_else(|| {
                    ToolError::Denied(
                        "goal.show is available only during an active goal run".into(),
                    )
                })?;
                self.execute_work_tool(&call, context, WorkOperation::GoalShow { id })
                    .await?
            }
            "goal.update" => {
                let id = context.goal_id.clone().ok_or_else(|| {
                    ToolError::Denied(
                        "goal.update is available only during an active goal run".into(),
                    )
                })?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::GoalUpdate {
                        id,
                        status: optional_tool_value(&call, "status")?.ok_or_else(|| {
                            ToolError::InvalidArguments {
                                tool: call.name.clone(),
                                message: "status is required".into(),
                            }
                        })?,
                        summary: optional_tool_string(&call, "summary")?
                            .unwrap_or_default()
                            .into(),
                        blocked_reason: optional_tool_string(&call, "blocked_reason")?
                            .unwrap_or_default()
                            .into(),
                    },
                )
                .await?
            }
            "plan.create" => {
                let session_id = Self::current_session(&context)?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::PlanCreate {
                        session_id,
                        prompt: required_tool_string(&call, "prompt")?.into(),
                        content: optional_tool_string(&call, "content")?
                            .unwrap_or_default()
                            .into(),
                        steps: tool_plan_steps(&call)?,
                    },
                )
                .await?
            }
            "plan.update" => {
                let id = context.draft_plan_id.clone().ok_or_else(|| {
                    ToolError::Denied(
                        "plan.update requires a runtime-bound Plan Mode draft target".into(),
                    )
                })?;
                let expected_revision = context.draft_plan_revision.ok_or_else(|| {
                    ToolError::Denied("plan.update requires a runtime-bound draft revision".into())
                })?;
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::PlanUpdate {
                        id,
                        expected_revision,
                        content: required_tool_string(&call, "content")?.into(),
                        steps: tool_plan_steps(&call)?,
                    },
                )
                .await?
            }
            "plan.show" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::PlanShow {
                        id: required_tool_string(&call, "id")?.into(),
                    },
                )
                .await?
            }
            "plan.approve_request" => {
                self.execute_work_tool(
                    &call,
                    context,
                    WorkOperation::PlanApprove {
                        id: required_tool_string(&call, "id")?.into(),
                    },
                )
                .await?
            }
            "memory.create" => {
                let session_id = Self::current_session(&context)?;
                let scope = match optional_tool_string(&call, "scope")?.unwrap_or("session") {
                    "global" => MemoryScope::Global,
                    "repository" => MemoryScope::Repository(self.repository_id.clone()),
                    "session" => MemoryScope::Session(session_id),
                    value => {
                        return Err(ToolError::InvalidArguments {
                            tool: call.name.clone(),
                            message: format!("unknown memory scope {value}"),
                        });
                    }
                };
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::Create {
                        scope,
                        kind: required_tool_string(&call, "kind")?.into(),
                        confidence: optional_tool_value(&call, "confidence")?.unwrap_or(1.0),
                        text: required_tool_string(&call, "text")?.into(),
                        rationale: optional_tool_string(&call, "rationale")?
                            .unwrap_or_default()
                            .into(),
                        expires_at: optional_tool_string(&call, "expires_at")?.map(str::to_owned),
                    },
                )
                .await?
            }
            "memory.update" => {
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::Update {
                        id: required_tool_string(&call, "id")?.into(),
                        text: optional_tool_string(&call, "text")?.map(str::to_owned),
                        rationale: optional_tool_string(&call, "rationale")?.map(str::to_owned),
                        confidence: optional_tool_value(&call, "confidence")?,
                    },
                )
                .await?
            }
            "memory.list" => {
                let session_id = Self::current_session(&context)?;
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::List {
                        status: optional_tool_value(&call, "status")?,
                        limit: tool_limit(&call, 100)?,
                        session_id: Some(session_id),
                        repository_id: Some(self.repository_id.clone()),
                    },
                )
                .await?
            }
            "memory.search" => {
                let session_id = Self::current_session(&context)?;
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::Search {
                        query: required_tool_string(&call, "query")?.into(),
                        session_id: Some(session_id),
                        repository_id: Some(self.repository_id.clone()),
                        limit: tool_limit(&call, 20)?,
                    },
                )
                .await?
            }
            "memory.archive" => {
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::Archive {
                        id: required_tool_string(&call, "id")?.into(),
                    },
                )
                .await?
            }
            "memory.supersede" => {
                self.execute_memory_tool(
                    &call,
                    context,
                    MemoryOperation::Supersede {
                        id: required_tool_string(&call, "id")?.into(),
                        text: required_tool_string(&call, "text")?.into(),
                        rationale: optional_tool_string(&call, "rationale")?
                            .unwrap_or_default()
                            .into(),
                    },
                )
                .await?
            }
            "skill.scaffold" => {
                self.execute_skill_tool(
                    &call,
                    context,
                    SkillOperation::Scaffold {
                        name: required_tool_string(&call, "name")?.into(),
                        description: required_tool_string(&call, "description")?.into(),
                        instructions: required_tool_string(&call, "instructions")?.into(),
                        resource_dirs: optional_tool_string_array(&call, "resource_dirs")?
                            .unwrap_or_default(),
                    },
                )
                .await?
            }
            "skill.inspect" => {
                self.execute_skill_tool(
                    &call,
                    context,
                    SkillOperation::Inspect {
                        name: required_tool_string(&call, "name")?.into(),
                    },
                )
                .await?
            }
            "skill.read" => {
                self.execute_skill_tool(
                    &call,
                    context,
                    SkillOperation::ReadFile {
                        name: required_tool_string(&call, "name")?.into(),
                        path: required_tool_string(&call, "path")?.into(),
                    },
                )
                .await?
            }
            "skill.write" => {
                self.execute_skill_tool(
                    &call,
                    context,
                    SkillOperation::WriteFile {
                        name: required_tool_string(&call, "name")?.into(),
                        path: required_tool_string(&call, "path")?.into(),
                        content: required_tool_string(&call, "content")?.into(),
                        expected_sha256: optional_tool_string(&call, "expected_sha256")?
                            .map(Into::into),
                    },
                )
                .await?
            }
            "skill.validate" => {
                let operation = if let Some(name) = optional_tool_string(&call, "name")? {
                    SkillOperation::ValidateInstalled { name: name.into() }
                } else {
                    SkillOperation::ValidateLocal {
                        path: required_tool_string(&call, "path")?.into(),
                    }
                };
                self.execute_skill_tool(&call, context, operation).await?
            }
            "skill.install" => {
                self.execute_skill_tool(
                    &call,
                    context,
                    SkillOperation::InstallLocal {
                        path: required_tool_string(&call, "path")?.into(),
                    },
                )
                .await?
            }
            "skill.resource.list" => {
                let active_skills = context.skill_ids.clone();
                self.execute_skill_tool(
                    &call,
                    context,
                    SkillOperation::ListResources {
                        skill_name: required_tool_string(&call, "name")?.into(),
                        active_skills,
                    },
                )
                .await?
            }
            "skill.resource.read" => {
                let active_skills = context.skill_ids.clone();
                self.execute_skill_tool(
                    &call,
                    context,
                    SkillOperation::ReadResource {
                        skill_name: required_tool_string(&call, "name")?.into(),
                        path: required_tool_string(&call, "path")?.into(),
                        active_skills,
                    },
                )
                .await?
            }
            "mcp.servers" => {
                let servers = self
                    .mcp
                    .as_deref()
                    .ok_or_else(|| ToolError::Failed("MCP adapter is unavailable".into()))?
                    .servers();
                serde_json::to_string(&servers)
                    .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "mcp.tools" => {
                self.discover_mcp_tool_output(
                    &call,
                    context,
                    optional_tool_string(&call, "server")?,
                )
                .await?
            }
            "mcp.call" => {
                let server = required_tool_string(&call, "server")?.to_owned();
                let tool = required_tool_string(&call, "tool")?.to_owned();
                let arguments = call.arguments.get("arguments").cloned().ok_or_else(|| {
                    ToolError::InvalidArguments {
                        tool: call.name.clone(),
                        message: "arguments must be an object".into(),
                    }
                })?;
                self.execute_mcp_tool(&call, context, &server, &tool, arguments)
                    .await?
            }
            "web.search" => {
                let query = required_tool_string(&call, "query")?.to_owned();
                let limit = usize::try_from(
                    optional_tool_u64(&call, "limit")?
                        .unwrap_or_else(|| u64::try_from(default_search_limit()).unwrap_or(10)),
                )
                .map_err(|_| ToolError::InvalidArguments {
                    tool: call.name.clone(),
                    message: "limit is too large".into(),
                })?;
                let response = self
                    .search
                    .as_deref()
                    .ok_or_else(|| ToolError::Failed("search provider is unavailable".into()))?
                    .search(
                        "agent",
                        model_actor(&call, &context),
                        SearchRequest { query, limit },
                        context,
                    )
                    .await
                    .map_err(search_tool_error)?;
                serde_json::to_string(&response)
                    .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            "network.http" | "web.fetch" | "docs.fetch" => {
                let url = required_tool_string(&call, "url")?;
                let mut request = effect_request(
                    model_actor(&call, &context),
                    "network.http",
                    url,
                    json!({"method": "GET", "headers": {"accept": "*/*"}}),
                );
                request.capabilities = vec!["network.http".into()];
                request.context = context;
                let result = self
                    .gateway
                    .execute(request, self.http.as_ref())
                    .await
                    .map_err(tool_gateway_error)?;
                bounded_tool_text(
                    &String::from_utf8(result.bytes)
                        .map_err(|_| ToolError::Failed("network.http returned non-UTF-8".into()))?,
                    1024 * 1024,
                )
            }
            name => {
                if let Some((output, code)) = self.execute_pack_tool(&call, context.clone()).await?
                {
                    exit_code = code;
                    output
                } else {
                    self.execute_integration_tool(&call, context)
                        .await?
                        .ok_or_else(|| ToolError::Unknown(name.into()))?
                }
            }
        };
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output,
            exit_code,
        })
    }
}
