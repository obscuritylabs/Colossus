//! Race-free ConPTY launch for the one bundled Colossus TUI.
//!
//! The child starts suspended with only two private authentication-pipe handles in
//! its inherited handle list. Its executable and workspace identities are checked,
//! it is assigned to a kill-on-close Job Object, and only then is its first thread
//! resumed. The renderer never receives a process handle, pipe handle, or path.

use crate::{FileIdentity, WindowsNativeError};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::Write,
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt as _,
        io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
    },
    path::Path,
    ptr::{null, null_mut},
    sync::{Arc, Mutex},
};
use windows_sys::Win32::{
    Foundation::{
        HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType},
    System::{
        Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole},
        Pipes::CreatePipe,
        Threading::{
            CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            INFINITE, InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTUPINFOEXW,
            UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

const MAX_ARGUMENTS: usize = 64;
const MAX_ARGUMENT_UNITS: usize = 4_096;
const MAX_COMMAND_LINE_UNITS: usize = 32_767;
const MAX_ENVIRONMENT_ENTRIES: usize = 64;
const MAX_ENVIRONMENT_UNITS: usize = 32_767;
const AUTH_INPUT_ENVIRONMENT: &str = "COLOSSUS_DESKTOP_TUI_AUTH_INPUT_HANDLE_V1";
const AUTH_OUTPUT_ENVIRONMENT: &str = "COLOSSUS_DESKTOP_TUI_AUTH_OUTPUT_HANDLE_V1";

/// Parent-owned ConPTY streams and process authority.
pub struct SpawnedConpty {
    /// Complete process-tree and pseudo-console control.
    pub control: ConptyControl,
    /// Exact verified child process.
    pub child: ConptyChild,
    /// Bytes written here become terminal input.
    pub input: File,
    /// Terminal output is read here.
    pub output: File,
    /// Private native-to-TUI authentication frames are written here.
    pub authentication_input: File,
    /// Private TUI-to-native authentication frames are read here.
    pub authentication_output: File,
}

/// Cloneable control for one ConPTY and its kill-on-close process Job.
#[derive(Clone)]
pub struct ConptyControl(Arc<ConptyControlInner>);

impl std::fmt::Debug for ConptyControl {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConptyControl")
            .finish_non_exhaustive()
    }
}

struct ConptyControlInner {
    pseudo_console: PseudoConsole,
    job: crate::windows::KillOnCloseJob,
    interrupt: Mutex<File>,
    terminated: Mutex<bool>,
}

impl ConptyControl {
    /// Resize the attached pseudo console.
    pub fn resize(&self, rows: u16, columns: u16) -> Result<(), WindowsNativeError> {
        if rows < 2 || columns < 2 || rows > 512 || columns > 512 {
            return Err(WindowsNativeError::InvalidInput);
        }
        self.0.pseudo_console.resize(rows, columns)
    }

    /// Deliver the terminal's interrupt byte without using a shell or console attach.
    pub fn interrupt(&self) -> Result<(), WindowsNativeError> {
        self.0
            .interrupt
            .lock()
            .map_err(|_| WindowsNativeError::InvalidInput)?
            .write_all(&[0x03])
            .map_err(|source| WindowsNativeError::Io {
                operation: "write ConPTY interrupt",
                source,
            })
    }

    /// Terminate every process in the child Job Object.
    pub fn terminate(&self) -> Result<(), WindowsNativeError> {
        let mut terminated = self
            .0
            .terminated
            .lock()
            .map_err(|_| WindowsNativeError::InvalidInput)?;
        if *terminated {
            return Ok(());
        }
        self.0.job.terminate()?;
        *terminated = true;
        Ok(())
    }
}

/// Wait authority for the exact verified ConPTY child.
pub struct ConptyChild {
    process: OwnedHandle,
    process_id: u32,
    control: ConptyControl,
    exit_code: Option<u32>,
}

impl std::fmt::Debug for ConptyChild {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConptyChild")
            .field("process_id", &self.process_id)
            .finish_non_exhaustive()
    }
}

