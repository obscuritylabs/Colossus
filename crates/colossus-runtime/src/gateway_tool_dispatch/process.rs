use super::*;

impl GatewayToolExecutor {
    pub(super) async fn execute_process(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let exit_code;
        let output = match call.name.as_str() {
            "shell.run" => {
                let danger_full_access = self.danger_full_access(&context);
                let command = optional_tool_string(&call, "command")?;
                let argv = optional_tool_string_array(&call, "argv")?;
                if command.is_some() == argv.is_some() {
                    return Err(ToolError::InvalidArguments {
                        tool: call.name.clone(),
                        message: "exactly one of command or argv is required".into(),
                    });
                }
                let (executable, args, invocation) = if let Some(command) = command {
                    let executable = self.shell_executable(danger_full_access)?;
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
                    let executable =
                        self.resolve_executable(requested, danger_full_access, &context)?;
                    (
                        executable,
                        argv.iter().skip(1).cloned().collect(),
                        json!({"argv": argv}),
                    )
                };
                let requested_cwd = optional_tool_string(&call, "cwd")?.unwrap_or(".");
                let cwd = if danger_full_access {
                    unrestricted_process_cwd(&self.workspace, requested_cwd)?
                } else if Path::new(requested_cwd).is_absolute() {
                    let cwd = fs::canonicalize(requested_cwd).map_err(|error| {
                        ToolError::Failed(format!("cannot resolve process cwd: {error}"))
                    })?;
                    if self
                        .selected_plugin_roots(&context)
                        .iter()
                        .any(|root| cwd.starts_with(root))
                    {
                        cwd
                    } else {
                        return Err(ToolError::Denied(
                            "shell cwd is not within a selected Agent Plugin".into(),
                        ));
                    }
                } else {
                    model_workspace_path(&self.workspace, requested_cwd)?
                };
                let mut environment = optional_tool_environment(&call, "env")?;
                let _isolated = if danger_full_access {
                    None
                } else {
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
                    Some(isolated)
                };
                let process = self
                    .execute_process_tool(
                        &call,
                        context.clone(),
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
                let displayed_cwd = if danger_full_access
                    || self
                        .selected_plugin_roots(&context)
                        .iter()
                        .any(|root| process.cwd.starts_with(root))
                {
                    process.cwd.display().to_string()
                } else {
                    workspace_relative(&self.workspace, &process.cwd)?
                };
                serde_json::to_string(&json!({
                    "invocation": invocation,
                    "resolved_argv": command,
                    "exit_code": process.exit_code,
                    "stdout": process.stdout,
                    "stderr": process.stderr,
                    "cwd": displayed_cwd,
                    "truncated": process.truncated,
                    "observed_origins": process.observed_origins,
                }))
                .map_err(|error| ToolError::Failed(error.to_string()))?
            }
            name => return Err(ToolError::Unknown(name.into())),
        };
        Ok(ToolResult {
            call_id: call.call_id,
            name: call.name,
            output,
            exit_code,
        })
    }
}
