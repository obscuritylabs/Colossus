use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PackToolEffectInput {
    pub(super) pack: String,
    pub(super) version: String,
    pub(super) manifest_sha256: String,
    pub(super) tool: String,
    pub(super) executable: PathBuf,
    pub(super) cwd: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) permissions: Vec<String>,
}

pub(super) struct PackProcessExecutor {
    pub(super) declarations: BTreeMap<String, PackProcessDeclaration>,
    pub(super) process: Arc<dyn EffectExecutor>,
}

impl PackProcessExecutor {
    pub(super) fn new(
        declarations: BTreeMap<String, PackProcessDeclaration>,
        process: Arc<dyn EffectExecutor>,
    ) -> Self {
        Self {
            declarations,
            process,
        }
    }

    pub(super) fn invocation(
        &self,
        tool: &str,
    ) -> Option<(PackProcessDeclaration, PackToolEffectInput)> {
        let declaration = self.declarations.get(tool)?.clone();
        let input = PackToolEffectInput {
            pack: declaration.pack.clone(),
            version: declaration.version.clone(),
            manifest_sha256: declaration.manifest_sha256.clone(),
            tool: declaration.tool.clone(),
            executable: declaration.executable.clone(),
            cwd: declaration.cwd.clone(),
            args: declaration.args.clone(),
            environment: declaration.environment.clone(),
            permissions: declaration.permissions.clone(),
        };
        Some((declaration, input))
    }
}

#[async_trait]
impl EffectExecutor for PackProcessExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let input: PackToolEffectInput = serde_json::from_value(request.content.clone())
            .map_err(|error| ExecutionError::Failed(error.to_string()))?;
        let declaration = self
            .declarations
            .get(&input.tool)
            .ok_or_else(|| ExecutionError::Failed("pack tool is no longer active".into()))?;
        let expected = PackToolEffectInput {
            pack: declaration.pack.clone(),
            version: declaration.version.clone(),
            manifest_sha256: declaration.manifest_sha256.clone(),
            tool: declaration.tool.clone(),
            executable: declaration.executable.clone(),
            cwd: declaration.cwd.clone(),
            args: declaration.args.clone(),
            environment: declaration.environment.clone(),
            permissions: declaration.permissions.clone(),
        };
        if request.action != declaration.action
            || request.resource != declaration.executable.display().to_string()
            || serde_json::to_value(&input).map_err(execution_failure)?
                != serde_json::to_value(&expected).map_err(execution_failure)?
        {
            return Err(ExecutionError::Failed(
                "pack tool request does not match its verified declaration".into(),
            ));
        }
        let expected_refs = declaration
            .environment
            .values()
            .cloned()
            .collect::<BTreeSet<_>>();
        let actual_refs = request
            .credential_references
            .iter()
            .map(|reference| reference.reference.clone())
            .collect::<BTreeSet<_>>();
        if expected_refs != actual_refs
            || request
                .credential_references
                .iter()
                .any(|reference| reference.value_hash.is_some())
        {
            return Err(ExecutionError::Failed(
                "pack tool credential references do not match its verified declaration".into(),
            ));
        }
        let mut secrets = Vec::new();
        let mut environment = BTreeMap::new();
        for (child_name, reference) in &declaration.environment {
            let variable = reference.strip_prefix("env:").ok_or_else(|| {
                ExecutionError::Failed("pack credential reference must use env:VARIABLE".into())
            })?;
            let value = std::env::var(variable).map_err(|_| {
                ExecutionError::Failed(format!(
                    "pack credential reference {reference} is unresolved"
                ))
            })?;
            secrets.push(value.as_bytes().to_vec());
            environment.insert(child_name.clone(), value);
        }
        let mut process_request = request.clone();
        process_request.content = serde_json::to_value(ProcessSpec {
            cwd: declaration.cwd.clone(),
            args: declaration.args.clone(),
            environment,
            stdin_base64: None,
            timeout_ms: None,
            max_output_bytes: None,
        })
        .map_err(execution_failure)?;
        let mut result = self.process.execute(&process_request, permit).await?;
        redact_process_credentials(&mut result.bytes, &secrets)?;
        Ok(result)
    }
}

pub(super) fn execution_failure(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

pub(super) fn redact_process_credentials(
    bytes: &mut Vec<u8>,
    secrets: &[Vec<u8>],
) -> Result<(), ExecutionError> {
    if secrets.is_empty() {
        return Ok(());
    }
    let mut value: Value = serde_json::from_slice(bytes).map_err(execution_failure)?;
    for field in ["stdout_base64", "stderr_base64"] {
        let Some(encoded) = value.get(field).and_then(Value::as_str) else {
            continue;
        };
        let mut decoded = BASE64.decode(encoded).map_err(execution_failure)?;
        for secret in secrets {
            decoded = redact_bytes(&decoded, secret);
        }
        value[field] = Value::String(BASE64.encode(decoded));
    }
    *bytes = serde_json::to_vec(&value).map_err(execution_failure)?;
    Ok(())
}

pub(super) fn redact_bytes(bytes: &[u8], secret: &[u8]) -> Vec<u8> {
    if secret.is_empty() || secret.len() > bytes.len() {
        return bytes.to_vec();
    }
    let mut redacted = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset..].starts_with(secret) {
            redacted.extend_from_slice(b"[REDACTED]");
            offset += secret.len();
        } else {
            redacted.push(bytes[offset]);
            offset += 1;
        }
    }
    redacted
}

pub(super) struct ProcessToolOutput {
    pub(super) executable: PathBuf,
    pub(super) cwd: PathBuf,
    pub(super) args: Vec<String>,
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) exit_code: i32,
    pub(super) truncated: bool,
    pub(super) observed_origins: Vec<String>,
}
