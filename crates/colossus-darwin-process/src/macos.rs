use std::{
    ffi::{CStr, CString, OsStr, OsString},
    fs::File,
    io,
    mem::MaybeUninit,
    os::{
        fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd},
        unix::ffi::OsStrExt as _,
        unix::process::ExitStatusExt as _,
    },
    path::Path,
    process::ExitStatus,
    time::{Duration, Instant},
};

const START_SUSPENDED: libc::c_short = 0x0080;
const SET_SESSION: libc::c_short = 0x0400;
const CLOSE_UNDECLARED_DESCRIPTORS: libc::c_short = 0x4000;
const SET_SIGNAL_DEFAULTS: libc::c_short = 0x0004;
const SET_SIGNAL_MASK: libc::c_short = 0x0008;
const START_SUSPENDED_TIMEOUT: Duration = Duration::from_secs(2);
/// Fixed child descriptor for native-to-TUI authentication frames.
pub const DESKTOP_TUI_AUTH_INPUT_FD: RawFd = 3;
/// Fixed child descriptor for TUI-to-native authentication frames.
pub const DESKTOP_TUI_AUTH_OUTPUT_FD: RawFd = 4;
const FIRST_CHANNEL_SOURCE_FD: RawFd = DESKTOP_TUI_AUTH_OUTPUT_FD + 1;

/// A direct child whose PID cannot be reused while this owner is alive.
///
/// New values are returned only after Darwin reports the child stopped before its
/// first userspace instruction. Dropping an unreaped value sends `SIGKILL` and waits
/// for the exact direct child, including on validation and cancellation failures.
pub struct DarwinChild {
    pid: libc::pid_t,
    status: Option<ExitStatus>,
    resumed: bool,
}

impl DarwinChild {
    /// Direct child process identifier.
    pub fn pid(&self) -> u32 {
        self.pid.cast_unsigned()
    }

