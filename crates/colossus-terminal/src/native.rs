use colossus_api::{ApiError, ApiErrorReason, ApiResult, HumanTerminalSignal};
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(unix, windows))]
use std::sync::Mutex;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) trait Control: Send + Sync {
    fn resize(&self, rows: u16, cols: u16) -> ApiResult<()>;
    fn signal(&self, signal: HumanTerminalSignal) -> ApiResult<()>;
    fn close(&self);
    /// Observe direct-child exit without releasing Unix PID/session identity.
    fn leader_exited(&self) -> bool;
}
pub(super) struct Spawned {
    pub(super) reader: Box<dyn Read + Send>,
    pub(super) writer: Box<dyn Write + Send>,
    pub(super) control: Arc<dyn Control>,
    pub(super) wait: Box<dyn FnOnce() -> Option<u32> + Send>,
}

pub(super) fn failure() -> ApiError {
    ApiError::failed_precondition(
        ApiErrorReason::InvalidRunTransition,
        "the native terminal is unavailable",
    )
}

/// Retain the exact native workspace directory; paths never come from terminal requests.
pub(super) struct Workspace {
    path: PathBuf,
    #[cfg(unix)]
    directory: std::fs::File,
    #[cfg(unix)]
    identity: (u64, u64),
    #[cfg(windows)]
    directory: colossus_windows_native::BoundPath,
}
impl Workspace {
    pub(super) fn new(path: &Path) -> ApiResult<Self> {
        #[cfg(target_os = "linux")]
        linux::require_pidfds()?;
        let path = std::fs::canonicalize(path).map_err(|_| failure())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let directory = std::fs::File::open(&path).map_err(|_| failure())?;
            let metadata = directory.metadata().map_err(|_| failure())?;
            if !metadata.is_dir() {
                return Err(failure());
            }
            Ok(Self {
                path,
                directory,
                identity: (metadata.dev(), metadata.ino()),
            })
        }
        #[cfg(windows)]
        {
            let directory =
                colossus_windows_native::BoundPath::open_directory(&path).map_err(|_| failure())?;
            Ok(Self { path, directory })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            Err(failure())
        }
    }
    pub(super) fn revalidate(&self) -> ApiResult<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let metadata = std::fs::symlink_metadata(&self.path).map_err(|_| failure())?;
            let retained = self.directory.metadata().map_err(|_| failure())?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || (metadata.dev(), metadata.ino()) != self.identity
                || (retained.dev(), retained.ino()) != self.identity
            {
                return Err(failure());
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            self.directory.revalidate().map_err(|_| failure())
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(failure())
        }
    }
    pub(super) fn spawn(&self, rows: u16, cols: u16) -> ApiResult<Spawned> {
        self.revalidate()?;
        spawn(self, rows, cols)
    }
}

#[cfg(unix)]
fn spawn(workspace: &Workspace, rows: u16, cols: u16) -> ApiResult<Spawned> {
    use portable_pty::CommandBuilder;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    #[cfg(target_os = "macos")]
    let shell = "/bin/zsh";
    #[cfg(not(target_os = "macos"))]
    let shell = "/bin/bash";
    let executable = std::fs::canonicalize(shell).map_err(|_| failure())?;
    let metadata = std::fs::metadata(&executable).map_err(|_| failure())?;
    if metadata.uid() != 0
        || !metadata.is_file()
        || metadata.permissions().mode() & 0o022 != 0
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(failure());
    }
    let pair = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|_| failure())?;
    #[cfg(target_os = "macos")]
    let exit_events = nix::sys::event::Kqueue::new().map_err(|_| failure())?;
    let mut command = CommandBuilder::new(executable);
    #[cfg(target_os = "macos")]
    command.arg("-f");
    #[cfg(not(target_os = "macos"))]
    command.args(["--noprofile", "--norc", "-i"]);
    command.cwd(&workspace.path);
    command.env_clear();
    for name in [
        "HOME", "USER", "LOGNAME", "PATH", "LANG", "LC_ALL", "TMPDIR",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    // Allocate fallible I/O handles before any child exists.
    let reader = pair.master.try_clone_reader().map_err(|_| failure())?;
    let writer = pair.master.take_writer().map_err(|_| failure())?;
    workspace.revalidate()?;
    let mut child = pair.slave.spawn_command(command).map_err(|_| failure())?;
    drop(pair.slave);
    let Some(group) = child.process_id().and_then(|v| i32::try_from(v).ok()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(failure());
    };
    let pid = nix::unistd::Pid::from_raw(group);
    if nix::unistd::getpgid(Some(pid)) != Ok(pid) || nix::unistd::getsid(Some(pid)) != Ok(pid) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(failure());
    }
    #[cfg(target_os = "macos")]
    {
        use nix::sys::event::{EvFlags, EventFilter, FilterFlag, KEvent};
        let event = KEvent::new(
            group as usize,
            EventFilter::EVFILT_PROC,
            EvFlags::EV_ADD | EvFlags::EV_ONESHOT,
            FilterFlag::NOTE_EXIT,
            0,
            0,
        );
        if exit_events
            .kevent(
                &[event],
                &mut [],
                Some(*nix::sys::time::TimeSpec::new(0, 0).as_ref()),
            )
            .is_err()
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failure());
        }
    }
    let control = Arc::new(UnixControl {
        master: Mutex::new(pair.master),
        killer: Mutex::new(child.clone_killer()),
        group,
        closed: Mutex::new(false),
        #[cfg(target_os = "macos")]
        exit_events,
    });
    Ok(Spawned {
        reader,
        writer,
        control,
        wait: Box::new(move || child.wait().ok().map(|v| v.exit_code())),
    })
}

