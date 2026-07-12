//! Minimal safe interface over the Win32 primitives required by the Colossus sandbox.
//!
//! The Windows implementation is intentionally isolated from the main sandbox crate. It
//! creates an AppContainer process with its Job Object and inherited standard-I/O handles
//! present in the same `STARTUPINFOEX` attribute list. That removes the create-then-assign
//! race that would otherwise let a child escape process ownership before Job assignment.

#![cfg_attr(windows, allow(unsafe_code))]

use std::{collections::BTreeMap, fs::File, path::PathBuf, time::Duration};
use thiserror::Error;

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
    process: windows_impl::OwnedHandle,
    #[cfg(windows)]
    job: windows_impl::OwnedHandle,
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
            windows_impl::wait_timeout(&self.process, timeout)
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
            windows_impl::terminate(&self.job, exit_code)
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
            windows_impl::resource_limit_violation(&self.job)
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
        windows_impl::spawn(request)
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

#[cfg(windows)]
mod windows_impl {
    use super::{ResourceLimitViolation, SandboxedChild, SpawnRequest, WindowsProcessError};
    use std::{
        ffi::{OsStr, c_void},
        fs::File,
        mem::{size_of, zeroed},
        os::windows::{ffi::OsStrExt as _, io::FromRawHandle as _},
        ptr::{null, null_mut},
        time::Duration,
    };
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_INSUFFICIENT_BUFFER, GetLastError, HANDLE, HANDLE_FLAG_INHERIT,
            HLOCAL, LocalFree, SetHandleInformation, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Security::{
            Authorization::ConvertStringSidToSidW, PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES,
        },
        System::{
            JobObjects::{
                CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_JOB_MEMORY,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_UILIMIT_DESKTOP,
                JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
                JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES,
                JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
                JOB_OBJECT_UILIMIT_WRITECLIPBOARD, JOBOBJECT_BASIC_UI_RESTRICTIONS,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOBOBJECT_LIMIT_VIOLATION_INFORMATION_2,
                JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation,
                JobObjectLimitViolationInformation2, QueryInformationJobObject,
                SetInformationJobObject, TerminateJobObject,
            },
            Pipes::CreatePipe,
            Threading::{
                CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
                EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
                InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
                PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
                UpdateProcThreadAttribute, WaitForSingleObject,
            },
        },
    };

    pub(super) struct OwnedHandle(HANDLE);

    impl OwnedHandle {
        fn new(handle: HANDLE, operation: &'static str) -> Result<Self, WindowsProcessError> {
            if handle.is_null() {
                Err(last_error(operation))
            } else {
                Ok(Self(handle))
            }
        }

        fn raw(&self) -> HANDLE {
            self.0
        }

        fn into_file(self) -> File {
            let handle = self.0;
            std::mem::forget(self);
            // SAFETY: ownership of a valid, uniquely owned HANDLE is transferred to File.
            unsafe { File::from_raw_handle(handle.cast()) }
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: this guard uniquely owns the non-null HANDLE.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    struct LocalSid(PSID);

    impl Drop for LocalSid {
        fn drop(&mut self) {
            // SAFETY: ConvertStringSidToSidW allocated this pointer with LocalAlloc.
            unsafe {
                LocalFree(self.0.cast::<c_void>() as HLOCAL);
            }
        }
    }

    struct AttributeList {
        storage: Vec<usize>,
    }

    impl AttributeList {
        fn new(count: u32) -> Result<Self, WindowsProcessError> {
            let mut bytes = 0_usize;
            // SAFETY: the first call intentionally queries the required buffer size.
            let result =
                unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut bytes) };
            // Windows reports ERROR_INSUFFICIENT_BUFFER for the sizing call.
            if result != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
                return Err(last_error("InitializeProcThreadAttributeList(size)"));
            }
            let words = bytes.div_ceil(size_of::<usize>());
            let mut storage = vec![0_usize; words];
            // SAFETY: storage is aligned and at least the size returned by the sizing call.
            if unsafe {
                InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), count, 0, &mut bytes)
            } == 0
            {
                return Err(last_error("InitializeProcThreadAttributeList"));
            }
            Ok(Self { storage })
        }

        fn raw(&mut self) -> *mut c_void {
            self.storage.as_mut_ptr().cast()
        }

        fn set(
            &mut self,
            attribute: usize,
            value: *const c_void,
            bytes: usize,
            operation: &'static str,
        ) -> Result<(), WindowsProcessError> {
            // SAFETY: value remains alive through CreateProcessW and the buffer is initialized.
            if unsafe {
                UpdateProcThreadAttribute(
                    self.raw(),
                    0,
                    attribute,
                    value,
                    bytes,
                    null_mut(),
                    null(),
                )
            } == 0
            {
                Err(last_error(operation))
            } else {
                Ok(())
            }
        }
    }

    impl Drop for AttributeList {
        fn drop(&mut self) {
            // SAFETY: this is the initialized list owned by this guard.
            unsafe {
                DeleteProcThreadAttributeList(self.raw());
            }
        }
    }

    struct PipeSet {
        child_stdin: OwnedHandle,
        parent_stdin: OwnedHandle,
        parent_stdout: OwnedHandle,
        child_stdout: OwnedHandle,
        parent_stderr: OwnedHandle,
        child_stderr: OwnedHandle,
    }

    impl PipeSet {
        fn new() -> Result<Self, WindowsProcessError> {
            let (child_stdin, parent_stdin) = pipe("CreatePipe(stdin)")?;
            let (parent_stdout, child_stdout) = pipe("CreatePipe(stdout)")?;
            let (parent_stderr, child_stderr) = pipe("CreatePipe(stderr)")?;
            for (handle, operation) in [
                (&parent_stdin, "SetHandleInformation(stdin)"),
                (&parent_stdout, "SetHandleInformation(stdout)"),
                (&parent_stderr, "SetHandleInformation(stderr)"),
            ] {
                // SAFETY: handle is valid; clearing inheritance does not transfer ownership.
                if unsafe { SetHandleInformation(handle.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
                    return Err(last_error(operation));
                }
            }
            Ok(Self {
                child_stdin,
                parent_stdin,
                parent_stdout,
                child_stdout,
                parent_stderr,
                child_stderr,
            })
        }
    }

    pub(super) fn spawn(request: &SpawnRequest) -> Result<SandboxedChild, WindowsProcessError> {
        let sid_wide = wide(OsStr::new(&request.appcontainer_sid));
        let mut sid: PSID = null_mut();
        // SAFETY: sid_wide is NUL terminated and sid receives a LocalAlloc-owned PSID.
        if unsafe { ConvertStringSidToSidW(sid_wide.as_ptr(), &mut sid) } == 0 {
            return Err(last_error("ConvertStringSidToSidW"));
        }
        let sid = LocalSid(sid);

        // SAFETY: zero is the documented initial state for these Win32 structures.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_JOB_MEMORY
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        limits.BasicLimitInformation.ActiveProcessLimit = request.max_processes;
        limits.JobMemoryLimit = usize::try_from(request.max_memory_bytes)
            .map_err(|_| WindowsProcessError::Invalid("memory limit overflow".into()))?;
        // SAFETY: null security/name pointers request an unnamed default-security Job Object.
        let job = OwnedHandle::new(
            unsafe { CreateJobObjectW(null(), null()) },
            "CreateJobObjectW",
        )?;
        // SAFETY: limits has the exact structure required by this information class.
        if unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .expect("structure size fits u32"),
            )
        } == 0
        {
            return Err(last_error("SetInformationJobObject"));
        }
        let ui_restrictions = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
                | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                | JOB_OBJECT_UILIMIT_EXITWINDOWS
                | JOB_OBJECT_UILIMIT_GLOBALATOMS
                | JOB_OBJECT_UILIMIT_HANDLES
                | JOB_OBJECT_UILIMIT_READCLIPBOARD
                | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
        };
        // SAFETY: ui_restrictions has the exact structure required by this information class.
        if unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectBasicUIRestrictions,
                (&raw const ui_restrictions).cast(),
                u32::try_from(size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>())
                    .expect("structure size fits u32"),
            )
        } == 0
        {
            return Err(last_error("SetInformationJobObject(UI restrictions)"));
        }

        let pipes = PipeSet::new()?;
        let inherited = [
            pipes.child_stdin.raw(),
            pipes.child_stdout.raw(),
            pipes.child_stderr.raw(),
        ];
        let mut capabilities = SECURITY_CAPABILITIES {
            AppContainerSid: sid.0,
            Capabilities: null_mut(),
            CapabilityCount: 0,
            Reserved: 0,
        };
        let job_list = [job.raw()];
        let mut attributes = AttributeList::new(3)?;
        attributes.set(
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            (&raw mut capabilities).cast(),
            size_of::<SECURITY_CAPABILITIES>(),
            "UpdateProcThreadAttribute(security capabilities)",
        )?;
        attributes.set(
            PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
            job_list.as_ptr().cast(),
            size_of::<HANDLE>(),
            "UpdateProcThreadAttribute(job list)",
        )?;
        attributes.set(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            inherited.as_ptr().cast(),
            size_of_val(&inherited),
            "UpdateProcThreadAttribute(handle list)",
        )?;

        let executable = wide(request.executable.as_os_str());
        let command_line_value =
            windows_command_line(request.executable.as_os_str(), &request.arguments);
        let mut command_line = wide(OsStr::new(&command_line_value));
        if command_line.len() > 32_767 {
            return Err(WindowsProcessError::Invalid(
                "Windows command line exceeds 32766 UTF-16 code units".into(),
            ));
        }
        let cwd = wide(request.cwd.as_os_str());
        let environment = environment_block(&request.environment);
        if environment.len() > 32_767 {
            return Err(WindowsProcessError::Invalid(
                "Windows environment block exceeds 32767 UTF-16 code units".into(),
            ));
        }
        // SAFETY: zero is the documented initial state for startup/process information.
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        startup.StartupInfo.cb =
            u32::try_from(size_of::<STARTUPINFOEXW>()).expect("structure size fits u32");
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = pipes.child_stdin.raw();
        startup.StartupInfo.hStdOutput = pipes.child_stdout.raw();
        startup.StartupInfo.hStdError = pipes.child_stderr.raw();
        startup.lpAttributeList = attributes.raw();
        // SAFETY: zero is the documented initial state for process information.
        let mut process_info: PROCESS_INFORMATION = unsafe { zeroed() };
        // SAFETY: every pointer references a live, correctly terminated buffer through this call.
        if unsafe {
            CreateProcessW(
                executable.as_ptr(),
                command_line.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast(),
                cwd.as_ptr(),
                (&raw mut startup.StartupInfo),
                &mut process_info,
            )
        } == 0
        {
            return Err(last_error("CreateProcessW"));
        }
        let process = OwnedHandle::new(process_info.hProcess, "CreateProcessW(process handle)")?;
        let thread = OwnedHandle::new(process_info.hThread, "CreateProcessW(thread handle)")?;
        drop(thread);

        let PipeSet {
            child_stdin,
            parent_stdin,
            parent_stdout,
            child_stdout,
            parent_stderr,
            child_stderr,
        } = pipes;
        drop(child_stdin);
        drop(child_stdout);
        drop(child_stderr);
        Ok(SandboxedChild {
            pid: process_info.dwProcessId,
            stdin: Some(parent_stdin.into_file()),
            stdout: Some(parent_stdout.into_file()),
            stderr: Some(parent_stderr.into_file()),
            process,
            job,
        })
    }

    pub(super) fn wait_timeout(
        process: &OwnedHandle,
        timeout: Duration,
    ) -> Result<Option<u32>, WindowsProcessError> {
        let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
        // SAFETY: process is a live process handle owned by the child guard.
        match unsafe { WaitForSingleObject(process.raw(), milliseconds) } {
            WAIT_OBJECT_0 => {
                let mut code = 0_u32;
                // SAFETY: process is signaled and code points to initialized writable memory.
                if unsafe { GetExitCodeProcess(process.raw(), &mut code) } == 0 {
                    Err(last_error("GetExitCodeProcess"))
                } else {
                    Ok(Some(code))
                }
            }
            WAIT_TIMEOUT => Ok(None),
            WAIT_FAILED => Err(last_error("WaitForSingleObject")),
            _ => Err(WindowsProcessError::Invalid(
                "WaitForSingleObject returned an unexpected value".into(),
            )),
        }
    }

    pub(super) fn terminate(job: &OwnedHandle, exit_code: u32) -> Result<(), WindowsProcessError> {
        // SAFETY: job is a live Job Object handle owned by the child guard.
        if unsafe { TerminateJobObject(job.raw(), exit_code) } == 0 {
            Err(last_error("TerminateJobObject"))
        } else {
            Ok(())
        }
    }

    pub(super) fn resource_limit_violation(
        job: &OwnedHandle,
    ) -> Result<Option<ResourceLimitViolation>, WindowsProcessError> {
        // SAFETY: zero is the documented initial state for this query structure.
        let mut information: JOBOBJECT_LIMIT_VIOLATION_INFORMATION_2 = unsafe { zeroed() };
        // SAFETY: job is live and information is a correctly sized writable buffer.
        if unsafe {
            QueryInformationJobObject(
                job.raw(),
                JobObjectLimitViolationInformation2,
                (&raw mut information).cast(),
                u32::try_from(size_of::<JOBOBJECT_LIMIT_VIOLATION_INFORMATION_2>())
                    .expect("structure size fits u32"),
                null_mut(),
            )
        } == 0
        {
            return Err(last_error("QueryInformationJobObject(limit violation)"));
        }
        if information.ViolationLimitFlags & JOB_OBJECT_LIMIT_JOB_MEMORY != 0 {
            Ok(Some(ResourceLimitViolation::Memory))
        } else if information.ViolationLimitFlags & JOB_OBJECT_LIMIT_ACTIVE_PROCESS != 0 {
            Ok(Some(ResourceLimitViolation::ProcessCount))
        } else {
            Ok(None)
        }
    }

    fn pipe(operation: &'static str) -> Result<(OwnedHandle, OwnedHandle), WindowsProcessError> {
        let mut read = null_mut();
        let mut write = null_mut();
        let attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
                .expect("structure size fits u32"),
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        };
        // SAFETY: output pointers and the security-attributes pointer are valid for this call.
        if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
            return Err(last_error(operation));
        }
        Ok((
            OwnedHandle::new(read, operation)?,
            OwnedHandle::new(write, operation)?,
        ))
    }

    fn windows_command_line(executable: &OsStr, arguments: &[String]) -> String {
        std::iter::once(executable.to_string_lossy().into_owned())
            .chain(arguments.iter().cloned())
            .map(|argument| quote_argument(&argument))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn quote_argument(argument: &str) -> String {
        if !argument.is_empty()
            && !argument
                .bytes()
                .any(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\x0b' | b'\r' | b'"'))
        {
            return argument.into();
        }
        let mut quoted = String::from("\"");
        let mut backslashes = 0_usize;
        for character in argument.chars() {
            if character == '\\' {
                backslashes += 1;
            } else if character == '"' {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            } else {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
        quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
        quoted.push('"');
        quoted
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn environment_block(environment: &std::collections::BTreeMap<String, String>) -> Vec<u16> {
        let mut entries = environment.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.0
                .to_ascii_lowercase()
                .cmp(&right.0.to_ascii_lowercase())
        });
        let mut block = Vec::new();
        for (name, value) in entries {
            block.extend(format!("{name}={value}").encode_utf16());
            block.push(0);
        }
        block.push(0);
        block
    }

    fn last_error(operation: &'static str) -> WindowsProcessError {
        WindowsProcessError::Win32 {
            operation,
            source: std::io::Error::last_os_error(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SpawnRequest, WindowsProcessError, spawn};
    use std::{collections::BTreeMap, path::PathBuf};

    #[test]
    fn rejects_relative_and_unbounded_launch_contracts_before_platform_dispatch() {
        let request = SpawnRequest {
            executable: PathBuf::from("relative.exe"),
            arguments: Vec::new(),
            cwd: PathBuf::from("."),
            environment: BTreeMap::new(),
            appcontainer_sid: "S-1-15-2-1".into(),
            max_processes: 1,
            max_memory_bytes: 1024,
        };
        assert!(matches!(
            spawn(&request),
            Err(WindowsProcessError::Invalid(_))
        ));
    }
}