impl ConptyChild {
    /// Return the kernel process identifier.
    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    /// Return the native process handle for trait adapters.
    pub fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
        self.process.as_raw_handle()
    }

    /// Poll for process completion.
    pub fn try_wait(&mut self) -> std::io::Result<Option<u32>> {
        if let Some(exit_code) = self.exit_code {
            return Ok(Some(exit_code));
        }
        // SAFETY: the retained process handle remains valid for this nonblocking wait.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle().cast(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let exit_code = process_exit_code(&self.process)?;
                self.exit_code = Some(exit_code);
                Ok(Some(exit_code))
            }
            WAIT_FAILED => Err(std::io::Error::last_os_error()),
            _ => Err(std::io::Error::other(
                "unexpected Windows process wait result",
            )),
        }
    }

    /// Wait for process completion.
    pub fn wait(&mut self) -> std::io::Result<u32> {
        if let Some(exit_code) = self.exit_code {
            return Ok(exit_code);
        }
        // SAFETY: the retained process handle remains valid for this blocking wait.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle().cast(), INFINITE) } {
            WAIT_OBJECT_0 => {
                let exit_code = process_exit_code(&self.process)?;
                self.exit_code = Some(exit_code);
                Ok(exit_code)
            }
            WAIT_FAILED => Err(std::io::Error::last_os_error()),
            _ => Err(std::io::Error::other(
                "unexpected Windows process wait result",
            )),
        }
    }

    /// Terminate the complete child process tree.
    pub fn kill(&mut self) -> std::io::Result<()> {
        self.control
            .terminate()
            .map_err(|error| std::io::Error::other(error.to_string()))
    }

    /// Clone the process-tree authority without cloning the process handle.
    pub fn control(&self) -> ConptyControl {
        self.control.clone()
    }
}

impl Drop for ConptyChild {
    fn drop(&mut self) {
        if self.exit_code.is_none() {
            let _ = self.control.terminate();
        }
    }
}

/// Child-owned authentication handles consumed before ordinary TUI input begins.
pub struct DesktopTuiAuthenticationChannels {
    /// Native-to-TUI frame reader.
    pub input: File,
    /// TUI-to-native frame writer.
    pub output: File,
}

/// Consume the two exact inherited authentication handles and make them non-inheritable.
pub fn take_desktop_tui_authentication_channels()
-> Result<DesktopTuiAuthenticationChannels, WindowsNativeError> {
    let input = take_environment_handle(AUTH_INPUT_ENVIRONMENT, None)?;
    let output = take_environment_handle(AUTH_OUTPUT_ENVIRONMENT, Some(input.as_raw_handle()))?;
    Ok(DesktopTuiAuthenticationChannels {
        input: File::from(input),
        output: File::from(output),
    })
}

