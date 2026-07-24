use std::sync::{Arc, Mutex};

use portable_pty::ChildKiller;
#[cfg(target_os = "macos")]
use portable_pty::{Child, ExitStatus};
#[cfg(target_os = "macos")]
use std::io;

use crate::terminal::TerminalError;

pub(crate) struct TuiAuthenticationChannel {
    pub(crate) reader: std::fs::File,
    pub(crate) writer: std::fs::File,
}

/// Native process authority retained for the lifetime of one verified TUI PTY.
///
/// The macOS MVP deliberately exposes no arbitrary Shell process. macOS has no
/// supported race-free descendant job primitive for an ordinary desktop app, so this
/// owner supervises only the signed CLI's original session and never claims that a
/// hostile process can be followed across `setsid` and reparenting.
#[derive(Clone)]
pub(crate) struct TerminalProcessTree(Arc<Mutex<TerminalProcessTreeInner>>);

struct TerminalProcessTreeInner {
    process_group: Option<i32>,
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

    #[cfg(unix)]
    pub(crate) fn interrupt(&self) -> Result<(), TerminalError> {
        let tree = self.0.lock().map_err(|_| TerminalError::Internal)?;
        let process_group = tree.process_group.ok_or(TerminalError::IoFailed)?;
        signal_process_group(process_group, nix::sys::signal::Signal::SIGINT)
    }

    #[cfg(not(unix))]
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
        tree.killer.kill().map_err(|_| TerminalError::IoFailed)
    }

    /// Freeze and kill the exact signed CLI session retained by the native host.
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
        let killed = tree.killer.kill().is_ok();
        if signalled || killed {
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
