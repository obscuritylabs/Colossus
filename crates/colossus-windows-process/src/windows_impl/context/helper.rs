//! The two fixed trusted roles; neither receives outer-Job or Core mutation authority.
use super::super::{
    LocalSid, OwnedHandle, WindowsProcessError, environment_block, last_error, wait_timeout, wide,
    wide_process_path, windows_command_line,
};
use super::{
    fields, identity,
    ipc::Pipe,
    launch::{self, Arguments, Created, Role},
    payload,
};
use colossus_windows_native::AppContainerPrincipal;
use std::{
    ffi::OsStr,
    mem::{size_of, zeroed},
    os::windows::io::BorrowedHandle,
    ptr::null_mut,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT},
    Security::{
        Authorization::ConvertStringSidToSidW,
        Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
    },
    Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
    System::{
        JobObjects::{
            IsProcessInJob, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            JobObjectBasicAccountingInformation, QueryInformationJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
            CreateProcessWithLogonW, GetCurrentProcess, OpenProcess, PROCESS_INFORMATION,
            PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW, TerminateProcess,
        },
    },
};
use zeroize::{Zeroize as _, Zeroizing};

const OBSERVE_CORE: u32 = PROCESS_QUERY_LIMITED_INFORMATION | 0x0010_0000;

pub(super) fn run(arguments: Arguments) -> Result<(), WindowsProcessError> {
    // SAFETY: current-process pseudo handle is observation-only and never owned.
    let own = identity::image(unsafe { GetCurrentProcess() })?;
    launch::fixed_executable(own.canonical_path())?;
    let mut pipe = super::ipc::client(&arguments.pipe_name(arguments.role), arguments.nonce)?;
    // SAFETY: pipe owns its live native handle through this borrow.
    let borrowed = unsafe { BorrowedHandle::borrow_raw(pipe.raw().cast()) };
    identity::native(colossus_windows_native::validate_named_pipe_server(
        &borrowed,
        arguments.core_pid,
    ))?;
    // SAFETY: only the kernel pipe-server PID, with query/synchronization rights;
    // birth/image are independently bound before any request or process creation.
    let core = OwnedHandle::new(
        unsafe { OpenProcess(OBSERVE_CORE, 0, arguments.core_pid) },
        "OpenProcess(observe exact Core)",
    )?;
    identity::verify_process(
        &core,
        arguments.core_pid,
        arguments.core_birth,
        arguments.core_image,
    )?;
    let core_token = identity::token(core.raw())?;
    // SAFETY: observes this trusted role's actual token, not an input identity.
    identity::ordinary(
        &identity::token(unsafe { GetCurrentProcess() })?,
        &core_token,
        arguments.role == Role::Context,
    )?;
    pipe.send(1, &[], &core, Instant::now() + Duration::from_secs(10))?;
    match arguments.role {
        Role::Launcher => launcher(&arguments, &own, &core, &mut pipe),
        Role::Context => context(&own, &core, &mut pipe),
    }
}

struct ReturnedChild(Option<Created>);
impl Drop for ReturnedChild {
    fn drop(&mut self) {
        if let Some(child) = &self.0 {
            // SAFETY: only this launcher's exact returned suspended child; never a PID lookup.
            unsafe {
                TerminateProcess(child.process.raw(), 1);
            }
            let _ = wait_timeout(&child.process, Duration::from_secs(1));
        }
    }
}

fn launcher(
    arguments: &Arguments,
    executable: &colossus_windows_native::BoundPath,
    core: &OwnedHandle,
    pipe: &mut Pipe,
) -> Result<(), WindowsProcessError> {
    fields(
        pipe.receive(core, Some(Instant::now() + Duration::from_secs(10)))?,
        10,
        0,
    )?;
    let result = cpwl(arguments, executable.canonical_path());
    let child = match result {
        Ok(child) => child,
        Err(error) => {
            let code = match &error {
                WindowsProcessError::Win32 { source, .. } => {
                    source.raw_os_error().unwrap_or(1) as u32
                }
                _ => 1,
            };
            pipe.send(
                12,
                &payload(&[u64::from(code)]),
                core,
                Instant::now() + Duration::from_secs(5),
            )?;
            return Err(error);
        }
    };
    let mut child = ReturnedChild(Some(child));
    let returned = child
        .0
        .as_ref()
        .ok_or_else(|| WindowsProcessError::Invalid("missing returned context".into()))?;
    pipe.send(
        11,
        &payload(&[
            returned.process.raw() as usize as u64,
            returned.thread.raw() as usize as u64,
            u64::from(returned.pid),
            u64::from(returned.tid),
        ]),
        core,
        Instant::now() + Duration::from_secs(10),
    )?;
    // Keep exact PROCESS_INFORMATION available for Core's bounded retries while
    // Core lives. A public spawn timeout does not destroy this return channel.
    fields(pipe.receive(core, None)?, 13, 0)?;
    child.0.take(); // Core positively pulled and bound both handles before this ACK.
    identity::require(pipe.finish(), "launcher private I/O remains pending")
}

