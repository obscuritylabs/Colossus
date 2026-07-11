//! Permit-bound filesystem, process-sandbox, and network adapters.

#![allow(clippy::missing_errors_doc)]

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use colossus_contracts::{
    EffectRequest, FilesystemGrant, PolicyObligations, QuarantinedEffectResult,
};
use colossus_policy::{
    EffectExecutor, ExecutionError, ExecutionPermit, MIN_OCI_EFFECT_TIMEOUT_MS,
    MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS,
};
use command_group::CommandGroup as _;
use futures::{StreamExt as _, stream::FuturesUnordered};
use globset::{Glob, GlobMatcher};
use hmac::{Hmac, Mac};
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use reqwest::{Client, Url, redirect::Policy as RedirectPolicy};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use sysinfo::{Pid as SystemPid, ProcessRefreshKind, ProcessesToUpdate, System};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, lookup_host},
    process::Command as TokioCommand,
    sync::{Semaphore, oneshot},
};
use uuid::Uuid;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use nono::{AccessMode, CapabilitySet, Sandbox};

type HmacSha256 = Hmac<Sha256>;

const HELPER_KEY_VARIABLE: &str = "COLOSSUS_SANDBOX_JOB_KEY";
const OCI_PROXY_CONFIG_VARIABLE: &str = "COLOSSUS_OCI_PROXY_CONFIG";
const OCI_PROXY_PORT: u16 = 18_080;
const MAX_JOB_BYTES: usize = 1024 * 1024;
const MAX_PROXY_HEADER_BYTES: usize = 16 * 1024;
const MAX_TLS_RECORD_BYTES: usize = 18 * 1024;
const MAX_TLS_CLIENT_HELLO_BYTES: usize = 64 * 1024;
const OCI_CLEANUP_RESERVE_MS: u64 = 2_000;
const OCI_NETWORK_CLEANUP_RESERVE_MS: u64 = 5_000;
const OCI_CONTROL_COMMAND_TIMEOUT_MS: u64 = 1_500;
const OCI_DNS_RESOLUTION_TIMEOUT_MS: u64 = 3_000;

fn adapter_failure(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Strict process request carried inside an effect request.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessSpec {
    /// Absolute working directory.
    pub cwd: PathBuf,
    /// Literal argv entries; no shell parsing occurs.
    #[serde(default)]
    pub args: Vec<String>,
    /// Explicit environment map after policy allowlisting.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    /// Optional base64-encoded standard input.
    pub stdin_base64: Option<String>,
}

/// Local helper and OCI settings that policy may select by backend obligation.
#[derive(Clone, Debug)]
pub struct SandboxExecutorConfig {
    /// Trusted helper executable. Colossus CLI normally points this at itself.
    pub helper_executable: PathBuf,
    /// Exact Docker or Podman executable for the `oci` backend.
    pub oci_runtime: Option<PathBuf>,
    /// Immutable image reference used by the `oci` backend.
    pub oci_image: Option<String>,
    /// Immutable Colossus allowlist-proxy image used by networked OCI jobs.
    pub oci_proxy_image: Option<String>,
}

/// Runtime support and configured fallback report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxDoctorReport {
    /// Platform identifier.
    pub platform: String,
    /// Whether native kernel isolation is available.
    pub native_supported: bool,
    /// Native backend details without secrets.
    pub native_details: String,
    /// Configured helper executable.
    pub helper_executable: PathBuf,
    /// Configured OCI runtime, if any.
    pub oci_runtime: Option<PathBuf>,
    /// Whether an OCI image was configured.
    pub oci_image_configured: bool,
}

/// Return bounded local sandbox readiness.
pub fn sandbox_doctor(config: &SandboxExecutorConfig) -> SandboxDoctorReport {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let (native_supported, native_details) = {
        let support = Sandbox::support_info();
        (support.is_supported, support.details)
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let (native_supported, native_details) = (
        false,
        "native isolation is unavailable; configure the OCI backend".to_owned(),
    );
    SandboxDoctorReport {
        platform: std::env::consts::OS.into(),
        native_supported,
        native_details,
        helper_executable: config.helper_executable.clone(),
        oci_runtime: config.oci_runtime.clone(),
        oci_image_configured: config.oci_image.is_some(),
    }
}

/// Permit-bound filesystem adapter.
#[derive(Default)]
pub struct FilesystemExecutor;

impl FilesystemExecutor {
    /// Construct the filesystem adapter. Authorization still requires a permit.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EffectExecutor for FilesystemExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        let mode = filesystem_mode(&request.action)?;
        let target = authorized_path(
            Path::new(&request.resource),
            mode,
            &permit.obligations().filesystem,
        )?;
        let max_output =
            usize::try_from(permit.obligations().max_output_bytes).map_err(adapter_failure)?;
        match request.action.as_str() {
            "filesystem.read" => {
                let metadata = fs::metadata(&target).map_err(adapter_failure)?;
                if !metadata.is_file() {
                    return Err(adapter_failure("filesystem.read requires a regular file"));
                }
                if metadata.len() > permit.obligations().max_output_bytes {
                    return Err(adapter_failure("file exceeds the permitted output bound"));
                }
                let bytes = fs::read(target).map_err(adapter_failure)?;
                Ok(QuarantinedEffectResult {
                    media_type: "application/octet-stream".into(),
                    bytes,
                    effect_succeeded: true,
                })
            }
            "filesystem.metadata" => {
                let metadata = fs::metadata(&target).map_err(adapter_failure)?;
                bounded_json(
                    json!({
                        "is_file": metadata.is_file(),
                        "is_directory": metadata.is_dir(),
                        "length": metadata.len(),
                        "readonly": metadata.permissions().readonly(),
                    }),
                    max_output,
                )
            }
            "filesystem.list" => {
                let mut entries = fs::read_dir(&target)
                    .map_err(adapter_failure)?
                    .map(|entry| {
                        let entry = entry.map_err(adapter_failure)?;
                        let metadata =
                            fs::symlink_metadata(entry.path()).map_err(adapter_failure)?;
                        Ok(json!({
                            "name": entry.file_name().to_string_lossy(),
                            "is_file": metadata.is_file(),
                            "is_directory": metadata.is_dir(),
                            "length": metadata.len(),
                        }))
                    })
                    .collect::<Result<Vec<_>, ExecutionError>>()?;
                entries.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
                bounded_json(json!({"entries": entries}), max_output)
            }
            "filesystem.search" => search_files(&target, &request.content, max_output),
            "filesystem.write" => {
                let bytes = proposed_write_bytes(&request.content, max_output)?;
                atomic_write(&target, &bytes)?;
                bounded_json(
                    json!({"bytes_written": bytes.len(), "sha256": sha256_hex(&bytes)}),
                    max_output,
                )
            }
            _ => Err(adapter_failure("unsupported filesystem action")),
        }
    }
}

fn filesystem_mode(action: &str) -> Result<&'static str, ExecutionError> {
    match action {
        "filesystem.read" | "filesystem.list" | "filesystem.search" => Ok("read"),
        "filesystem.metadata" => Ok("metadata"),
        "filesystem.write" => Ok("write"),
        _ => Err(adapter_failure("unsupported filesystem action")),
    }
}

const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;
const MAX_SEARCH_LINE_BYTES: usize = 4096;

