use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    thread,
};

use colossus_sdk::WorkspaceIdentity;
use portable_pty::{Child, MasterPty, PtySize};
#[cfg(target_os = "macos")]
use portable_pty::{CommandBuilder, NativePtySystem, PtySystem};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::terminal_process::{TerminalProcessTree, TuiAuthenticationChannel};

const MAX_TERMINAL_SESSIONS: usize = 8;
const MAX_TERMINAL_INPUT_BYTES: usize = 64 * 1024;
const MAX_TERMINAL_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const OUTPUT_CHUNK_BYTES: usize = 8 * 1024;
const MIN_TERMINAL_DIMENSION: u16 = 2;
const MAX_TERMINAL_DIMENSION: u16 = 512;
const TERMINAL_OWNER: &str = "terminal";
const TUI_AUTHENTICATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(target_os = "macos")]
const MACOS_SYSTEM_SHELL: &str = "/bin/zsh";

/// Fixed process types the native desktop is allowed to open in a PTY.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalKind {
    ColossusTui,
    Shell,
}

/// Bounded read-only selection the dedicated terminal renderer may apply after
/// opening the authenticated TUI. These values never become process arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminalPlanContext {
    pub(crate) session_id: String,
    pub(crate) plan_id: String,
}

/// Native-only launch context. None of these paths cross into renderer state.
#[derive(Clone)]
pub(crate) struct TerminalWorkspace {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) workspace: PathBuf,
    pub(crate) workspace_identity: WorkspaceIdentity,
    pub(crate) config: Option<PathBuf>,
    pub(crate) worker_authentication: Option<TerminalWorkerAuthentication>,
}

impl std::fmt::Debug for TerminalWorkspace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TerminalWorkspace")
            .field("id", &self.id)
            .field("display_name", &self.display_name)
            .field("workspace", &"[PRIVATE PATH]")
            .field("workspace_identity", &"[OPAQUE IDENTITY]")
            .field("config", &self.config.as_ref().map(|_| "[PRIVATE PATH]"))
            .field("worker_authentication", &"[REDACTED]")
            .finish()
    }
}

/// Exact selected workspace object retained across signed TUI spawn and authentication.
///
/// This binding is independent of the managed sidecar's binding. It prevents the
/// renderer-visible TUI action from turning a stale same-path selection into worker
/// authority for a replacement directory.
#[derive(Clone)]
struct BoundTerminalWorkspace {
    #[cfg(target_os = "macos")]
    directory: Arc<fs::File>,
    #[cfg(target_os = "windows")]
    binding: Arc<colossus_windows_native::BoundPath>,
    canonical_path: PathBuf,
    identity: WorkspaceIdentity,
    #[cfg(target_os = "macos")]
    device: u64,
    #[cfg(target_os = "macos")]
    inode: u64,
    #[cfg(target_os = "macos")]
    birth_seconds: i64,
    #[cfg(target_os = "macos")]
    birth_nanoseconds: i64,
}

impl BoundTerminalWorkspace {
    #[cfg(target_os = "macos")]
    fn open(path: &Path, expected: &WorkspaceIdentity) -> Result<Self, TerminalError> {
        use std::os::macos::fs::MetadataExt as _;

        expected
            .validate_current()
            .map_err(|_| TerminalError::InvalidWorkspace)?;
        let canonical_path = fs::canonicalize(path).map_err(|_| TerminalError::InvalidWorkspace)?;
        let before = fs::symlink_metadata(path).map_err(|_| TerminalError::InvalidWorkspace)?;
        if canonical_path != path
            || !canonical_path.is_absolute()
            || canonical_path.parent().is_none()
            || before.file_type().is_symlink()
            || !before.is_dir()
        {
            return Err(TerminalError::InvalidWorkspace);
        }
        let directory = fs::File::from(
            rustix::fs::open(
                &canonical_path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|_| TerminalError::InvalidWorkspace)?,
        );
        let opened = directory
            .metadata()
            .map_err(|_| TerminalError::InvalidWorkspace)?;
        let after =
            fs::symlink_metadata(&canonical_path).map_err(|_| TerminalError::InvalidWorkspace)?;
        if !opened.is_dir()
            || after.file_type().is_symlink()
            || !after.is_dir()
            || before.st_dev() != opened.st_dev()
            || before.st_ino() != opened.st_ino()
            || before.st_birthtime() != opened.st_birthtime()
            || before.st_birthtime_nsec() != opened.st_birthtime_nsec()
            || after.st_dev() != opened.st_dev()
            || after.st_ino() != opened.st_ino()
            || after.st_birthtime() != opened.st_birthtime()
            || after.st_birthtime_nsec() != opened.st_birthtime_nsec()
        {
            return Err(TerminalError::InvalidWorkspace);
        }
        let identity = WorkspaceIdentity::from_macos_parts(
            opened.st_dev(),
            opened.st_ino(),
            opened.st_birthtime(),
            opened.st_birthtime_nsec(),
        )
        .map_err(|_| TerminalError::InvalidWorkspace)?;
        if &identity != expected {
            return Err(TerminalError::InvalidWorkspace);
        }
        Ok(Self {
            directory: Arc::new(directory),
            canonical_path,
            identity,
            device: opened.st_dev(),
            inode: opened.st_ino(),
            birth_seconds: opened.st_birthtime(),
            birth_nanoseconds: opened.st_birthtime_nsec(),
        })
    }

