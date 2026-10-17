//! Fixed helper arguments and suspended creation by the exact trusted caller.
use super::{
    super::{
        AttributeList, OwnedHandle, SpawnRequest, WindowsProcessError, environment_block,
        last_error, wide, wide_process_path, windows_command_line,
    },
    identity,
};
use colossus_windows_native::FileIdentity;
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    mem::{size_of, zeroed},
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    Security::{PSID, SECURITY_CAPABILITIES},
    System::Threading::{
        CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW,
        CreateProcessW, EXTENDED_STARTUPINFO_PRESENT, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
        PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    },
};

pub(super) const MARKER: &str = "--colossus-browser-context-v1";
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Role {
    Launcher,
    Context,
}
impl Role {
    pub(super) fn text(self) -> &'static str {
        match self {
            Self::Launcher => "launcher",
            Self::Context => "context",
        }
    }
}

pub(super) struct Arguments {
    pub role: Role,
    pub nonce: [u8; 16],
    pub core_pid: u32,
    pub core_birth: u64,
    pub core_image: FileIdentity,
}
impl Arguments {
    pub(super) fn command(&self, role: Role) -> Vec<String> {
        vec![
            MARKER.into(),
            role.text().into(),
            hex(&self.nonce),
            self.core_pid.to_string(),
            self.core_birth.to_string(),
            self.core_image.volume_serial_number.to_string(),
            hex(&self.core_image.file_id),
        ]
    }
    pub(super) fn pipe_name(&self, role: Role) -> String {
        format!(
            r"\\.\pipe\ColossusBrowserContext-{}-{}",
            hex(&self.nonce),
            role.text()
        )
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn unhex(value: &str) -> Result<[u8; 16], WindowsProcessError> {
    identity::require(
        value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "helper nonce syntax",
    )?;
    let mut bytes = [0; 16];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| WindowsProcessError::Invalid("helper nonce encoding".into()))?;
    }
    Ok(bytes)
}

pub(super) fn parse() -> Option<Result<Arguments, WindowsProcessError>> {
    parse_values(std::env::args_os().skip(1).collect())
}

