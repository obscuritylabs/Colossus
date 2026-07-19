use super::*;

/// Permit-bound process executor using an authenticated one-shot helper.
pub struct SandboxProcessExecutor {
    config: SandboxExecutorConfig,
    job_key: [u8; 32],
}

pub(super) struct OciCancellationGuard {
    pub(super) runtime: Option<PathBuf>,
    pub(super) resources: OciResourceNames,
    pub(super) armed: bool,
}

impl OciCancellationGuard {
    fn new(runtime: Option<PathBuf>, resources: OciResourceNames, armed: bool) -> Self {
        Self {
            runtime,
            resources,
            armed,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for OciCancellationGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(runtime) = self.runtime.clone() else {
            return;
        };
        let resources = self.resources.clone();
        thread::spawn(move || {
            cleanup_oci_resources(&runtime, &resources);
        });
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct OciResourceNames {
    pub(super) workload: String,
    pub(super) proxy: String,
    pub(super) internal_network: String,
    pub(super) egress_network: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OciRuntimeKind {
    Docker,
    Podman,
}

pub(super) fn oci_runtime_kind(runtime: &Path) -> Option<OciRuntimeKind> {
    match runtime
        .file_stem()
        .and_then(|name| name.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "docker" => Some(OciRuntimeKind::Docker),
        "podman" | "podman-remote" => Some(OciRuntimeKind::Podman),
        _ => None,
    }
}

#[cfg(unix)]
pub(super) fn oci_mount_identity(cwd: &Path) -> Result<(u32, u32), SandboxHelperError> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata =
        fs::metadata(cwd).map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
    Ok((metadata.uid(), metadata.gid()))
}

#[cfg(not(unix))]
pub(super) fn oci_mount_identity(_cwd: &Path) -> Result<(u32, u32), SandboxHelperError> {
    Err(SandboxHelperError::Setup(
        "OCI execution is unavailable on this platform".into(),
    ))
}

pub(super) fn oci_remove_arguments(runtime: &Path, name: &str) -> Option<Vec<String>> {
    let kind = oci_runtime_kind(runtime)?;
    let mut arguments = vec!["container".into(), "rm".into(), "--force".into()];
    if kind == OciRuntimeKind::Podman {
        arguments.extend(["--time".into(), "0".into()]);
    }
    arguments.push(name.into());
    Some(arguments)
}

impl SandboxProcessExecutor {
    /// Construct a process executor with a private IPC authentication key.
    pub fn new(config: SandboxExecutorConfig, job_key: [u8; 32]) -> Self {
        Self { config, job_key }
    }
}

pub(super) fn is_sandbox_process_action(action: &str) -> bool {
    action.starts_with("pack.tool.")
        || action.starts_with("pack.mcp.")
        || matches!(
            action,
            "process.spawn"
                | "shell.run"
                | "git.status"
                | "git.diff"
                | "git.show"
                | "mcp.tools"
                | "mcp.call"
        )
}

pub(super) fn sandbox_helper_budget(
    obligations: &PolicyObligations,
    effective_timeout_ms: u64,
) -> u64 {
    let cleanup_reserve_ms =
        if obligations.sandbox_backend == "oci" && !obligations.network_destinations.is_empty() {
            OCI_NETWORK_CLEANUP_RESERVE_MS
        } else if obligations.sandbox_backend == "oci" {
            OCI_CLEANUP_RESERVE_MS
        } else if obligations.sandbox_backend == "windows_job" {
            WINDOWS_JOB_CLEANUP_RESERVE_MS
        } else {
            NATIVE_CLEANUP_RESERVE_MS
        };
    obligations
        .timeout_ms
        .min(effective_timeout_ms)
        .saturating_sub(cleanup_reserve_ms)
        .max(1)
}

#[async_trait]
impl EffectExecutor for SandboxProcessExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if !is_sandbox_process_action(&request.action) {
            return Err(adapter_failure("process executor received another action"));
        }
        let mut spec: ProcessSpec = serde_json::from_value(request.content.clone())
            .map_err(|error| adapter_failure(format!("invalid process request: {error}")))?;
        validate_process_spec(&spec, &request.resource, permit.obligations())?;
        normalize_path_arguments(&mut spec, permit.obligations())?;
        let effective_timeout_ms = spec.timeout_ms.unwrap_or(permit.obligations().timeout_ms);
        let effective_output_bytes = spec
            .max_output_bytes
            .unwrap_or(permit.obligations().max_output_bytes);
        if permit.obligations().sandbox_backend == "oci"
            && effective_timeout_ms < MIN_OCI_EFFECT_TIMEOUT_MS
        {
            return Err(adapter_failure(format!(
                "OCI process execution requires at least {MIN_OCI_EFFECT_TIMEOUT_MS}ms"
            )));
        }
        if permit.obligations().sandbox_backend == "oci"
            && !permit.obligations().network_destinations.is_empty()
            && effective_timeout_ms < MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS
        {
            return Err(adapter_failure(format!(
                "networked OCI process execution requires at least {MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS}ms"
            )));
        }
        if permit.obligations().sandbox_backend == "windows_job"
            && effective_timeout_ms < MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS
        {
            return Err(adapter_failure(format!(
                "Windows Job Object process execution requires at least {MIN_WINDOWS_JOB_EFFECT_TIMEOUT_MS}ms"
            )));
        }
        let proxy_credential = if permit.obligations().network_destinations.is_empty()
            || permit.obligations().sandbox_backend == "oci"
        {
            None
        } else {
            let mut mac = HmacSha256::new_from_slice(&self.job_key).map_err(adapter_failure)?;
            mac.update(b"colossus-process-proxy-v1\0");
            mac.update(permit.request_hash().as_bytes());
            mac.update(b"\0");
            mac.update(permit.nonce().as_bytes());
            Some(hex::encode(mac.finalize().into_bytes()))
        };
        let proxy = if permit.obligations().network_destinations.is_empty() {
            None
        } else if matches!(
            permit.obligations().sandbox_backend.as_str(),
            "native" | "windows_job"
        ) {
            Some(
                AllowlistProxy::start_authenticated(
                    permit.obligations().network_destinations.clone(),
                    proxy_credential.as_deref().expect("credential is present"),
                )
                .await?,
            )
        } else if permit.obligations().sandbox_backend == "oci" {
            if self.config.oci_proxy_image.is_none() {
                return Err(adapter_failure(
                    "networked OCI process execution requires an immutable proxy image",
                ));
            }
            None
        } else {
            return Err(adapter_failure(
                "networked process execution currently requires the native proxy-only backend",
            ));
        };
        let helper_budget = sandbox_helper_budget(permit.obligations(), effective_timeout_ms);
        let mut job_obligations = permit.obligations().clone();
        job_obligations.timeout_ms = effective_timeout_ms;
        job_obligations.max_output_bytes = effective_output_bytes;
        let temporary_root = sandbox_temporary_root(&job_obligations.sandbox_backend)?;
        let job = SandboxJob {
            schema_version: 2,
            job_id: Uuid::now_v7().to_string(),
            request_id: request.request_id.clone(),
            request_hash: permit.request_hash().into(),
            decision_id: permit.decision_id().into(),
            permit_nonce: permit.nonce().into(),
            permit_expires_at_unix_ms: permit.expires_at_unix_ms(),
            executable: PathBuf::from(&request.resource),
            process: spec,
            obligations: job_obligations,
            timeout_ms: helper_budget,
            proxy_port: proxy.as_ref().map(AllowlistProxy::port),
            proxy_credential,
            oci_runtime: self.config.oci_runtime.clone(),
            oci_image: self.config.oci_image.clone(),
            oci_proxy_image: self.config.oci_proxy_image.clone(),
            temporary_root,
        };
        let resources = oci_resource_names(&job.job_id);
        let is_oci = job.obligations.sandbox_backend == "oci";
        let mut cleanup_guard =
            OciCancellationGuard::new(self.config.oci_runtime.clone(), resources.clone(), is_oci);
        let signed = SignedSandboxJob::sign(job, &self.job_key)?;
        let encoded = serde_json::to_vec(&signed).map_err(adapter_failure)?;
        if encoded.len() > MAX_JOB_BYTES {
            return Err(adapter_failure("sandbox helper job exceeds IPC bound"));
        }
        let mut command = TokioCommand::new(&self.config.helper_executable);
        command
            .arg("__sandbox-helper")
            .env_clear()
            .env(HELPER_KEY_VARIABLE, hex::encode(self.job_key))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(target_os = "windows")]
        for name in ["SystemRoot", "WINDIR", "LOCALAPPDATA"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut child = command.spawn().map_err(adapter_failure)?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| adapter_failure("sandbox helper stdin is absent"))?;
        stdin.write_all(&encoded).await.map_err(adapter_failure)?;
        drop(stdin);
        let output = match child.wait_with_output().await {
            Ok(output) => output,
            Err(error) => {
                if is_oci
                    && !ensure_oci_resources_absent_async(
                        self.config.oci_runtime.as_deref(),
                        &resources,
                    )
                    .await
                {
                    return Err(ExecutionError::OutcomeUnknown(format!(
                        "sandbox helper failed and OCI cleanup could not be confirmed: {error}"
                    )));
                }
                cleanup_guard.disarm();
                return Err(adapter_failure(error));
            }
        };
        if !output.status.success() {
            if is_oci
                && !ensure_oci_resources_absent_async(
                    self.config.oci_runtime.as_deref(),
                    &resources,
                )
                .await
            {
                return Err(ExecutionError::OutcomeUnknown(
                    "sandbox helper failed and OCI cleanup could not be confirmed".into(),
                ));
            }
            cleanup_guard.disarm();
            return Err(adapter_failure(format!(
                "sandbox helper failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let mut result: SandboxJobResult = match serde_json::from_slice(&output.stdout) {
            Ok(result) => result,
            Err(error) => {
                if is_oci
                    && !ensure_oci_resources_absent_async(
                        self.config.oci_runtime.as_deref(),
                        &resources,
                    )
                    .await
                {
                    return Err(ExecutionError::OutcomeUnknown(format!(
                        "sandbox result was invalid and OCI cleanup could not be confirmed: {error}"
                    )));
                }
                cleanup_guard.disarm();
                return Err(adapter_failure(error));
            }
        };
        if let Some(proxy) = proxy.as_ref() {
            result.observed_origins = proxy.observed_origins();
        }
        drop(proxy);
        cleanup_guard.disarm();
        if result.timed_out {
            return Err(adapter_failure("sandboxed process exceeded its timeout"));
        }
        if let Some(limit) = &result.resource_limit_exceeded {
            return Err(adapter_failure(format!(
                "sandboxed process exceeded its {limit} limit"
            )));
        }
        bounded_json(
            serde_json::to_value(result).map_err(adapter_failure)?,
            usize::try_from(effective_output_bytes).map_err(adapter_failure)?,
        )
    }
}

pub(super) fn validate_process_spec(
    spec: &ProcessSpec,
    executable: &str,
    obligations: &PolicyObligations,
) -> Result<(), ExecutionError> {
    let executable_allowed = if obligations.sandbox_backend == "oci" {
        normalized_oci_path(executable)
            && obligations
                .filesystem
                .iter()
                .any(|grant| grant.mode == "execute" && grant.root == executable)
    } else {
        let executable = fs::canonicalize(executable).map_err(adapter_failure)?;
        if !executable.is_file() {
            return Err(adapter_failure("process executable is not a regular file"));
        }
        obligations.filesystem.iter().any(|grant| {
            grant.mode == "execute"
                && fs::canonicalize(&grant.root).is_ok_and(|root| root == executable)
        })
    };
    if !executable_allowed {
        return Err(adapter_failure(
            "process executable is not explicitly granted",
        ));
    }
    let cwd = fs::canonicalize(&spec.cwd).map_err(adapter_failure)?;
    if !cwd.is_dir() {
        return Err(adapter_failure("process cwd is not a directory"));
    }
    let cwd_allowed = obligations.filesystem.iter().any(|grant| {
        matches!(grant.mode.as_str(), "read" | "write" | "metadata")
            && fs::canonicalize(&grant.root).is_ok_and(|root| cwd.starts_with(root))
    });
    if !cwd_allowed {
        return Err(adapter_failure(
            "process cwd is outside policy-authorized filesystem roots",
        ));
    }
    if spec.args.len() > 256
        || spec
            .args
            .iter()
            .any(|argument| argument.len() > 64 * 1024 || argument.contains('\0'))
    {
        return Err(adapter_failure(
            "process argv exceeds bounds or contains NUL",
        ));
    }
    if spec
        .timeout_ms
        .is_some_and(|timeout| timeout == 0 || timeout > obligations.timeout_ms)
        || spec
            .max_output_bytes
            .is_some_and(|limit| limit < 1024 || limit > obligations.max_output_bytes)
    {
        return Err(adapter_failure(
            "requested process timeout or output cap exceeds policy bounds",
        ));
    }
    if spec.environment.len() > 128 {
        return Err(adapter_failure("process environment exceeds entry bound"));
    }
    for (name, value) in &spec.environment {
        if !obligations.allowed_environment.contains(name)
            || !valid_environment_name(name)
            || value.len() > 64 * 1024
            || value.contains('\0')
        {
            return Err(adapter_failure(format!(
                "process environment entry {name} is invalid or not permitted"
            )));
        }
    }
    if let Some(input) = &spec.stdin_base64 {
        let length = BASE64.decode(input).map_err(adapter_failure)?.len();
        if u64::try_from(length).map_err(adapter_failure)? > obligations.max_output_bytes {
            return Err(adapter_failure("process stdin exceeds the permitted bound"));
        }
    }
    Ok(())
}

pub(super) fn normalized_oci_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() > 1
        && value
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

pub(super) fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

pub(super) fn normalize_path_arguments(
    spec: &mut ProcessSpec,
    obligations: &PolicyObligations,
) -> Result<(), ExecutionError> {
    for argument in &mut spec.args {
        let path = Path::new(argument);
        let path_like = path.is_absolute()
            || argument.starts_with("./")
            || argument.starts_with("../")
            || argument.starts_with(".\\")
            || argument.starts_with("..\\");
        if !path_like {
            continue;
        }
        let candidate = if path.is_absolute() {
            path.to_owned()
        } else {
            spec.cwd.join(path)
        };
        let (canonical, required_mode) = if candidate.exists() {
            (
                fs::canonicalize(&candidate).map_err(adapter_failure)?,
                "read",
            )
        } else {
            let parent = candidate
                .parent()
                .ok_or_else(|| adapter_failure("process path argument has no parent"))?;
            let name = candidate
                .file_name()
                .ok_or_else(|| adapter_failure("process path argument has no filename"))?;
            (
                fs::canonicalize(parent)
                    .map(|parent| parent.join(name))
                    .map_err(adapter_failure)?,
                "write",
            )
        };
        let allowed = obligations.filesystem.iter().any(|grant| {
            let mode_allowed = grant.mode == "write"
                || (required_mode == "read" && matches!(grant.mode.as_str(), "read" | "metadata"));
            mode_allowed
                && fs::canonicalize(&grant.root).is_ok_and(|root| canonical.starts_with(root))
        });
        if !allowed {
            return Err(adapter_failure(format!(
                "process path argument {} escapes filesystem grants",
                candidate.display()
            )));
        }
        #[cfg(target_os = "windows")]
        {
            *argument = windows_process_path(&canonical);
        }
        #[cfg(not(target_os = "windows"))]
        {
            *argument = canonical.display().to_string();
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SandboxJob {
    pub(super) schema_version: u16,
    pub(super) job_id: String,
    pub(super) request_id: String,
    pub(super) request_hash: String,
    pub(super) decision_id: String,
    pub(super) permit_nonce: String,
    pub(super) permit_expires_at_unix_ms: i128,
    pub(super) executable: PathBuf,
    pub(super) process: ProcessSpec,
    pub(super) obligations: PolicyObligations,
    pub(super) timeout_ms: u64,
    pub(super) proxy_port: Option<u16>,
    pub(super) proxy_credential: Option<String>,
    pub(super) oci_runtime: Option<PathBuf>,
    pub(super) oci_image: Option<String>,
    pub(super) oci_proxy_image: Option<String>,
    pub(super) temporary_root: Option<PathBuf>,
}

#[cfg(target_os = "windows")]
pub(super) fn sandbox_temporary_root(backend: &str) -> Result<Option<PathBuf>, ExecutionError> {
    if backend != "windows_job" {
        return Ok(None);
    }
    let root = fs::canonicalize(std::env::temp_dir()).map_err(|error| {
        adapter_failure(format!("canonicalize Windows temporary root: {error}"))
    })?;
    if !root.is_dir() {
        return Err(adapter_failure(
            "canonical Windows temporary root is not a directory",
        ));
    }
    Ok(Some(root))
}

#[cfg(not(target_os = "windows"))]
pub(super) fn sandbox_temporary_root(_backend: &str) -> Result<Option<PathBuf>, ExecutionError> {
    Ok(None)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SignedSandboxJob {
    pub(super) job: SandboxJob,
    pub(super) authentication_tag: String,
}

impl SignedSandboxJob {
    pub(super) fn sign(job: SandboxJob, key: &[u8; 32]) -> Result<Self, ExecutionError> {
        let mut mac = HmacSha256::new_from_slice(key).map_err(adapter_failure)?;
        mac.update(&serde_json::to_vec(&job).map_err(adapter_failure)?);
        Ok(Self {
            job,
            authentication_tag: hex::encode(mac.finalize().into_bytes()),
        })
    }

    pub(super) fn verify(self, key: &[u8; 32]) -> Result<SandboxJob, SandboxHelperError> {
        let mut mac = HmacSha256::new_from_slice(key)
            .map_err(|error| SandboxHelperError::InvalidJob(error.to_string()))?;
        mac.update(&serde_json::to_vec(&self.job)?);
        let tag = hex::decode(self.authentication_tag)
            .map_err(|error| SandboxHelperError::InvalidJob(error.to_string()))?;
        mac.verify_slice(&tag)
            .map_err(|_| SandboxHelperError::InvalidJob("job authentication failed".into()))?;
        if self.job.schema_version != 2
            || Uuid::parse_str(&self.job.job_id).is_err()
            || self.job.request_id.is_empty()
            || self.job.request_hash.is_empty()
            || self.job.decision_id.is_empty()
            || self.job.permit_nonce.is_empty()
            || self.job.proxy_port.is_some() != self.job.proxy_credential.is_some()
            || self
                .job
                .proxy_credential
                .as_ref()
                .is_some_and(|credential| {
                    credential.len() != 64
                        || !credential.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            || (self.job.obligations.sandbox_backend == "windows_job"
                && self.job.temporary_root.is_none())
            || (self.job.obligations.sandbox_backend != "windows_job"
                && self.job.temporary_root.is_some())
        {
            return Err(SandboxHelperError::InvalidJob(
                "required authenticated job field is absent".into(),
            ));
        }
        let now_ms = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
        if self.job.permit_expires_at_unix_ms < now_ms {
            return Err(SandboxHelperError::InvalidJob("job permit expired".into()));
        }
        Ok(self.job)
    }
}
