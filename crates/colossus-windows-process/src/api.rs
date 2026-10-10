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

impl Drop for SandboxedChild {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            // Kill-on-close is asynchronous. Keep the WFP egress fence until
            // the entire retained Job has positively reached zero processes.
            // Explicit supervisors must retain/retry this owner on Unknown;
            // Drop cannot acknowledge cleanup. A failed fallback deliberately
            // leaks the dynamic-session guard until parent-process exit rather
            // than silently granting surviving descendants new network access.
            let terminated = crate::windows_impl::terminate(&self.job, 1).is_ok();
            let drained = matches!(
                crate::windows_impl::wait_tree_timeout(&self.job, Duration::from_secs(5)),
                Ok(true)
            );
            if (!terminated || !drained)
                && let Some(network) = self._network.take()
            {
                std::mem::forget(network);
            }
        }
    }
}

#[cfg(windows)]
pub(crate) struct SandboxedChildParts {
    pub(crate) pid: u32,
    pub(crate) stdin: File,
    pub(crate) stdout: File,
    pub(crate) stderr: File,
    pub(crate) process: crate::windows_impl::OwnedHandle,
    pub(crate) job: crate::windows_impl::OwnedHandle,
    pub(crate) completion_port: crate::windows_impl::OwnedHandle,
    pub(crate) network: Option<crate::windows_impl::NetworkGuard>,
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
    /// Explicitly retire the exact dynamic WFP session only after the whole Job
    /// reaches zero processes. Failure retains the same guard for an idempotent
    /// retry; Drop never substitutes for this acknowledged network cleanup.
    pub fn retire_network(&mut self) -> Result<(), WindowsProcessError> {
        #[cfg(windows)]
        {
            if !crate::windows_impl::wait_tree_timeout(&self.job, Duration::ZERO)? {
                return Err(WindowsProcessError::Invalid(
                    "cannot retire egress beneath a live Job".into(),
                ));
            }
            if let Some(network) = self._network.as_mut() {
                network.retire()?;
            }
            self._network.take();
            Ok(())
        }
        #[cfg(not(windows))]
        {
            Err(WindowsProcessError::UnsupportedPlatform)
        }
    }
    /// Borrow the exact retained process object for native token/path attestation.
    /// No PID lookup or handle ownership transfer occurs. This handle must never
    /// enter renderer/model IPC or an ordinary application DTO.
    #[cfg(windows)]
    pub fn process_handle(&self) -> std::os::windows::io::BorrowedHandle<'_> {
        self.process.borrow()
    }
    #[cfg(windows)]
    pub(crate) fn from_parts(parts: SandboxedChildParts) -> Self {
        Self {
            pid: parts.pid,
            stdin: Some(parts.stdin),
            stdout: Some(parts.stdout),
            stderr: Some(parts.stderr),
            process: parts.process,
            job: parts.job,
            completion_port: parts.completion_port,
            _network: parts.network,
        }
    }

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

    /// Wait until every process in the atomically attached Job has exited.
    ///
    /// `false` retains the process, Job and WFP obligations. The direct child's
    /// process exit alone cannot prove renderer, network, worker or helper exit.
    /// Keep this owner and network filters alive through successful acknowledgement
    /// and through a separately completed private-channel I/O drain.
    pub fn wait_tree_timeout(&self, timeout: Duration) -> Result<bool, WindowsProcessError> {
        #[cfg(windows)]
        {
            crate::windows_impl::wait_tree_timeout(&self.job, timeout)
        }
        #[cfg(not(windows))]
        {
            let _ = timeout;
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

pub(super) fn validate_request(request: &SpawnRequest) -> Result<(), WindowsProcessError> {
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
