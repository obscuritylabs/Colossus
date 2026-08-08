use super::*;

pub(super) fn search_tool_error(error: SearchError) -> ToolError {
    match error {
        SearchError::Denied(message) => ToolError::Denied(message),
        SearchError::OutcomeUnknown(message) => ToolError::OutcomeUnknown(message),
        SearchError::Unavailable(message)
        | SearchError::Configuration(message)
        | SearchError::Failed(message) => ToolError::Failed(message),
    }
}

pub(super) fn required_tool_string<'a>(
    call: &'a ToolCall,
    field: &str,
) -> Result<&'a str, ToolError> {
    call.arguments
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: format!("{field} must be a string"),
        })
}

pub(super) fn optional_tool_string<'a>(
    call: &'a ToolCall,
    field: &str,
) -> Result<Option<&'a str>, ToolError> {
    match call.arguments.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: format!("{field} must be a string"),
        }),
    }
}

pub(super) fn optional_tool_bool(call: &ToolCall, field: &str) -> Result<Option<bool>, ToolError> {
    match call.arguments.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: format!("{field} must be a boolean"),
        }),
    }
}

pub(super) fn tool_plan_steps(call: &ToolCall) -> Result<Vec<PlanStep>, ToolError> {
    let values = call
        .arguments
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: "steps must be an array".into(),
        })?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let object = value
                .as_object()
                .ok_or_else(|| ToolError::InvalidArguments {
                    tool: call.name.clone(),
                    message: "each plan step must be an object".into(),
                })?;
            let title = object.get("title").and_then(Value::as_str).ok_or_else(|| {
                ToolError::InvalidArguments {
                    tool: call.name.clone(),
                    message: "each plan step title must be a string".into(),
                }
            })?;
            let detail = object
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let requires_mutation = object
                .get("requires_mutation")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok(PlanStep {
                index: u32::try_from(index + 1).map_err(|_| ToolError::InvalidArguments {
                    tool: call.name.clone(),
                    message: "too many plan steps".into(),
                })?,
                title: title.into(),
                detail: detail.into(),
                requires_mutation,
            })
        })
        .collect()
}

pub(super) fn optional_tool_u64(call: &ToolCall, field: &str) -> Result<Option<u64>, ToolError> {
    match call.arguments.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => {
            value
                .as_u64()
                .map(Some)
                .ok_or_else(|| ToolError::InvalidArguments {
                    tool: call.name.clone(),
                    message: format!("{field} must be a non-negative integer"),
                })
        }
        Some(_) => Err(ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: format!("{field} must be an integer"),
        }),
    }
}

pub(super) fn optional_tool_value<T: serde::de::DeserializeOwned>(
    call: &ToolCall,
    field: &str,
) -> Result<Option<T>, ToolError> {
    call.arguments
        .get(field)
        .cloned()
        .map(|value| {
            serde_json::from_value(value).map_err(|error| ToolError::InvalidArguments {
                tool: call.name.clone(),
                message: format!("{field} is invalid: {error}"),
            })
        })
        .transpose()
}

pub(super) fn tool_limit(call: &ToolCall, default: usize) -> Result<usize, ToolError> {
    optional_tool_u64(call, "limit")?.map_or(Ok(default), |value| {
        usize::try_from(value).map_err(|error| ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: format!("limit is invalid: {error}"),
        })
    })
}

pub(super) fn optional_tool_string_array(
    call: &ToolCall,
    field: &str,
) -> Result<Option<Vec<String>>, ToolError> {
    let Some(value) = call.arguments.get(field) else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: format!("{field} must be an array"),
        })?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| ToolError::InvalidArguments {
                    tool: call.name.clone(),
                    message: format!("{field} entries must be strings"),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(values))
}

pub(super) fn optional_tool_environment(
    call: &ToolCall,
    field: &str,
) -> Result<BTreeMap<String, String>, ToolError> {
    let Some(value) = call.arguments.get(field) else {
        return Ok(BTreeMap::new());
    };
    value
        .as_object()
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: format!("{field} must be an object"),
        })?
        .iter()
        .map(|(name, value)| {
            value
                .as_str()
                .map(|value| (name.clone(), value.to_owned()))
                .ok_or_else(|| ToolError::InvalidArguments {
                    tool: call.name.clone(),
                    message: format!("{field}.{name} must be a string"),
                })
        })
        .collect()
}

pub(super) fn tool_process_spec(
    cwd: PathBuf,
    args: Vec<String>,
    environment: BTreeMap<String, String>,
    timeout_ms: Option<u64>,
    max_output_bytes: Option<u64>,
) -> ProcessSpec {
    ProcessSpec {
        cwd,
        args,
        environment,
        stdin_base64: None,
        timeout_ms,
        max_output_bytes,
    }
}

pub(super) fn safe_git_path(value: &str) -> Result<String, ToolError> {
    let path = Path::new(value);
    if value.starts_with(':')
        || value.contains('\0')
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(component, std::path::Component::ParentDir)
                || matches!(component.as_os_str().to_str(), Some(".git" | ".colossus"))
        })
    {
        return Err(ToolError::Denied(
            "Git pathspecs must stay inside the workspace and outside control state".into(),
        ));
    }
    Ok(value.into())
}