    #[cfg(target_os = "windows")]
    fn open(path: &Path, expected: &WorkspaceIdentity) -> Result<Self, TerminalError> {
        expected
            .validate_current()
            .map_err(|_| TerminalError::InvalidWorkspace)?;
        let binding = colossus_windows_native::BoundPath::open_directory(path)
            .map_err(|_| TerminalError::InvalidWorkspace)?;
        let kernel = binding.identity();
        let identity =
            WorkspaceIdentity::from_windows_parts(kernel.volume_serial_number, kernel.file_id)
                .map_err(|_| TerminalError::InvalidWorkspace)?;
        if &identity != expected {
            return Err(TerminalError::InvalidWorkspace);
        }
        let canonical_path = binding.canonical_path().to_owned();
        Ok(Self {
            binding: Arc::new(binding),
            canonical_path,
            identity,
        })
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn open(_path: &Path, _expected: &WorkspaceIdentity) -> Result<Self, TerminalError> {
        Err(TerminalError::InvalidWorkspace)
    }

    #[cfg(target_os = "macos")]
    fn revalidate(&self) -> Result<(), TerminalError> {
        use std::os::macos::fs::MetadataExt as _;

        let retained = self
            .directory
            .metadata()
            .map_err(|_| TerminalError::InvalidWorkspace)?;
        if !retained.is_dir()
            || retained.st_dev() != self.device
            || retained.st_ino() != self.inode
            || retained.st_birthtime() != self.birth_seconds
            || retained.st_birthtime_nsec() != self.birth_nanoseconds
        {
            return Err(TerminalError::InvalidWorkspace);
        }
        let current = Self::open(&self.canonical_path, &self.identity)?;
        if current.device != self.device || current.inode != self.inode {
            return Err(TerminalError::InvalidWorkspace);
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn revalidate(&self) -> Result<(), TerminalError> {
        self.binding
            .revalidate()
            .map_err(|_| TerminalError::InvalidWorkspace)?;
        let current = Self::open(&self.canonical_path, &self.identity)?;
        if current.binding.identity() != self.binding.identity() {
            return Err(TerminalError::InvalidWorkspace);
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn revalidate(&self) -> Result<(), TerminalError> {
        let _ = self;
        Err(TerminalError::InvalidWorkspace)
    }

    fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }
}

/// Native-only managed worker key retained in shared zeroizing memory.
#[derive(Clone)]
pub(crate) struct TerminalWorkerAuthentication(Arc<zeroize::Zeroizing<[u8; 32]>>);

impl TerminalWorkerAuthentication {
    pub(crate) fn random() -> Result<Self, TerminalError> {
        // Six uniformly random bits per byte retain exactly 192 bits of entropy while
        // satisfying the SDK's visible-ASCII credential boundary without encoding the
        // worker key into argv, environment variables, configuration, or a keychain.
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut source = zeroize::Zeroizing::new([0_u8; 32]);
        getrandom::fill(source.as_mut()).map_err(|_| TerminalError::Internal)?;
        let mut authentication = zeroize::Zeroizing::new([0_u8; 32]);
        for (destination, random) in authentication.iter_mut().zip(source.iter()) {
            *destination = ALPHABET[usize::from(*random & 0x3f)];
        }
        Ok(Self(Arc::new(authentication)))
    }

    pub(crate) fn copy_secret(&self) -> zeroize::Zeroizing<[u8; 32]> {
        zeroize::Zeroizing::new(**self.0)
    }
}

impl std::fmt::Debug for TerminalWorkerAuthentication {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TerminalWorkerAuthentication([REDACTED])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TerminalEvent {
    Output {
        session_id: String,
        bytes: Vec<u8>,
    },
    Exited {
        session_id: String,
        exit_code: Option<u32>,
        signal: Option<String>,
    },
    Failed {
        session_id: String,
        code: &'static str,
        message: &'static str,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalSignal {
    Interrupt,
    Terminate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalError {
    Disabled,
    NotReady,
    InvalidOwner,
    InvalidWorkspace,
    InvalidConfiguration,
    ProgramUnavailable,
    InvalidSize,
    InputTooLarge,
    InputBackpressure,
    SessionLimit,
    SessionNotFound,
    SessionOwnerMismatch,
    SpawnFailed,
    IoFailed,
    Internal,
}

impl TerminalError {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::Disabled => "terminal_disabled",
            Self::NotReady => "terminal_not_ready",
            Self::InvalidOwner | Self::SessionOwnerMismatch => "terminal_forbidden",
            Self::InvalidWorkspace => "invalid_workspace",
            Self::InvalidConfiguration => "invalid_configuration",
            Self::ProgramUnavailable => "program_unavailable",
            Self::InvalidSize | Self::InputTooLarge => "invalid_argument",
            Self::InputBackpressure => "terminal_backpressure",
            Self::SessionLimit => "terminal_limit",
            Self::SessionNotFound => "terminal_not_found",
            Self::SpawnFailed => "terminal_spawn_failed",
            Self::IoFailed => "terminal_io_failed",
            Self::Internal => "internal",
        }
    }

    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::Disabled => "Local terminals are disabled until the user opts in.",
            Self::NotReady => "The local terminal window is still loading. Retry shortly.",
            Self::InvalidOwner | Self::SessionOwnerMismatch => {
                "This window is not allowed to control that terminal session."
            }
            Self::InvalidWorkspace => "The selected local workspace is unavailable.",
            Self::InvalidConfiguration => "The managed Colossus configuration is unavailable.",
            Self::ProgramUnavailable => "The requested bundled terminal program is unavailable.",
            Self::InvalidSize => "The requested terminal size is outside the allowed bounds.",
            Self::InputTooLarge => "The terminal input exceeds the per-request limit.",
            Self::InputBackpressure => {
                "The terminal input queue is full. Wait for the process and retry."
            }
            Self::SessionLimit => "The local terminal session limit is active.",
            Self::SessionNotFound => "The local terminal session is no longer available.",
            Self::SpawnFailed => "The local terminal process could not be started safely.",
            Self::IoFailed => "The local terminal process could not continue.",
            Self::Internal => "The native terminal bridge failed safely.",
        }
    }

    pub(crate) fn retryable(self) -> bool {
        matches!(
            self,
            Self::NotReady
                | Self::InputBackpressure
                | Self::SessionLimit
                | Self::SessionNotFound
                | Self::SpawnFailed
                | Self::IoFailed
        )
    }
}

#[derive(Clone)]
pub(crate) struct TerminalManager {
    inner: Arc<TerminalManagerInner>,
}

struct TerminalManagerInner {
    sessions: Arc<Mutex<HashMap<String, TerminalSession>>>,
    colossus_cli: RwLock<Option<VerifiedTerminalExecutable>>,
}

#[derive(Clone)]
struct VerifiedTerminalExecutable {
    path: PathBuf,
    sha256: [u8; 32],
    #[cfg(target_os = "macos")]
    macos_identity: colossus_sdk::MacosCodeIdentity,
    #[cfg(target_os = "windows")]
    windows_identity: colossus_windows_native::FileIdentity,
}

struct TerminalSession {
    owner: String,
    kind: TerminalKind,
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    process_tree: TerminalProcessTree,
    _workspace: BoundTerminalWorkspace,
}

pub(crate) struct SpawnedTerminal {
    pub(crate) master: Box<dyn MasterPty + Send>,
    pub(crate) reader: Box<dyn Read + Send>,
    pub(crate) writer: Box<dyn Write + Send>,
    pub(crate) child: Box<dyn Child + Send + Sync>,
    pub(crate) process_tree: TerminalProcessTree,
    pub(crate) authentication_channel: Option<TuiAuthenticationChannel>,
}

type EventSink = Arc<dyn Fn(TerminalEvent) -> bool + Send + Sync + 'static>;

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalManager {
    fn new() -> Self {
        Self {
            inner: Arc::new(TerminalManagerInner {
                sessions: Arc::new(Mutex::new(HashMap::new())),
                colossus_cli: RwLock::new(None),
            }),
        }
    }

    /// Install the path already selected and integrity-verified by the native bundle
    /// lifecycle. The PTY boundary independently revalidates that it is one absolute,
    /// non-symlink regular executable before retaining it.
    #[allow(dead_code)]
    pub(crate) fn set_verified_colossus_cli(
        &self,
        path: &Path,
        sha256: [u8; 32],
        macos_code_signing_requirement: colossus_sdk::MacosCodeSigningRequirement,
    ) -> Result<(), TerminalError> {
        let path = validate_executable(path)?;
        if sha256_file(&path)? != sha256 {
            return Err(TerminalError::ProgramUnavailable);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = macos_code_signing_requirement;
        #[cfg(target_os = "macos")]
        let macos_identity = colossus_sdk::verify_macos_executable_identity(
            &colossus_sdk::VerifiedExecutable::new(
                &path,
                colossus_sdk::Sha256Digest::from_bytes(sha256),
            )
            .map_err(|_| TerminalError::ProgramUnavailable)?
            .with_macos_code_signing_requirement(macos_code_signing_requirement),
        )
        .map_err(|_| TerminalError::ProgramUnavailable)?;
        #[cfg(target_os = "windows")]
        let windows_identity = {
            let binding = colossus_windows_native::BoundPath::open_file(&path)
                .map_err(|_| TerminalError::ProgramUnavailable)?;
            let mut file = binding
                .try_clone_file()
                .map_err(|_| TerminalError::ProgramUnavailable)?;
            if sha256_reader(&mut file)? != sha256 {
                return Err(TerminalError::ProgramUnavailable);
            }
            binding
                .revalidate()
                .map_err(|_| TerminalError::ProgramUnavailable)?;
            binding.identity()
        };
        *self
            .inner
            .colossus_cli
            .write()
            .map_err(|_| TerminalError::Internal)? = Some(VerifiedTerminalExecutable {
            path,
            sha256,
            #[cfg(target_os = "macos")]
            macos_identity,
            #[cfg(target_os = "windows")]
            windows_identity,
        });
        Ok(())
    }

    pub(crate) fn open(
        &self,
        owner: &str,
        terminal_workspace: &TerminalWorkspace,
        kind: TerminalKind,
        rows: u16,
        cols: u16,
        sink: EventSink,
    ) -> Result<String, TerminalError> {
        validate_owner(owner)?;
        let size = validate_size(rows, cols)?;
        let workspace = BoundTerminalWorkspace::open(
            &terminal_workspace.workspace,
            &terminal_workspace.workspace_identity,
        )?;
        let (program, arguments) =
            self.command(kind, terminal_workspace, workspace.canonical_path())?;

        let mut sessions = self
            .inner
            .sessions
            .lock()
            .map_err(|_| TerminalError::Internal)?;
        if sessions.len() >= MAX_TERMINAL_SESSIONS {
            return Err(TerminalError::SessionLimit);
        }

        // Close the lookup-to-spawn window before the verified child is created.
        workspace.revalidate()?;
        let SpawnedTerminal {
            master,
            reader,
            writer,
            mut child,
            process_tree,
            authentication_channel,
        } = self.spawn(
            size,
            kind,
            &program,
            &arguments,
            workspace.canonical_path(),
            &workspace,
        )?;
        match kind {
            TerminalKind::ColossusTui => {
                let authentication_channel =
                    authentication_channel.ok_or(TerminalError::InvalidConfiguration)?;
                let authentication = terminal_workspace
                    .worker_authentication
                    .clone()
                    .ok_or(TerminalError::InvalidConfiguration)?;
                if let Err(error) = authenticate_tui(
                    authentication_channel,
                    authentication,
                    workspace.clone(),
                    &process_tree,
                ) {
                    let _ = process_tree.force_close();
                    let _ = child.wait();
                    return Err(error);
                }
            }
            TerminalKind::Shell if authentication_channel.is_some() => {
                let _ = process_tree.force_close();
                let _ = child.wait();
                return Err(TerminalError::Internal);
            }
            TerminalKind::Shell => {}
        }

        let session_id = Uuid::new_v4().simple().to_string();
        sessions.insert(
            session_id.clone(),
            TerminalSession {
                owner: owner.to_owned(),
                kind,
                master,
                writer: Arc::new(Mutex::new(writer)),
                process_tree: process_tree.clone(),
                _workspace: workspace,
            },
        );
        drop(sessions);

        if let Err(error) = spawn_output_reader(
            Arc::clone(&self.inner.sessions),
            session_id.clone(),
            child,
            process_tree,
            reader,
            sink,
        ) {
            if let Ok(mut sessions) = self.inner.sessions.lock()
                && let Some(session) = sessions.remove(&session_id)
            {
                let _ = session.process_tree.force_close();
            }
            return Err(error);
        }
        Ok(session_id)
    }

    pub(crate) fn write(
        &self,
        owner: &str,
        session_id: &str,
        bytes: &[u8],
    ) -> Result<(), TerminalError> {
        validate_owner(owner)?;
        if bytes.len() > MAX_TERMINAL_INPUT_BYTES {
            return Err(TerminalError::InputTooLarge);
        }
        let writer = {
            let sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| TerminalError::Internal)?;
            Arc::clone(&owned_session(&sessions, owner, session_id)?.writer)
        };
        let mut writer = match writer.try_lock() {
            Ok(writer) => writer,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(TerminalError::InputBackpressure);
            }
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(TerminalError::Internal),
        };
        writer.write_all(bytes).map_err(|_| TerminalError::IoFailed)
    }

    pub(crate) fn resize(
        &self,
        owner: &str,
        session_id: &str,
        rows: u16,
        cols: u16,
    ) -> Result<(), TerminalError> {
        validate_owner(owner)?;
        let size = validate_size(rows, cols)?;
        let sessions = self
            .inner
            .sessions
            .lock()
            .map_err(|_| TerminalError::Internal)?;
        let session = owned_session(&sessions, owner, session_id)?;
        session
            .master
            .resize(size)
            .map_err(|_| TerminalError::IoFailed)
    }

    pub(crate) fn signal(
        &self,
        owner: &str,
        session_id: &str,
        signal: TerminalSignal,
    ) -> Result<(), TerminalError> {
        validate_owner(owner)?;
        let sessions = self
            .inner
            .sessions
            .lock()
            .map_err(|_| TerminalError::Internal)?;
        let session = owned_session(&sessions, owner, session_id)?;
        match signal {
            TerminalSignal::Interrupt => session.process_tree.interrupt(),
            TerminalSignal::Terminate => session.process_tree.terminate(),
        }
    }

    pub(crate) fn close(&self, owner: &str, session_id: &str) -> Result<(), TerminalError> {
        validate_owner(owner)?;
        let session = {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| TerminalError::Internal)?;
            let session = owned_session(&sessions, owner, session_id)?;
            if session.owner != owner {
                return Err(TerminalError::SessionOwnerMismatch);
            }
            sessions
                .remove(session_id)
                .ok_or(TerminalError::SessionNotFound)?
        };
        session.process_tree.force_close()
    }

    pub(crate) fn close_owner(&self, owner: &str) {
        let removed = self.inner.sessions.lock().ok().map(|mut sessions| {
            let session_ids = sessions
                .iter()
                .filter_map(|(id, session)| (session.owner == owner).then_some(id.clone()))
                .collect::<Vec<_>>();
            session_ids
                .into_iter()
                .filter_map(|id| sessions.remove(&id))
                .collect::<Vec<_>>()
        });
        if let Some(removed) = removed {
            for session in removed {
                let _ = session.process_tree.force_close();
            }
        }
    }

    pub(crate) fn close_kind(&self, owner: &str, kind: TerminalKind) {
        let removed = self.inner.sessions.lock().ok().map(|mut sessions| {
            let session_ids = sessions
                .iter()
                .filter_map(|(id, session)| {
                    (session.owner == owner && session.kind == kind).then_some(id.clone())
                })
                .collect::<Vec<_>>();
            session_ids
                .into_iter()
                .filter_map(|id| sessions.remove(&id))
                .collect::<Vec<_>>()
        });
        if let Some(removed) = removed {
            for session in removed {
                let _ = session.process_tree.force_close();
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn spawn(
        &self,
        size: PtySize,
        kind: TerminalKind,
        program: &Path,
        arguments: &[PathBuf],
        workspace: &Path,
        workspace_binding: &BoundTerminalWorkspace,
    ) -> Result<SpawnedTerminal, TerminalError> {
        let pair = NativePtySystem::default()
            .openpty(size)
            .map_err(|_| TerminalError::SpawnFailed)?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|_| TerminalError::SpawnFailed)?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|_| TerminalError::SpawnFailed)?;
        match kind {
            TerminalKind::ColossusTui => {
                let tty = pair.master.tty_name().ok_or(TerminalError::SpawnFailed)?;
                let executable = self
                    .inner
                    .colossus_cli
                    .read()
                    .map_err(|_| TerminalError::Internal)?
                    .clone()
                    .ok_or(TerminalError::ProgramUnavailable)?;
                if executable.path != program {
                    return Err(TerminalError::ProgramUnavailable);
                }
                drop(pair.slave);
                let (child, process_tree, authentication_channel) =
                    crate::terminal_process::spawn_verified_tui(
                        &tty,
                        program,
                        arguments,
                        &executable.macos_identity,
                    )?;
                Ok(SpawnedTerminal {
                    master: pair.master,
                    reader,
                    writer,
                    child,
                    process_tree,
                    authentication_channel: Some(authentication_channel),
                })
            }
            TerminalKind::Shell => {
                workspace_binding.revalidate()?;
                let command = shell_command(program, arguments, workspace)?;
                let child = pair
                    .slave
                    .spawn_command(command)
                    .map_err(|_| TerminalError::SpawnFailed)?;
                let process_tree = TerminalProcessTree::from_spawned_macos_session(child.as_ref())?;
                drop(pair.slave);
                Ok(SpawnedTerminal {
                    master: pair.master,
                    reader,
                    writer,
                    child,
                    process_tree,
                    authentication_channel: None,
                })
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn spawn(
        &self,
        size: PtySize,
        kind: TerminalKind,
        program: &Path,
        arguments: &[PathBuf],
        workspace: &Path,
        workspace_binding: &BoundTerminalWorkspace,
    ) -> Result<SpawnedTerminal, TerminalError> {
        if kind != TerminalKind::ColossusTui {
            return Err(TerminalError::ProgramUnavailable);
        }
        let executable = self
            .inner
            .colossus_cli
            .read()
            .map_err(|_| TerminalError::Internal)?
            .clone()
            .ok_or(TerminalError::ProgramUnavailable)?;
        if executable.path != program {
            return Err(TerminalError::ProgramUnavailable);
        }
        crate::terminal_process::spawn_verified_windows_tui(
            program,
            arguments,
            workspace,
            workspace_binding.binding.identity(),
            executable.windows_identity,
            size,
        )
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn spawn(
        &self,
        size: PtySize,
        kind: TerminalKind,
        program: &Path,
        arguments: &[PathBuf],
        workspace: &Path,
        workspace_binding: &BoundTerminalWorkspace,
    ) -> Result<SpawnedTerminal, TerminalError> {
        let _ = (
            self,
            size,
            kind,
            program,
            arguments,
            workspace,
            workspace_binding,
        );
        Err(TerminalError::ProgramUnavailable)
    }

    fn command(
        &self,
        kind: TerminalKind,
        terminal_workspace: &TerminalWorkspace,
        workspace: &Path,
    ) -> Result<(PathBuf, Vec<PathBuf>), TerminalError> {
        match kind {
            TerminalKind::ColossusTui => {
                let executable = self
                    .inner
                    .colossus_cli
                    .read()
                    .map_err(|_| TerminalError::Internal)?
                    .clone()
                    .ok_or(TerminalError::ProgramUnavailable)?;
                let cli = validate_executable(&executable.path)?;
                if sha256_file(&cli)? != executable.sha256 {
                    return Err(TerminalError::ProgramUnavailable);
                }
                let config = terminal_workspace
                    .config
                    .as_deref()
                    .ok_or(TerminalError::InvalidConfiguration)?;
                let config = validate_regular_file(config, TerminalError::InvalidConfiguration)?;
                if terminal_workspace.worker_authentication.is_none() {
                    return Err(TerminalError::InvalidConfiguration);
                }
                Ok((
                    cli,
                    vec![
                        PathBuf::from("--workspace"),
                        workspace.to_path_buf(),
                        PathBuf::from("--config"),
                        config,
                        PathBuf::from("--worker-required"),
                        PathBuf::from("--desktop-worker-auth"),
                        PathBuf::from("tui"),
                    ],
                ))
            }
            TerminalKind::Shell => {
                #[cfg(target_os = "macos")]
                {
                    if terminal_workspace.config.is_some()
                        || terminal_workspace.worker_authentication.is_some()
                    {
                        return Err(TerminalError::InvalidConfiguration);
                    }
                    Ok((
                        validate_macos_system_shell(Path::new(MACOS_SYSTEM_SHELL))?,
                        vec![PathBuf::from("-l")],
                    ))
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = (terminal_workspace, workspace);
                    Err(TerminalError::ProgramUnavailable)
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn shell_command(
    program: &Path,
    arguments: &[PathBuf],
    workspace: &Path,
) -> Result<CommandBuilder, TerminalError> {
    if program != Path::new(MACOS_SYSTEM_SHELL) || arguments != [PathBuf::from("-l")] {
        return Err(TerminalError::ProgramUnavailable);
    }
    let mut command = CommandBuilder::new(program);
    command.args(arguments);
    command.cwd(workspace);
    command.env_clear();
    for (name, value) in [
        ("TERM", std::ffi::OsString::from("xterm-256color")),
        ("COLORTERM", std::ffi::OsString::from("truecolor")),
        ("LANG", std::ffi::OsString::from("en_US.UTF-8")),
        (
            "PATH",
            std::ffi::OsString::from(
                "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
            ),
        ),
        ("SHELL", std::ffi::OsString::from(MACOS_SYSTEM_SHELL)),
    ] {
        command.env(name, value);
    }
    if let Some(base_directories) = directories::BaseDirs::new() {
        command.env("HOME", base_directories.home_dir());
    }
    Ok(command)
}

#[cfg(target_os = "macos")]
fn validate_macos_system_shell(path: &Path) -> Result<PathBuf, TerminalError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    if path != Path::new(MACOS_SYSTEM_SHELL) {
        return Err(TerminalError::ProgramUnavailable);
    }
    let shell = validate_executable(path)?;
    let metadata = fs::metadata(&shell).map_err(|_| TerminalError::ProgramUnavailable)?;
    if metadata.uid() != 0
        || metadata.permissions().mode() & 0o022 != 0
        || metadata.permissions().mode() & 0o111 == 0
    {
        return Err(TerminalError::ProgramUnavailable);
    }
    Ok(shell)
}

impl Drop for TerminalManagerInner {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            for (_, session) in sessions.drain() {
                let _ = session.process_tree.force_close();
            }
        }
    }
}

fn spawn_output_reader(
    sessions: Arc<Mutex<HashMap<String, TerminalSession>>>,
    session_id: String,
    mut child: Box<dyn Child + Send + Sync>,
    process_tree: TerminalProcessTree,
    mut reader: Box<dyn Read + Send>,
    sink: EventSink,
) -> Result<(), TerminalError> {
    thread::Builder::new()
        .name(format!("terminal-output-{}", &session_id[..8]))
        .spawn(move || {
            let mut raw_bytes = 0_usize;
            let mut sanitizer = OscSanitizer::default();
            let mut buffer = [0_u8; OUTPUT_CHUNK_BYTES];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        raw_bytes = raw_bytes.saturating_add(read);
                        if raw_bytes > MAX_TERMINAL_OUTPUT_BYTES {
                            let _ = sink(TerminalEvent::Failed {
                                session_id: session_id.clone(),
                                code: "terminal_output_limit",
                                message: "The terminal was stopped after reaching its output limit.",
                            });
                            let _ = process_tree.force_close();
                            break;
                        }
                        let released = sanitizer.filter(&buffer[..read]);
                        if !released.is_empty()
                            && !sink(TerminalEvent::Output {
                                session_id: session_id.clone(),
                                bytes: released,
                            })
                        {
                            let _ = process_tree.force_close();
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = sink(TerminalEvent::Failed {
                            session_id: session_id.clone(),
                            code: "terminal_io_failed",
                            message: "The terminal output stream closed unexpectedly.",
                        });
                        let _ = process_tree.force_close();
                        break;
                    }
                }
            }

            // Keep the direct child unreaped while addressing its original session.
            // That pins the PID/PGID and prevents cleanup from racing identifier reuse.
            let _ = process_tree.force_close();
            let status = child.wait().ok();
            if let Ok(mut sessions) = sessions.lock() {
                sessions.remove(&session_id);
            }
            let _ = sink(TerminalEvent::Exited {
                session_id,
                exit_code: status.as_ref().map(portable_pty::ExitStatus::exit_code),
                signal: status.and_then(|status| status.signal().map(ToOwned::to_owned)),
            });
        })
        .map_err(|_| TerminalError::SpawnFailed)?;
    Ok(())
}

fn authenticate_tui(
    channel: TuiAuthenticationChannel,
    authentication: TerminalWorkerAuthentication,
    workspace: BoundTerminalWorkspace,
    process_tree: &TerminalProcessTree,
) -> Result<(), TerminalError> {
    let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
    let exchange = thread::Builder::new()
        .name("terminal-tui-authentication".into())
        .spawn(move || {
            let mut reader = channel.reader;
            let mut writer = channel.writer;
            let result =
                exchange_tui_authentication(&mut reader, &mut writer, &authentication, &workspace);
            let _ = result_tx.send(result);
        })
        .map_err(|_| TerminalError::SpawnFailed)?;

    match result_rx.recv_timeout(TUI_AUTHENTICATION_TIMEOUT) {
        Ok(Ok(())) => {
            exchange.join().map_err(|_| TerminalError::Internal)?;
            Ok(())
        }
        Ok(Err(error)) => {
            exchange.join().map_err(|_| TerminalError::Internal)?;
            Err(error)
        }
        Err(_) => {
            let _ = process_tree.force_close();
            let _ = exchange.join();
            Err(TerminalError::SpawnFailed)
        }
    }
}

fn exchange_tui_authentication<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    authentication: &TerminalWorkerAuthentication,
    workspace: &BoundTerminalWorkspace,
) -> Result<(), TerminalError> {
    use colossus_sidecar_protocol::{
        DESKTOP_TUI_PROTOCOL_VERSION, DesktopTuiAuthenticationRequest, DesktopTuiChildFrame,
        DesktopTuiParentFrame, encode_worker_authentication, read_frame, write_frame,
    };

    let ready = match read_frame::<_, DesktopTuiChildFrame>(reader)
        .map_err(|_| TerminalError::SpawnFailed)?
    {
        DesktopTuiChildFrame::Ready(ready) => ready,
        DesktopTuiChildFrame::Authenticated(_) => return Err(TerminalError::SpawnFailed),
    };
    ready.validate().map_err(|_| TerminalError::SpawnFailed)?;
    if ready.workspace_identity != workspace.identity {
        return Err(TerminalError::InvalidWorkspace);
    }
    let authentication = authentication.copy_secret();
    let request = DesktopTuiParentFrame::Authenticate(DesktopTuiAuthenticationRequest {
        protocol_version: DESKTOP_TUI_PROTOCOL_VERSION,
        exchange_id: ready.exchange_id.clone(),
        worker_ipc_authentication: encode_worker_authentication(&authentication)
            .map_err(|_| TerminalError::SpawnFailed)?,
    });
    // This is the final operation before the worker credential crosses the inherited
    // channel. If the selected pathname was replaced after spawn, kill the verified
    // TUI without writing any authentication bytes.
    workspace.revalidate()?;
    write_frame(writer, &request).map_err(|_| TerminalError::SpawnFailed)?;
    let acknowledged = match read_frame::<_, DesktopTuiChildFrame>(reader)
        .map_err(|_| TerminalError::SpawnFailed)?
    {
        DesktopTuiChildFrame::Authenticated(acknowledged) => acknowledged,
        DesktopTuiChildFrame::Ready(_) => return Err(TerminalError::SpawnFailed),
    };
    acknowledged
        .validate(&ready.exchange_id)
        .map_err(|_| TerminalError::SpawnFailed)
}

fn owned_session<'a>(
    sessions: &'a HashMap<String, TerminalSession>,
    owner: &str,
    session_id: &str,
) -> Result<&'a TerminalSession, TerminalError> {
    let session = sessions
        .get(session_id)
        .ok_or(TerminalError::SessionNotFound)?;
    if session.owner != owner {
        return Err(TerminalError::SessionOwnerMismatch);
    }
    Ok(session)
}

fn validate_owner(owner: &str) -> Result<(), TerminalError> {
    if owner == TERMINAL_OWNER {
        Ok(())
    } else {
        Err(TerminalError::InvalidOwner)
    }
}

fn validate_size(rows: u16, cols: u16) -> Result<PtySize, TerminalError> {
    if !(MIN_TERMINAL_DIMENSION..=MAX_TERMINAL_DIMENSION).contains(&rows)
        || !(MIN_TERMINAL_DIMENSION..=MAX_TERMINAL_DIMENSION).contains(&cols)
    {
        return Err(TerminalError::InvalidSize);
    }
    Ok(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })
}

fn validate_executable(path: &Path) -> Result<PathBuf, TerminalError> {
    let canonical = validate_regular_file(path, TerminalError::ProgramUnavailable)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if fs::metadata(&canonical)
            .map_err(|_| TerminalError::ProgramUnavailable)?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err(TerminalError::ProgramUnavailable);
        }
    }
    Ok(canonical)
}