    /// Resume the exact dynamically validated process image.
    pub fn resume(&mut self) -> io::Result<()> {
        if self.resumed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Darwin child was already resumed",
            ));
        }
        // SAFETY: `pid` is an unreaped direct child owned by this value. SIGCONT has
        // no memory-safety preconditions and does not access Rust memory.
        cvt_minus_one(unsafe { libc::kill(self.pid, libc::SIGCONT) })?;
        self.resumed = true;
        Ok(())
    }

    /// Poll for a terminal status without blocking.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        self.wait_with_flags(libc::WNOHANG)
    }

    /// Wait for and reap the direct child.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        loop {
            if let Some(status) = self.wait_with_flags(0)? {
                return Ok(status);
            }
        }
    }

    /// Send `SIGKILL` to the direct child. Call [`Self::wait`] to reap it.
    pub fn start_kill(&mut self) -> io::Result<()> {
        if self.status.is_some() {
            return Ok(());
        }
        // SAFETY: `pid` is an unreaped direct child. A signal does not dereference
        // caller memory. ESRCH is an idempotent terminal condition.
        let result = unsafe { libc::kill(self.pid, libc::SIGKILL) };
        if result == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }

    /// Kill and synchronously reap the direct child.
    pub fn kill_and_reap(&mut self) -> io::Result<ExitStatus> {
        self.start_kill()?;
        self.wait()
    }

    fn wait_with_flags(&mut self, flags: libc::c_int) -> io::Result<Option<ExitStatus>> {
        let mut status = 0;
        loop {
            // SAFETY: `status` is writable for one `c_int`; `pid` is our exact direct
            // child and the flags are the documented waitpid flags used here.
            let waited = unsafe { libc::waitpid(self.pid, &mut status, flags) };
            if waited == self.pid {
                let status = ExitStatus::from_raw(status);
                self.status = Some(status);
                return Ok(Some(status));
            }
            if waited == 0 {
                return Ok(None);
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
    }
}

impl Drop for DarwinChild {
    fn drop(&mut self) {
        if self.status.is_none() {
            let _ = self.kill_and_reap();
        }
    }
}

/// Anonymous parent pipe ends paired with one start-suspended child.
pub struct SpawnedPipes {
    /// Owned child process, still stopped before userspace.
    pub child: DarwinChild,
    /// Parent writer connected to the child's standard input.
    pub input: File,
    /// Parent reader connected to the child's standard output.
    pub output: File,
}

/// Parent-owned private authentication channel paired with one PTY child.
pub struct SpawnedTty {
    /// Owned child process, still stopped before userspace.
    pub child: DarwinChild,
    /// Parent writer connected only to the child's fixed authentication input fd.
    pub input: File,
    /// Parent reader connected only to the child's fixed authentication output fd.
    pub output: File,
}

/// CLOEXEC duplicates of the fixed authentication descriptors inherited by the TUI.
pub struct DesktopTuiAuthenticationChannels {
    /// Native-to-TUI frame reader.
    pub input: File,
    /// TUI-to-native frame writer.
    pub output: File,
}

/// Consume the fixed authentication descriptors inherited by the bundled TUI.
///
/// The returned duplicates are close-on-exec. Both original descriptors are closed
/// before this function returns so the one-use worker capability cannot leak into a
/// process later launched by the CLI.
pub fn take_desktop_tui_authentication_channels() -> io::Result<DesktopTuiAuthenticationChannels> {
    take_authentication_channels(DESKTOP_TUI_AUTH_INPUT_FD, DESKTOP_TUI_AUTH_OUTPUT_FD)
}

fn take_authentication_channels(
    input_fd: RawFd,
    output_fd: RawFd,
) -> io::Result<DesktopTuiAuthenticationChannels> {
    let input = duplicate_cloexec(input_fd);
    let output = duplicate_cloexec(output_fd);
    // Always attempt both closes, including after a duplicate failure. These raw
    // descriptors are transferred to this function and must never remain inherited.
    let closed_input = close_raw_fd(input_fd);
    let closed_output = close_raw_fd(output_fd);
    let input = input?;
    let output = output?;
    closed_input?;
    closed_output?;
    Ok(DesktopTuiAuthenticationChannels {
        input: File::from(input),
        output: File::from(output),
    })
}

fn duplicate_cloexec(fd: RawFd) -> io::Result<OwnedFd> {
    // SAFETY: `fd` is an inherited descriptor transferred to this function.
    // F_DUPFD_CLOEXEC returns a distinct owned descriptor without consuming it.
    let duplicate =
        cvt_fd(unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, FIRST_CHANNEL_SOURCE_FD) })?;
    // SAFETY: successful fcntl returned one new uniquely owned descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
}

fn close_raw_fd(fd: RawFd) -> io::Result<()> {
    // SAFETY: ownership of this inherited raw descriptor was transferred to the
    // caller of `take_authentication_channels`; it is closed exactly once here.
    cvt_minus_one(unsafe { libc::close(fd) })
}

/// Spawn an exact executable with empty/fixed environment and anonymous bootstrap pipes.
pub fn spawn_suspended_pipes(
    executable: &Path,
    arguments: &[OsString],
    environment: &[OsString],
) -> io::Result<SpawnedPipes> {
    let (child_input, parent_input) = cloexec_pipe()?;
    let (parent_output, child_output) = cloexec_pipe()?;
    // Native GUI processes are permitted to start with one or more standard
    // descriptors closed. Move every pipe end above all child destinations before
    // recording dup/close actions so one low-numbered source cannot be overwritten
    // by an earlier action.
    let child_input = normalize_channel_source(child_input)?;
    let parent_input = normalize_channel_source(parent_input)?;
    let parent_output = normalize_channel_source(parent_output)?;
    let child_output = normalize_channel_source(child_output)?;
    let null = CString::new("/dev/null").expect("fixed null path has no NUL");
    let mut actions = FileActions::new()?;
    actions.dup2(child_input.as_raw_fd(), 0)?;
    actions.dup2(child_output.as_raw_fd(), 1)?;
    actions.open(2, null.as_c_str(), libc::O_WRONLY, 0)?;
    for fd in [
        child_input.as_raw_fd(),
        parent_input.as_raw_fd(),
        parent_output.as_raw_fd(),
        child_output.as_raw_fd(),
    ] {
        actions.close(fd)?;
    }
    let child = spawn_suspended(executable, arguments, environment, &actions)?;
    drop(child_input);
    drop(child_output);
    Ok(SpawnedPipes {
        child,
        input: File::from(parent_input),
        output: File::from(parent_output),
    })
}