fn cpwl(
    arguments: &Arguments,
    executable: &std::path::Path,
) -> Result<Created, WindowsProcessError> {
    let executable_wide = wide(executable.as_os_str());
    let values = arguments.command(Role::Context);
    let mut command = wide(OsStr::new(&windows_command_line(
        executable.as_os_str(),
        &values,
    )));
    identity::require(command.len() <= 1024, "CPWL command bound exceeded")?;
    let environment = std::env::vars()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "SystemRoot" | "windir" | "SystemDrive" | "TEMP" | "TMP" | "LOCALAPPDATA"
            )
        })
        .collect();
    let environment = environment_block(&environment);
    let cwd = wide_process_path(
        executable
            .parent()
            .ok_or_else(|| WindowsProcessError::Invalid("helper parent absent".into()))?,
    );
    let user = wide(OsStr::new("colossus_context"));
    let domain = wide(OsStr::new("colossus.invalid"));
    let mut random = Zeroizing::new([0_u8; 32]);
    // SAFETY: bounded writable secret allocation, system preferred RNG.
    identity::require(
        unsafe {
            BCryptGenRandom(
                null_mut(),
                random.as_mut_ptr(),
                random.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        } >= 0,
        "context credential RNG failed",
    )?;
    let alphabet = b"0123456789abcdef";
    let mut password = Zeroizing::new(vec![0_u16; 65]);
    for (index, value) in random.iter().enumerate() {
        password[index * 2] = u16::from(alphabet[(value >> 4) as usize]);
        password[index * 2 + 1] = u16::from(alphabet[(value & 15) as usize]);
    }
    // SAFETY: Win32 startup/output zero states and all fixed buffers live for the call.
    let mut startup: STARTUPINFOW = unsafe { zeroed() };
    startup.cb = size_of::<STARTUPINFOW>() as u32;
    // SAFETY: documented zero state for PROCESS_INFORMATION.
    let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
    // SAFETY: basic CPWL accepts no handle/parent extensions; NETONLY uses only the
    // synthetic process-local secret. No real account or service is configured.
    let status = unsafe {
        CreateProcessWithLogonW(
            user.as_ptr(),
            domain.as_ptr(),
            password.as_ptr(),
            2,
            executable_wide.as_ptr(),
            command.as_mut_ptr(),
            CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            &startup,
            &mut info,
        )
    };
    let error = if status == 0 {
        Some(last_error("CreateProcessWithLogonW(browser context)"))
    } else {
        None
    };
    password.zeroize();
    random.zeroize();
    if let Some(error) = error {
        return Err(error);
    }
    Created::from_raw(info)
}

