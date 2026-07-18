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
    #[cfg(target_os = "windows")]
    let (native_supported, native_details) = (
        true,
        "AppContainer filesystem and authenticated WFP proxy-only network isolation with atomically attached Job Object"
            .to_owned(),
    );
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
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