/// Spawn an exact executable attached to a PTY slave and stopped before userspace.
pub fn spawn_suspended_tty(
    executable: &Path,
    arguments: &[OsString],
    environment: &[OsString],
    tty: &Path,
) -> io::Result<SpawnedTty> {
    let (child_input, parent_input) = cloexec_pipe()?;
    let (parent_output, child_output) = cloexec_pipe()?;
    // Keep every source above the fixed child destinations. This prevents a low
    // descriptor returned after a caller closed stdio from being overwritten or
    // closed by a later dup/close action in the child.
    let child_input = normalize_channel_source(child_input)?;
    let parent_input = normalize_channel_source(parent_input)?;
    let parent_output = normalize_channel_source(parent_output)?;
    let child_output = normalize_channel_source(child_output)?;
    let tty = cstring(tty.as_os_str())?;
    let mut actions = FileActions::new()?;
    actions.open(0, tty.as_c_str(), libc::O_RDWR, 0)?;
    actions.dup2(0, 1)?;
    actions.dup2(0, 2)?;
    actions.dup2(child_input.as_raw_fd(), DESKTOP_TUI_AUTH_INPUT_FD)?;
    actions.dup2(child_output.as_raw_fd(), DESKTOP_TUI_AUTH_OUTPUT_FD)?;
    for fd in [
        child_input.as_raw_fd(),
        parent_input.as_raw_fd(),
        parent_output.as_raw_fd(),
        child_output.as_raw_fd(),
    ] {
        actions.close(fd)?;
    }
    let child = spawn_suspended(executable, arguments, environment, &actions)?;
    drop(child_input);
    drop(child_output);
    Ok(SpawnedTty {
        child,
        input: File::from(parent_input),
        output: File::from(parent_output),
    })
}

fn spawn_suspended(
    executable: &Path,
    arguments: &[OsString],
    environment: &[OsString],
    actions: &FileActions,
) -> io::Result<DarwinChild> {
    if !executable.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Darwin executable path must be absolute",
        ));
    }
    let executable = cstring(executable.as_os_str())?;
    let mut argument_values = Vec::with_capacity(arguments.len() + 1);
    argument_values.push(executable.clone());
    argument_values.extend(
        arguments
            .iter()
            .map(|argument| cstring(argument))
            .collect::<io::Result<Vec<_>>>()?,
    );
    let arguments = CStringArray::new(argument_values);
    let environment = CStringArray::new(
        environment
            .iter()
            .map(|value| cstring(value))
            .collect::<io::Result<Vec<_>>>()?,
    );
    let attributes = SpawnAttributes::new()?;
    let mut pid = 0;
    // SAFETY: every C string and pointer array remains alive for the call, both arrays
    // have a trailing null, the initialized opaque objects own valid Darwin handles,
    // and `pid` is writable. No pointer escapes `posix_spawn`.
    let result = unsafe {
        libc::posix_spawn(
            &mut pid,
            executable.as_ptr(),
            &actions.raw,
            &attributes.raw,
            arguments.as_ptr(),
            environment.as_ptr(),
        )
    };
    cvt_errno(result)?;
    let mut child = DarwinChild {
        pid,
        status: None,
        resumed: false,
    };
    if let Err(error) = confirm_start_suspended(&mut child) {
        let _ = child.kill_and_reap();
        return Err(error);
    }
    Ok(child)
}