fn context(
    executable: &colossus_windows_native::BoundPath,
    core: &OwnedHandle,
    pipe: &mut Pipe,
) -> Result<(), WindowsProcessError> {
    let values = fields(
        pipe.receive(core, Some(Instant::now() + Duration::from_secs(15)))?,
        20,
        usize::MAX,
    )?;
    identity::require(
        matches!(values.len(), 9 | 11),
        "fixed context endpoint count",
    )?;
    let mut sorted = values.clone();
    sorted.sort_unstable();
    identity::require(
        sorted.iter().all(|value| *value != 0 && *value != u64::MAX)
            && !sorted.windows(2).any(|pair| pair[0] == pair[1]),
        "duplicate or invalid context endpoint",
    )?;
    let handles = values
        .into_iter()
        .map(remote_handle)
        .collect::<Result<Vec<_>, _>>()?;
    identity::require(
        handles
            .iter()
            .all(|raw| *raw != core.raw() && *raw != pipe.raw()),
        "endpoint aliases retained control authority",
    )?;
    let mut endpoints: Vec<_> = handles.into_iter().map(OwnedHandle).collect();
    for endpoint in &endpoints {
        let mut flags = 0;
        // SAFETY: these exact nonzero unique Core-created handles belong to this process.
        identity::require(
            unsafe { GetFileType(endpoint.raw()) } == FILE_TYPE_PIPE
                && unsafe { GetHandleInformation(endpoint.raw(), &mut flags) } != 0
                && flags & HANDLE_FLAG_INHERIT != 0,
            "context endpoint is not an inherited pipe",
        )?;
    }
    pipe.send(21, &[], core, Instant::now() + Duration::from_secs(5))?;
    let values = fields(pipe.receive(core, None)?, 26, 2)?;
    let job_raw = remote_handle(values[0])?;
    identity::require(
        job_raw != core.raw()
            && job_raw != pipe.raw()
            && !endpoints.iter().any(|endpoint| endpoint.raw() == job_raw),
        "inner Job aliases context authority",
    )?;
    // Core grants only ASSIGN_PROCESS|QUERY; no outer Job or mutation of Core.
    let job = OwnedHandle(job_raw);
    let length = usize::try_from(values[1])
        .map_err(|_| WindowsProcessError::Invalid("browser envelope width".into()))?;
    identity::require(
        length > 0 && length <= launch::MAX_ENVELOPE,
        "browser envelope length bound",
    )?;
    let mut bytes = Vec::with_capacity(length);
    while bytes.len() < length {
        let frame = pipe.receive(core, None)?;
        identity::require(
            frame.op == 27
                && !frame.payload.is_empty()
                && frame.payload.len() <= length - bytes.len(),
            "browser envelope chunk rejected",
        )?;
        bytes.extend(frame.payload);
    }
    let values = launch::BrowserValues::decode(&bytes)?;
    let package_text = wide(OsStr::new(&values.package));
    let mut package = null_mut();
    // SAFETY: bounded terminated package text; Windows owns allocation on success.
    if unsafe { ConvertStringSidToSidW(package_text.as_ptr(), &mut package) } == 0 {
        return Err(last_error("parse context browser package"));
    }
    let package = LocalSid(package);
    // No process creation may begin until this exact final commit frame arrives.
    fields(pipe.receive(core, None)?, 28, 0)?;
    let raw_endpoints: Vec<_> = endpoints.iter().map(OwnedHandle::raw).collect();
    let primary = identity::restricted_browser_primary()?;
    let mut returned = ReturnedChild(Some(launch::browser(
        &values,
        executable.canonical_path(),
        &job,
        package.0,
        &raw_endpoints,
        primary.as_ref(),
    )?));
    drop(primary);
    let created = returned
        .0
        .as_ref()
        .ok_or_else(|| WindowsProcessError::Invalid("missing suspended browser custody".into()))?;
    let birth = identity::birth(created.process.raw())?;
    identity::verify_process(&created.process, created.pid, birth, executable.identity())?;
    let facts = identity::token(created.process.raw())?;
    // SAFETY: current context token is queried from the actual current process.
    let current = identity::token(unsafe { GetCurrentProcess() })?;
    identity::browser(&facts, &identity::token(core.raw())?, &current, package.0)?;
    let mut member = 0;
    // SAFETY: exact returned process and query-capable Job remain retained.
    identity::require(
        unsafe { IsProcessInJob(created.process.raw(), job.raw(), &mut member) } != 0
            && member != 0,
        "station child inner Job mismatch",
    )?;
    // Retain reduced local query objects before releasing the returned full PI.
    // SAFETY: current-process pseudo handle is used only as a duplicate target.
    let own = unsafe { GetCurrentProcess() };
    let child = OwnedHandle(remote_handle(identity::duplicate(
        created.process.raw(),
        own,
        PROCESS_QUERY_LIMITED_INFORMATION,
        false,
    )?)?);
    let query_job = OwnedHandle(remote_handle(identity::duplicate(
        job.raw(),
        own,
        windows_sys::Win32::System::SystemServices::JOB_OBJECT_QUERY,
        false,
    )?)?);
    pipe.send(
        29,
        &payload(&[
            created.process.raw() as usize as u64,
            created.thread.raw() as usize as u64,
            u64::from(created.pid),
            u64::from(created.tid),
            birth,
        ]),
        core,
        Instant::now() + Duration::from_secs(10),
    )?;
    // Keep both exact suspended outputs available until Core pulled and attested them.
    fields(pipe.receive(core, None)?, 30, 0)?;
    drop(returned.0.take());
    drop(job);
    let job = query_job;
    fields(pipe.receive(core, None)?, 22, 0)?;
    let principal = identity::native(AppContainerPrincipal::of_process(child.borrow()))?;
    let mut station = identity::native(principal.create_window_station())?;
    for endpoint in endpoints.drain(..) {
        let raw = endpoint.raw();
        std::mem::forget(endpoint);
        // SAFETY: consumes this one uniquely owned local pipe handle exactly once.
        if unsafe { windows_sys::Win32::Foundation::CloseHandle(raw) } == 0 {
            return Err(last_error("close context endpoint copy"));
        }
    }
    pipe.send(23, &[], core, Instant::now() + Duration::from_secs(5))?;
    fields(pipe.receive(core, None)?, 24, 0)?;
    // SAFETY: exact query-only Job handle and correctly sized initialized output.
    let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
    identity::require(
        unsafe {
            QueryInformationJobObject(
                job.raw(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            )
        } != 0
            && accounting.ActiveProcesses == 0,
        "context close requires exact inner Job zero",
    )?;
    identity::native(station.close())?;
    drop(station);
    drop(principal);
    drop(child);
    drop(job);
    pipe.send(25, &[], core, Instant::now() + Duration::from_secs(5))?;
    identity::require(pipe.finish(), "context private I/O remains pending")
}

fn remote_handle(value: u64) -> Result<HANDLE, WindowsProcessError> {
    let value = usize::try_from(value)
        .map_err(|_| WindowsProcessError::Invalid("remote handle width".into()))?;
    identity::require(
        value != 0 && value <= isize::MAX as usize,
        "invalid or pseudo remote handle",
    )?;
    Ok(value as HANDLE)
}
