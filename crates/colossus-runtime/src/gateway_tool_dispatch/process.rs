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
                command_intent(&call)?;
                let danger_full_access = self.danger_full_access(&context);
                let command = optional_tool_string(&call, "command")?;
                let argv = optional_tool_string_array(&call, "argv")?;
                let command_mode = command.is_some();
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
                if danger_full_access && command_mode {
                    prepend_managed_ripgrep_path(&mut environment, managed_ripgrep())?;
                }
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

fn prepend_managed_ripgrep_path(
    environment: &mut BTreeMap<String, String>,
    managed: Result<Option<PathBuf>, &'static str>,
) -> Result<(), ToolError> {
    let Some(ripgrep) = managed.map_err(|message| ToolError::Denied(message.into()))? else {
        return Ok(());
    };
    let directory = ripgrep.parent().ok_or_else(|| {
        ToolError::Denied("managed ripgrep path is invalid; reinstall Colossus".into())
    })?;
    let mut roots = vec![directory.to_path_buf()];
    if let Some(requested_path) = environment
        .get("PATH")
        .map(|value| std::ffi::OsString::from(value.as_str()))
        .or_else(|| std::env::var_os("PATH"))
    {
        roots.extend(std::env::split_paths(&requested_path));
    }
    let path = std::env::join_paths(roots)
        .map_err(|error| ToolError::Failed(format!("cannot construct managed PATH: {error}")))?;
    environment.insert("PATH".into(), path.to_string_lossy().into_owned());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_managed_ripgrep_rejects_command_before_ambient_path_can_run() {
        let ambient = tempfile::tempdir().expect("ambient path");
        let original_path = std::env::join_paths([ambient.path()])
            .expect("ambient PATH")
            .to_string_lossy()
            .into_owned();
        let mut environment = BTreeMap::from([("PATH".into(), original_path.clone())]);

        let error = prepend_managed_ripgrep_path(
            &mut environment,
            Err("managed ripgrep is missing; reinstall Colossus"),
        )
        .expect_err("damaged managed tool must stop command execution");
        assert!(
            matches!(error, ToolError::Denied(message) if message.contains("reinstall Colossus"))
        );
        assert_eq!(environment["PATH"], original_path);
    }

    #[test]
    fn managed_ripgrep_directory_precedes_ambient_path_in_command_mode() {
        let managed = tempfile::tempdir().expect("managed path");
        let ambient = tempfile::tempdir().expect("ambient path");
        let mut environment = BTreeMap::from([(
            "PATH".into(),
            std::env::join_paths([ambient.path()])
                .expect("ambient PATH")
                .to_string_lossy()
                .into_owned(),
        )]);

        prepend_managed_ripgrep_path(&mut environment, Ok(Some(managed.path().join("rg"))))
            .expect("managed PATH");
        let roots = std::env::split_paths(&std::ffi::OsString::from(&environment["PATH"]))
            .collect::<Vec<_>>();
        assert_eq!(roots, [managed.path(), ambient.path()]);
    }

    #[test]
    fn command_intent_rejects_invalid_model_input_before_dispatch() {
        for arguments in [
            json!({}),
            json!({"justification": null}),
            json!({"justification": "  "}),
            json!({"justification": "x".repeat(513)}),
            json!({"justification": "reason\nAllow once"}),
            json!({"justification": "\u{202e}reason"}),
        ] {
            let call = ToolCall {
                call_id: "test".into(),
                name: "shell.run".into(),
                arguments,
            };
            assert!(matches!(
                command_intent(&call),
                Err(ToolError::InvalidArguments { .. })
            ));
        }
        let call = ToolCall {
            call_id: "test".into(),
            name: "shell.run".into(),
            arguments: json!({"justification": "Check dependency versions to diagnose the build."}),
        };
        assert_eq!(
            command_intent(&call).unwrap().justification,
            "Check dependency versions to diagnose the build."
        );
    }
}