fn confirm_start_suspended(child: &mut DarwinChild) -> io::Result<()> {
    let mut status = 0;
    let deadline = Instant::now() + START_SUSPENDED_TIMEOUT;
    loop {
        // SAFETY: `status` is valid writable storage and `child.pid` is the exact
        // unreaped child returned by the immediately preceding posix_spawn.
        let waited =
            unsafe { libc::waitpid(child.pid, &mut status, libc::WUNTRACED | libc::WNOHANG) };
        if waited == child.pid {
            if libc::WIFSTOPPED(status) {
                return Ok(());
            }
            child.status = Some(ExitStatus::from_raw(status));
            return Err(io::Error::other(format!(
                "Darwin child escaped start-suspended launch (wait status {status:#x})"
            )));
        }
        if waited == 0 {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Darwin child did not enter start-suspended state",
                ));
            }
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

struct SpawnAttributes {
    raw: libc::posix_spawnattr_t,
}

impl SpawnAttributes {
    fn new() -> io::Result<Self> {
        let mut raw = MaybeUninit::uninit();
        // SAFETY: Darwin initializes one opaque pointer in `raw` on success.
        cvt_errno(unsafe { libc::posix_spawnattr_init(raw.as_mut_ptr()) })?;
        // SAFETY: successful initialization above produced a valid owned handle.
        let raw = unsafe { raw.assume_init() };
        let mut attributes = Self { raw };
        let flags = START_SUSPENDED
            | SET_SESSION
            | CLOSE_UNDECLARED_DESCRIPTORS
            | SET_SIGNAL_DEFAULTS
            | SET_SIGNAL_MASK;
        // SAFETY: `raw` is initialized and exclusively borrowed. These fixed flags
        // are supported Darwin spawn attributes and fit the API's c_short.
        cvt_errno(unsafe { libc::posix_spawnattr_setflags(&mut attributes.raw, flags) })?;
        let mut actual_flags = 0;
        // SAFETY: `raw` is initialized and `actual_flags` is writable storage for
        // the exact c_short used by Darwin's attribute API.
        cvt_errno(unsafe { libc::posix_spawnattr_getflags(&attributes.raw, &mut actual_flags) })?;
        if actual_flags != flags {
            return Err(io::Error::other(
                "Darwin rejected required sidecar spawn attributes",
            ));
        }
        let defaults = signal_set(true)?;
        let mask = signal_set(false)?;
        // SAFETY: both signal sets and the opaque attributes remain valid for calls.
        cvt_errno(unsafe { libc::posix_spawnattr_setsigdefault(&mut attributes.raw, &defaults) })?;
        // SAFETY: same as above; this installs a deliberately empty child mask.
        cvt_errno(unsafe { libc::posix_spawnattr_setsigmask(&mut attributes.raw, &mask) })?;
        Ok(attributes)
    }
}

impl Drop for SpawnAttributes {
    fn drop(&mut self) {
        // SAFETY: this value owns one successfully initialized opaque attribute.
        let _ = unsafe { libc::posix_spawnattr_destroy(&mut self.raw) };
    }
}

struct FileActions {
    raw: libc::posix_spawn_file_actions_t,
}

impl FileActions {
    fn new() -> io::Result<Self> {
        let mut raw = MaybeUninit::uninit();
        // SAFETY: Darwin initializes one opaque pointer in `raw` on success.
        cvt_errno(unsafe { libc::posix_spawn_file_actions_init(raw.as_mut_ptr()) })?;
        // SAFETY: successful initialization produced a valid owned handle.
        let raw = unsafe { raw.assume_init() };
        Ok(Self { raw })
    }