/// Start the exact bundled executable in a ConPTY, suspended and Job-bound.
#[allow(clippy::too_many_arguments)]
pub fn spawn_verified_conpty(
    executable: &Path,
    expected_image: FileIdentity,
    arguments: &[OsString],
    environment: &[(OsString, OsString)],
    workspace: &Path,
    expected_workspace: FileIdentity,
    rows: u16,
    columns: u16,
) -> Result<SpawnedConpty, WindowsNativeError> {
    if rows < 2
        || columns < 2
        || rows > 512
        || columns > 512
        || arguments.len() > MAX_ARGUMENTS
        || environment.len() > MAX_ENVIRONMENT_ENTRIES
    {
        return Err(WindowsNativeError::InvalidInput);
    }
    let executable_binding =
        crate::windows::open_bound(executable, crate::windows::BoundKind::File)?;
    if executable_binding.identity != expected_image {
        return Err(WindowsNativeError::IdentityChanged);
    }
    let workspace_binding =
        crate::windows::open_bound(workspace, crate::windows::BoundKind::Directory)?;
    if workspace_binding.identity != expected_workspace {
        return Err(WindowsNativeError::IdentityChanged);
    }

    let (conpty_input_read, conpty_input_write) = anonymous_pipe(false)?;
    let (conpty_output_read, conpty_output_write) = anonymous_pipe(false)?;
    let pseudo_console =
        PseudoConsole::new(rows, columns, &conpty_input_read, &conpty_output_write)?;
    drop(conpty_input_read);
    drop(conpty_output_write);

    let (child_authentication_input, parent_authentication_input) = anonymous_pipe(true)?;
    clear_inherit(&parent_authentication_input)?;
    let (parent_authentication_output, child_authentication_output) = anonymous_pipe(true)?;
    clear_inherit(&parent_authentication_output)?;
    let inherited_handles = [
        child_authentication_input.as_raw_handle().cast(),
        child_authentication_output.as_raw_handle().cast(),
    ];

    let mut environment = environment.to_vec();
    environment.push((
        OsString::from(AUTH_INPUT_ENVIRONMENT),
        OsString::from(handle_text(&child_authentication_input)),
    ));
    environment.push((
        OsString::from(AUTH_OUTPUT_ENVIRONMENT),
        OsString::from(handle_text(&child_authentication_output)),
    ));
    let mut environment = environment_block(&environment)?;
    let application = nul_terminated(executable.as_os_str(), MAX_ARGUMENT_UNITS)?;
    let mut command_line = command_line(executable.as_os_str(), arguments)?;
    let current_directory = nul_terminated(workspace.as_os_str(), MAX_ARGUMENT_UNITS)?;

    let mut attributes = AttributeList::new(2)?;
    attributes.set_pseudo_console(pseudo_console.handle())?;
    attributes.set_handle_list(&inherited_handles)?;
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb =
        u32::try_from(size_of::<STARTUPINFOEXW>()).expect("startup structure size fits u32");
    startup.lpAttributeList = attributes.as_mut_ptr();
    let mut process_information: PROCESS_INFORMATION = unsafe { zeroed() };
    // SAFETY: every pointer references an initialized, bounded buffer for the duration
    // of CreateProcessW. The handle list contains only the two authentication pipes.
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_mut_ptr().cast(),
            current_directory.as_ptr(),
            &raw const startup.StartupInfo,
            &raw mut process_information,
        )
    } == 0
    {
        return Err(last_error("create suspended ConPTY process"));
    }
    // SAFETY: CreateProcessW returned two newly owned non-null handles.
    let process = unsafe { OwnedHandle::from_raw_handle(process_information.hProcess.cast()) };
    // SAFETY: CreateProcessW returned two newly owned non-null handles.
    let thread = unsafe { OwnedHandle::from_raw_handle(process_information.hThread.cast()) };
    drop(child_authentication_input);
    drop(child_authentication_output);

    let job = crate::windows::KillOnCloseJob::assign_and_verify(
        process.as_raw_handle(),
        process_information.dwProcessId,
        expected_image,
    )?;
    executable_binding.revalidate()?;
    workspace_binding.revalidate()?;
    crate::windows::resume_suspended_process(process_information.dwProcessId)?;
    drop(thread);

    let input = File::from(conpty_input_write);
    let interrupt = input.try_clone().map_err(|source| WindowsNativeError::Io {
        operation: "clone ConPTY input",
        source,
    })?;
    let control = ConptyControl(Arc::new(ConptyControlInner {
        pseudo_console,
        job,
        interrupt: Mutex::new(interrupt),
        terminated: Mutex::new(false),
    }));
    let child = ConptyChild {
        process,
        process_id: process_information.dwProcessId,
        control: control.clone(),
        exit_code: None,
    };
    Ok(SpawnedConpty {
        control,
        child,
        input,
        output: File::from(conpty_output_read),
        authentication_input: File::from(parent_authentication_input),
        authentication_output: File::from(parent_authentication_output),
    })
}

fn process_exit_code(process: &OwnedHandle) -> std::io::Result<u32> {
    let mut exit_code = 0;
    // SAFETY: the process handle is valid and the output pointer is writable.
    if unsafe { GetExitCodeProcess(process.as_raw_handle().cast(), &raw mut exit_code) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(exit_code)
    }
}

struct PseudoConsole(HPCON);

// SAFETY: an HPCON is an opaque kernel handle. Access is limited to the documented
// thread-safe resize/close operations and close runs exactly once.
unsafe impl Send for PseudoConsole {}
// SAFETY: see the Send rationale; no pointee memory is dereferenced.
unsafe impl Sync for PseudoConsole {}

impl PseudoConsole {
    fn new(
        rows: u16,
        columns: u16,
        input: &OwnedHandle,
        output: &OwnedHandle,
    ) -> Result<Self, WindowsNativeError> {
        let mut handle = 0;
        // SAFETY: both pipe handles remain valid and the output HPCON pointer is writable.
        let result = unsafe {
            CreatePseudoConsole(
                COORD {
                    X: i16::try_from(columns).expect("validated columns fit i16"),
                    Y: i16::try_from(rows).expect("validated rows fit i16"),
                },
                input.as_raw_handle().cast(),
                output.as_raw_handle().cast(),
                0,
                &raw mut handle,
            )
        };
        if result < 0 || handle == 0 {
            Err(WindowsNativeError::Io {
                operation: "create pseudo console",
                source: std::io::Error::from_raw_os_error(result),
            })
        } else {
            Ok(Self(handle))
        }
    }

    fn handle(&self) -> HPCON {
        self.0
    }

    fn resize(&self, rows: u16, columns: u16) -> Result<(), WindowsNativeError> {
        // SAFETY: the retained HPCON remains valid and COORD values were bounded.
        let result = unsafe {
            ResizePseudoConsole(
                self.0,
                COORD {
                    X: i16::try_from(columns).expect("validated columns fit i16"),
                    Y: i16::try_from(rows).expect("validated rows fit i16"),
                },
            )
        };
        if result < 0 {
            Err(WindowsNativeError::Io {
                operation: "resize pseudo console",
                source: std::io::Error::from_raw_os_error(result),
            })
        } else {
            Ok(())
        }
    }
}

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        // SAFETY: this type owns the nonzero HPCON and closes it exactly once.
        unsafe { ClosePseudoConsole(self.0) };
    }
}