fn sha256_file(path: &Path) -> Result<[u8; 32], TerminalError> {
    let mut file = fs::File::open(path).map_err(|_| TerminalError::ProgramUnavailable)?;
    sha256_reader(&mut file)
}

fn sha256_reader(reader: &mut impl Read) -> Result<[u8; 32], TerminalError> {
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| TerminalError::ProgramUnavailable)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest.finalize().into())
}

fn validate_regular_file(path: &Path, error: TerminalError) -> Result<PathBuf, TerminalError> {
    if !path.is_absolute() {
        return Err(error);
    }
    let link_metadata = fs::symlink_metadata(path).map_err(|_| error)?;
    if link_metadata.file_type().is_symlink() || !link_metadata.is_file() {
        return Err(error);
    }
    let canonical = fs::canonicalize(path).map_err(|_| error)?;
    if canonical != path {
        return Err(error);
    }
    Ok(canonical)
}

#[derive(Default)]
struct OscSanitizer {
    state: OscState,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum OscState {
    #[default]
    Ground,
    // Retain a possible UTF-8 C1 lead byte across reader chunks without releasing it.
    GroundUtf8C2,
    Escape,
    Osc,
    OscEscape,
    // OSC content is discarded, but its UTF-8 C1 lead must remain pending so ST can
    // terminate the sequence even when the pair is split across chunks.
    OscUtf8C2,
}

impl OscSanitizer {
    /// Strip all operating-system-command sequences, including title, hyperlink, and
    /// clipboard controls. Ordinary ANSI cursor/color sequences remain available.
    fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut released = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            match self.state {
                OscState::Ground | OscState::GroundUtf8C2 | OscState::Escape if byte == 0x9d => {
                    self.state = OscState::Osc;
                }
                OscState::Ground | OscState::GroundUtf8C2 if byte == 0x9c => {
                    self.state = OscState::Ground;
                }
                OscState::Ground | OscState::GroundUtf8C2 if byte == 0x1b => {
                    self.state = OscState::Escape;
                }
                OscState::Ground if byte == 0xc2 => self.state = OscState::GroundUtf8C2,
                OscState::Ground => released.push(byte),
                OscState::GroundUtf8C2 | OscState::OscUtf8C2 if byte == 0xc2 => {}
                OscState::GroundUtf8C2 => {
                    released.push(0xc2);
                    released.push(byte);
                    self.state = OscState::Ground;
                }
                OscState::Escape if byte == b']' => self.state = OscState::Osc,
                OscState::Escape | OscState::OscEscape if byte == 0x1b => {}
                OscState::Escape if byte == 0xc2 => self.state = OscState::GroundUtf8C2,
                OscState::Escape => {
                    released.push(0x1b);
                    released.push(byte);
                    self.state = OscState::Ground;
                }
                OscState::Osc | OscState::OscEscape | OscState::OscUtf8C2
                    if byte == 0x07 || byte == 0x9c =>
                {
                    self.state = OscState::Ground;
                }
                OscState::Osc | OscState::OscUtf8C2 if byte == 0x1b => {
                    self.state = OscState::OscEscape;
                }
                OscState::Osc if byte == 0xc2 => self.state = OscState::OscUtf8C2,
                OscState::Osc => {}
                OscState::OscEscape if byte == b'\\' => self.state = OscState::Ground,
                OscState::OscEscape if byte == 0xc2 => self.state = OscState::OscUtf8C2,
                OscState::OscEscape | OscState::OscUtf8C2 => self.state = OscState::Osc,
            }
        }
        released
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    fn test_workspace_identity(path: &Path) -> WorkspaceIdentity {
        use std::os::macos::fs::MetadataExt as _;

        let metadata = fs::symlink_metadata(path).expect("workspace metadata");
        WorkspaceIdentity::from_macos_parts(
            metadata.st_dev(),
            metadata.st_ino(),
            metadata.st_birthtime(),
            metadata.st_birthtime_nsec(),
        )
        .expect("current workspace identity")
    }

