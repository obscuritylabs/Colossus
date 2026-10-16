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
    #[cfg(windows)]
    context: Option<crate::windows_impl::context::BrowserContextOwner>,
}

/// Observation-only retained process object positively bound to the exact Job.
/// This native-only value grants no termination rights and must never enter IPC.
#[cfg(windows)]
pub struct JobProcessObservation {
    pub(crate) id: u32,
    pub(crate) process: crate::windows_impl::OwnedHandle,
}

#[cfg(windows)]
impl JobProcessObservation {
    /// Categorical identifier for matching the retained root within this snapshot.
    #[must_use]
    pub fn process_id(&self) -> u32 {
        self.id
    }

    /// Borrow the exact non-inherited observation handle for kernel token checks.
    /// Membership was positively checked after opening; PID reuse cannot substitute
    /// a process outside the Job. Retain this value throughout those checks.
    #[must_use]
    pub fn process_handle(&self) -> std::os::windows::io::BorrowedHandle<'_> {
        self.process.borrow()
    }
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
            let context_drained = if drained {
                self.context
                    .as_mut()
                    .is_none_or(|context| context.close(Duration::from_secs(1)).is_ok())
            } else {
                false
            };
            if (!terminated || !drained || !context_drained)
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
    pub(crate) context: Option<crate::windows_impl::context::BrowserContextOwner>,
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
    /// Failed browser startup whose native resources still require owned cleanup.
    /// Retain this payload beside the package and installation until cleanup succeeds.
    #[cfg(windows)]
    #[error("Windows browser setup failed; owned cleanup is pending: {source}")]
    BrowserSetup {
        /// Original categorical startup failure.
        #[source]
        source: Box<WindowsProcessError>,
        /// Exact partial startup owner, never a fabricated browser child.
        cleanup: PendingBrowserCleanup,
    },
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

/// Opaque native obligations retained after an unsuccessful browser startup.
/// This value must stay beside the exact package, profile and installation owner.
/// Drop is fail-closed fallback and never a cleanup acknowledgement.
#[cfg(windows)]
pub struct PendingBrowserCleanup {
    pub(crate) owner: Option<Box<crate::windows_impl::context::BrowserSetupOwner>>,
}

#[cfg(windows)]
impl std::fmt::Debug for PendingBrowserCleanup {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("PendingBrowserCleanup")
            .finish_non_exhaustive()
    }
}

#[cfg(windows)]
impl PendingBrowserCleanup {
    /// Retry cleanup within the caller's bound. Only true acknowledges every
    /// allocated process, Job, private I/O and WFP obligation; false or error
    /// retains the same owner. Unreturned CPWL cannot be discharged by Job zero.
    pub fn cleanup_timeout(&mut self, timeout: Duration) -> Result<bool, WindowsProcessError> {
        let Some(owner) = &mut self.owner else {
            return Ok(true);
        };
        if !owner.cleanup(timeout)? {
            return Ok(false);
        }
        self.owner.take();
        Ok(true)
    }
}

#[cfg(windows)]
impl Drop for PendingBrowserCleanup {
    fn drop(&mut self) {
        if !matches!(self.cleanup_timeout(Duration::from_secs(1)), Ok(true))
            && let Some(owner) = self.owner.take()
        {
            std::mem::forget(owner);
        }
    }
}

/// How the private browser context reached acknowledged retirement.
#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextShutdown {
    /// This direct command or Debug browser has no private context.
    NotPresent,
    /// Station-close receipt, exact helper exits, I/O and outer Job zero proved.
    Graceful,
    /// Core forced its exact outer Job and positively drained all owned resources.
    Forced,
}

impl SandboxedChild {
    /// Retire the private station/context after the browser's inner Job is empty.
    /// An unknown result retains the same context and WFP owners for retry.
    #[cfg(windows)]
    pub fn close_browser_context(
        &mut self,
        timeout: Duration,
    ) -> Result<ContextShutdown, WindowsProcessError> {
        if !crate::windows_impl::wait_tree_timeout(&self.job, Duration::ZERO)? {
            return Err(WindowsProcessError::Invalid(
                "browser context close requires inner Job zero".into(),
            ));
        }
        match &mut self.context {
            Some(context) => context.close(timeout),
            None => Ok(ContextShutdown::NotPresent),
        }
    }
    /// Transfer read/query authority to one exact profile directory into this
    /// retained child, after native composition verifies its AppContainer token.
    /// The returned process-local handle belongs only in private bootstrap. It
    /// is non-inherited; an undelivered handle is retired by full process cleanup.
    /// Neither a path nor a caller-supplied PID can request this transfer.
    #[cfg(windows)]
    pub fn enroll_profile_directory(&self, directory: &File) -> Result<u64, WindowsProcessError> {
        crate::windows_impl::bootstrap::profile_directory(&self.process, directory)
    }
    /// Retain every member of a complete, stable, bounded live Job snapshot.
    /// Query failure, an inaccessible helper or membership churn fails explicitly.
    /// This does not attest helpers that already exited, nor replace full Job-zero
    /// cleanup. Callers must validate each returned kernel token independently.
    #[cfg(windows)]
    pub fn observe_process_tree(&self) -> Result<Vec<JobProcessObservation>, WindowsProcessError> {
        crate::windows_impl::tree::observe(&self.job)
    }

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
            if let Some(context) = &mut self.context
                && !context.retired()?
            {
                return Err(WindowsProcessError::Invalid(
                    "cannot retire egress beneath an unresolved browser context".into(),
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
            context: parts.context,
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
            let inner =
                crate::windows_impl::resource_limit_violation(&self.job, &self.completion_port)?;
            let outer = self
                .context
                .as_ref()
                .map(|context| context.resource_limit_violation())
                .transpose()?
                .flatten();
            Ok(
                if inner == Some(ResourceLimitViolation::Memory)
                    || outer == Some(ResourceLimitViolation::Memory)
                {
                    Some(ResourceLimitViolation::Memory)
                } else {
                    inner.or(outer)
                },
            )
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
    validate_launch(request)?;
    if request.appcontainer_sid.is_empty() || request.appcontainer_sid.contains('\0') {
        return Err(WindowsProcessError::Invalid(
            "AppContainer SID is required".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_launch(request: &SpawnRequest) -> Result<(), WindowsProcessError> {
    if !request.executable.is_absolute() || !request.cwd.is_absolute() {
        return Err(WindowsProcessError::Invalid(
            "executable and cwd must be absolute".into(),
        ));
    }
    if request.max_processes == 0
        || request.max_memory_bytes == 0
        || request.proxy_port.is_some() != request.network_filter_id.is_some()
        || request.proxy_port == Some(0)
        || request.network_filter_id == Some(0)
    {
        return Err(WindowsProcessError::Invalid(
            "resource limits and paired network fields must be valid".into(),
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