fn search_files(
    root: &Path,
    content: &Value,
    max_output: usize,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    if !root.is_dir() {
        return Err(adapter_failure(
            "filesystem.search requires a directory root",
        ));
    }
    let pattern = content
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| adapter_failure("filesystem.search pattern is absent"))?;
    if pattern.is_empty() || pattern.len() > 4096 {
        return Err(adapter_failure(
            "filesystem.search pattern must contain 1..=4096 bytes",
        ));
    }
    let case_sensitive = content
        .get("case_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let regex_enabled = content
        .get("regex")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let max_matches = content
        .get("max_matches")
        .and_then(Value::as_u64)
        .unwrap_or(100);
    if !(1..=1000).contains(&max_matches) {
        return Err(adapter_failure(
            "filesystem.search max_matches must be in 1..=1000",
        ));
    }
    let glob = content
        .get("glob")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(|value| {
            Glob::new(value)
                .map(|glob| glob.compile_matcher())
                .map_err(adapter_failure)
        })
        .transpose()?;
    let matcher = SearchMatcher::new(pattern, regex_enabled, case_sensitive)?;
    let mut matches = Vec::new();
    let mut truncated = false;
    let mut walker = WalkBuilder::new(root);
    walker
        .follow_links(false)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .max_filesize(Some(MAX_SEARCH_FILE_BYTES));
    for entry in walker.build().filter_map(Result::ok) {
        let path = entry.path();
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        let relative = path.strip_prefix(root).map_err(adapter_failure)?;
        if is_control_path(relative) || !glob_matches(glob.as_ref(), relative) {
            continue;
        }
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        if bytes.len() > usize::try_from(MAX_SEARCH_FILE_BYTES).unwrap_or(usize::MAX)
            || bytes.contains(&0)
        {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        for (line_index, line) in text.lines().enumerate() {
            let Some(column) = matcher.find(line) else {
                continue;
            };
            matches.push(json!({
                "path": relative.to_string_lossy(),
                "line": line_index.saturating_add(1),
                "column": column.saturating_add(1),
                "text": bounded_search_line(line),
            }));
            if matches.len() >= usize::try_from(max_matches).unwrap_or(usize::MAX) {
                truncated = true;
                break;
            }
            if serde_json::to_vec(&json!({"matches": matches, "truncated": false}))
                .is_ok_and(|bytes| bytes.len() > max_output)
            {
                matches.pop();
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
    }
    bounded_json(
        json!({"matches": matches, "truncated": truncated}),
        max_output,
    )
}

enum SearchMatcher {
    Regex(Regex),
    Literal {
        pattern: String,
        case_sensitive: bool,
    },
}

impl SearchMatcher {
    fn new(
        pattern: &str,
        regex_enabled: bool,
        case_sensitive: bool,
    ) -> Result<Self, ExecutionError> {
        if regex_enabled {
            RegexBuilder::new(pattern)
                .case_insensitive(!case_sensitive)
                .size_limit(1024 * 1024)
                .build()
                .map(Self::Regex)
                .map_err(adapter_failure)
        } else {
            Ok(Self::Literal {
                pattern: if case_sensitive {
                    pattern.into()
                } else {
                    pattern.to_lowercase()
                },
                case_sensitive,
            })
        }
    }

    fn find(&self, line: &str) -> Option<usize> {
        match self {
            Self::Regex(regex) => regex.find(line).map(|found| found.start()),
            Self::Literal {
                pattern,
                case_sensitive,
            } if *case_sensitive => line.find(pattern),
            Self::Literal { pattern, .. } => line.to_lowercase().find(pattern),
        }
    }
}

fn glob_matches(matcher: Option<&GlobMatcher>, relative: &Path) -> bool {
    matcher.is_none_or(|matcher| matcher.is_match(relative))
}

fn is_control_path(path: &Path) -> bool {
    path.components().any(|component| {
        let value = component.as_os_str();
        value == ".colossus" || value == ".git"
    })
}

fn bounded_search_line(line: &str) -> &str {
    if line.len() <= MAX_SEARCH_LINE_BYTES {
        return line;
    }
    let mut end = MAX_SEARCH_LINE_BYTES;
    while !line.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    &line[..end]
}

fn authorized_path(
    requested: &Path,
    mode: &str,
    grants: &[FilesystemGrant],
) -> Result<PathBuf, ExecutionError> {
    if !requested.is_absolute() {
        return Err(adapter_failure("effect paths must be absolute"));
    }
    if fs::symlink_metadata(requested).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(adapter_failure("symbolic-link effect targets are rejected"));
    }
    let target = if mode == "write" && !requested.exists() {
        let parent = requested
            .parent()
            .ok_or_else(|| adapter_failure("write target has no parent"))?;
        let filename = requested
            .file_name()
            .ok_or_else(|| adapter_failure("write target has no filename"))?;
        fs::canonicalize(parent)
            .map(|parent| parent.join(filename))
            .map_err(adapter_failure)?
    } else {
        fs::canonicalize(requested).map_err(adapter_failure)?
    };
    let allowed = grants.iter().any(|grant| {
        let mode_allowed = grant.mode == "write"
            || grant.mode == mode
            || (mode == "metadata" && grant.mode == "read");
        mode_allowed && fs::canonicalize(&grant.root).is_ok_and(|root| target.starts_with(root))
    });
    if !allowed {
        return Err(adapter_failure(format!(
            "{} is outside permitted {mode} roots",
            requested.display()
        )));
    }
    Ok(target)
}

fn proposed_write_bytes(content: &Value, limit: usize) -> Result<Vec<u8>, ExecutionError> {
    let bytes = if let Some(encoded) = content.get("content_base64").and_then(Value::as_str) {
        BASE64.decode(encoded).map_err(adapter_failure)?
    } else if let Some(text) = content.get("text").and_then(Value::as_str) {
        text.as_bytes().to_vec()
    } else {
        return Err(adapter_failure(
            "filesystem.write requires text or content_base64",
        ));
    };
    if bytes.len() > limit {
        return Err(adapter_failure("write content exceeds the permitted bound"));
    }
    Ok(bytes)
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), ExecutionError> {
    let parent = target
        .parent()
        .ok_or_else(|| adapter_failure("write target has no parent"))?;
    let temporary = parent.join(format!(".colossus-write-{}.tmp", Uuid::now_v7()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(adapter_failure)?;
        file.write_all(bytes).map_err(adapter_failure)?;
        file.sync_all().map_err(adapter_failure)?;
        fs::rename(&temporary, target).map_err(adapter_failure)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn bounded_json(value: Value, limit: usize) -> Result<QuarantinedEffectResult, ExecutionError> {
    let bytes = serde_json::to_vec(&value).map_err(adapter_failure)?;
    if bytes.len() > limit {
        return Err(adapter_failure(
            "adapter output exceeds the permitted bound",
        ));
    }
    Ok(QuarantinedEffectResult {
        media_type: "application/json".into(),
        bytes,
        effect_succeeded: true,
    })
}

/// Permit-bound process executor using an authenticated one-shot helper.
pub struct SandboxProcessExecutor {
    config: SandboxExecutorConfig,
    job_key: [u8; 32],
}

struct OciCancellationGuard {
    runtime: Option<PathBuf>,
    resources: OciResourceNames,
    armed: bool,
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
struct OciResourceNames {
    workload: String,
    proxy: String,
    internal_network: String,
    egress_network: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OciRuntimeKind {
    Docker,
    Podman,
}

fn oci_runtime_kind(runtime: &Path) -> Option<OciRuntimeKind> {
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

fn oci_remove_arguments(runtime: &Path, name: &str) -> Option<Vec<String>> {
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

#[async_trait]
impl EffectExecutor for SandboxProcessExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if request.action != "process.spawn" {
            return Err(adapter_failure("process executor received another action"));
        }
        let mut spec: ProcessSpec = serde_json::from_value(request.content.clone())
            .map_err(|error| adapter_failure(format!("invalid process request: {error}")))?;
        validate_process_spec(&spec, &request.resource, permit.obligations())?;
        normalize_path_arguments(&mut spec, permit.obligations())?;
        if permit.obligations().sandbox_backend == "oci"
            && permit.obligations().timeout_ms < MIN_OCI_EFFECT_TIMEOUT_MS
        {
            return Err(adapter_failure(format!(
                "OCI process execution requires at least {MIN_OCI_EFFECT_TIMEOUT_MS}ms"
            )));
        }
        if permit.obligations().sandbox_backend == "oci"
            && !permit.obligations().network_destinations.is_empty()
            && permit.obligations().timeout_ms < MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS
        {
            return Err(adapter_failure(format!(
                "networked OCI process execution requires at least {MIN_OCI_NETWORK_EFFECT_TIMEOUT_MS}ms"
            )));
        }
        let proxy = if permit.obligations().network_destinations.is_empty() {
            None
        } else if permit.obligations().sandbox_backend == "native" {
            Some(AllowlistProxy::start(permit.obligations().network_destinations.clone()).await?)
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
        let helper_reserve = if permit.obligations().sandbox_backend == "oci"
            && !permit.obligations().network_destinations.is_empty()
        {
            OCI_NETWORK_CLEANUP_RESERVE_MS
        } else if permit.obligations().sandbox_backend == "oci" {
            OCI_CLEANUP_RESERVE_MS
        } else {
            250
        };
        let helper_budget = permit
            .obligations()
            .timeout_ms
            .saturating_sub(helper_reserve)
            .max(1);
        let job = SandboxJob {
            schema_version: 1,
            job_id: Uuid::now_v7().to_string(),
            request_id: request.request_id.clone(),
            request_hash: permit.request_hash().into(),
            decision_id: permit.decision_id().into(),
            permit_nonce: permit.nonce().into(),
            permit_expires_at_unix_ms: permit.expires_at_unix_ms(),
            executable: PathBuf::from(&request.resource),
            process: spec,
            obligations: permit.obligations().clone(),
            timeout_ms: helper_budget,
            proxy_port: proxy.as_ref().map(AllowlistProxy::port),
            oci_runtime: self.config.oci_runtime.clone(),
            oci_image: self.config.oci_image.clone(),
            oci_proxy_image: self.config.oci_proxy_image.clone(),
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
        drop(proxy);
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
        let result: SandboxJobResult = match serde_json::from_slice(&output.stdout) {
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
        cleanup_guard.disarm();
        if result.timed_out {
            return Err(adapter_failure("sandboxed process exceeded its timeout"));
        }
        if let Some(limit) = &result.resource_limit_exceeded {
            return Err(adapter_failure(format!(
                "sandboxed process exceeded its {limit} limit"
            )));
        }
        if !result.success {
            let stderr = BASE64.decode(&result.stderr_base64).unwrap_or_default();
            return Err(adapter_failure(format!(
                "sandboxed process exited with {:?}; stderr_bytes={}; stderr_sha256={}",
                result.exit_code,
                stderr.len(),
                sha256_hex(&stderr),
            )));
        }
        bounded_json(
            serde_json::to_value(result).map_err(adapter_failure)?,
            usize::try_from(permit.obligations().max_output_bytes).map_err(adapter_failure)?,
        )
    }
}

fn validate_process_spec(
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

fn normalized_oci_path(value: &str) -> bool {
    value.starts_with('/')
        && value.len() > 1
        && value
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn normalize_path_arguments(
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
        *argument = canonical.display().to_string();
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SandboxJob {
    schema_version: u16,
    job_id: String,
    request_id: String,
    request_hash: String,
    decision_id: String,
    permit_nonce: String,
    permit_expires_at_unix_ms: i128,
    executable: PathBuf,
    process: ProcessSpec,
    obligations: PolicyObligations,
    timeout_ms: u64,
    proxy_port: Option<u16>,
    oci_runtime: Option<PathBuf>,
    oci_image: Option<String>,
    oci_proxy_image: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedSandboxJob {
    job: SandboxJob,
    authentication_tag: String,
}

impl SignedSandboxJob {
    fn sign(job: SandboxJob, key: &[u8; 32]) -> Result<Self, ExecutionError> {
        let mut mac = HmacSha256::new_from_slice(key).map_err(adapter_failure)?;
        mac.update(&serde_json::to_vec(&job).map_err(adapter_failure)?);
        Ok(Self {
            job,
            authentication_tag: hex::encode(mac.finalize().into_bytes()),
        })
    }

    fn verify(self, key: &[u8; 32]) -> Result<SandboxJob, SandboxHelperError> {
        let mut mac = HmacSha256::new_from_slice(key)
            .map_err(|error| SandboxHelperError::InvalidJob(error.to_string()))?;
        mac.update(&serde_json::to_vec(&self.job)?);
        let tag = hex::decode(self.authentication_tag)
            .map_err(|error| SandboxHelperError::InvalidJob(error.to_string()))?;
        mac.verify_slice(&tag)
            .map_err(|_| SandboxHelperError::InvalidJob("job authentication failed".into()))?;
        if self.job.schema_version != 1
            || Uuid::parse_str(&self.job.job_id).is_err()
            || self.job.request_id.is_empty()
            || self.job.request_hash.is_empty()
            || self.job.decision_id.is_empty()
            || self.job.permit_nonce.is_empty()
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

/// Structured helper failure printed only to the trusted parent process.
#[derive(Debug, Error)]
pub enum SandboxHelperError {
    /// The authenticated job was malformed, expired, or tampered with.
    #[error("invalid sandbox job: {0}")]
    InvalidJob(String),
    /// Native or OCI isolation could not be established.
    #[error("sandbox setup failed: {0}")]
    Setup(String),
    /// The sandboxed command could not be supervised.
    #[error("sandbox execution failed: {0}")]
    Execution(String),
    /// Strict JSON IPC failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Helper standard I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Run the trusted one-shot sandbox helper protocol over stdin/stdout.
pub fn run_helper_stdio() -> Result<(), SandboxHelperError> {
    let encoded_key = std::env::var(HELPER_KEY_VARIABLE)
        .map_err(|_| SandboxHelperError::InvalidJob("helper key is absent".into()))?;
    let key: [u8; 32] = hex::decode(encoded_key)
        .map_err(|error| SandboxHelperError::InvalidJob(error.to_string()))?
        .try_into()
        .map_err(|_| SandboxHelperError::InvalidJob("helper key length is invalid".into()))?;
    let mut input = std::io::stdin().take(
        u64::try_from(MAX_JOB_BYTES)
            .unwrap_or(u64::MAX)
            .saturating_add(1),
    );
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    if bytes.len() > MAX_JOB_BYTES {
        return Err(SandboxHelperError::InvalidJob(
            "helper input exceeds IPC bound".into(),
        ));
    }
    let signed: SignedSandboxJob = serde_json::from_slice(&bytes)?;
    let job = signed.verify(&key)?;
    let result = execute_sandbox_job(job)?;
    serde_json::to_writer(std::io::stdout(), &result)?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SandboxJobResult {
    backend: String,
    exit_code: Option<i32>,
    success: bool,
    timed_out: bool,
    resource_limit_exceeded: Option<String>,
    output_truncated: bool,
    stdout_base64: String,
    stderr_base64: String,
}

fn execute_sandbox_job(job: SandboxJob) -> Result<SandboxJobResult, SandboxHelperError> {
    let backend = job.obligations.sandbox_backend.clone();
    #[cfg(target_os = "windows")]
    if backend == "oci" {
        return Err(SandboxHelperError::Setup(
            "OCI execution is disabled on Windows until path mapping passes live acceptance".into(),
        ));
    }
    let mut oci_network = if backend == "oci" && !job.obligations.network_destinations.is_empty() {
        Some(OciNetworkResources::start(&job)?)
    } else {
        None
    };
    let mut command = match backend.as_str() {
        "native" => native_command(&job)?,
        "oci" => oci_command(
            &job,
            oci_network.as_ref().map(OciNetworkResources::proxy_address),
        )?,
        "broker" if job.obligations.allow_sandbox_downgrade => direct_command(&job),
        "broker" => {
            return Err(SandboxHelperError::Setup(
                "broker downgrade was not explicitly authorized".into(),
            ));
        }
        "windows_job" => {
            return Err(SandboxHelperError::Setup(
                "windows_job is not available in this build; configure OCI".into(),
            ));
        }
        other => {
            return Err(SandboxHelperError::Setup(format!(
                "unknown sandbox backend {other}"
            )));
        }
    };
    let result = supervise(&mut command, &job, backend.clone());
    if let Some(network) = oci_network.as_mut() {
        network.cleanup();
    }
    if backend == "oci" && !ensure_oci_resources_absent(&job) {
        return Err(SandboxHelperError::Execution(
            "OCI container or network cleanup could not be confirmed".into(),
        ));
    }
    result
}

fn direct_command(job: &SandboxJob) -> Command {
    let mut command = Command::new(&job.executable);
    configure_command(&mut command, job);
    command
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn native_command(job: &SandboxJob) -> Result<Command, SandboxHelperError> {
    if !Sandbox::is_supported() {
        return Err(SandboxHelperError::Setup(
            "native kernel sandbox is unavailable".into(),
        ));
    }
    let mut capabilities = CapabilitySet::new();
    for grant in &job.obligations.filesystem {
        let path = fs::canonicalize(&grant.root)
            .map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
        let access = if grant.mode == "write" {
            AccessMode::ReadWrite
        } else {
            AccessMode::Read
        };
        capabilities = if path.is_dir() {
            capabilities.allow_path(&path, access)
        } else {
            capabilities.allow_file(&path, access)
        }
        .map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
    }
    for path in native_runtime_paths() {
        if !path.exists() {
            continue;
        }
        capabilities = if path.is_dir() {
            capabilities.allow_path(path, AccessMode::Read)
        } else {
            capabilities.allow_file(path, AccessMode::Read)
        }
        .map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
    }
    capabilities = if let Some(port) = job.proxy_port {
        capabilities.proxy_only(port)
    } else {
        capabilities.block_network()
    };
    Sandbox::apply_auto(&capabilities)
        .map_err(|error| SandboxHelperError::Setup(format!("native apply: {error}")))?;
    let mut command = Command::new(&job.executable);
    configure_command(&mut command, job);
    Ok(command)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn native_command(_job: &SandboxJob) -> Result<Command, SandboxHelperError> {
    Err(SandboxHelperError::Setup(
        "native sandboxing is unsupported on this platform".into(),
    ))
}

#[cfg(target_os = "macos")]
fn native_runtime_paths() -> Vec<&'static Path> {
    [
        "/System/Library",
        "/usr/lib",
        "/Library/Apple/usr/lib",
        "/dev/null",
        "/dev/urandom",
    ]
    .into_iter()
    .map(Path::new)
    .collect()
}

#[cfg(target_os = "linux")]
fn native_runtime_paths() -> Vec<&'static Path> {
    [
        "/lib",
        "/lib64",
        "/usr/lib",
        "/usr/lib64",
        "/dev/null",
        "/dev/urandom",
    ]
    .into_iter()
    .map(Path::new)
    .collect()
}

struct OciNetworkResources {
    runtime: PathBuf,
    names: OciResourceNames,
    proxy_address: SocketAddr,
    armed: bool,
}

impl OciNetworkResources {
    fn start(job: &SandboxJob) -> Result<Self, SandboxHelperError> {
        let runtime = job
            .oci_runtime
            .as_ref()
            .ok_or_else(|| SandboxHelperError::Setup("OCI runtime is not configured".into()))?;
        oci_runtime_kind(runtime).ok_or_else(|| {
            SandboxHelperError::Setup("OCI runtime must be the Docker or Podman executable".into())
        })?;
        let proxy_image = job
            .oci_proxy_image
            .as_ref()
            .ok_or_else(|| SandboxHelperError::Setup("OCI proxy image is not configured".into()))?;
        if !valid_oci_image_reference(proxy_image) {
            return Err(SandboxHelperError::Setup(
                "OCI proxy image must use a complete immutable SHA-256 reference".into(),
            ));
        }
        let names = oci_resource_names(&job.job_id);
        let mut resources = Self {
            runtime: runtime.clone(),
            names,
            proxy_address: SocketAddr::from(([0, 0, 0, 0], OCI_PROXY_PORT)),
            armed: true,
        };
        run_oci_control(
            runtime,
            &[
                "network".into(),
                "create".into(),
                "--internal".into(),
                "--label".into(),
                format!("dev.colossus.job={}", job.job_id),
                resources.names.internal_network.clone(),
            ],
            &[],
            "create the internal OCI network",
        )?;
        run_oci_control(
            runtime,
            &[
                "network".into(),
                "create".into(),
                "--label".into(),
                format!("dev.colossus.job={}", job.job_id),
                resources.names.egress_network.clone(),
            ],
            &[],
            "create the OCI proxy egress network",
        )?;
        let bootstrap = OciProxyBootstrap {
            schema_version: 1,
            request_hash: job.request_hash.clone(),
            decision_id: job.decision_id.clone(),
            permit_nonce: job.permit_nonce.clone(),
            expires_at_unix_ms: job.permit_expires_at_unix_ms,
            allowed_origins: job.obligations.network_destinations.clone(),
            resolved_origins: resolve_oci_origins(&job.obligations.network_destinations)?,
            max_connections: usize::try_from(job.obligations.max_processes)
                .unwrap_or(256)
                .clamp(1, 256),
            connection_timeout_ms: job.timeout_ms,
        };
        let encoded = BASE64.encode(serde_json::to_vec(&bootstrap)?);
        let proxy_environment = [(OCI_PROXY_CONFIG_VARIABLE, encoded.as_str())];
        run_oci_control(
            runtime,
            &[
                "run".into(),
                "--detach".into(),
                "--rm".into(),
                "--pull=never".into(),
                "--network".into(),
                resources.names.internal_network.clone(),
                "--read-only".into(),
                "--cap-drop=ALL".into(),
                "--security-opt=no-new-privileges".into(),
                "--pids-limit=16".into(),
                "--memory=67108864".into(),
                "--name".into(),
                resources.names.proxy.clone(),
                "--env".into(),
                OCI_PROXY_CONFIG_VARIABLE.into(),
                proxy_image.clone(),
            ],
            &proxy_environment,
            "start the OCI allowlist proxy",
        )?;
        run_oci_control(
            runtime,
            &[
                "network".into(),
                "connect".into(),
                resources.names.egress_network.clone(),
                resources.names.proxy.clone(),
            ],
            &[],
            "connect the OCI proxy to its egress network",
        )?;
        let inspected = run_oci_control(
            runtime,
            &[
                "container".into(),
                "inspect".into(),
                resources.names.proxy.clone(),
            ],
            &[],
            "inspect the OCI proxy address",
        )?;
        resources.proxy_address =
            oci_network_address(&inspected, &resources.names.internal_network)?;
        let mut ready = false;
        for _ in 0..20 {
            let logs = run_oci_control(
                runtime,
                &["logs".into(), resources.names.proxy.clone()],
                &[],
                "read OCI proxy readiness",
            )?;
            if logs
                .windows(b"colossus-oci-proxy-ready".len())
                .any(|window| window == b"colossus-oci-proxy-ready")
            {
                ready = true;
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        if !ready {
            return Err(SandboxHelperError::Setup(
                "OCI allowlist proxy did not become ready".into(),
            ));
        }
        Ok(resources)
    }

    fn proxy_address(&self) -> SocketAddr {
        self.proxy_address
    }

    fn cleanup(&mut self) {
        if self.armed {
            cleanup_oci_resources(&self.runtime, &self.names);
            self.armed = false;
        }
    }
}

impl Drop for OciNetworkResources {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn resolve_oci_origins(
    origins: &[String],
) -> Result<BTreeMap<String, Vec<SocketAddr>>, SandboxHelperError> {
    let origins = origins.to_vec();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {
        let _ = sender.send(resolve_oci_origins_blocking(&origins));
    });
    receiver
        .recv_timeout(Duration::from_millis(OCI_DNS_RESOLUTION_TIMEOUT_MS))
        .map_err(|_| SandboxHelperError::Setup("OCI proxy DNS resolution timed out".into()))?
}

fn resolve_oci_origins_blocking(
    origins: &[String],
) -> Result<BTreeMap<String, Vec<SocketAddr>>, SandboxHelperError> {
    let mut resolved = BTreeMap::new();
    for origin in origins {
        let url =
            Url::parse(origin).map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
        let host = url
            .host_str()
            .ok_or_else(|| SandboxHelperError::Setup("OCI proxy origin has no host".into()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| SandboxHelperError::Setup("OCI proxy origin has no port".into()))?;
        let host_is_ip = host.parse::<IpAddr>().is_ok();
        let mut addresses = (host, port)
            .to_socket_addrs()
            .map_err(|error| SandboxHelperError::Setup(error.to_string()))?
            .filter(|address| host_is_ip || !non_public_ip(address.ip()))
            .collect::<Vec<_>>();
        addresses.sort_by_key(|address| usize::from(address.is_ipv6()));
        addresses.dedup();
        addresses.truncate(16);
        if addresses.is_empty() {
            return Err(SandboxHelperError::Setup(format!(
                "OCI proxy origin resolved to no permitted address: {origin}"
            )));
        }
        resolved.insert(origin.clone(), addresses);
    }
    Ok(resolved)
}

fn oci_network_address(
    inspection: &[u8],
    network_name: &str,
) -> Result<SocketAddr, SandboxHelperError> {
    let documents: Value = serde_json::from_slice(inspection)?;
    let address = documents
        .as_array()
        .and_then(|documents| documents.first())
        .and_then(|document| document.get("NetworkSettings"))
        .and_then(|settings| settings.get("Networks"))
        .and_then(|networks| networks.get(network_name))
        .and_then(|network| network.get("IPAddress"))
        .and_then(Value::as_str)
        .ok_or_else(|| SandboxHelperError::Setup("OCI proxy has no internal address".into()))?
        .parse::<IpAddr>()
        .map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
    if address.is_unspecified() {
        return Err(SandboxHelperError::Setup(
            "OCI proxy internal address is unspecified".into(),
        ));
    }
    Ok(SocketAddr::new(address, OCI_PROXY_PORT))
}

fn oci_command(
    job: &SandboxJob,
    proxy_address: Option<SocketAddr>,
) -> Result<Command, SandboxHelperError> {
    if job.proxy_port.is_some() {
        return Err(SandboxHelperError::Setup(
            "OCI jobs cannot use a host loopback proxy".into(),
        ));
    }
    if job.obligations.network_destinations.is_empty() != proxy_address.is_none() {
        return Err(SandboxHelperError::Setup(
            "OCI proxy resources do not match the network obligations".into(),
        ));
    }
    let runtime = job
        .oci_runtime
        .as_ref()
        .ok_or_else(|| SandboxHelperError::Setup("OCI runtime is not configured".into()))?;
    oci_runtime_kind(runtime).ok_or_else(|| {
        SandboxHelperError::Setup("OCI runtime must be the Docker or Podman executable".into())
    })?;
    let image = job
        .oci_image
        .as_ref()
        .ok_or_else(|| SandboxHelperError::Setup("OCI image is not configured".into()))?;
    if !valid_oci_image_reference(image) {
        return Err(SandboxHelperError::Setup(
            "OCI image must use a complete immutable SHA-256 reference".into(),
        ));
    }
    let mut command = Command::new(runtime);
    command.env_clear().args(["run", "--rm", "--pull=never"]);
    if proxy_address.is_some() {
        command
            .arg("--network")
            .arg(oci_resource_names(&job.job_id).internal_network)
            .arg("--dns=127.0.0.1");
    } else {
        command.arg("--network=none");
    }
    command.args([
        "--read-only",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
    ]);
    command.arg("--name").arg(oci_container_name(&job.job_id));
    command.arg(format!("--pids-limit={}", job.obligations.max_processes));
    command.arg(format!("--memory={}", job.obligations.max_memory_bytes));
    let mut mounts = BTreeMap::<PathBuf, bool>::new();
    for grant in &job.obligations.filesystem {
        if grant.mode == "execute" {
            continue;
        }
        let root = fs::canonicalize(&grant.root)
            .map_err(|error| SandboxHelperError::Setup(error.to_string()))?;
        if root.to_string_lossy().contains([',', '\0']) {
            return Err(SandboxHelperError::Setup(
                "OCI bind mount paths may not contain commas or NUL".into(),
            ));
        }
        mounts
            .entry(root)
            .and_modify(|writable| *writable |= grant.mode == "write")
            .or_insert(grant.mode == "write");
    }
    for (root, writable) in mounts {
        let readonly = if writable { "" } else { ",readonly" };
        command.arg("--mount").arg(format!(
            "type=bind,source={},target={}{}",
            root.display(),
            root.display(),
            readonly
        ));
    }
    let mut environment = job.process.environment.clone();
    if let Some(proxy_address) = proxy_address {
        let proxy = format!("http://{proxy_address}");
        environment.insert("HTTP_PROXY".into(), proxy.clone());
        environment.insert("HTTPS_PROXY".into(), proxy.clone());
        environment.insert("ALL_PROXY".into(), proxy);
        environment.insert("NO_PROXY".into(), String::new());
    }
    for (name, value) in &environment {
        command.env(name, value).arg("--env").arg(name);
    }
    let mut bootstrap = "exec /usr/bin/env -i --".to_owned();
    for name in environment.keys() {
        if !valid_environment_name(name) {
            return Err(SandboxHelperError::Setup(format!(
                "invalid OCI environment name {name}"
            )));
        }
        bootstrap.push(' ');
        bootstrap.push_str(name);
        bootstrap.push_str("=\"${");
        bootstrap.push_str(name);
        bootstrap.push_str("}\"");
    }
    bootstrap.push_str(" \"$@\"");
    command
        .arg("--workdir")
        .arg(&job.process.cwd)
        .arg("--entrypoint")
        .arg("/bin/sh")
        .arg(image)
        .arg("-c")
        .arg(bootstrap)
        .arg("colossus-bootstrap")
        .arg(&job.executable)
        .args(&job.process.args);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command)
}

fn valid_oci_image_reference(image: &str) -> bool {
    if let Some(digest) = image.strip_prefix("sha256:") {
        return digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    let Some((repository, digest)) = image.rsplit_once("@sha256:") else {
        return false;
    };
    !repository.is_empty()
        && digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn oci_container_name(job_id: &str) -> String {
    let sanitized = job_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>();
    format!("colossus-{sanitized}")
}

fn oci_resource_names(job_id: &str) -> OciResourceNames {
    let workload = oci_container_name(job_id);
    let suffix = workload.trim_start_matches("colossus-").to_owned();
    OciResourceNames {
        workload,
        proxy: format!("colossus-proxy-{suffix}"),
        internal_network: format!("colossus-int-{suffix}"),
        egress_network: format!("colossus-egress-{suffix}"),
    }
}

fn bounded_control_command(mut command: Command) -> Option<(std::process::ExitStatus, Vec<u8>)> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + Duration::from_millis(OCI_CONTROL_COMMAND_TIMEOUT_MS);
    loop {
        if let Some(status) = child.try_wait().ok()? {
            let mut stdout = Vec::new();
            child.stdout.take()?.read_to_end(&mut stdout).ok()?;
            return Some((status, stdout));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn run_oci_control(
    runtime: &Path,
    arguments: &[String],
    environment: &[(&str, &str)],
    operation: &str,
) -> Result<Vec<u8>, SandboxHelperError> {
    let mut command = Command::new(runtime);
    command
        .env_clear()
        .envs(environment.iter().copied())
        .args(arguments);
    match bounded_control_command(command) {
        Some((status, stdout)) if status.success() => Ok(stdout),
        Some((status, _)) => Err(SandboxHelperError::Setup(format!(
            "failed to {operation}: runtime exited with {status}"
        ))),
        None => Err(SandboxHelperError::Setup(format!(
            "failed to {operation}: runtime command timed out"
        ))),
    }
}

fn cleanup_oci_resources(runtime: &Path, names: &OciResourceNames) {
    for name in [&names.workload, &names.proxy] {
        let Some(arguments) = oci_remove_arguments(runtime, name) else {
            continue;
        };
        let mut remove = Command::new(runtime);
        remove.env_clear().args(arguments);
        let _ = bounded_control_command(remove);
    }
    for name in [&names.internal_network, &names.egress_network] {
        let mut remove = Command::new(runtime);
        remove.env_clear().args(["network", "rm", "--force", name]);
        let _ = bounded_control_command(remove);
    }
}

fn oci_resources_absent(runtime: &Path, names: &OciResourceNames) -> bool {
    let containers_absent = [&names.workload, &names.proxy].iter().all(|name| {
        let mut list = Command::new(runtime);
        list.env_clear().args([
            "container",
            "ls",
            "--all",
            "--filter",
            &format!("name=^/{name}$"),
            "--format",
            "{{.ID}}",
        ]);
        bounded_control_command(list).is_some_and(|(status, stdout)| {
            status.success() && stdout.iter().all(u8::is_ascii_whitespace)
        })
    });
    let networks_absent = [&names.internal_network, &names.egress_network]
        .iter()
        .all(|name| {
            let mut list = Command::new(runtime);
            list.env_clear().args([
                "network",
                "ls",
                "--filter",
                &format!("name=^{name}$"),
                "--format",
                "{{.Name}}",
            ]);
            bounded_control_command(list).is_some_and(|(status, stdout)| {
                status.success() && stdout.iter().all(u8::is_ascii_whitespace)
            })
        });
    containers_absent && networks_absent
}

fn ensure_oci_resources_absent(job: &SandboxJob) -> bool {
    let Some(runtime) = job.oci_runtime.as_ref() else {
        return false;
    };
    let names = oci_resource_names(&job.job_id);
    cleanup_oci_resources(runtime, &names);
    oci_resources_absent(runtime, &names)
}

async fn ensure_oci_resources_absent_async(
    runtime: Option<&Path>,
    names: &OciResourceNames,
) -> bool {
    let Some(runtime) = runtime else {
        return false;
    };
    for name in [&names.workload, &names.proxy] {
        let Some(remove_arguments) = oci_remove_arguments(runtime, name) else {
            return false;
        };
        let mut remove = TokioCommand::new(runtime);
        remove
            .env_clear()
            .args(remove_arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let _ = tokio::time::timeout(
            Duration::from_millis(OCI_CONTROL_COMMAND_TIMEOUT_MS),
            remove.status(),
        )
        .await;
    }
    for name in [&names.internal_network, &names.egress_network] {
        let mut remove = TokioCommand::new(runtime);
        remove
            .env_clear()
            .args(["network", "rm", "--force", name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let _ = tokio::time::timeout(
            Duration::from_millis(OCI_CONTROL_COMMAND_TIMEOUT_MS),
            remove.status(),
        )
        .await;
    }
    let mut containers_absent = true;
    for name in [&names.workload, &names.proxy] {
        let mut list = TokioCommand::new(runtime);
        list.env_clear()
            .args([
                "container",
                "ls",
                "--all",
                "--filter",
                &format!("name=^/{name}$"),
                "--format",
                "{{.ID}}",
            ])
            .stdin(Stdio::null())
            .kill_on_drop(true);
        containers_absent &= tokio::time::timeout(
            Duration::from_millis(OCI_CONTROL_COMMAND_TIMEOUT_MS),
            list.output(),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .is_some_and(|output| {
            output.status.success() && output.stdout.iter().all(u8::is_ascii_whitespace)
        });
    }
    let mut networks_absent = true;
    for name in [&names.internal_network, &names.egress_network] {
        let mut list = TokioCommand::new(runtime);
        list.env_clear()
            .args([
                "network",
                "ls",
                "--filter",
                &format!("name=^{name}$"),
                "--format",
                "{{.Name}}",
            ])
            .stdin(Stdio::null())
            .kill_on_drop(true);
        networks_absent &= tokio::time::timeout(
            Duration::from_millis(OCI_CONTROL_COMMAND_TIMEOUT_MS),
            list.output(),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .is_some_and(|output| {
            output.status.success() && output.stdout.iter().all(u8::is_ascii_whitespace)
        });
    }
    containers_absent && networks_absent
}

fn configure_command(command: &mut Command, job: &SandboxJob) {
    command
        .args(&job.process.args)
        .current_dir(&job.process.cwd)
        .env_clear()
        .envs(&job.process.environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(port) = job.proxy_port {
        let proxy = format!("http://127.0.0.1:{port}");
        command
            .env("HTTP_PROXY", &proxy)
            .env("HTTPS_PROXY", &proxy)
            .env("ALL_PROXY", &proxy)
            .env("NO_PROXY", "");
    }
}

#[derive(Default)]
struct CaptureState {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    remaining: usize,
    truncated: bool,
}

#[derive(Clone, Copy)]
enum CaptureStream {
    Stdout,
    Stderr,
}

fn capture<R: Read + Send + 'static>(
    mut reader: R,
    state: Arc<Mutex<CaptureState>>,
    stream: CaptureStream,
) -> thread::JoinHandle<Result<(), std::io::Error>> {
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                return Ok(());
            }
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let retained = count.min(state.remaining);
            match stream {
                CaptureStream::Stdout => state.stdout.extend_from_slice(&buffer[..retained]),
                CaptureStream::Stderr => state.stderr.extend_from_slice(&buffer[..retained]),
            }
            state.remaining = state.remaining.saturating_sub(retained);
            state.truncated |= retained < count;
        }
    })
}

fn supervise(
    command: &mut Command,
    job: &SandboxJob,
    backend: String,
) -> Result<SandboxJobResult, SandboxHelperError> {
    let mut child = command
        .group_spawn()
        .map_err(|error| SandboxHelperError::Execution(error.to_string()))?;
    if let Some(encoded) = &job.process.stdin_base64 {
        let input = BASE64
            .decode(encoded)
            .map_err(|error| SandboxHelperError::Execution(error.to_string()))?;
        let mut stdin = child
            .inner()
            .stdin
            .take()
            .ok_or_else(|| SandboxHelperError::Execution("child stdin is absent".into()))?;
        stdin.write_all(&input)?;
    }
    drop(child.inner().stdin.take());
    let stdout = child
        .inner()
        .stdout
        .take()
        .ok_or_else(|| SandboxHelperError::Execution("child stdout is absent".into()))?;
    let stderr = child
        .inner()
        .stderr
        .take()
        .ok_or_else(|| SandboxHelperError::Execution("child stderr is absent".into()))?;
    let output_limit = usize::try_from(job.obligations.max_output_bytes).unwrap_or(usize::MAX);
    let capture_limit = output_limit.saturating_sub(1024).saturating_mul(3) / 4;
    let state = Arc::new(Mutex::new(CaptureState {
        remaining: capture_limit,
        ..CaptureState::default()
    }));
    let stdout_handle = capture(stdout, Arc::clone(&state), CaptureStream::Stdout);
    let stderr_handle = capture(stderr, Arc::clone(&state), CaptureStream::Stderr);
    let started = Instant::now();
    let timeout = Duration::from_millis(job.timeout_ms);
    let mut system = System::new();
    let root_pid = SystemPid::from_u32(child.id());
    let (status, timed_out, resource_limit_exceeded) = loop {
        if let Some(status) = child.try_wait()? {
            // The group/job can outlive its leader when an executable backgrounds work.
            // Always terminate remaining descendants before returning a terminal result.
            let _ = child.kill();
            break (status, false, None);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            break (child.wait()?, true, None);
        }
        let (processes, memory) = process_tree_usage(&mut system, root_pid);
        let limit =
            if processes > usize::try_from(job.obligations.max_processes).unwrap_or(usize::MAX) {
                Some("process-count")
            } else if memory > job.obligations.max_memory_bytes {
                Some("memory")
            } else {
                None
            };
        if let Some(limit) = limit {
            let _ = child.kill();
            break (child.wait()?, false, Some(limit.into()));
        }
        thread::sleep(Duration::from_millis(10));
    };
    stdout_handle
        .join()
        .map_err(|_| SandboxHelperError::Execution("stdout capture panicked".into()))??;
    stderr_handle
        .join()
        .map_err(|_| SandboxHelperError::Execution("stderr capture panicked".into()))??;
    let state = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Ok(SandboxJobResult {
        backend,
        exit_code: status.code(),
        success: status.success() && !timed_out,
        timed_out,
        resource_limit_exceeded,
        output_truncated: state.truncated,
        stdout_base64: BASE64.encode(&state.stdout),
        stderr_base64: BASE64.encode(&state.stderr),
    })
}

fn process_tree_usage(system: &mut System, root: SystemPid) -> (usize, u64) {
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    let mut members = std::collections::HashSet::from([root]);
    loop {
        let before = members.len();
        for (pid, process) in system.processes() {
            if process
                .parent()
                .is_some_and(|parent| members.contains(&parent))
            {
                members.insert(*pid);
            }
        }
        if members.len() == before {
            break;
        }
    }
    let memory = members
        .iter()
        .filter_map(|pid| system.process(*pid))
        .fold(0_u64, |total, process| {
            total.saturating_add(process.memory())
        });
    (members.len(), memory)
}

/// Permit-bound HTTP adapter with exact-origin authorization, pinned DNS, no redirects,
/// and bounded response streaming.
#[derive(Default)]
pub struct HttpExecutor;

impl HttpExecutor {
    /// Construct the brokered HTTP adapter.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EffectExecutor for HttpExecutor {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if request.action != "network.http" {
            return Err(adapter_failure("HTTP executor received another action"));
        }
        let url = Url::parse(&request.resource).map_err(adapter_failure)?;
        let origin = url.origin().ascii_serialization();
        if !permit.obligations().network_destinations.contains(&origin) {
            return Err(adapter_failure("HTTP origin is not permitted"));
        }
        let host = url
            .host_str()
            .ok_or_else(|| adapter_failure("HTTP URL has no host"))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| adapter_failure("HTTP URL has no port"))?;
        let addresses = resolve_destinations(host, port).await?;
        let client = Client::builder()
            .redirect(RedirectPolicy::none())
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .timeout(Duration::from_millis(permit.obligations().timeout_ms))
            .build()
            .map_err(adapter_failure)?;
        let method = request
            .content
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .parse()
            .map_err(adapter_failure)?;
        let mut builder = client.request(method, url);
        if let Some(headers) = request.content.get("headers").and_then(Value::as_object) {
            for (name, value) in headers {
                let normalized = name.to_ascii_lowercase();
                if !matches!(
                    normalized.as_str(),
                    "accept" | "content-type" | "user-agent"
                ) {
                    return Err(adapter_failure(format!(
                        "HTTP header {name} is not in the safe adapter allowlist"
                    )));
                }
                let value = value
                    .as_str()
                    .ok_or_else(|| adapter_failure("HTTP header values must be strings"))?;
                builder = builder.header(name, value);
            }
        }
        if let Some(encoded) = request.content.get("body_base64").and_then(Value::as_str) {
            let body = BASE64.decode(encoded).map_err(adapter_failure)?;
            if u64::try_from(body.len()).map_err(adapter_failure)?
                > permit.obligations().max_output_bytes
            {
                return Err(adapter_failure(
                    "HTTP request body exceeds the permitted bound",
                ));
            }
            builder = builder.body(body);
        }
        let response = builder.send().await.map_err(adapter_failure)?;
        if !response.status().is_success() {
            return Err(adapter_failure(format!(
                "HTTP destination returned {}",
                response.status()
            )));
        }
        let media_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let limit =
            usize::try_from(permit.obligations().max_output_bytes).map_err(adapter_failure)?;
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(adapter_failure)?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(adapter_failure("HTTP response exceeds the permitted bound"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(QuarantinedEffectResult {
            media_type,
            bytes,
            effect_succeeded: true,
        })
    }
}

async fn resolve_destinations(host: &str, port: u16) -> Result<Vec<SocketAddr>, ExecutionError> {
    let host_is_ip = host.parse::<IpAddr>().is_ok();
    let mut addresses = lookup_host((host, port))
        .await
        .map_err(adapter_failure)?
        .filter(|address| host_is_ip || !non_public_ip(address.ip()))
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(adapter_failure(
            "network destination resolved to no permitted address",
        ));
    }
    addresses.sort_by_key(|address| usize::from(address.is_ipv6()));
    addresses.dedup();
    addresses.truncate(16);
    Ok(addresses)
}

async fn connect_destination(
    host: &str,
    port: u16,
    pinned: Option<&[SocketAddr]>,
) -> Result<TcpStream, ExecutionError> {
    let mut attempts = FuturesUnordered::new();
    let addresses = if let Some(pinned) = pinned {
        pinned.to_vec()
    } else {
        resolve_destinations(host, port).await?
    };
    for address in addresses {
        attempts.push(TcpStream::connect(address));
    }
    while let Some(result) = attempts.next().await {
        if let Ok(stream) = result {
            return Ok(stream);
        }
    }
    Err(adapter_failure(
        "network destination did not accept a connection on any permitted address",
    ))
}

fn non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

struct AllowlistProxy {
    address: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OciProxyBootstrap {
    schema_version: u16,
    request_hash: String,
    decision_id: String,
    permit_nonce: String,
    expires_at_unix_ms: i128,
    allowed_origins: Vec<String>,
    resolved_origins: BTreeMap<String, Vec<SocketAddr>>,
    max_connections: usize,
    connection_timeout_ms: u64,
}

/// Run the trusted OCI proxy sidecar from its bounded environment bootstrap.
pub async fn run_oci_proxy_from_environment() -> Result<(), ExecutionError> {
    let encoded = std::env::var(OCI_PROXY_CONFIG_VARIABLE).map_err(adapter_failure)?;
    let bytes = BASE64.decode(encoded).map_err(adapter_failure)?;
    if bytes.len() > MAX_JOB_BYTES {
        return Err(adapter_failure(
            "OCI proxy bootstrap exceeds its input bound",
        ));
    }
    let bootstrap: OciProxyBootstrap = serde_json::from_slice(&bytes).map_err(adapter_failure)?;
    let now_ms = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
    if bootstrap.schema_version != 1
        || bootstrap.request_hash.is_empty()
        || bootstrap.decision_id.is_empty()
        || bootstrap.permit_nonce.is_empty()
        || bootstrap.expires_at_unix_ms < now_ms
        || bootstrap.allowed_origins.is_empty()
        || bootstrap.resolved_origins.len() != bootstrap.allowed_origins.len()
        || bootstrap.max_connections == 0
        || bootstrap.max_connections > 256
        || bootstrap.connection_timeout_ms == 0
    {
        return Err(adapter_failure("invalid OCI proxy bootstrap"));
    }
    for origin in &bootstrap.allowed_origins {
        let url = Url::parse(origin).map_err(adapter_failure)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.origin().ascii_serialization() != *origin
        {
            return Err(adapter_failure(format!(
                "OCI proxy origin is not canonical: {origin}"
            )));
        }
        let host = url
            .host_str()
            .ok_or_else(|| adapter_failure("OCI proxy origin has no host"))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| adapter_failure("OCI proxy origin has no port"))?;
        let host_ip = host.parse::<IpAddr>().ok();
        let addresses = bootstrap
            .resolved_origins
            .get(origin)
            .ok_or_else(|| adapter_failure("OCI proxy origin has no pinned addresses"))?;
        if addresses.is_empty()
            || addresses.len() > 16
            || addresses.iter().any(|address| {
                address.port() != port
                    || host_ip.map_or_else(
                        || non_public_ip(address.ip()),
                        |host_ip| address.ip() != host_ip,
                    )
            })
        {
            return Err(adapter_failure(format!(
                "OCI proxy origin has invalid pinned addresses: {origin}"
            )));
        }
    }
    let listener = TcpListener::bind(("0.0.0.0", OCI_PROXY_PORT))
        .await
        .map_err(adapter_failure)?;
    let allowed = Arc::new(bootstrap.allowed_origins);
    let resolved = Arc::new(bootstrap.resolved_origins);
    let concurrency = Arc::new(Semaphore::new(bootstrap.max_connections));
    let connection_timeout = Duration::from_millis(bootstrap.connection_timeout_ms);
    println!("colossus-oci-proxy-ready");
    loop {
        let (stream, _) = listener.accept().await.map_err(adapter_failure)?;
        let now_ms = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
        if now_ms >= bootstrap.expires_at_unix_ms {
            drop(stream);
            return Err(adapter_failure("OCI proxy permit expired"));
        }
        let Ok(permit) = Arc::clone(&concurrency).try_acquire_owned() else {
            drop(stream);
            continue;
        };
        let allowed = Arc::clone(&allowed);
        let resolved = Arc::clone(&resolved);
        tokio::spawn(async move {
            let _permit = permit;
            match tokio::time::timeout(
                connection_timeout,
                proxy_connection(stream, allowed.as_slice(), resolved.as_ref()),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("colossus-oci-proxy-connection-failed: {error}"),
                Err(_) => eprintln!("colossus-oci-proxy-connection-timed-out"),
            }
        });
    }
}

impl AllowlistProxy {
    async fn start(origins: Vec<String>) -> Result<Self, ExecutionError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(adapter_failure)?;
        let address = listener.local_addr().map_err(adapter_failure)?;
        let allowed = Arc::new(origins);
        let resolved = Arc::new(BTreeMap::new());
        let (shutdown, mut shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        let allowed = Arc::clone(&allowed);
                        let resolved = Arc::clone(&resolved);
                        tokio::spawn(async move {
                            let _ = proxy_connection(
                                stream,
                                allowed.as_slice(),
                                resolved.as_ref(),
                            )
                            .await;
                        });
                    }
                }
            }
        });
        Ok(Self {
            address,
            shutdown: Some(shutdown),
            task,
        })
    }

    fn port(&self) -> u16 {
        self.address.port()
    }
}

impl Drop for AllowlistProxy {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

async fn proxy_connection(
    mut client: TcpStream,
    allowed_origins: &[String],
    resolved_origins: &BTreeMap<String, Vec<SocketAddr>>,
) -> Result<(), ExecutionError> {
    let mut header = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !header.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = client.read(&mut buffer).await.map_err(adapter_failure)?;
        if count == 0 || header.len().saturating_add(count) > MAX_PROXY_HEADER_BYTES {
            return Err(adapter_failure(
                "proxy request header is absent or oversized",
            ));
        }
        header.extend_from_slice(&buffer[..count]);
    }
    let header_end = header
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position.saturating_add(4))
        .ok_or_else(|| adapter_failure("proxy request header terminator is absent"))?;
    let text = std::str::from_utf8(&header[..header_end]).map_err(adapter_failure)?;
    let first_line = text
        .lines()
        .next()
        .ok_or_else(|| adapter_failure("proxy request line is absent"))?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = authority(target, 443)?;
        let origin = canonical_origin("https", &host, port)?;
        if !allowed_origins.contains(&origin) {
            client
                .write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")
                .await
                .map_err(adapter_failure)?;
            return Ok(());
        }
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .map_err(adapter_failure)?;
        let client_hello = read_tls_client_hello(&mut client, &header[header_end..]).await?;
        let server_name = tls_server_name(&client_hello)?;
        if host.parse::<IpAddr>().is_err()
            && !server_name.is_some_and(|server_name| server_name.eq_ignore_ascii_case(&host))
        {
            return Err(adapter_failure(
                "TLS server name does not match the permitted CONNECT authority",
            ));
        }
        let mut upstream = connect_destination(
            &host,
            port,
            resolved_origins.get(&origin).map(Vec::as_slice),
        )
        .await?;
        upstream
            .write_all(&client_hello)
            .await
            .map_err(adapter_failure)?;
        tokio::io::copy_bidirectional(&mut client, &mut upstream)
            .await
            .map_err(adapter_failure)?;
        return Ok(());
    }
    let url = Url::parse(target).map_err(adapter_failure)?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(adapter_failure(
            "plain proxy requests require an absolute credential-free HTTP URL",
        ));
    }
    let origin = url.origin().ascii_serialization();
    if !allowed_origins.contains(&origin) {
        client
            .write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")
            .await
            .map_err(adapter_failure)?;
        return Ok(());
    }
    let host = url
        .host_str()
        .ok_or_else(|| adapter_failure("proxy URL has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| adapter_failure("proxy URL has no port"))?;
    let host_header = single_header_value(text, "host")?
        .ok_or_else(|| adapter_failure("proxy request has no Host header"))?;
    let (header_host, header_port) = authority(host_header, port)?;
    if canonical_origin("http", &header_host, header_port)? != origin {
        return Err(adapter_failure(
            "HTTP Host header does not match the permitted request origin",
        ));
    }
    let mut upstream =
        connect_destination(host, port, resolved_origins.get(&origin).map(Vec::as_slice)).await?;
    let path = if let Some(query) = url.query() {
        format!("{}?{query}", url.path())
    } else {
        url.path().to_owned()
    };
    let rewritten = text
        .lines()
        .filter(|line| {
            !line
                .to_ascii_lowercase()
                .starts_with("proxy-authorization:")
        })
        .collect::<Vec<_>>()
        .join("\r\n")
        .replacen(first_line, &format!("{method} {path} HTTP/1.1"), 1);
    upstream
        .write_all(format!("{rewritten}\r\n").as_bytes())
        .await
        .map_err(adapter_failure)?;
    upstream
        .write_all(&header[header_end..])
        .await
        .map_err(adapter_failure)?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream)
        .await
        .map_err(adapter_failure)?;
    Ok(())
}

fn single_header_value<'a>(
    header: &'a str,
    expected_name: &str,
) -> Result<Option<&'a str>, ExecutionError> {
    let mut value = None;
    for line in header.lines().skip(1) {
        let Some((name, candidate)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(expected_name) {
            if value.is_some() {
                return Err(adapter_failure(format!(
                    "proxy request contains multiple {expected_name} headers"
                )));
            }
            let candidate = candidate.trim();
            if candidate.is_empty() {
                return Err(adapter_failure(format!(
                    "proxy request contains an empty {expected_name} header"
                )));
            }
            value = Some(candidate);
        }
    }
    Ok(value)
}

async fn read_tls_client_hello(
    client: &mut TcpStream,
    initial: &[u8],
) -> Result<Vec<u8>, ExecutionError> {
    let mut captured = initial.to_vec();
    let mut handshake = Vec::new();
    let mut offset = 0_usize;
    loop {
        read_proxy_bytes(client, &mut captured, offset.saturating_add(5)).await?;
        if captured[offset] != 22 {
            return Err(adapter_failure(
                "CONNECT tunnel did not begin with a TLS handshake record",
            ));
        }
        let record_len = usize::from(u16::from_be_bytes([
            captured[offset + 3],
            captured[offset + 4],
        ]));
        if record_len == 0 || record_len > MAX_TLS_RECORD_BYTES {
            return Err(adapter_failure("TLS handshake record is oversized"));
        }
        let record_end = offset.saturating_add(5).saturating_add(record_len);
        read_proxy_bytes(client, &mut captured, record_end).await?;
        handshake.extend_from_slice(&captured[offset + 5..record_end]);
        if handshake.len() > MAX_TLS_CLIENT_HELLO_BYTES {
            return Err(adapter_failure("TLS ClientHello is oversized"));
        }
        if handshake.len() >= 4 {
            if handshake[0] != 1 {
                return Err(adapter_failure(
                    "CONNECT tunnel did not begin with a TLS ClientHello",
                ));
            }
            let hello_len = (usize::from(handshake[1]) << 16)
                | (usize::from(handshake[2]) << 8)
                | usize::from(handshake[3]);
            if hello_len > MAX_TLS_CLIENT_HELLO_BYTES.saturating_sub(4) {
                return Err(adapter_failure("TLS ClientHello is oversized"));
            }
            if handshake.len() >= hello_len.saturating_add(4) {
                return Ok(captured);
            }
        }
        offset = record_end;
    }
}

async fn read_proxy_bytes(
    client: &mut TcpStream,
    captured: &mut Vec<u8>,
    required: usize,
) -> Result<(), ExecutionError> {
    while captured.len() < required {
        if required > MAX_TLS_CLIENT_HELLO_BYTES.saturating_add(MAX_TLS_RECORD_BYTES) {
            return Err(adapter_failure("TLS ClientHello is oversized"));
        }
        let mut buffer = [0_u8; 4096];
        let count = client.read(&mut buffer).await.map_err(adapter_failure)?;
        if count == 0 {
            return Err(adapter_failure("TLS ClientHello ended unexpectedly"));
        }
        captured.extend_from_slice(&buffer[..count]);
    }
    Ok(())
}

fn tls_server_name(client_hello_records: &[u8]) -> Result<Option<String>, ExecutionError> {
    let mut handshake = Vec::new();
    let mut offset = 0_usize;
    while offset.saturating_add(5) <= client_hello_records.len() {
        if client_hello_records[offset] != 22 {
            break;
        }
        let record_len = usize::from(u16::from_be_bytes([
            client_hello_records[offset + 3],
            client_hello_records[offset + 4],
        ]));
        let record_end = offset.saturating_add(5).saturating_add(record_len);
        if record_end > client_hello_records.len() {
            return Err(adapter_failure("TLS ClientHello record is truncated"));
        }
        handshake.extend_from_slice(&client_hello_records[offset + 5..record_end]);
        if handshake.len() >= 4 {
            let hello_len = (usize::from(handshake[1]) << 16)
                | (usize::from(handshake[2]) << 8)
                | usize::from(handshake[3]);
            if handshake.len() >= hello_len.saturating_add(4) {
                break;
            }
        }
        offset = record_end;
    }
    let hello_len = tls_u24(&handshake, 1)?;
    if handshake.first() != Some(&1) || handshake.len() < hello_len.saturating_add(4) {
        return Err(adapter_failure("TLS ClientHello is invalid"));
    }
    let body = &handshake[4..4 + hello_len];
    let mut cursor = 34;
    cursor = skip_tls_vector(body, cursor, 1)?;
    cursor = skip_tls_vector(body, cursor, 2)?;
    cursor = skip_tls_vector(body, cursor, 1)?;
    if cursor == body.len() {
        return Ok(None);
    }
    let extensions_len = tls_u16(body, cursor)?;
    cursor = cursor.saturating_add(2);
    let extensions_end = cursor.saturating_add(extensions_len);
    if extensions_end != body.len() {
        return Err(adapter_failure("TLS ClientHello extensions are invalid"));
    }
    while cursor < extensions_end {
        let extension_type = tls_u16(body, cursor)?;
        let extension_len = tls_u16(body, cursor.saturating_add(2))?;
        cursor = cursor.saturating_add(4);
        let extension_end = cursor.saturating_add(extension_len);
        if extension_end > extensions_end {
            return Err(adapter_failure("TLS ClientHello extension is truncated"));
        }
        if extension_type == 0 {
            let names_len = tls_u16(body, cursor)?;
            let mut name_cursor = cursor.saturating_add(2);
            if name_cursor.saturating_add(names_len) != extension_end {
                return Err(adapter_failure("TLS server-name extension is invalid"));
            }
            while name_cursor < extension_end {
                let name_type = *body
                    .get(name_cursor)
                    .ok_or_else(|| adapter_failure("TLS server name is truncated"))?;
                let name_len = tls_u16(body, name_cursor.saturating_add(1))?;
                name_cursor = name_cursor.saturating_add(3);
                let name_end = name_cursor.saturating_add(name_len);
                if name_end > extension_end {
                    return Err(adapter_failure("TLS server name is truncated"));
                }
                if name_type == 0 {
                    let name = std::str::from_utf8(&body[name_cursor..name_end])
                        .map_err(adapter_failure)?;
                    if name.is_empty() || !name.is_ascii() {
                        return Err(adapter_failure("TLS server name is invalid"));
                    }
                    return Ok(Some(name.to_owned()));
                }
                name_cursor = name_end;
            }
            return Ok(None);
        }
        cursor = extension_end;
    }
    Ok(None)
}

fn tls_u16(bytes: &[u8], offset: usize) -> Result<usize, ExecutionError> {
    let high = *bytes
        .get(offset)
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    let low = *bytes
        .get(offset.saturating_add(1))
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    Ok((usize::from(high) << 8) | usize::from(low))
}

fn tls_u24(bytes: &[u8], offset: usize) -> Result<usize, ExecutionError> {
    let first = *bytes
        .get(offset)
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    let second = *bytes
        .get(offset.saturating_add(1))
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    let third = *bytes
        .get(offset.saturating_add(2))
        .ok_or_else(|| adapter_failure("TLS structure is truncated"))?;
    Ok((usize::from(first) << 16) | (usize::from(second) << 8) | usize::from(third))
}

fn skip_tls_vector(
    bytes: &[u8],
    offset: usize,
    length_bytes: usize,
) -> Result<usize, ExecutionError> {
    let length = match length_bytes {
        1 => usize::from(
            *bytes
                .get(offset)
                .ok_or_else(|| adapter_failure("TLS vector is truncated"))?,
        ),
        2 => tls_u16(bytes, offset)?,
        _ => return Err(adapter_failure("TLS vector length is unsupported")),
    };
    let end = offset.saturating_add(length_bytes).saturating_add(length);
    if end > bytes.len() {
        return Err(adapter_failure("TLS vector is truncated"));
    }
    Ok(end)
}

fn authority(value: &str, default_port: u16) -> Result<(String, u16), ExecutionError> {
    let url = Url::parse(&format!("https://{value}")).map_err(adapter_failure)?;
    let host = url
        .host_str()
        .ok_or_else(|| adapter_failure("proxy authority has no host"))?;
    Ok((host.into(), url.port().unwrap_or(default_port)))
}

fn canonical_origin(scheme: &str, host: &str, port: u16) -> Result<String, ExecutionError> {
    Url::parse(&format!("{scheme}://{host}:{port}"))
        .map(|url| url.origin().ascii_serialization())
        .map_err(adapter_failure)
}

#[cfg(test)]
mod tests {
    use super::{
        AllowlistProxy, FilesystemExecutor, HttpExecutor, SandboxJob, SignedSandboxJob,
        atomic_write, authority, non_public_ip, oci_command, oci_remove_arguments,
        proposed_write_bytes, resolve_oci_origins, tls_server_name, validate_process_spec,
    };
    use colossus_contracts::{DecisionOutcome, PolicyObligations};
    use colossus_policy::{
        BuiltInPolicy, DenyApproval, EffectGateway, SafetyKernel, effect_request, system_actor,
    };
    use colossus_ports::EventJournal;
    use colossus_testkit::InMemoryEventJournal;
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        net::{IpAddr, Ipv4Addr, SocketAddr},
        path::PathBuf,
        sync::Arc,
    };
    use tempfile::tempdir;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    #[test]
    fn atomic_write_replaces_content_without_following_leaf_symlinks() {
        let directory = tempdir().expect("tempdir");
        let target = directory.path().join("target");
        atomic_write(&target, b"first").expect("first");
        atomic_write(&target, b"second").expect("second");
        assert_eq!(std::fs::read(target).expect("read"), b"second");
    }

    #[test]
    fn write_payload_is_strict_and_bounded() {
        assert_eq!(
            proposed_write_bytes(&json!({"text": "ok"}), 2).expect("text"),
            b"ok"
        );
        assert!(proposed_write_bytes(&json!({"text": "too large"}), 2).is_err());
        assert!(proposed_write_bytes(&json!({"unknown": true}), 20).is_err());
    }

    #[test]
    fn authenticated_helper_job_rejects_tampering_and_expiry() {
        let job = SandboxJob {
            schema_version: 1,
            job_id: "018f0f9b-7b6e-7cc0-8000-000000000001".into(),
            request_id: "request".into(),
            request_hash: "hash".into(),
            decision_id: "decision".into(),
            permit_nonce: "nonce".into(),
            permit_expires_at_unix_ms: i128::MAX,
            executable: PathBuf::from("/bin/echo"),
            process: super::ProcessSpec {
                cwd: PathBuf::from("/tmp"),
                args: Vec::new(),
                environment: BTreeMap::new(),
                stdin_base64: None,
            },
            obligations: PolicyObligations::default(),
            timeout_ms: 1,
            proxy_port: None,
            oci_runtime: None,
            oci_image: None,
            oci_proxy_image: None,
        };
        let key = [7_u8; 32];
        let signed = SignedSandboxJob::sign(job, &key).expect("sign");
        assert!(signed.clone().verify(&key).is_ok());
        assert!(signed.verify(&[8_u8; 32]).is_err());
    }

    #[test]
    fn proxy_authorities_and_private_ranges_are_strict() {
        assert_eq!(
            authority("example.com:8443", 443).expect("authority"),
            ("example.com".into(), 8443)
        );
        assert!(non_public_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(!non_public_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        let resolved =
            resolve_oci_origins(&["http://127.0.0.1:18080".into()]).expect("explicit IP origin");
        assert_eq!(
            resolved["http://127.0.0.1:18080"],
            [SocketAddr::from(([127, 0, 0, 1], 18_080))]
        );
    }

    #[test]
    fn tls_client_hello_server_name_is_extracted_for_connect_enforcement() {
        let records = tls_client_hello("api.example.com");
        assert_eq!(
            tls_server_name(&records).expect("server name"),
            Some("api.example.com".into())
        );
        let mut truncated = records;
        truncated.pop();
        assert!(tls_server_name(&truncated).is_err());
    }

    fn tls_client_hello(server_name: &str) -> Vec<u8> {
        let mut server_name_extension = Vec::new();
        let name_len = u16::try_from(server_name.len()).expect("name length");
        server_name_extension.extend_from_slice(&(name_len + 3).to_be_bytes());
        server_name_extension.push(0);
        server_name_extension.extend_from_slice(&name_len.to_be_bytes());
        server_name_extension.extend_from_slice(server_name.as_bytes());

        let mut extensions = Vec::new();
        extensions.extend_from_slice(&0_u16.to_be_bytes());
        extensions.extend_from_slice(
            &u16::try_from(server_name_extension.len())
                .expect("extension length")
                .to_be_bytes(),
        );
        extensions.extend_from_slice(&server_name_extension);

        let mut body = Vec::new();
        body.extend_from_slice(&[3, 3]);
        body.extend_from_slice(&[7; 32]);
        body.push(0);
        body.extend_from_slice(&2_u16.to_be_bytes());
        body.extend_from_slice(&[0x13, 0x01]);
        body.push(1);
        body.push(0);
        body.extend_from_slice(
            &u16::try_from(extensions.len())
                .expect("extensions length")
                .to_be_bytes(),
        );
        body.extend_from_slice(&extensions);

        let mut handshake = vec![
            1,
            u8::try_from((body.len() >> 16) & 0xff).expect("length"),
            u8::try_from((body.len() >> 8) & 0xff).expect("length"),
            u8::try_from(body.len() & 0xff).expect("length"),
        ];
        handshake.extend_from_slice(&body);

        let mut record = vec![22, 3, 1];
        record.extend_from_slice(
            &u16::try_from(handshake.len())
                .expect("record length")
                .to_be_bytes(),
        );
        record.extend_from_slice(&handshake);
        record
    }

    #[test]
    fn oci_profile_applies_resource_and_privilege_limits_without_argv_secrets() {
        let directory = tempdir().expect("directory");
        let mut obligations = PolicyObligations {
            sandbox_backend: "oci".into(),
            sandbox_profile: "test".into(),
            max_output_bytes: 1024,
            max_processes: 2,
            max_memory_bytes: 64 * 1024 * 1024,
            max_concurrency: 1,
            timeout_ms: 1000,
            retention: "test".into(),
            ..PolicyObligations::default()
        };
        obligations
            .filesystem
            .push(colossus_contracts::FilesystemGrant {
                root: directory.path().display().to_string(),
                mode: "write".into(),
            });
        obligations
            .filesystem
            .push(colossus_contracts::FilesystemGrant {
                root: "/usr/bin/example".into(),
                mode: "execute".into(),
            });
        obligations.allowed_environment.push("TOKEN".into());
        let mut job = SandboxJob {
            schema_version: 1,
            job_id: "018f0f9b-7b6e-7cc0-8000-000000000002".into(),
            request_id: "request".into(),
            request_hash: "hash".into(),
            decision_id: "decision".into(),
            permit_nonce: "nonce".into(),
            permit_expires_at_unix_ms: i128::MAX,
            executable: PathBuf::from("/usr/bin/example"),
            process: super::ProcessSpec {
                cwd: directory.path().into(),
                args: vec!["check".into()],
                environment: BTreeMap::from([("TOKEN".into(), "secret-value".into())]),
                stdin_base64: None,
            },
            obligations,
            timeout_ms: 1000,
            proxy_port: None,
            oci_runtime: Some(PathBuf::from("/usr/bin/docker")),
            oci_image: Some(format!("example@sha256:{}", "a".repeat(64))),
            oci_proxy_image: None,
        };
        validate_process_spec(&job.process, "/usr/bin/example", &job.obligations)
            .expect("exact OCI image executable");
        let command = oci_command(&job, None).expect("OCI command");
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.contains(&"--network=none".into()));
        assert!(args.contains(&"--pull=never".into()));
        assert!(args.contains(&"--read-only".into()));
        assert!(args.contains(&"--cap-drop=ALL".into()));
        assert!(args.contains(&"--pids-limit=2".into()));
        assert!(args.contains(&format!("--memory={}", 64 * 1024 * 1024)));
        assert!(args.contains(&"--entrypoint".into()));
        assert!(args.contains(&"colossus-018f0f9b7b6e7cc08000000000000002".into()));
        assert!(
            !args
                .iter()
                .any(|argument| argument.contains("secret-value"))
        );
        assert!(command.get_envs().any(|(name, value)| {
            name == "TOKEN" && value.is_some_and(|value| value == "secret-value")
        }));
        assert_eq!(
            oci_remove_arguments(PathBuf::from("/usr/bin/docker").as_path(), "job")
                .expect("Docker cleanup"),
            ["container", "rm", "--force", "job"]
        );
        assert_eq!(
            oci_remove_arguments(PathBuf::from("/usr/bin/podman").as_path(), "job")
                .expect("Podman cleanup"),
            ["container", "rm", "--force", "--time", "0", "job"]
        );
        assert_eq!(
            oci_remove_arguments(PathBuf::from("/usr/bin/podman-remote").as_path(), "job")
                .expect("Podman remote cleanup"),
            ["container", "rm", "--force", "--time", "0", "job"]
        );
        assert!(oci_remove_arguments(PathBuf::from("/usr/bin/unknown").as_path(), "job").is_none());

        job.obligations
            .network_destinations
            .push("https://example.com".into());
        job.oci_proxy_image = Some(format!("sha256:{}", "b".repeat(64)));
        let proxy_address = SocketAddr::from(([10, 88, 0, 2], super::OCI_PROXY_PORT));
        let command = oci_command(&job, Some(proxy_address)).expect("networked OCI command");
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(!args.contains(&"--network=none".into()));
        assert!(args.contains(&"--dns=127.0.0.1".into()));
        assert!(args.contains(&super::oci_resource_names(&job.job_id).internal_network));
        assert!(!args.iter().any(|argument| argument.contains("10.88.0.2")));
        assert!(command.get_envs().any(|(name, value)| {
            name == "HTTPS_PROXY" && value.is_some_and(|value| value == "http://10.88.0.2:18080")
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn filesystem_symlink_escape_is_denied_before_release() {
        use std::os::unix::fs::symlink;

        let allowed = tempdir().expect("allowed");
        let denied = tempdir().expect("denied");
        let secret = denied.path().join("secret");
        std::fs::write(&secret, "secret").expect("secret");
        let escape = allowed.path().join("escape");
        symlink(&secret, &escape).expect("symlink");
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy = BuiltInPolicy::offline_default()
            .with_action("filesystem.read", DecisionOutcome::Allow)
            .with_filesystem_read_root(allowed.path().display().to_string());
        let gateway = EffectGateway::new(
            journal,
            Arc::new(policy),
            Arc::new(DenyApproval),
            SafetyKernel::new(["filesystem.read".into()]),
            [4_u8; 32],
        );
        let mut request = effect_request(
            system_actor("test"),
            "filesystem.read",
            escape.display().to_string(),
            json!({}),
        );
        request.capabilities = vec!["filesystem.read".into()];
        assert!(
            gateway
                .execute(request, &FilesystemExecutor::new())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn filesystem_write_is_permit_bound_and_atomic() {
        let directory = tempdir().expect("directory");
        let target = directory.path().join("created.txt");
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy = BuiltInPolicy::offline_default()
            .with_action("filesystem.write", DecisionOutcome::Allow)
            .with_filesystem_root(directory.path().display().to_string(), "write");
        let gateway = EffectGateway::new(
            journal,
            Arc::new(policy),
            Arc::new(DenyApproval),
            SafetyKernel::new(["filesystem.write".into()]),
            [4_u8; 32],
        );
        let mut request = effect_request(
            system_actor("test"),
            "filesystem.write",
            target.display().to_string(),
            json!({"text": "durable"}),
        );
        request.capabilities = vec!["filesystem.write".into()];
        gateway
            .execute(request, &FilesystemExecutor::new())
            .await
            .expect("write");
        assert_eq!(std::fs::read_to_string(target).expect("read"), "durable");
    }

    #[tokio::test]
    async fn filesystem_search_is_bounded_utf8_only_and_skips_control_state() {
        let directory = tempdir().expect("directory");
        std::fs::create_dir_all(directory.path().join("src")).expect("src");
        std::fs::create_dir_all(directory.path().join(".colossus")).expect("control");
        std::fs::write(
            directory.path().join("src/example.rs"),
            "first\nNeedle here\nneedle again\n",
        )
        .expect("fixture");
        std::fs::write(directory.path().join("src/blob.bin"), b"needle\0hidden")
            .expect("binary fixture");
        std::fs::write(directory.path().join(".colossus/secret"), "needle secret")
            .expect("control fixture");
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy = BuiltInPolicy::offline_default()
            .with_action("filesystem.search", DecisionOutcome::Allow)
            .with_filesystem_read_root(directory.path().display().to_string());
        let gateway = EffectGateway::new(
            journal,
            Arc::new(policy),
            Arc::new(DenyApproval),
            SafetyKernel::new(["filesystem.search".into()]),
            [4_u8; 32],
        );
        let mut request = effect_request(
            system_actor("test"),
            "filesystem.search",
            directory.path().display().to_string(),
            json!({
                "pattern": "needle",
                "regex": false,
                "case_sensitive": false,
                "glob": "**/*.rs",
                "max_matches": 1,
            }),
        );
        request.capabilities = vec!["filesystem.search".into()];
        let result = gateway
            .execute(request, &FilesystemExecutor::new())
            .await
            .expect("search");
        let value: serde_json::Value = serde_json::from_slice(&result.bytes).expect("JSON");
        assert_eq!(value["matches"][0]["path"], "src/example.rs");
        assert_eq!(value["matches"][0]["line"], 2);
        assert_eq!(value["matches"][0]["column"], 1);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["matches"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn brokered_http_is_exact_origin_bounded_and_post_authorized() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listen");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.expect("read");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: text/plain\r\n\r\nok",
                )
                .await
                .expect("write");
        });
        let origin = format!("http://{address}");
        let url = format!("{origin}/health");
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let policy = BuiltInPolicy::offline_default()
            .with_action("network.http", DecisionOutcome::Allow)
            .with_network_destination(&origin);
        let gateway = EffectGateway::new(
            Arc::clone(&journal),
            Arc::new(policy),
            Arc::new(DenyApproval),
            SafetyKernel::new(["network.http".into()]),
            [4_u8; 32],
        );
        let mut request = effect_request(
            system_actor("test"),
            "network.http",
            &url,
            json!({"method": "GET", "headers": {}}),
        );
        request.capabilities = vec!["network.http".into()];
        let result = gateway
            .execute(request, &HttpExecutor::new())
            .await
            .expect("request");
        assert_eq!(result.bytes, b"ok");
        assert!(
            journal
                .read_global(1, 30)
                .expect("events")
                .iter()
                .any(|event| event.event_type == "effect.release_requested.v1")
        );
        server.await.expect("server");
    }

    #[tokio::test]
    async fn allowlist_proxy_rejects_an_unlisted_origin_without_connecting_upstream() {
        let proxy = AllowlistProxy::start(vec!["https://example.com".into()])
            .await
            .expect("proxy");
        let mut stream = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .expect("connect");
        stream
            .write_all(b"CONNECT denied.example:443 HTTP/1.1\r\nHost: denied.example\r\n\r\n")
            .await
            .expect("write");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.expect("response");
        assert!(response.starts_with(b"HTTP/1.1 403"));
    }

    #[tokio::test]
    async fn allowlist_proxy_forwards_an_exact_http_origin() {
        let upstream = TcpListener::bind(("127.0.0.1", 0)).await.expect("listen");
        let address = upstream.local_addr().expect("address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = upstream.accept().await.expect("accept");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).await.expect("read");
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            assert!(request.starts_with(b"GET /health HTTP/1.1\r\n"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .expect("response");
        });
        let origin = format!("http://{address}");
        let proxy = AllowlistProxy::start(vec![origin.clone()])
            .await
            .expect("proxy");
        let mut client = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .expect("connect");
        client
            .write_all(
                format!(
                    "GET {origin}/health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .expect("request");
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.expect("response");
        assert!(response.starts_with(b"HTTP/1.1 200 OK"));
        assert!(response.ends_with(b"ok"));
        server.await.expect("server");
    }

    #[tokio::test]
    async fn allowlist_proxy_rejects_conflicting_http_host_and_tls_server_name() {
        let proxy = AllowlistProxy::start(vec![
            "http://example.com".into(),
            "https://example.com".into(),
        ])
        .await
        .expect("proxy");

        let mut http = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .expect("connect HTTP");
        http.write_all(b"GET http://example.com/ HTTP/1.1\r\nHost: attacker.example\r\n\r\n")
            .await
            .expect("write HTTP");
        let mut response = Vec::new();
        http.read_to_end(&mut response).await.expect("HTTP close");
        assert!(response.is_empty());

        let mut tls = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .expect("connect TLS");
        tls.write_all(b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\n")
            .await
            .expect("write CONNECT");
        let expected = b"HTTP/1.1 200 Connection Established\r\n\r\n";
        let mut established = vec![0_u8; expected.len()];
        tls.read_exact(&mut established)
            .await
            .expect("CONNECT response");
        assert_eq!(&established, expected);
        tls.write_all(&tls_client_hello("attacker.example"))
            .await
            .expect("write ClientHello");
        let mut response = Vec::new();
        tls.read_to_end(&mut response).await.expect("TLS close");
        assert!(response.is_empty());
    }
}