pub(super) fn validate_git_revision(value: &str) -> Result<(), ToolError> {
    if value.starts_with('-')
        || value.contains('\0')
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'.' | b'_' | b'/' | b'-' | b'^' | b'~' | b':' | b'@' | b'{' | b'}'
                )
        })
    {
        return Err(ToolError::Denied(
            "Git revision contains an option or unsupported character".into(),
        ));
    }
    Ok(())
}

pub(super) fn is_shell_wrapper(value: &str) -> bool {
    Path::new(value)
        .file_stem()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "sh" | "bash"
                    | "zsh"
                    | "fish"
                    | "dash"
                    | "ksh"
                    | "cmd"
                    | "powershell"
                    | "pwsh"
                    | "wscript"
                    | "cscript"
            )
        })
}

pub(super) fn shell_command_arguments(
    executable: &Path,
    command: &str,
) -> Result<Vec<String>, ToolError> {
    let shell = executable
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match shell.as_str() {
        "powershell" | "pwsh" => Ok(vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            command.into(),
        ]),
        "cmd" => Ok(vec!["/D".into(), "/S".into(), "/C".into(), command.into()]),
        "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" => Ok(vec!["-c".into(), command.into()]),
        _ => Err(ToolError::Denied(
            "configured command interpreter is not a supported platform shell".into(),
        )),
    }
}

pub(super) fn reject_shell_startup_profiles(
    call: &ToolCall,
    arguments: &[String],
) -> Result<(), ToolError> {
    if arguments.iter().any(|argument| {
        matches!(
            argument.as_str(),
            "-l" | "--login" | "-i" | "--interactive" | "--noprofile" | "--rcfile" | "--init-file"
        ) || argument.starts_with("--rcfile=")
            || argument.starts_with("--init-file=")
    }) {
        return Err(ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: "shell wrappers must be non-interactive and may not load startup profiles"
                .into(),
        });
    }
    Ok(())
}

pub(super) fn reject_reserved_shell_environment(
    call: &ToolCall,
    environment: &BTreeMap<String, String>,
) -> Result<(), ToolError> {
    const RESERVED: [&str; 10] = [
        "HOME",
        "PATH",
        "TEMP",
        "TMP",
        "TMPDIR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "USERPROFILE",
    ];
    if let Some(name) = environment.keys().find(|name| {
        RESERVED
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
    }) {
        return Err(ToolError::InvalidArguments {
            tool: call.name.clone(),
            message: format!("shell environment variable {name} is reserved by the sandbox"),
        });
    }
    Ok(())
}

pub(super) fn configure_shell_environment(
    environment: &mut BTreeMap<String, String>,
    isolated: &Path,
    sanitized_path: &str,
) {
    #[cfg(not(target_os = "windows"))]
    {
        let isolated = isolated.display().to_string();
        environment.insert("HOME".into(), isolated.clone());
        environment.insert("TMPDIR".into(), isolated);
        environment.insert("PATH".into(), sanitized_path.into());
    }
    #[cfg(target_os = "windows")]
    {
        let _ = (environment, isolated, sanitized_path);
    }
}

pub(super) fn model_workspace_path(workspace: &Path, input: &str) -> Result<PathBuf, ToolError> {
    Ok(workspace.join(model_workspace_relative(workspace, input)?))
}

pub(super) fn unrestricted_process_cwd(
    workspace: &Path,
    input: &str,
) -> Result<PathBuf, ToolError> {
    if input.is_empty() || input.contains('\0') {
        return Err(ToolError::InvalidArguments {
            tool: "shell.run".into(),
            message: "cwd must name a directory".into(),
        });
    }
    let input = Path::new(input);
    let candidate = if input.is_absolute() {
        input.to_path_buf()
    } else {
        workspace.join(input)
    };
    let cwd = fs::canonicalize(&candidate)
        .map_err(|error| ToolError::Failed(format!("cannot resolve process cwd: {error}")))?;
    if !cwd.is_dir() {
        return Err(ToolError::InvalidArguments {
            tool: "shell.run".into(),
            message: "cwd must name a directory".into(),
        });
    }
    Ok(cwd)
}

/// Normalizes a model-supplied path into its workspace-relative spelling.
///
/// Absolute paths that resolve inside the workspace are rewritten to the relative
/// spelling so downstream executors, which reject absolute paths outright, observe a
/// confined path. Absolute paths outside the workspace, parent traversal, and the
/// `.colossus` control state remain denied.
pub(super) fn model_workspace_relative(
    workspace: &Path,
    input: &str,
) -> Result<PathBuf, ToolError> {
    let requested = Path::new(input);
    let requested = if requested.is_absolute() {
        strip_workspace_prefix(workspace, requested)?
    } else {
        requested
    };
    if requested.components().any(|component| {
        matches!(component, std::path::Component::ParentDir) || component.as_os_str() == ".colossus"
    }) {
        return Err(model_workspace_denied());
    }
    Ok(requested.to_path_buf())
}

