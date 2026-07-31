use std::sync::{Arc, Mutex};

use portable_pty::ChildKiller;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use portable_pty::{Child, ExitStatus};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::io;

use crate::terminal::TerminalError;

pub(crate) struct TuiAuthenticationChannel {
    pub(crate) reader: std::fs::File,
    pub(crate) writer: std::fs::File,
}

/// Native process authority retained for the lifetime of one local PTY.
///
/// macOS has no supported race-free descendant job primitive for an ordinary desktop
/// app. This owner therefore supervises the original process session and never claims
/// that a hostile process can be followed across `setsid` and reparenting.
#[derive(Clone)]
pub(crate) struct TerminalProcessTree(Arc<Mutex<TerminalProcessTreeInner>>);

struct TerminalProcessTreeInner {
    #[cfg(unix)]
    process_group: Option<i32>,
    #[cfg(target_os = "windows")]
    windows_control: colossus_windows_native::ConptyControl,
    killer: Box<dyn ChildKiller + Send + Sync>,
    closed: bool,
}

impl TerminalProcessTree {
    #[cfg(target_os = "macos")]
    fn from_suspended_pid(pid: nix::unistd::Pid, child: &dyn Child) -> Self {
        Self(Arc::new(Mutex::new(TerminalProcessTreeInner {
            process_group: Some(pid.as_raw()),
            killer: child.clone_killer(),
            closed: false,
        })))
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn from_spawned_macos_session(child: &dyn Child) -> Result<Self, TerminalError> {
        let pid = child
            .process_id()
            .and_then(|process_id| i32::try_from(process_id).ok())
            .map(nix::unistd::Pid::from_raw)
            .ok_or(TerminalError::SpawnFailed)?;
        let tree = Self::from_suspended_pid(pid, child);
        if nix::unistd::getpgid(Some(pid)) != Ok(pid) || nix::unistd::getsid(Some(pid)) != Ok(pid) {
            let _ = tree.force_close();
            return Err(TerminalError::SpawnFailed);
        }
        Ok(tree)
    }

    #[cfg(target_os = "windows")]
    fn from_windows(control: colossus_windows_native::ConptyControl, child: &dyn Child) -> Self {
        Self(Arc::new(Mutex::new(TerminalProcessTreeInner {
            windows_control: control,
            killer: child.clone_killer(),
            closed: false,
        })))
    }

    #[cfg(unix)]
    pub(crate) fn interrupt(&self) -> Result<(), TerminalError> {
        let tree = self.0.lock().map_err(|_| TerminalError::Internal)?;
        let process_group = tree.process_group.ok_or(TerminalError::IoFailed)?;
        signal_process_group(process_group, nix::sys::signal::Signal::SIGINT)
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn interrupt(&self) -> Result<(), TerminalError> {
        self.0
            .lock()
            .map_err(|_| TerminalError::Internal)?
            .windows_control
            .interrupt()
            .map_err(|_| TerminalError::IoFailed)
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    pub(crate) fn interrupt(&self) -> Result<(), TerminalError> {
        Err(TerminalError::ProgramUnavailable)
    }

    pub(crate) fn terminate(&self) -> Result<(), TerminalError> {
        let mut tree = self.0.lock().map_err(|_| TerminalError::Internal)?;
        #[cfg(unix)]
        if let Some(process_group) = tree.process_group
            && signal_process_group(process_group, nix::sys::signal::Signal::SIGTERM).is_ok()
        {
            return Ok(());
        }
        #[cfg(target_os = "windows")]
        if tree.windows_control.terminate().is_ok() {
            return Ok(());
        }
        tree.killer.kill().map_err(|_| TerminalError::IoFailed)
    }

    /// Freeze and kill the exact process session retained by the native host.
    pub(crate) fn force_close(&self) -> Result<(), TerminalError> {
        let mut tree = self.0.lock().map_err(|_| TerminalError::Internal)?;
        if tree.closed {
            return Ok(());
        }
        tree.closed = true;
        #[cfg(unix)]
        let signalled = tree.process_group.is_some_and(|process_group| {
            let _ = signal_process_group(process_group, nix::sys::signal::Signal::SIGSTOP);
            signal_process_group(process_group, nix::sys::signal::Signal::SIGKILL).is_ok()
        });
        #[cfg(not(unix))]
        let signalled = false;
        #[cfg(target_os = "windows")]
        let job_terminated = tree.windows_control.terminate().is_ok();
        #[cfg(not(target_os = "windows"))]
        let job_terminated = false;
        let killed = tree.killer.kill().is_ok();
        if signalled || job_terminated || killed {
            Ok(())
        } else {
            Err(TerminalError::IoFailed)
        }
    }
}

#[cfg(unix)]
fn signal_process_group(
    process_group: i32,
    signal: nix::sys::signal::Signal,
) -> Result<(), TerminalError> {
    nix::sys::signal::killpg(nix::unistd::Pid::from_raw(process_group), signal)
        .map_err(|_| TerminalError::IoFailed)
}

#[cfg(target_os = "windows")]
pub(crate) fn spawn_verified_windows_tui(
    executable: &std::path::Path,
    arguments: &[std::path::PathBuf],
    workspace: &std::path::Path,
    workspace_identity: colossus_windows_native::FileIdentity,
    executable_identity: colossus_windows_native::FileIdentity,
    size: portable_pty::PtySize,
) -> Result<crate::terminal::SpawnedTerminal, TerminalError> {
    use portable_pty::MasterPty as _;

    let arguments = arguments
        .iter()
        .map(|argument| argument.as_os_str().to_owned())
        .collect::<Vec<_>>();
    let spawned = colossus_windows_native::spawn_verified_conpty(
        executable,
        executable_identity,
        &arguments,
        &minimal_windows_environment(),
        workspace,
        workspace_identity,
        size.rows,
        size.cols,
    )
    .map_err(|_| TerminalError::SpawnFailed)?;
    let colossus_windows_native::SpawnedConpty {
        control,
        child,
        input,
        output,
        authentication_input,
        authentication_output,
    } = spawned;
    let master = WindowsConptyMaster {
        control: control.clone(),
        readable: output,
        writable: Mutex::new(Some(input)),
        size: Mutex::new(size),
    };
    let reader = master
        .try_clone_reader()
        .map_err(|_| TerminalError::SpawnFailed)?;
    let writer = master
        .take_writer()
        .map_err(|_| TerminalError::SpawnFailed)?;
    let child = WindowsPtyChild { inner: child };
    let process_tree = TerminalProcessTree::from_windows(control, &child);
    Ok(crate::terminal::SpawnedTerminal {
        master: Box::new(master),
        reader,
        writer,
        child: Box::new(child),
        process_tree,
        authentication_channel: Some(TuiAuthenticationChannel {
            reader: authentication_output,
            writer: authentication_input,
        }),
    })
}

#[cfg(target_os = "windows")]
fn minimal_windows_environment() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let mut environment = [
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("LANG", "en_US.UTF-8"),
    ]
    .into_iter()
    .map(|(name, value)| (name.into(), value.into()))
    .collect::<Vec<_>>();
    for name in [
        "SystemRoot",
        "WINDIR",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "PATH",
    ] {
        if let Some(value) = std::env::var_os(name) {
            environment.push((name.into(), value));
        }
    }
    environment
}

#[cfg(target_os = "windows")]
struct WindowsConptyMaster {
    control: colossus_windows_native::ConptyControl,
    readable: std::fs::File,
    writable: Mutex<Option<std::fs::File>>,
    size: Mutex<portable_pty::PtySize>,
}

#[cfg(target_os = "windows")]
impl portable_pty::MasterPty for WindowsConptyMaster {
    fn resize(&self, size: portable_pty::PtySize) -> anyhow::Result<()> {
        self.control.resize(size.rows, size.cols)?;
        *self
            .size
            .lock()
            .map_err(|_| anyhow::anyhow!("ConPTY size lock failed"))? = size;
        Ok(())
    }

    fn get_size(&self) -> anyhow::Result<portable_pty::PtySize> {
        self.size
            .lock()
            .map(|size| *size)
            .map_err(|_| anyhow::anyhow!("ConPTY size lock failed"))
    }

    fn try_clone_reader(&self) -> anyhow::Result<Box<dyn std::io::Read + Send>> {
        Ok(Box::new(self.readable.try_clone()?))
    }

    fn take_writer(&self) -> anyhow::Result<Box<dyn std::io::Write + Send>> {
        self.writable
            .lock()
            .map_err(|_| anyhow::anyhow!("ConPTY writer lock failed"))?
            .take()
            .map(|writer| Box::new(writer) as Box<dyn std::io::Write + Send>)
            .ok_or_else(|| anyhow::anyhow!("ConPTY writer already taken"))
    }
}

#[cfg(target_os = "windows")]
struct WindowsPtyChild {
    inner: colossus_windows_native::ConptyChild,
}

#[cfg(target_os = "windows")]
impl std::fmt::Debug for WindowsPtyChild {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowsPtyChild")
            .field("process_id", &self.inner.process_id())
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "windows")]
impl ChildKiller for WindowsPtyChild {
    fn kill(&mut self) -> io::Result<()> {
        self.inner.kill()
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(WindowsConptyKiller(self.inner.control()))
    }
}

#[cfg(target_os = "windows")]
impl Child for WindowsPtyChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner
            .try_wait()
            .map(|status| status.map(ExitStatus::with_exit_code))
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        self.inner.wait().map(ExitStatus::with_exit_code)
    }

    fn process_id(&self) -> Option<u32> {
        Some(self.inner.process_id())
    }

    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        Some(self.inner.as_raw_handle())
    }
}