struct AttributeList {
    storage: Vec<usize>,
}

impl AttributeList {
    fn new(attribute_count: u32) -> Result<Self, WindowsNativeError> {
        let mut bytes = 0;
        // SAFETY: the null first call queries the required allocation size.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), attribute_count, 0, &raw mut bytes);
        }
        if bytes == 0 || bytes > 1024 * 1024 {
            return Err(last_error("size process attribute list"));
        }
        let words = bytes.div_ceil(size_of::<usize>());
        let mut storage = vec![0_usize; words];
        // SAFETY: the aligned storage contains at least the queried number of bytes.
        if unsafe {
            InitializeProcThreadAttributeList(
                storage.as_mut_ptr().cast(),
                attribute_count,
                0,
                &raw mut bytes,
            )
        } == 0
        {
            return Err(last_error("initialize process attribute list"));
        }
        Ok(Self { storage })
    }

    fn as_mut_ptr(
        &mut self,
    ) -> windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }

    fn set_pseudo_console(&mut self, pseudo_console: HPCON) -> Result<(), WindowsNativeError> {
        // Windows requires the HPCON value itself as lpValue, not a pointer to a local
        // handle variable.
        // SAFETY: the attribute list is initialized and the HPCON remains live.
        if unsafe {
            UpdateProcThreadAttribute(
                self.as_mut_ptr(),
                0,
                usize::try_from(PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE)
                    .expect("attribute constant fits usize"),
                pseudo_console as *const core::ffi::c_void,
                size_of::<HPCON>(),
                null_mut(),
                null(),
            )
        } == 0
        {
            Err(last_error("attach pseudo console"))
        } else {
            Ok(())
        }
    }

    fn set_handle_list(&mut self, handles: &[HANDLE]) -> Result<(), WindowsNativeError> {
        // SAFETY: the handle slice remains live through CreateProcessW and contains
        // exactly the inheritable authentication handles.
        if unsafe {
            UpdateProcThreadAttribute(
                self.as_mut_ptr(),
                0,
                usize::try_from(PROC_THREAD_ATTRIBUTE_HANDLE_LIST)
                    .expect("attribute constant fits usize"),
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles),
                null_mut(),
                null(),
            )
        } == 0
        {
            Err(last_error("restrict inherited handle list"))
        } else {
            Ok(())
        }
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: the list was initialized successfully and is deleted exactly once.
        unsafe { DeleteProcThreadAttributeList(self.as_mut_ptr()) };
    }
}

fn anonymous_pipe(inheritable: bool) -> Result<(OwnedHandle, OwnedHandle), WindowsNativeError> {
    let mut read = null_mut();
    let mut write = null_mut();
    let security = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
            .expect("security attributes size fits u32"),
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: i32::from(inheritable),
    };
    // SAFETY: both output handle pointers are writable and security attributes remain live.
    if unsafe {
        CreatePipe(
            &raw mut read,
            &raw mut write,
            if inheritable {
                &raw const security
            } else {
                null()
            },
            0,
        )
    } == 0
        || read.is_null()
        || write.is_null()
    {
        return Err(last_error("create anonymous pipe"));
    }
    // SAFETY: CreatePipe returned two distinct newly owned non-null handles.
    let read = unsafe { OwnedHandle::from_raw_handle(read.cast()) };
    // SAFETY: CreatePipe returned two distinct newly owned non-null handles.
    let write = unsafe { OwnedHandle::from_raw_handle(write.cast()) };
    Ok((read, write))
}

