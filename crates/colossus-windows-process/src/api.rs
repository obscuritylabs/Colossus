use super::*;

/// A fully validated Windows AppContainer launch request.
#[derive(Clone, Debug)]
pub struct SpawnRequest {
    /// Canonical executable path.
    pub executable: PathBuf,
    /// Literal argument vector, excluding `argv[0]`.
    pub arguments: Vec<String>,
    /// Canonical working directory.
    pub cwd: PathBuf,
    /// Complete child environment after trusted runtime additions.
    pub environment: BTreeMap<String, String>,
    /// AppContainer package SID in SDDL form.
    pub appcontainer_sid: String,
    /// Maximum number of processes in the atomic Job Object.
    pub max_processes: u32,
    /// Aggregate committed-memory ceiling for the Job Object.
    pub max_memory_bytes: u64,
    /// Parent proxy port that is the only permitted network destination.
    pub proxy_port: Option<u16>,
    /// Unique WFP dynamic-session identity paired with `proxy_port`.
    pub network_filter_id: Option<u128>,
}

/// A process and its atomically attached Job Object.
pub struct SandboxedChild {
    /// Process identifier.
    pub pid: u32,
    /// Parent-side standard input pipe.
    pub stdin: Option<File>,
    /// Parent-side standard output pipe.
    pub stdout: Option<File>,
    /// Parent-side standard error pipe.
    pub stderr: Option<File>,
    #[cfg(windows)]
    process: crate::windows_impl::OwnedHandle,
    #[cfg(windows)]
    job: crate::windows_impl::OwnedHandle,
    #[cfg(windows)]
    completion_port: crate::windows_impl::OwnedHandle,
    #[cfg(windows)]
    _network: Option<crate::windows_impl::NetworkGuard>,
}

/// Hard Job Object limit observed during execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceLimitViolation {
    /// The aggregate Job Object memory ceiling was exceeded.
    Memory,
    /// The active-process ceiling was exceeded.
    ProcessCount,
}

/// Windows process-launch failure.
#[derive(Debug, Error)]
pub enum WindowsProcessError {
    /// This API was invoked on a non-Windows build.
    #[error("Windows AppContainer process launch is unavailable on this platform")]
    UnsupportedPlatform,
    /// A launch value exceeded a hard contract bound.
    #[error("invalid Windows sandbox request: {0}")]
    Invalid(String),
    /// A Win32 operation failed.
    #[error("Windows sandbox {operation} failed: {source}")]
    Win32 {
        /// Stable operation label.
        operation: &'static str,
        /// Captured operating-system error.
        #[source]
        source: std::io::Error,
    },
}

impl SandboxedChild {
    /// Wait for the process for at most `timeout`; `None` means it remains active.
    pub fn wait_timeout(&self, timeout: Duration) -> Result<Option<u32>, WindowsProcessError> {
        #[cfg(windows)]
        {
            crate::windows_impl::wait_timeout(&self.process, timeout)
        }
        #[cfg(not(windows))]
        {
            let _ = timeout;
            Err(WindowsProcessError::UnsupportedPlatform)
        }
    }

    /// Terminate every process in the Job Object.
    pub fn terminate(&self, exit_code: u32) -> Result<(), WindowsProcessError> {
        #[cfg(windows)]
        {
            crate::windows_impl::terminate(&self.job, exit_code)
        }
        #[cfg(not(windows))]
        {
            let _ = exit_code;
            Err(WindowsProcessError::UnsupportedPlatform)
        }
    }

    /// Return a hard Job Object limit violation, if Windows recorded one.
    pub fn resource_limit_violation(
        &self,
    ) -> Result<Option<ResourceLimitViolation>, WindowsProcessError> {
        #[cfg(windows)]
        {
            crate::windows_impl::resource_limit_violation(&self.job, &self.completion_port)
        }
        #[cfg(not(windows))]
        {
            Err(WindowsProcessError::UnsupportedPlatform)
        }
    }
}

/// Create an AppContainer process whose Job Object is attached atomically.
pub fn spawn(request: &SpawnRequest) -> Result<SandboxedChild, WindowsProcessError> {
    validate_request(request)?;
    #[cfg(windows)]
    {
        crate::windows_impl::spawn(request)
    }
    #[cfg(not(windows))]
    {
        Err(WindowsProcessError::UnsupportedPlatform)
    }
}

fn validate_request(request: &SpawnRequest) -> Result<(), WindowsProcessError> {
    if !request.executable.is_absolute() || !request.cwd.is_absolute() {
        return Err(WindowsProcessError::Invalid(
            "executable and cwd must be absolute".into(),
        ));
    }
    if request.appcontainer_sid.is_empty()
        || request.appcontainer_sid.contains('\0')
        || request.max_processes == 0
        || request.max_memory_bytes == 0
        || request.proxy_port.is_some() != request.network_filter_id.is_some()
        || request.proxy_port == Some(0)
        || request.network_filter_id == Some(0)
    {
        return Err(WindowsProcessError::Invalid(
            "SID and resource limits must be present".into(),
        ));
    }
    if request.arguments.len() > 256
        || request.arguments.iter().any(|value| value.contains('\0'))
        || request.environment.iter().any(|(name, value)| {
            name.is_empty() || name.contains(['=', '\0']) || value.contains('\0')
        })
    {
        return Err(WindowsProcessError::Invalid(
            "arguments or environment contain invalid values".into(),
        ));
    }
    usize::try_from(request.max_memory_bytes).map_err(|_| {
        WindowsProcessError::Invalid("memory limit does not fit this platform".into())
    })?;
    Ok(())
}