fn parse_values(values: Vec<std::ffi::OsString>) -> Option<Result<Arguments, WindowsProcessError>> {
    if !values.iter().any(|value| {
        value
            .to_string_lossy()
            .starts_with("--colossus-browser-context")
    }) {
        return None;
    }
    Some((|| {
        identity::require(values.len() == 7, "mixed or unexpected helper arguments")?;
        let values = values
            .into_iter()
            .map(|value| {
                value
                    .into_string()
                    .map_err(|_| WindowsProcessError::Invalid("helper argument encoding".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        identity::require(values[0] == MARKER, "unknown helper dispatch version")?;
        let role = match values[1].as_str() {
            "launcher" => Role::Launcher,
            "context" => Role::Context,
            _ => return Err(WindowsProcessError::Invalid("unknown helper role".into())),
        };
        let number = |index: usize| {
            values[index]
                .parse::<u64>()
                .map_err(|_| WindowsProcessError::Invalid("helper identity encoding".into()))
        };
        let core_pid = u32::try_from(number(3)?)
            .map_err(|_| WindowsProcessError::Invalid("helper Core PID bound".into()))?;
        identity::require(
            core_pid != 0 && number(4)? != 0,
            "helper Core identity absent",
        )?;
        Ok(Arguments {
            role,
            nonce: unhex(&values[2])?,
            core_pid,
            core_birth: number(4)?,
            core_image: FileIdentity {
                volume_serial_number: number(5)?,
                file_id: unhex(&values[6])?,
            },
        })
    })())
}

pub(super) struct Created {
    pub process: OwnedHandle,
    pub thread: OwnedHandle,
    pub pid: u32,
    pub tid: u32,
}
impl Created {
    pub(super) fn from_raw(info: PROCESS_INFORMATION) -> Result<Self, WindowsProcessError> {
        // Own BOTH raw outputs before the first fallible consistency check.
        let process = OwnedHandle(info.hProcess);
        let thread = OwnedHandle(info.hThread);
        identity::require(
            !process.raw().is_null() && !thread.raw().is_null(),
            "created PROCESS_INFORMATION is incomplete",
        )?;
        Ok(Self {
            process,
            thread,
            pid: info.dwProcessId,
            tid: info.dwThreadId,
        })
    }
}

pub(super) fn helper(
    request: &SpawnRequest,
    arguments: &Arguments,
    outer: &OwnedHandle,
) -> Result<Created, WindowsProcessError> {
    let jobs = [outer.raw()];
    let mut attributes = AttributeList::new(1)?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
        jobs.as_ptr().cast(),
        size_of::<HANDLE>(),
        "helper atomic outer Job",
    )?;
    let arguments = arguments.command(Role::Launcher);
    // SAFETY: zero is the documented initial state; no standard handles are inherited.
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.lpAttributeList = attributes.raw();
    let environment = request
        .environment
        .iter()
        .filter(|(key, _)| {
            matches!(
                key.as_str(),
                "SystemRoot" | "windir" | "SystemDrive" | "TEMP" | "TMP" | "LOCALAPPDATA"
            )
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    create(
        &request.executable,
        &request.cwd,
        &environment,
        &arguments,
        &mut startup,
        false,
        None,
    )
}

pub(super) fn browser(
    values: &BrowserValues,
    executable: &Path,
    inner: &OwnedHandle,
    package: PSID,
    endpoints: &[HANDLE],
    primary: Option<&OwnedHandle>,
) -> Result<Created, WindowsProcessError> {
    identity::require(
        matches!(endpoints.len(), 9 | 11)
            && values.arguments.len() + 1 + endpoints.len() - 3 <= 256,
        "fixed local endpoint count",
    )?;
    let jobs = [inner.raw()];
    let mut capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: package,
        Capabilities: null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let mut attributes = AttributeList::new(3)?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
        (&raw mut capabilities).cast(),
        size_of::<SECURITY_CAPABILITIES>(),
        "browser exact capabilities",
    )?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
        jobs.as_ptr().cast(),
        size_of::<HANDLE>(),
        "browser atomic inner Job",
    )?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
        endpoints.as_ptr().cast(),
        std::mem::size_of_val(endpoints),
        "browser local handle list",
    )?;
    let mut arguments = values.arguments.clone();
    arguments.push("--inherited-browser-pipes".into());
    arguments.extend(
        endpoints[3..]
            .iter()
            .map(|handle| (*handle as usize).to_string()),
    );
    // SAFETY: zero initializes all optional Win32 fields; attribute storage remains
    // live and unmoved through CreateProcess and attribute-list destruction.
    let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = endpoints[0];
    startup.StartupInfo.hStdOutput = endpoints[1];
    startup.StartupInfo.hStdError = endpoints[2];
    startup.lpAttributeList = attributes.raw();
    let cwd = executable
        .parent()
        .ok_or_else(|| WindowsProcessError::Invalid("browser executable parent absent".into()))?;
    create(
        executable,
        cwd,
        &values.environment,
        &arguments,
        &mut startup,
        true,
        primary,
    )
}

fn create(
    executable: &Path,
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    arguments: &[String],
    startup: &mut STARTUPINFOEXW,
    inherit: bool,
    primary: Option<&OwnedHandle>,
) -> Result<Created, WindowsProcessError> {
    let executable_wide = wide(executable.as_os_str());
    let mut command = wide(OsStr::new(&windows_command_line(
        executable.as_os_str(),
        arguments,
    )));
    let cwd = wide_process_path(cwd);
    let environment = environment_block(environment);
    identity::require(
        command.len() <= 32_767 && environment.len() <= 32_767,
        "context process values exceed Win32 bound",
    )?;
    // SAFETY: zero is the documented process output state; all buffers and attributes
    // are retained until this synchronous call returns. No breakaway flag is used.
    let mut info: PROCESS_INFORMATION = unsafe { zeroed() };
    let flags = CREATE_SUSPENDED
        | CREATE_NO_WINDOW
        | CREATE_UNICODE_ENVIRONMENT
        | EXTENDED_STARTUPINFO_PRESENT;
    let operation = if primary.is_some() {
        "CreateProcessAsUserW(browser context)"
    } else {
        "CreateProcessW(browser context)"
    };
    let status = match primary {
        // SAFETY: the retained restricted own primary passed identity readback; all
        // buffers, exact inherited handles and three attributes remain live as above.
        Some(primary) => unsafe {
            CreateProcessAsUserW(
                primary.raw(),
                executable_wide.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                i32::from(inherit),
                flags,
                environment.as_ptr().cast(),
                cwd.as_ptr(),
                &startup.StartupInfo,
                &mut info,
            )
        },
        // SAFETY: ordinary creation retains the same buffers, flags and startup data.
        None => unsafe {
            CreateProcessW(
                executable_wide.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                i32::from(inherit),
                flags,
                environment.as_ptr().cast(),
                cwd.as_ptr(),
                &startup.StartupInfo,
                &mut info,
            )
        },
    };
    if status == 0 {
        return Err(last_error(operation));
    }
    Created::from_raw(info)
}

pub(super) fn fixed_executable(path: &Path) -> Result<(), WindowsProcessError> {
    identity::require(
        path.file_name() == Some(OsStr::new("colossus-native-browser-host.exe")),
        "fixed browser context host required",
    )
}

pub(super) const MAX_ENVELOPE: usize = 262_144;

/// Closed data for the one fixed browser image; no executable, cwd or identity selector.
pub(super) struct BrowserValues {
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub package: String,
}

impl BrowserValues {
    pub(super) fn encode(request: &SpawnRequest) -> Result<Vec<u8>, WindowsProcessError> {
        let values = Self {
            arguments: request.arguments.clone(),
            environment: request.environment.clone(),
            package: request.appcontainer_sid.clone(),
        };
        values.validate()?;
        let mut bytes = Vec::new();
        put_string(&mut bytes, &values.package)?;
        bytes.extend((values.arguments.len() as u32).to_le_bytes());
        for argument in &values.arguments {
            put_string(&mut bytes, argument)?;
        }
        bytes.extend((values.environment.len() as u32).to_le_bytes());
        for (key, value) in &values.environment {
            put_string(&mut bytes, key)?;
            put_string(&mut bytes, value)?;
        }
        identity::require(bytes.len() <= MAX_ENVELOPE, "browser envelope bound")?;
        Ok(bytes)
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self, WindowsProcessError> {
        identity::require(bytes.len() <= MAX_ENVELOPE, "browser envelope bound")?;
        let mut reader = EnvelopeReader { bytes, offset: 0 };
        let package = reader.string()?;
        let count = reader.count()?;
        identity::require(count <= 256, "browser argument count bound")?;
        let mut arguments = Vec::new();
        for _ in 0..count {
            arguments.push(reader.string()?);
        }
        let count = reader.count()?;
        let mut environment = BTreeMap::new();
        for _ in 0..count {
            let key = reader.string()?;
            let value = reader.string()?;
            identity::require(
                environment.insert(key, value).is_none(),
                "duplicate browser environment entry",
            )?;
        }
        identity::require(
            reader.offset == bytes.len(),
            "trailing browser envelope data",
        )?;
        let values = Self {
            arguments,
            environment,
            package,
        };
        values.validate()?;
        Ok(values)
    }

    fn validate(&self) -> Result<(), WindowsProcessError> {
        identity::require(
            self.arguments.len() <= 256
                && self.environment.len() <= 32_767
                && self.package.len() <= 184
                && self.package.starts_with("S-1-15-2-")
                && self
                    .package
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'S' | b'-'))
                && self.arguments.iter().all(|value| {
                    !value.contains('\0')
                        && !value.starts_with("--colossus-browser-context")
                        && !value.starts_with("--inherited-browser-pipes")
                        && value != "--type"
                        && !value.starts_with("--type=")
                })
                && self.environment.iter().all(|(key, value)| {
                    !key.is_empty() && !key.contains(['\0', '=']) && !value.contains('\0')
                }),
            "fixed browser envelope values rejected",
        )
    }
}

fn put_string(bytes: &mut Vec<u8>, value: &str) -> Result<(), WindowsProcessError> {
    identity::require(
        value.len() <= 131_068 && bytes.len() + value.len() + 4 <= MAX_ENVELOPE,
        "browser envelope string bound",
    )?;
    bytes.extend((value.len() as u32).to_le_bytes());
    bytes.extend(value.as_bytes());
    Ok(())
}

struct EnvelopeReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl EnvelopeReader<'_> {
    fn take(&mut self, length: usize) -> Result<&[u8], WindowsProcessError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| WindowsProcessError::Invalid("browser envelope overflow".into()))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| WindowsProcessError::Invalid("truncated browser envelope".into()))?;
        self.offset = end;
        Ok(bytes)
    }

    fn number(&mut self) -> Result<u32, WindowsProcessError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| {
            WindowsProcessError::Invalid("browser envelope integer".into())
        })?))
    }

    fn count(&mut self) -> Result<u32, WindowsProcessError> {
        let value = self.number()?;
        identity::require(value <= 32_767, "browser envelope count bound")?;
        Ok(value)
    }

    fn string(&mut self) -> Result<String, WindowsProcessError> {
        let length = self.number()? as usize;
        identity::require(length <= 131_068, "browser envelope string bound")?;
        std::str::from_utf8(self.take(length)?)
            .map(str::to_owned)
            .map_err(|_| WindowsProcessError::Invalid("browser envelope encoding".into()))
    }
}

#[cfg(test)]
mod tests;