fn clear_inherit(handle: &OwnedHandle) -> Result<(), WindowsNativeError> {
    // SAFETY: the owned handle remains valid and the mask clears only inheritance.
    if unsafe { SetHandleInformation(handle.as_raw_handle().cast(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        Err(last_error("clear pipe inheritance"))
    } else {
        Ok(())
    }
}

fn handle_text(handle: &OwnedHandle) -> String {
    format!("{}", handle.as_raw_handle() as usize)
}

fn take_environment_handle(
    name: &'static str,
    distinct_from: Option<std::os::windows::io::RawHandle>,
) -> Result<OwnedHandle, WindowsNativeError> {
    let value = std::env::var_os(name).ok_or(WindowsNativeError::InvalidInput)?;
    let value = value.to_str().ok_or(WindowsNativeError::InvalidInput)?;
    if value.is_empty() || value.len() > 32 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(WindowsNativeError::InvalidInput);
    }
    let raw = value
        .parse::<usize>()
        .map_err(|_| WindowsNativeError::InvalidInput)?
        as std::os::windows::io::RawHandle;
    if raw.is_null() || distinct_from == Some(raw) {
        return Err(WindowsNativeError::InvalidInput);
    }
    // SAFETY: the inherited value is treated only as a borrowed handle until it has
    // been proven to be an anonymous/named pipe.
    if unsafe { GetFileType(raw.cast()) } != FILE_TYPE_PIPE {
        return Err(WindowsNativeError::InvalidInput);
    }
    // SAFETY: the inherited handle is live; clearing inheritance prevents descendants
    // from receiving the one-use authentication channel.
    if unsafe { SetHandleInformation(raw.cast(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(last_error("consume inherited authentication handle"));
    }
    // SAFETY: this function consumes one unique inherited handle after validating
    // its type and clearing descendant inheritance.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

fn command_line(
    executable: &OsStr,
    arguments: &[OsString],
) -> Result<Vec<u16>, WindowsNativeError> {
    let mut command = quote_argument(executable)?;
    for argument in arguments {
        command.push(u16::from(b' '));
        command.extend(quote_argument(argument)?);
    }
    if command.len() >= MAX_COMMAND_LINE_UNITS {
        return Err(WindowsNativeError::InvalidInput);
    }
    command.push(0);
    Ok(command)
}

fn quote_argument(argument: &OsStr) -> Result<Vec<u16>, WindowsNativeError> {
    let units = argument.encode_wide().collect::<Vec<_>>();
    if units.is_empty() || units.len() > MAX_ARGUMENT_UNITS || units.contains(&0) {
        return Err(WindowsNativeError::InvalidInput);
    }
    let quoted = units.iter().any(|unit| {
        *unit == u16::from(b' ') || *unit == u16::from(b'\t') || *unit == u16::from(b'"')
    });
    if !quoted {
        return Ok(units);
    }
    let mut output = vec![u16::from(b'"')];
    let mut backslashes = 0_usize;
    for unit in units {
        if unit == u16::from(b'\\') {
            backslashes += 1;
        } else if unit == u16::from(b'"') {
            output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2 + 1));
            output.push(unit);
            backslashes = 0;
        } else {
            output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes));
            output.push(unit);
            backslashes = 0;
        }
    }
    output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2));
    output.push(u16::from(b'"'));
    Ok(output)
}

fn environment_block(entries: &[(OsString, OsString)]) -> Result<Vec<u16>, WindowsNativeError> {
    if entries.len() > MAX_ENVIRONMENT_ENTRIES + 2 {
        return Err(WindowsNativeError::InvalidInput);
    }
    let mut encoded = Vec::with_capacity(entries.len());
    for (name, value) in entries {
        let name = name.encode_wide().collect::<Vec<_>>();
        let value = value.encode_wide().collect::<Vec<_>>();
        if name.is_empty()
            || name.contains(&0)
            || name.contains(&u16::from(b'='))
            || value.contains(&0)
        {
            return Err(WindowsNativeError::InvalidInput);
        }
        encoded.push((name, value));
    }
    encoded.sort_by(|left, right| {
        String::from_utf16_lossy(&left.0)
            .to_lowercase()
            .cmp(&String::from_utf16_lossy(&right.0).to_lowercase())
    });
    if encoded.windows(2).any(|pair| {
        String::from_utf16_lossy(&pair[0].0)
            .eq_ignore_ascii_case(&String::from_utf16_lossy(&pair[1].0))
    }) {
        return Err(WindowsNativeError::InvalidInput);
    }
    let mut block = Vec::new();
    for (name, value) in encoded {
        block.extend(name);
        block.push(u16::from(b'='));
        block.extend(value);
        block.push(0);
    }
    block.push(0);
    if block.len() > MAX_ENVIRONMENT_UNITS {
        return Err(WindowsNativeError::InvalidInput);
    }
    Ok(block)
}

fn nul_terminated(value: &OsStr, maximum: usize) -> Result<Vec<u16>, WindowsNativeError> {
    let mut encoded = value.encode_wide().collect::<Vec<_>>();
    if encoded.is_empty() || encoded.len() > maximum || encoded.contains(&0) {
        return Err(WindowsNativeError::InvalidInput);
    }
    encoded.push(0);
    Ok(encoded)
}

fn last_error(operation: &'static str) -> WindowsNativeError {
    WindowsNativeError::Io {
        operation,
        source: std::io::Error::last_os_error(),
    }
}
