use super::*;

pub(super) type HmacSha256 = Hmac<Sha256>;

pub(super) const HELPER_KEY_VARIABLE: &str = "COLOSSUS_SANDBOX_JOB_KEY";
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) const NATIVE_INNER_VARIABLE: &str = "COLOSSUS_SANDBOX_NATIVE_INNER";
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) const NATIVE_TARGET_PID_PREFIX: &[u8] = b"colossus-native-target-pid:";
pub(super) const OCI_PROXY_CONFIG_VARIABLE: &str = "COLOSSUS_OCI_PROXY_CONFIG";
pub(super) const OCI_PROXY_PORT: u16 = 18_080;
pub(super) const MAX_JOB_BYTES: usize = 1024 * 1024;
pub(super) const MAX_PROXY_HEADER_BYTES: usize = 16 * 1024;
pub(super) const MAX_OBSERVED_ORIGINS: usize = 64;
pub(super) const OBSERVED_ORIGIN_PREFIX: &str = "colossus-observed-origin:";
pub(super) const MAX_TLS_RECORD_BYTES: usize = 18 * 1024;
pub(super) const MAX_TLS_CLIENT_HELLO_BYTES: usize = 64 * 1024;
#[cfg(target_os = "windows")]
pub(super) const MAX_WINDOWS_ACL_TARGETS: usize = 100_000;
#[cfg(target_os = "windows")]
pub(super) const WINDOWS_REPARSE_POINT_ATTRIBUTE: u32 = 0x400;
pub(super) const OCI_CLEANUP_RESERVE_MS: u64 = 2_000;
pub(super) const OCI_NETWORK_CLEANUP_RESERVE_MS: u64 = 5_000;
pub(super) const WINDOWS_JOB_CLEANUP_RESERVE_MS: u64 = 7_000;
pub(super) const NATIVE_CLEANUP_RESERVE_MS: u64 = 250;
// Rootless Podman may need more than a second to initialize its user namespace and
// network helpers on a cold host. Keep control operations bounded, but allow the
// runtime enough time to complete that trusted setup without a spurious failure.
pub(super) const OCI_CONTROL_COMMAND_TIMEOUT_MS: u64 = 5_000;
pub(super) const OCI_DNS_RESOLUTION_TIMEOUT_MS: u64 = 3_000;
pub(super) const MAX_OCI_CONTROL_DIAGNOSTIC_BYTES: usize = 4 * 1024;

pub(super) fn adapter_failure(error: impl std::fmt::Display) -> ExecutionError {
    ExecutionError::Failed(error.to_string())
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
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
    /// Optional caller-requested timeout, bounded by the policy maximum.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Optional caller-requested output cap, bounded by the policy maximum.
    #[serde(default)]
    pub max_output_bytes: Option<u64>,
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
    /// Exact backend selected by runtime configuration.
    #[serde(default)]
    pub selected_backend: String,
    /// Whether the selected backend supplies Colossus-owned process isolation.
    #[serde(default)]
    pub colossus_process_isolation: bool,
    /// Whether a direct-execution mode was acknowledged globally for headless callers.
    #[serde(default)]
    pub direct_execution_globally_acknowledged: bool,
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
    /// Canonical workspace selected by the runtime, when supplied by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_workspace: Option<PathBuf>,
    /// Configured resource profile.
    #[serde(default)]
    pub sandbox_profile: String,
    /// Whether the selected backend can hide protected control-state paths.
    #[serde(default)]
    pub protected_path_exclusions_supported: bool,
    /// Canonical control-state paths hidden from development shells.
    #[serde(default)]
    pub protected_paths: Vec<String>,
    /// Trusted shell resolved by the development preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_shell: Option<PathBuf>,
    /// Scope that receives automatic development grants.
    #[serde(default)]
    pub development_actor_scope: String,
    /// Read-only command roots used to construct the shell PATH.
    #[serde(default)]
    pub sanitized_command_roots: Vec<PathBuf>,
    /// Filesystem grants written explicitly in configuration.
    #[serde(default)]
    pub explicit_filesystem: Vec<FilesystemGrant>,
    /// Filesystem grants derived from the selected sandbox profile.
    #[serde(default)]
    pub derived_filesystem: Vec<FilesystemGrant>,
    /// Executables written explicitly in configuration.
    #[serde(default)]
    pub explicit_executables: Vec<PathBuf>,
    /// Executables derived from the selected sandbox profile.
    #[serde(default)]
    pub derived_executables: Vec<PathBuf>,
    /// Configured network destinations after validation.
    #[serde(default)]
    pub network_destinations: Vec<String>,
    /// Meaning of the public network wildcard, when configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_network_wildcard: Option<String>,
}