    fn dup2(&mut self, fd: RawFd, destination: RawFd) -> io::Result<()> {
        // SAFETY: the opaque action object is initialized and exclusively borrowed;
        // integer descriptors are copied by Darwin and never dereferenced by Rust.
        cvt_errno(unsafe { libc::posix_spawn_file_actions_adddup2(&mut self.raw, fd, destination) })
    }

    fn close(&mut self, fd: RawFd) -> io::Result<()> {
        // SAFETY: same initialized action ownership; Darwin records only the integer.
        cvt_errno(unsafe { libc::posix_spawn_file_actions_addclose(&mut self.raw, fd) })
    }

    fn open(
        &mut self,
        fd: RawFd,
        path: &CStr,
        flags: libc::c_int,
        mode: libc::mode_t,
    ) -> io::Result<()> {
        // SAFETY: `path` is NUL-terminated and remains live through the call; the
        // initialized action object is exclusively borrowed.
        cvt_errno(unsafe {
            libc::posix_spawn_file_actions_addopen(&mut self.raw, fd, path.as_ptr(), flags, mode)
        })
    }
}

impl Drop for FileActions {
    fn drop(&mut self) {
        // SAFETY: this value owns one successfully initialized opaque action object.
        let _ = unsafe { libc::posix_spawn_file_actions_destroy(&mut self.raw) };
    }
}

fn cloexec_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut descriptors = [0; 2];
    // SAFETY: `descriptors` is writable storage for exactly two file descriptors.
    cvt_minus_one(unsafe { libc::pipe(descriptors.as_mut_ptr()) })?;
    // SAFETY: successful pipe returned two uniquely owned descriptors.
    let read = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
    // SAFETY: successful pipe returned a distinct uniquely owned write descriptor.
    let write = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
    for fd in [&read, &write] {
        // SAFETY: each descriptor is valid and owned; F_SETFD consumes only its integer.
        cvt_minus_one(unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) })?;
    }
    Ok((read, write))
}

fn normalize_channel_source(fd: OwnedFd) -> io::Result<OwnedFd> {
    if fd.as_raw_fd() >= FIRST_CHANNEL_SOURCE_FD {
        return Ok(fd);
    }
    // SAFETY: `fd` is a valid owned descriptor. F_DUPFD_CLOEXEC creates one distinct
    // descriptor at or above the requested floor without consuming the source.
    let duplicate = cvt_fd(unsafe {
        libc::fcntl(
            fd.as_raw_fd(),
            libc::F_DUPFD_CLOEXEC,
            FIRST_CHANNEL_SOURCE_FD,
        )
    })?;
    // SAFETY: the successful fcntl call returned a new uniquely owned descriptor.
    let duplicate = unsafe { OwnedFd::from_raw_fd(duplicate) };
    Ok(duplicate)
}

fn signal_set(all: bool) -> io::Result<libc::sigset_t> {
    let mut set = MaybeUninit::uninit();
    cvt_minus_one(if all {
        // SAFETY: sigfillset initializes exactly one sigset_t at the valid pointer.
        unsafe { libc::sigfillset(set.as_mut_ptr()) }
    } else {
        // SAFETY: sigemptyset initializes exactly one sigset_t at the valid pointer.
        unsafe { libc::sigemptyset(set.as_mut_ptr()) }
    })?;
    // SAFETY: the successful call initialized `set`.
    let mut set = unsafe { set.assume_init() };
    if all {
        // SIGKILL and SIGSTOP cannot have handlers or defaults installed.
        // SAFETY: `set` is initialized and exclusively borrowed.
        cvt_minus_one(unsafe { libc::sigdelset(&mut set, libc::SIGKILL) })?;
        // SAFETY: same as above.
        cvt_minus_one(unsafe { libc::sigdelset(&mut set, libc::SIGSTOP) })?;
    }
    Ok(set)
}

fn cstring(value: &OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Darwin spawn value contains NUL",
        )
    })
}

struct CStringArray {
    _values: Vec<CString>,
    pointers: Vec<*mut libc::c_char>,
}