    #[test]
    fn dimensions_and_input_are_bounded() {
        assert_eq!(validate_size(1, 80), Err(TerminalError::InvalidSize));
        assert_eq!(validate_size(24, 513), Err(TerminalError::InvalidSize));
        assert!(validate_size(24, 80).is_ok());

        let manager = TerminalManager::new();
        assert_eq!(
            manager.write(
                TERMINAL_OWNER,
                "missing",
                &vec![0_u8; MAX_TERMINAL_INPUT_BYTES + 1]
            ),
            Err(TerminalError::InputTooLarge)
        );
    }

    #[test]
    fn managed_worker_authentication_is_visible_ascii_and_round_trips() {
        for _ in 0..32 {
            let authentication =
                TerminalWorkerAuthentication::random().expect("worker authentication");
            let bytes = authentication.copy_secret();
            assert!(bytes.iter().all(|byte| (0x21..=0x7e).contains(byte)));
            let sdk_secret = colossus_sdk::Secret::new(bytes.to_vec()).expect("SDK secret");
            assert_eq!(sdk_secret.expose(), bytes.as_slice());
            let encoded = colossus_sidecar_protocol::encode_worker_authentication(&bytes)
                .expect("encoded worker authentication");
            assert_eq!(
                *colossus_sidecar_protocol::decode_worker_authentication(&encoded)
                    .expect("decoded worker authentication"),
                *bytes
            );
        }
    }