#[cfg(target_os = "windows")]
#[derive(Debug)]
struct WindowsConptyKiller(colossus_windows_native::ConptyControl);

#[cfg(target_os = "windows")]
impl ChildKiller for WindowsConptyKiller {
    fn kill(&mut self) -> io::Result<()> {
        self.0
            .terminate()
            .map_err(|error| io::Error::other(error.to_string()))
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Self(self.0.clone()))
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn spawn_verified_tui(
    tty: &std::path::Path,
    executable: &std::path::Path,
    arguments: &[std::path::PathBuf],
    identity: &colossus_sdk::MacosCodeIdentity,
) -> Result<
    (
        Box<dyn Child + Send + Sync>,
        TerminalProcessTree,
        TuiAuthenticationChannel,
    ),
    TerminalError,
> {
    let arguments = arguments
        .iter()
        .map(|argument| argument.as_os_str().to_owned())
        .collect::<Vec<_>>();
    let environment = minimal_environment();
    let spawned =
        colossus_sdk::spawn_suspended_macos_tty(executable, &arguments, &environment, tty)
            .map_err(|_| TerminalError::SpawnFailed)?;
    let colossus_sdk::MacosSuspendedTty {
        child,
        input,
        output,
    } = spawned;
    let pid = nix::unistd::Pid::from_raw(
        i32::try_from(child.pid()).map_err(|_| TerminalError::SpawnFailed)?,
    );
    let mut child = MacosPtyChild::new(child);
    let process_tree = TerminalProcessTree::from_suspended_pid(pid, &child);
    if nix::unistd::getpgid(Some(pid)) != Ok(pid) || nix::unistd::getsid(Some(pid)) != Ok(pid) {
        let _ = process_tree.force_close();
        let _ = child.wait();
        return Err(TerminalError::SpawnFailed);
    }
    if colossus_sdk::validate_suspended_macos_process(&child.inner, identity).is_err() {
        let _ = process_tree.force_close();
        let _ = child.wait();
        return Err(TerminalError::ProgramUnavailable);
    }
    if child.inner.resume().is_err() {
        let _ = process_tree.force_close();
        let _ = child.wait();
        return Err(TerminalError::SpawnFailed);
    }
    Ok((
        Box::new(child),
        process_tree,
        TuiAuthenticationChannel {
            reader: output,
            writer: input,
        },
    ))
}

#[cfg(target_os = "macos")]
fn minimal_environment() -> Vec<std::ffi::OsString> {
    let mut environment = [
        "TERM=xterm-256color".to_owned(),
        "COLORTERM=truecolor".to_owned(),
        "LANG=en_US.UTF-8".to_owned(),
        "PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_owned(),
        "SHELL=/bin/zsh".to_owned(),
    ]
    .into_iter()
    .map(std::ffi::OsString::from)
    .collect::<Vec<_>>();
    if let Some(home) = directories::BaseDirs::new() {
        use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

        let mut value = b"HOME=".to_vec();
        value.extend_from_slice(home.home_dir().as_os_str().as_bytes());
        environment.push(std::ffi::OsString::from_vec(value));
    }
    environment
}

#[cfg(target_os = "macos")]
struct MacosPtyChild {
    inner: colossus_sdk::MacosSuspendedChild,
}

#[cfg(target_os = "macos")]
impl std::fmt::Debug for MacosPtyChild {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MacosPtyChild")
            .field("pid", &self.inner.pid())
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "macos")]
impl MacosPtyChild {
    fn new(inner: colossus_sdk::MacosSuspendedChild) -> Self {
        Self { inner }
    }