impl CStringArray {
    fn new(values: Vec<CString>) -> Self {
        let mut pointers = values
            .iter()
            .map(|value| value.as_ptr().cast_mut())
            .collect::<Vec<_>>();
        pointers.push(std::ptr::null_mut());
        Self {
            _values: values,
            pointers,
        }
    }

    fn as_ptr(&self) -> *const *mut libc::c_char {
        self.pointers.as_ptr()
    }
}

fn cvt_errno(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}

fn cvt_minus_one(result: libc::c_int) -> io::Result<()> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn cvt_fd(result: RawFd) -> io::Result<RawFd> {
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::os::fd::IntoRawFd as _;

    #[test]
    fn start_suspended_pipe_child_cannot_write_before_resume() {
        let mut spawned = spawn_suspended_pipes(
            Path::new("/bin/echo"),
            &[OsString::from("unverified-image-ran")],
            &[],
        )
        .expect("spawn suspended child");
        spawned
            .child
            .kill_and_reap()
            .expect("kill and reap suspended child");
        drop(spawned.input);
        let mut output = Vec::new();
        spawned
            .output
            .read_to_end(&mut output)
            .expect("read child output");
        assert!(output.is_empty());
    }

    #[test]
    fn tty_authentication_uses_only_fixed_inherited_descriptors() {
        let mut spawned = spawn_suspended_tty(
            Path::new("/bin/sh"),
            &[
                OsString::from("-c"),
                OsString::from("IFS= read -r value <&3; printf '%s' \"$value\" >&4"),
            ],
            &[],
            Path::new("/dev/null"),
        )
        .expect("spawn suspended TTY child");
        spawned.child.resume().expect("resume TTY child");
        spawned
            .input
            .write_all(b"private-authentication\n")
            .expect("write private channel");
        drop(spawned.input);
        let mut output = String::new();
        spawned
            .output
            .read_to_string(&mut output)
            .expect("read private channel");
        assert_eq!(output, "private-authentication");
        assert!(spawned.child.wait().expect("wait for TTY child").success());
    }

    #[test]
    fn inherited_authentication_descriptors_are_consumed() {
        let (input_read, input_write) = cloexec_pipe().expect("input pipe");
        let (output_read, output_write) = cloexec_pipe().expect("output pipe");
        let input_fd = input_read.into_raw_fd();
        let output_fd = output_write.into_raw_fd();

        let mut channels =
            take_authentication_channels(input_fd, output_fd).expect("consume inherited channels");

        // SAFETY: F_GETFD only inspects the integer descriptor and does not access
        // caller memory. Both originals must now be invalid.
        assert_eq!(unsafe { libc::fcntl(input_fd, libc::F_GETFD) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EBADF));
        // SAFETY: same fixed descriptor inspection as above.
        assert_eq!(unsafe { libc::fcntl(output_fd, libc::F_GETFD) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EBADF));
        for duplicate in [&channels.input, &channels.output] {
            // SAFETY: the returned File owns a live descriptor; F_GETFD only reads
            // its descriptor flags.
            let flags = unsafe { libc::fcntl(duplicate.as_raw_fd(), libc::F_GETFD) };
            assert_ne!(flags, -1);
            assert_ne!(flags & libc::FD_CLOEXEC, 0);
        }

        let mut input_write = File::from(input_write);
        input_write
            .write_all(b"native-to-tui")
            .expect("write input channel");
        let mut received = [0_u8; 13];
        channels
            .input
            .read_exact(&mut received)
            .expect("read duplicate input channel");
        assert_eq!(&received, b"native-to-tui");

        channels
            .output
            .write_all(b"tui-to-native")
            .expect("write duplicate output channel");
        let mut output_read = File::from(output_read);
        output_read
            .read_exact(&mut received)
            .expect("read output channel");
        assert_eq!(&received, b"tui-to-native");
    }
}