fn model_workspace_denied() -> ToolError {
    ToolError::Denied(
        "model filesystem paths must be workspace-relative and outside .colossus".into(),
    )
}

fn strip_workspace_prefix<'input>(
    workspace: &Path,
    requested: &'input Path,
) -> Result<&'input Path, ToolError> {
    requested
        .strip_prefix(workspace)
        .ok()
        .or_else(|| equivalent_spelling_relative(workspace, requested))
        .ok_or_else(model_workspace_denied)
}

/// Unix path spellings are compared byte for byte, so only the lexical prefix applies.
#[cfg(not(target_os = "windows"))]
fn equivalent_spelling_relative<'input>(
    _workspace: &Path,
    _requested: &'input Path,
) -> Option<&'input Path> {
    None
}

/// Compares Windows paths that name the same location with different spellings.
///
/// The workspace is stored canonically, which yields the extended-length form
/// (`\\?\C:\repo`), while models emit the conventional form (`C:\repo\file`). Prefix
/// components are reduced to their conventional spelling and comparisons ignore case,
/// matching Windows filesystem semantics.
#[cfg(target_os = "windows")]
fn equivalent_spelling_relative<'input>(
    workspace: &Path,
    requested: &'input Path,
) -> Option<&'input Path> {
    fn comparable(component: std::path::Component<'_>) -> Option<String> {
        let text = match component {
            std::path::Component::Prefix(prefix) => {
                let raw = prefix.as_os_str().to_str()?;
                raw.strip_prefix(r"\\?\UNC\")
                    .map(|rest| format!(r"\\{rest}"))
                    .unwrap_or_else(|| raw.strip_prefix(r"\\?\").unwrap_or(raw).to_owned())
            }
            other => other.as_os_str().to_str()?.to_owned(),
        };
        Some(text.to_lowercase())
    }

    let mut remainder = requested.components();
    for expected in workspace.components() {
        if comparable(remainder.next()?)? != comparable(expected)? {
            return None;
        }
    }
    Some(remainder.as_path())
}

pub(super) fn workspace_relative(workspace: &Path, path: &Path) -> Result<String, ToolError> {
    let relative = path
        .strip_prefix(workspace)
        .map_err(|_| ToolError::Denied("filesystem result escaped the active workspace".into()))?;
    if relative.as_os_str().is_empty() {
        Ok(".".into())
    } else {
        Ok(relative.to_string_lossy().into_owned())
    }
}

pub(super) fn bounded_tool_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.into();
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text[..end].into()
}

pub(super) fn goal_objective_from_plan(plan: &PlanRecord) -> String {
    let mut objective = format!(
        "Execute approved plan {}.\n\nOriginal request:\n{}",
        plan.id, plan.prompt
    );
    if !plan.content.trim().is_empty() {
        objective.push_str("\n\nApproved plan:\n");
        objective.push_str(&plan.content);
    }
    objective.push_str("\n\nOrdered steps:");
    for step in &plan.steps {
        objective.push_str(&format!(
            "\n{}. {}{}",
            step.index,
            step.title,
            if step.requires_mutation {
                " [mutation]"
            } else {
                ""
            }
        ));
        if !step.detail.is_empty() {
            objective.push_str(" — ");
            objective.push_str(&step.detail);
        }
    }
    bounded_tool_text(&objective, 64 * 1024)
}

pub(super) fn model_actor(call: &ToolCall, context: &ExecutionContext) -> Actor {
    Actor {
        actor_type: if context.subagent_id.is_some() {
            ActorType::Subagent
        } else {
            ActorType::Model
        },
        id: context.subagent_id.as_ref().map_or_else(
            || format!("tool-call:{}", call.call_id),
            |id| format!("subagent:{id}:tool-call:{}", call.call_id),
        ),
    }
}

pub(super) fn terminal_actor() -> Actor {
    Actor {
        actor_type: ActorType::User,
        id: "terminal-user".into(),
    }
}

pub(super) fn tool_gateway_error(error: GatewayError) -> ToolError {
    match error {
        GatewayError::Denied(message) | GatewayError::Approval(message) => {
            ToolError::Denied(message)
        }
        GatewayError::OutcomeUnknown(message) => ToolError::OutcomeUnknown(message),
        error => ToolError::Failed(error.to_string()),
    }
}

pub(super) fn mcp_runtime_tool_error(error: RuntimeError) -> ToolError {
    match error {
        RuntimeError::Gateway(error) => tool_gateway_error(error),
        RuntimeError::Mcp(McpError::UnknownServer(message) | McpError::ToolDenied(message)) => {
            ToolError::Denied(message)
        }
        RuntimeError::Mcp(McpError::InvalidArguments(message)) => ToolError::InvalidArguments {
            tool: "mcp.call".into(),
            message,
        },
        error => ToolError::Failed(error.to_string()),
    }
}
