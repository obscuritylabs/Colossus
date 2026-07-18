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
    process: windows_impl::OwnedHandle,
    #[cfg(windows)]
    job: windows_impl::OwnedHandle,
    #[cfg(windows)]
    completion_port: windows_impl::OwnedHandle,
    #[cfg(windows)]
    _network: Option<windows_impl::NetworkGuard>,
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
            windows_impl::resource_limit_violation(&self.job, &self.completion_port)
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
            HLOCAL, INVALID_HANDLE_VALUE, LocalFree, SetHandleInformation, WAIT_FAILED,
            WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        NetworkManagement::WindowsFilteringPlatform::{
            FWP_ACTION_BLOCK, FWP_ACTION_PERMIT, FWP_CONDITION_VALUE0, FWP_CONDITION_VALUE0_0,
            FWP_MATCH_EQUAL, FWP_SID, FWP_UINT8, FWP_UINT16, FWP_UINT64, FWP_V4_ADDR_AND_MASK,
            FWP_V4_ADDR_MASK, FWP_VALUE0, FWP_VALUE0_0, FWPM_ACTION0,
            FWPM_CONDITION_ALE_PACKAGE_ID, FWPM_CONDITION_IP_PROTOCOL,
            FWPM_CONDITION_IP_REMOTE_ADDRESS, FWPM_CONDITION_IP_REMOTE_PORT, FWPM_DISPLAY_DATA0,
            FWPM_FILTER_CONDITION0, FWPM_FILTER0, FWPM_LAYER_ALE_AUTH_CONNECT_V4,
            FWPM_LAYER_ALE_AUTH_CONNECT_V6, FWPM_SESSION_FLAG_DYNAMIC, FWPM_SESSION0,
            FWPM_SUBLAYER0, FwpmEngineClose0, FwpmEngineOpen0, FwpmFilterAdd0, FwpmSubLayerAdd0,
        },
        Security::{
            Authorization::ConvertStringSidToSidW, PSID, SECURITY_ATTRIBUTES,
            SECURITY_CAPABILITIES, SID,
        },
        System::{
            IO::{CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED},
            JobObjects::{
                CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_JOB_MEMORY,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
                JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS,
                JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS,
                JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
                JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
                JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_BASIC_UI_RESTRICTIONS,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOBOBJECT_LIMIT_VIOLATION_INFORMATION_2,
                JobObjectAssociateCompletionPortInformation, JobObjectBasicUIRestrictions,
                JobObjectExtendedLimitInformation, JobObjectLimitViolationInformation2,
                QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
            },
            Pipes::CreatePipe,
            Rpc::RPC_C_AUTHN_DEFAULT,
            SystemServices::{
                JOB_OBJECT_MSG_ACTIVE_PROCESS_LIMIT, JOB_OBJECT_MSG_JOB_MEMORY_LIMIT,
                JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT,
            },
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
    use windows_sys::core::GUID;

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

    pub(super) struct NetworkGuard(HANDLE);

    impl NetworkGuard {
        fn install(
            appcontainer_sid: PSID,
            proxy_port: u16,
            identity: u128,
        ) -> Result<Self, WindowsProcessError> {
            let session_name = wide(OsStr::new("Colossus AppContainer proxy-only session"));
            // SAFETY: zero is the documented initial state for an FWPM session.
            let mut session: FWPM_SESSION0 = unsafe { zeroed() };
            session.sessionKey = derived_guid(identity, 0x10);
            session.displayData = FWPM_DISPLAY_DATA0 {
                name: session_name.as_ptr().cast_mut(),
                description: null_mut(),
            };
            session.flags = FWPM_SESSION_FLAG_DYNAMIC;
            session.txnWaitTimeoutInMSec = 5_000;
            let mut engine = null_mut();
            // SAFETY: all optional pointers are null and engine is a valid output pointer.
            let result = unsafe {
                FwpmEngineOpen0(
                    null(),
                    RPC_C_AUTHN_DEFAULT as u32,
                    null(),
                    &raw const session,
                    &raw mut engine,
                )
            };
            if result != 0 {
                return Err(code_error("FwpmEngineOpen0", result));
            }
            let guard = Self(engine);
            guard.configure(appcontainer_sid, proxy_port, identity)?;
            Ok(guard)
        }

        fn configure(
            &self,
            appcontainer_sid: PSID,
            proxy_port: u16,
            identity: u128,
        ) -> Result<(), WindowsProcessError> {
            let sublayer_name = wide(OsStr::new("Colossus AppContainer proxy-only filters"));
            let sublayer_key = derived_guid(identity, 0x20);
            // SAFETY: zero is the documented initial state for an FWPM sublayer.
            let mut sublayer: FWPM_SUBLAYER0 = unsafe { zeroed() };
            sublayer.subLayerKey = sublayer_key;
            sublayer.displayData = FWPM_DISPLAY_DATA0 {
                name: sublayer_name.as_ptr().cast_mut(),
                description: null_mut(),
            };
            sublayer.weight = u16::MAX;
            // SAFETY: the engine is live and WFP copies the supplied sublayer structure.
            let result = unsafe { FwpmSubLayerAdd0(self.0, &raw const sublayer, null_mut()) };
            if result != 0 {
                return Err(code_error("FwpmSubLayerAdd0", result));
            }

            let package = package_condition(appcontainer_sid);
            let protocol = FWPM_FILTER_CONDITION0 {
                fieldKey: FWPM_CONDITION_IP_PROTOCOL,
                matchType: FWP_MATCH_EQUAL,
                conditionValue: FWP_CONDITION_VALUE0 {
                    r#type: FWP_UINT8,
                    Anonymous: FWP_CONDITION_VALUE0_0 { uint8: 6 },
                },
            };
            let mut loopback = FWP_V4_ADDR_AND_MASK {
                addr: u32::from_be_bytes([127, 0, 0, 1]),
                mask: u32::MAX,
            };
            let address = FWPM_FILTER_CONDITION0 {
                fieldKey: FWPM_CONDITION_IP_REMOTE_ADDRESS,
                matchType: FWP_MATCH_EQUAL,
                conditionValue: FWP_CONDITION_VALUE0 {
                    r#type: FWP_V4_ADDR_MASK,
                    Anonymous: FWP_CONDITION_VALUE0_0 {
                        v4AddrMask: &raw mut loopback,
                    },
                },
            };
            let port = FWPM_FILTER_CONDITION0 {
                fieldKey: FWPM_CONDITION_IP_REMOTE_PORT,
                matchType: FWP_MATCH_EQUAL,
                conditionValue: FWP_CONDITION_VALUE0 {
                    r#type: FWP_UINT16,
                    Anonymous: FWP_CONDITION_VALUE0_0 { uint16: proxy_port },
                },
            };
            add_filter(
                self.0,
                derived_guid(identity, 0x31),
                "Colossus allow authenticated proxy",
                FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                sublayer_key,
                &mut [package, protocol, address, port],
                FWP_ACTION_PERMIT,
                15,
            )?;
            add_filter(
                self.0,
                derived_guid(identity, 0x32),
                "Colossus block other IPv4",
                FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                sublayer_key,
                &mut [package_condition(appcontainer_sid)],
                FWP_ACTION_BLOCK,
                1,
            )?;
            add_filter(
                self.0,
                derived_guid(identity, 0x33),
                "Colossus block IPv6",
                FWPM_LAYER_ALE_AUTH_CONNECT_V6,
                sublayer_key,
                &mut [package_condition(appcontainer_sid)],
                FWP_ACTION_BLOCK,
                1,
            )
        }
    }

    impl Drop for NetworkGuard {
        fn drop(&mut self) {
            // SAFETY: the dynamic WFP engine handle is uniquely owned by this guard. Closing it
            // atomically removes the sublayer and every per-job filter.
            unsafe {
                FwpmEngineClose0(self.0);
            }
        }
    }

    fn package_condition(appcontainer_sid: PSID) -> FWPM_FILTER_CONDITION0 {
        FWPM_FILTER_CONDITION0 {
            fieldKey: FWPM_CONDITION_ALE_PACKAGE_ID,
            matchType: FWP_MATCH_EQUAL,
            conditionValue: FWP_CONDITION_VALUE0 {
                r#type: FWP_SID,
                Anonymous: FWP_CONDITION_VALUE0_0 {
                    sid: appcontainer_sid.cast::<SID>(),
                },
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn add_filter(
        engine: HANDLE,
        filter_key: GUID,
        name: &str,
        layer_key: GUID,
        sublayer_key: GUID,
        conditions: &mut [FWPM_FILTER_CONDITION0],
        action_type: u32,
        weight_value: u64,
    ) -> Result<(), WindowsProcessError> {
        let name = wide(OsStr::new(name));
        let mut weight_value = weight_value;
        // SAFETY: zero is the documented initial state for an FWPM filter.
        let mut filter: FWPM_FILTER0 = unsafe { zeroed() };
        filter.filterKey = filter_key;
        filter.displayData = FWPM_DISPLAY_DATA0 {
            name: name.as_ptr().cast_mut(),
            description: null_mut(),
        };
        filter.layerKey = layer_key;
        filter.subLayerKey = sublayer_key;
        filter.weight = FWP_VALUE0 {
            r#type: FWP_UINT64,
            Anonymous: FWP_VALUE0_0 {
                uint64: &raw mut weight_value,
            },
        };
        filter.numFilterConditions = u32::try_from(conditions.len())
            .map_err(|_| WindowsProcessError::Invalid("too many WFP conditions".into()))?;
        filter.filterCondition = conditions.as_mut_ptr();
        filter.action = FWPM_ACTION0 {
            r#type: action_type,
            ..FWPM_ACTION0::default()
        };
        let mut filter_id = 0_u64;
        // SAFETY: every pointer in filter remains live for the call and WFP copies the object.
        let result =
            unsafe { FwpmFilterAdd0(engine, &raw const filter, null_mut(), &mut filter_id) };
        if result == 0 {
            Ok(())
        } else {
            Err(code_error("FwpmFilterAdd0", result))
        }
    }

    fn derived_guid(identity: u128, discriminator: u128) -> GUID {
        GUID::from_u128(identity ^ (0x9d9a_7f44_5183_4f45_8f39_0000_0000_0000 | discriminator))
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
        let network = match (request.proxy_port, request.network_filter_id) {
            (Some(port), Some(identity)) => Some(NetworkGuard::install(sid.0, port, identity)?),
            (None, None) => None,
            _ => {
                return Err(WindowsProcessError::Invalid(
                    "proxy port and network filter identity must be paired".into(),
                ));
            }
        };

        // SAFETY: zero is the documented initial state for these Win32 structures.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_JOB_MEMORY
            | JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        limits.BasicLimitInformation.ActiveProcessLimit = request.max_processes;
        let memory_limit = usize::try_from(request.max_memory_bytes)
            .map_err(|_| WindowsProcessError::Invalid("memory limit overflow".into()))?;
        limits.ProcessMemoryLimit = memory_limit;
        limits.JobMemoryLimit = memory_limit;
        // SAFETY: null security/name pointers request an unnamed default-security Job Object.
        let job = OwnedHandle::new(
            unsafe { CreateJobObjectW(null(), null()) },
            "CreateJobObjectW",
        )?;
        let completion_port = OwnedHandle::new(
            unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, null_mut(), 0, 1) },
            "CreateIoCompletionPort",
        )?;
        let completion = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: job.raw(),
            CompletionPort: completion_port.raw(),
        };
        // SAFETY: completion references live handles and has the documented structure layout.
        if unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectAssociateCompletionPortInformation,
                (&raw const completion).cast(),
                u32::try_from(size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>())
                    .expect("structure size fits u32"),
            )
        } == 0
        {
            return Err(last_error("SetInformationJobObject(completion port)"));
        }
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
        let cwd = wide_process_path(&request.cwd);
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
            completion_port,
            _network: network,
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
        completion_port: &OwnedHandle,
    ) -> Result<Option<ResourceLimitViolation>, WindowsProcessError> {
        let mut observed = None;
        let mut wait_for_delivery = false;
        loop {
            let mut message = 0_u32;
            let mut key = 0_usize;
            let mut overlapped: *mut OVERLAPPED = null_mut();
            let timeout_ms = if wait_for_delivery { 100 } else { 0 };
            // SAFETY: all output pointers are valid and the timeout is strictly bounded.
            let status = unsafe {
                GetQueuedCompletionStatus(
                    completion_port.raw(),
                    &mut message,
                    &mut key,
                    &mut overlapped,
                    timeout_ms,
                )
            };
            if status == 0 {
                // SAFETY: GetLastError immediately follows the failed Win32 call.
                let error = unsafe { GetLastError() };
                if error == WAIT_TIMEOUT {
                    if !wait_for_delivery {
                        wait_for_delivery = true;
                        continue;
                    }
                    break;
                }
                return Err(code_error("GetQueuedCompletionStatus", error));
            }
            if key != job.raw() as usize {
                return Err(WindowsProcessError::Invalid(
                    "Job Object completion key did not match its job".into(),
                ));
            }
            match message {
                JOB_OBJECT_MSG_JOB_MEMORY_LIMIT | JOB_OBJECT_MSG_PROCESS_MEMORY_LIMIT => {
                    observed = Some(ResourceLimitViolation::Memory);
                }
                JOB_OBJECT_MSG_ACTIVE_PROCESS_LIMIT
                    if observed != Some(ResourceLimitViolation::Memory) =>
                {
                    observed = Some(ResourceLimitViolation::ProcessCount);
                }
                _ => {}
            }
        }
        if observed.is_some() {
            return Ok(observed);
        }
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

    pub(super) fn windows_command_line(executable: &OsStr, arguments: &[String]) -> String {
        let cmd_command_index = std::path::Path::new(executable)
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.eq_ignore_ascii_case("cmd.exe"))
            .then(|| {
                arguments.iter().position(|argument| {
                    argument.eq_ignore_ascii_case("/c") || argument.eq_ignore_ascii_case("/k")
                })
            })
            .flatten()
            .filter(|index| index + 2 == arguments.len());
        if let Some(index) = cmd_command_index {
            let prefix = std::iter::once(executable.to_string_lossy().into_owned())
                .chain(arguments[..=index].iter().cloned())
                .map(|argument| quote_argument(&argument))
                .collect::<Vec<_>>()
                .join(" ");
            return format!("{prefix} \"{}\"", arguments[index + 1]);
        }
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

    pub(super) fn wide_process_path(value: &std::path::Path) -> Vec<u16> {
        let encoded = value.as_os_str().encode_wide().collect::<Vec<_>>();
        let verbatim_drive =
            encoded.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16])
                && encoded.get(5) == Some(&(b':' as u16));
        encoded
            .into_iter()
            .skip(if verbatim_drive { 4 } else { 0 })
            .chain(std::iter::once(0))
            .collect()
    }

    pub(super) fn environment_block(
        environment: &std::collections::BTreeMap<String, String>,
    ) -> Vec<u16> {
        let mut entries = environment.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.0
                .to_ascii_lowercase()
                .cmp(&right.0.to_ascii_lowercase())
        });
        let mut block = Vec::new();
        for (name, value) in entries {
            let value = process_value(value);
            block.extend(format!("{name}={value}").encode_utf16());
            block.push(0);
        }
        block.push(0);
        block
    }

    fn process_value(value: &str) -> &str {
        value
            .strip_prefix(r"\\?\")
            .filter(|path| {
                path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                    && path.as_bytes().get(1) == Some(&b':')
            })
            .unwrap_or(value)
    }

    fn last_error(operation: &'static str) -> WindowsProcessError {
        WindowsProcessError::Win32 {
            operation,
            source: std::io::Error::last_os_error(),
        }
    }

    fn code_error(operation: &'static str, code: u32) -> WindowsProcessError {
        WindowsProcessError::Win32 {
            operation,
            source: std::io::Error::from_raw_os_error(i32::from_ne_bytes(code.to_ne_bytes())),
        }
    }
}

#[cfg(test)]
mod tests;