/// Return bounded local sandbox readiness.
pub fn sandbox_doctor(config: &SandboxExecutorConfig) -> SandboxDoctorReport {
    #[cfg(target_os = "linux")]
    let (native_supported, native_details, protected_path_exclusions_supported) = {
        let support = Sandbox::support_info();
        let (protected, protection_details) = if support.is_supported {
            probe_linux_protected_paths(&config.helper_executable)
        } else {
            (
                false,
                "protected-path namespaces require the native backend".into(),
            )
        };
        (
            support.is_supported,
            format!("{}; {protection_details}", support.details),
            protected,
        )
    };
    #[cfg(target_os = "macos")]
    let (native_supported, native_details, protected_path_exclusions_supported) = {
        let support = Sandbox::support_info();
        (support.is_supported, support.details, support.is_supported)
    };
    #[cfg(target_os = "windows")]
    let (native_supported, native_details, protected_path_exclusions_supported) = (
        true,
        "AppContainer filesystem and authenticated WFP proxy-only network isolation with atomically attached Job Object"
            .to_owned(),
        true,
    );
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let (native_supported, native_details, protected_path_exclusions_supported) = (
        false,
        "native isolation is unavailable; configure the OCI backend".to_owned(),
        false,
    );
    SandboxDoctorReport {
        platform: std::env::consts::OS.into(),
        selected_backend: String::new(),
        colossus_process_isolation: false,
        direct_execution_globally_acknowledged: false,
        native_supported,
        native_details,
        helper_executable: config.helper_executable.clone(),
        oci_runtime: config.oci_runtime.clone(),
        oci_image_configured: config.oci_image.is_some(),
        canonical_workspace: None,
        sandbox_profile: String::new(),
        protected_path_exclusions_supported,
        protected_paths: Vec::new(),
        resolved_shell: None,
        development_actor_scope: String::new(),
        sanitized_command_roots: Vec::new(),
        explicit_filesystem: Vec::new(),
        derived_filesystem: Vec::new(),
        explicit_executables: Vec::new(),
        derived_executables: Vec::new(),
        network_destinations: Vec::new(),
        public_network_wildcard: None,
    }
}

#[cfg(target_os = "linux")]
fn probe_linux_protected_paths(helper_executable: &Path) -> (bool, String) {
    const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

    let mut child = match Command::new(helper_executable)
        .arg("__sandbox-protection-probe")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return (
                false,
                format!("protected-path namespace probe could not start: {error}"),
            );
        }
    };
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                return (
                    true,
                    "rootless protected-path mount namespaces are available".into(),
                );
            }
            Ok(Some(_)) => return (false, linux_protection_failure_details()),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return (
                    false,
                    "protected-path namespace probe exceeded its 3 second bound".into(),
                );
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return (
                    false,
                    format!("protected-path namespace probe failed: {error}"),
                );
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_protection_failure_details() -> String {
    let apparmor_restricted =
        fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
            .is_ok_and(|value| value.trim() == "1");
    if apparmor_restricted {
        "rootless protected-path namespaces are blocked by the host AppArmor user-namespace restriction; install the exact-path Colossus AppArmor profile or use OCI"
            .into()
    } else {
        "rootless protected-path mount namespaces are unavailable; use a supported native host or OCI"
            .into()
    }
}
