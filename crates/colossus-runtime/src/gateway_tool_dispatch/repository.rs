use super::*;

impl GatewayToolExecutor {
    pub(super) async fn execute_repository(
        &self,
        call: ToolCall,
        context: ExecutionContext,
    ) -> Result<ToolResult, ToolError> {
        let mut exit_code = 0;
        let output = match call.name.as_str() {
            "git.status" => {
                let git = self.git_executable(self.danger_full_access(&context))?;
                let process = self
                    .execute_process_tool(
                        &call,
                        context,
                        "git.status",
                        git,
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
                let git = self.git_executable(self.danger_full_access(&context))?;
                let process = self
                    .execute_process_tool(
                        &call,
                        context,
                        "git.diff",
                        git,
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
                let git = self.git_executable(self.danger_full_access(&context))?;
                let process = self
                    .execute_process_tool(
                        &call,
                        context,
                        "git.show",
                        git,
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