#[cfg(unix)]
struct UnixControl {
    master: Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    group: i32,
    closed: Mutex<bool>,
    #[cfg(target_os = "macos")]
    exit_events: nix::sys::event::Kqueue,
}
#[cfg(unix)]
impl Control for UnixControl {
    fn leader_exited(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
            matches!(waitid(Id::Pid(nix::unistd::Pid::from_raw(self.group)), WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT), Ok(status) if status != WaitStatus::StillAlive)
        }
        #[cfg(target_os = "macos")]
        {
            use nix::sys::event::{EventFilter, FilterFlag, KEvent};
            let mut events = [KEvent::new(
                0,
                EventFilter::EVFILT_PROC,
                nix::sys::event::EvFlags::empty(),
                FilterFlag::empty(),
                0,
                0,
            )];
            self.exit_events
                .kevent(
                    &[],
                    &mut events,
                    Some(*nix::sys::time::TimeSpec::new(0, 0).as_ref()),
                )
                .is_ok_and(|count| {
                    count == 1
                        && events[0].filter() == Ok(EventFilter::EVFILT_PROC)
                        && events[0].fflags().contains(FilterFlag::NOTE_EXIT)
                })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            false
        }
    }
    fn resize(&self, rows: u16, cols: u16) -> ApiResult<()> {
        self.master
            .lock()
            .map_err(|_| failure())?
            .resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|_| failure())
    }
    fn signal(&self, signal: HumanTerminalSignal) -> ApiResult<()> {
        if signal == HumanTerminalSignal::Terminate {
            self.close();
            return Ok(());
        }
        let closed = self.closed.lock().map_err(|_| failure())?;
        if *closed {
            return Err(failure());
        }
        let foreground = self
            .master
            .lock()
            .map_err(|_| failure())?
            .process_group_leader()
            .filter(|id| *id > 0)
            .ok_or_else(failure)?;
        let foreground = nix::unistd::Pid::from_raw(foreground);
        if nix::unistd::getsid(Some(foreground)) != Ok(nix::unistd::Pid::from_raw(self.group)) {
            return Err(failure());
        }
        nix::sys::signal::killpg(foreground, nix::sys::signal::Signal::SIGINT)
            .map_err(|_| failure())
    }
    fn close(&self) {
        if let Ok(mut closed) = self.closed.lock() {
            if *closed {
                return;
            }
            *closed = true;
            let group = nix::unistd::Pid::from_raw(self.group);
            #[cfg(target_os = "macos")]
            {
                // Let the interactive shell notify its ordinary job table before
                // forceful cleanup. Deliberately detached descendants are not contained.
                let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGHUP);
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGSTOP);
            #[cfg(target_os = "linux")]
            linux::close_session(self.group);
            let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
            if let Ok(mut killer) = self.killer.lock() {
                let _ = killer.kill();
            }
        }
    }
}
#[cfg(unix)]
impl Drop for UnixControl {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(windows)]
fn spawn(workspace: &Workspace, rows: u16, cols: u16) -> ApiResult<Spawned> {
    let mut environment = Vec::new();
    for name in [
        "SystemRoot",
        "WINDIR",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "PATH",
        "PATHEXT",
    ] {
        if let Some(value) = std::env::var_os(name) {
            environment.push((name.into(), value));
        }
    }
    environment.push(("TERM".into(), "xterm-256color".into()));
    environment.push(("COLORTERM".into(), "truecolor".into()));
    let spawned = colossus_windows_native::spawn_system_shell_conpty(
        &environment,
        &workspace.path,
        workspace.directory.identity(),
        rows,
        cols,
    )
    .map_err(|_| failure())?;
    let child = Arc::new(Mutex::new(spawned.child));
    let waiter = child.clone();
    Ok(Spawned {
        reader: Box::new(spawned.output),
        writer: Box::new(spawned.input),
        control: Arc::new(WindowsControl {
            control: spawned.control,
            child,
        }),
        wait: Box::new(move || waiter.lock().ok().and_then(|mut child| child.wait().ok())),
    })
}
#[cfg(windows)]
struct WindowsControl {
    control: colossus_windows_native::ConptyControl,
    child: Arc<Mutex<colossus_windows_native::ConptyChild>>,
}
#[cfg(windows)]
impl Control for WindowsControl {
    fn leader_exited(&self) -> bool {
        self.child
            .lock()
            .is_ok_and(|mut child| child.try_wait().is_ok_and(|status| status.is_some()))
    }
    fn resize(&self, rows: u16, cols: u16) -> ApiResult<()> {
        self.control.resize(rows, cols).map_err(|_| failure())
    }
    fn signal(&self, signal: HumanTerminalSignal) -> ApiResult<()> {
        match signal {
            HumanTerminalSignal::Interrupt => self.control.interrupt(),
            HumanTerminalSignal::Terminate => self.control.terminate(),
        }
        .map_err(|_| failure())
    }
    fn close(&self) {
        let _ = self.control.terminate();
    }
}
#[cfg(windows)]
impl Drop for WindowsControl {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(not(any(unix, windows)))]
fn spawn(_workspace: &Workspace, _rows: u16, _cols: u16) -> ApiResult<Spawned> {
    Err(failure())
}