    fn portable_status(status: std::process::ExitStatus) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt as _;

        if let Some(code) = status.code() {
            ExitStatus::with_exit_code(u32::try_from(code).unwrap_or(1))
        } else if let Some(signal) = status.signal() {
            ExitStatus::with_signal(&format!("SIG{signal}"))
        } else {
            ExitStatus::with_exit_code(1)
        }
    }
}

#[cfg(target_os = "macos")]
impl ChildKiller for MacosPtyChild {
    fn kill(&mut self) -> io::Result<()> {
        self.inner.start_kill()
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(MacosPidKiller(nix::unistd::Pid::from_raw(
            i32::try_from(self.inner.pid()).expect("Darwin child PID fits pid_t"),
        )))
    }
}

#[cfg(target_os = "macos")]
impl Child for MacosPtyChild {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner
            .try_wait()
            .map(|status| status.map(Self::portable_status))
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        self.inner.wait().map(Self::portable_status)
    }

    fn process_id(&self) -> Option<u32> {
        Some(self.inner.pid())
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct MacosPidKiller(nix::unistd::Pid);

#[cfg(target_os = "macos")]
impl ChildKiller for MacosPidKiller {
    fn kill(&mut self) -> io::Result<()> {
        match nix::sys::signal::kill(self.0, nix::sys::signal::Signal::SIGKILL) {
            Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
            Err(error) => Err(io::Error::from_raw_os_error(error as i32)),
        }
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Self(self.0))
    }
}