    #[test]
    fn only_the_dedicated_terminal_webview_is_an_owner() {
        assert_eq!(validate_owner("main"), Err(TerminalError::InvalidOwner));
        assert_eq!(
            validate_owner("terminal-pretender"),
            Err(TerminalError::InvalidOwner)
        );
        assert_eq!(validate_owner(TERMINAL_OWNER), Ok(()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn same_path_workspace_replacement_blocks_spawn_and_authentication() {
        use colossus_sidecar_protocol::{
            DESKTOP_TUI_PROTOCOL_VERSION, DesktopTuiChildFrame, DesktopTuiReady, write_frame,
        };
        use std::io::Cursor;

        let root = tempfile::tempdir().expect("workspace parent");
        let workspace = root.path().join("workspace");
        let moved = root.path().join("workspace-moved");
        fs::create_dir(&workspace).expect("workspace");
        let workspace = fs::canonicalize(workspace).expect("canonical workspace");
        let expected = test_workspace_identity(&workspace);
        let binding =
            BoundTerminalWorkspace::open(&workspace, &expected).expect("workspace binding");

        fs::rename(&workspace, &moved).expect("move workspace");
        fs::create_dir(&workspace).expect("replacement workspace");

        let terminal_workspace = TerminalWorkspace {
            id: "workspace:test".into(),
            display_name: "Test".into(),
            workspace: workspace.clone(),
            workspace_identity: expected.clone(),
            config: None,
            worker_authentication: None,
        };
        let manager = TerminalManager::new();
        assert_eq!(
            manager.open(
                TERMINAL_OWNER,
                &terminal_workspace,
                TerminalKind::ColossusTui,
                24,
                80,
                Arc::new(|_| true),
            ),
            Err(TerminalError::InvalidWorkspace),
            "workspace rejection must happen before command validation or process spawn"
        );
        assert_eq!(binding.revalidate(), Err(TerminalError::InvalidWorkspace));
        assert_eq!(
            BoundTerminalWorkspace::open(&workspace, &expected).err(),
            Some(TerminalError::InvalidWorkspace)
        );

        let exchange_id = Uuid::now_v7().to_string();
        let mut ready = Vec::new();
        write_frame(
            &mut ready,
            &DesktopTuiChildFrame::Ready(DesktopTuiReady {
                protocol_version: DESKTOP_TUI_PROTOCOL_VERSION,
                exchange_id,
                workspace_identity: expected,
            }),
        )
        .expect("ready frame");
        let mut reader = Cursor::new(ready);
        let mut released = Vec::new();
        let authentication = TerminalWorkerAuthentication::random().expect("worker authentication");
        assert_eq!(
            exchange_tui_authentication(&mut reader, &mut released, &authentication, &binding,),
            Err(TerminalError::InvalidWorkspace)
        );
        assert!(
            released.is_empty(),
            "worker authentication must not cross the channel after replacement"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn child_workspace_attestation_mismatch_blocks_worker_authentication() {
        use colossus_sidecar_protocol::{
            DESKTOP_TUI_PROTOCOL_VERSION, DesktopTuiChildFrame, DesktopTuiReady, write_frame,
        };
        use std::io::Cursor;

        let workspace = tempfile::tempdir().expect("workspace");
        let workspace = fs::canonicalize(workspace.path()).expect("canonical workspace");
        let expected = test_workspace_identity(&workspace);
        let binding =
            BoundTerminalWorkspace::open(&workspace, &expected).expect("workspace binding");
        let child_identity = WorkspaceIdentity::from_macos_parts(42, 84, 1_700_000_000, 0)
            .expect("different child identity");
        assert_ne!(child_identity, expected);

        let mut ready = Vec::new();
        write_frame(
            &mut ready,
            &DesktopTuiChildFrame::Ready(DesktopTuiReady {
                protocol_version: DESKTOP_TUI_PROTOCOL_VERSION,
                exchange_id: Uuid::now_v7().to_string(),
                workspace_identity: child_identity,
            }),
        )
        .expect("ready frame");
        let mut released = Vec::new();
        let authentication = TerminalWorkerAuthentication::random().expect("worker authentication");

        assert_eq!(
            exchange_tui_authentication(
                &mut Cursor::new(ready),
                &mut released,
                &authentication,
                &binding,
            ),
            Err(TerminalError::InvalidWorkspace)
        );
        assert!(
            released.is_empty(),
            "a child bound to another workspace must receive no worker key"
        );
    }

    #[test]
    fn osc_controls_are_removed_even_across_chunks() {
        let mut sanitizer = OscSanitizer::default();
        let mut released = sanitizer.filter(b"safe\x1b]52;c;secret");
        released.extend(sanitizer.filter(b"\x07after\x1b]8;;https://example.com\x1b"));
        released.extend(sanitizer.filter(b"\\link\x1b]8;;\x1b\\ done"));
        assert_eq!(released, b"safeafterlink done");
    }

    #[test]
    fn repeated_escape_cannot_hide_an_osc_introducer_even_across_chunks() {
        let mut sanitizer = OscSanitizer::default();
        let mut released = sanitizer.filter(b"before\x1b");
        released.extend(sanitizer.filter(b"\x1b"));
        released.extend(sanitizer.filter(b"]52;c;secret"));
        released.extend(sanitizer.filter(b"\x07after"));
        assert_eq!(released, b"beforeafter");
    }

    #[test]
    fn utf8_c1_osc_and_st_are_removed_even_across_chunks() {
        let mut sanitizer = OscSanitizer::default();
        let mut released = sanitizer.filter(b"safe\xc2");
        released.extend(sanitizer.filter(b"\x9d52;c;secret\xc2"));
        released.extend(sanitizer.filter(b"\x9cafter\xc2"));
        released.extend(sanitizer.filter(b"\x9ctail"));
        assert_eq!(released, b"safeaftertail");
    }

    #[test]
    fn normal_ansi_sequences_are_preserved() {
        let mut sanitizer = OscSanitizer::default();
        let mut released = sanitizer.filter(b"\x1b[31mred\x1b[0m price \xc2");
        released.extend(sanitizer.filter(b"\xa2 caf\xc3\xa9 \xf0\x9f\x98\x80"));
        assert_eq!(
            released,
            b"\x1b[31mred\x1b[0m price \xc2\xa2 caf\xc3\xa9 \xf0\x9f\x98\x80"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tui_command_is_fixed_and_requires_native_config() {
        let directory = tempfile::tempdir().expect("terminal context");
        let cli = fs::canonicalize(std::env::current_exe().expect("test executable"))
            .expect("canonical CLI fixture");
        let config = directory.path().join("managed.yaml");
        fs::write(&config, b"schema_version: 1\n").expect("write config");
        let config = fs::canonicalize(config).expect("canonical config");
        let workspace = fs::canonicalize(directory.path()).expect("canonical workspace");
        let workspace_identity = test_workspace_identity(&workspace);

        let manager = TerminalManager::new();
        let cli_sha256 = sha256_file(&cli).expect("CLI digest");
        let mut invalid_sha256 = cli_sha256;
        invalid_sha256[0] ^= 0xff;
        let signing = colossus_sdk::MacosCodeSigningRequirement::AppleTeam;
        assert_eq!(
            manager.set_verified_colossus_cli(&cli, invalid_sha256, signing),
            Err(TerminalError::ProgramUnavailable)
        );
        manager
            .set_verified_colossus_cli(&cli, cli_sha256, signing)
            .expect("verified CLI path");
        let terminal_workspace = TerminalWorkspace {
            id: "workspace:test".into(),
            display_name: "Test".into(),
            workspace: workspace.clone(),
            workspace_identity,
            config: Some(config.clone()),
            worker_authentication: Some(
                TerminalWorkerAuthentication::random().expect("worker authentication"),
            ),
        };
        let (program, arguments) = manager
            .command(TerminalKind::ColossusTui, &terminal_workspace, &workspace)
            .expect("fixed TUI command");
        assert_eq!(program, cli);
        assert_eq!(
            arguments,
            vec![
                PathBuf::from("--workspace"),
                workspace,
                PathBuf::from("--config"),
                config,
                PathBuf::from("--worker-required"),
                PathBuf::from("--desktop-worker-auth"),
                PathBuf::from("tui"),
            ]
        );

        let missing_config = TerminalWorkspace {
            config: None,
            ..terminal_workspace.clone()
        };
        assert_eq!(
            manager.command(
                TerminalKind::ColossusTui,
                &missing_config,
                &missing_config.workspace
            ),
            Err(TerminalError::InvalidConfiguration)
        );

        let missing_authentication = TerminalWorkspace {
            worker_authentication: None,
            ..terminal_workspace.clone()
        };
        assert_eq!(
            manager.command(
                TerminalKind::ColossusTui,
                &missing_authentication,
                &missing_authentication.workspace
            ),
            Err(TerminalError::InvalidConfiguration)
        );

        let debug = format!("{terminal_workspace:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(&terminal_workspace.workspace_identity.sha256));
        assert!(
            !debug.contains(&hex::encode(
                *terminal_workspace
                    .worker_authentication
                    .as_ref()
                    .expect("authentication")
                    .copy_secret()
            ))
        );

        manager
            .inner
            .colossus_cli
            .write()
            .expect("verified CLI")
            .as_mut()
            .expect("verified CLI")
            .sha256[0] ^= 0xff;
        assert_eq!(
            manager.command(
                TerminalKind::ColossusTui,
                &terminal_workspace,
                &terminal_workspace.workspace
            ),
            Err(TerminalError::ProgramUnavailable)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shell_command_is_fixed_native_and_has_no_worker_authority() {
        use std::ffi::OsStr;

        let workspace = tempfile::tempdir().expect("shell workspace");
        let workspace = fs::canonicalize(workspace.path()).expect("canonical workspace");
        let terminal_workspace = TerminalWorkspace {
            id: "workspace:shell".into(),
            display_name: "Shell workspace".into(),
            workspace: workspace.clone(),
            workspace_identity: test_workspace_identity(&workspace),
            config: None,
            worker_authentication: None,
        };
        let manager = TerminalManager::new();
        let (program, arguments) = manager
            .command(TerminalKind::Shell, &terminal_workspace, &workspace)
            .expect("fixed shell command");
        assert_eq!(program, PathBuf::from(MACOS_SYSTEM_SHELL));
        assert_eq!(arguments, [PathBuf::from("-l")]);

        let command =
            shell_command(&program, &arguments, &workspace).expect("native shell builder");
        assert_eq!(
            command.get_argv(),
            &[
                std::ffi::OsString::from(MACOS_SYSTEM_SHELL),
                std::ffi::OsString::from("-l"),
            ]
        );
        assert_eq!(command.get_cwd(), Some(&workspace.as_os_str().to_owned()));
        assert_eq!(
            command.get_env("PATH"),
            Some(OsStr::new(
                "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
            ))
        );
        assert_eq!(
            command.get_env("SHELL"),
            Some(OsStr::new(MACOS_SYSTEM_SHELL))
        );

        let privileged = TerminalWorkspace {
            config: Some(workspace.join("managed.yaml")),
            ..terminal_workspace
        };
        assert_eq!(
            manager.command(TerminalKind::Shell, &privileged, &workspace),
            Err(TerminalError::InvalidConfiguration)
        );
    }
}
