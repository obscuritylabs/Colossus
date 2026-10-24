use crate::{
    native::{self, Control, Workspace},
    output::{OscFilter, Output},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_api::{
    ApiError, ApiErrorCode, ApiErrorReason, ApiResult, HumanTerminalFrameKind,
    HumanTerminalOperation as Op, HumanTerminalReply as Reply, HumanTerminalRequest,
    HumanTerminalSession, HumanTerminalStatus, OutcomeCertainty,
};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read as _, Write as _},
    path::Path,
    sync::{Arc, Mutex, Weak, mpsc},
    time::{Duration, Instant},
};
use tokio::sync::{Notify, Semaphore, oneshot};

const MAX_SESSIONS: usize = 8;
const MAX_OWNER_SESSIONS: usize = 4;
const DETACHED_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);
const RETAIN_EXITED: Duration = Duration::from_secs(60);

/// Process-local supervisor for human terminals beneath explicit host opt-in.
/// Browser and connector disconnects do not own its lifetime. Restart ends every shell.
#[derive(Clone)]
pub struct HumanTerminalService(Arc<Inner>);
struct Inner {
    workspace: Workspace,
    incarnation: String,
    sessions: Mutex<BTreeMap<String, Arc<Session>>>,
    reads: Arc<Semaphore>,
    controls: Arc<Semaphore>,
}
struct Session {
    application: String,
    owner: String,
    request_id: String,
    initial_size: (u16, u16),
    state: Mutex<State>,
    control: Arc<dyn Control>,
    input: mpsc::SyncSender<Input>,
    changed: Notify,
}
struct State {
    summary: HumanTerminalSession,
    output: Output,
    last_input: u64,
    last_digest: Option<[u8; 32]>,
    pending: Option<(u64, [u8; 32])>,
    created: Instant,
    touched: Instant,
}
struct Input {
    sequence: u64,
    digest: [u8; 32],
    data: Vec<u8>,
    answer: oneshot::Sender<ApiResult<()>>,
}

impl HumanTerminalService {
    /// Bind one native-owned canonical workspace. No request can select another path.
    pub fn new(workspace: &Path) -> ApiResult<Self> {
        Ok(Self(Arc::new(Inner {
            workspace: Workspace::new(workspace)?,
            incarnation: uuid::Uuid::new_v4().simple().to_string(),
            sessions: Mutex::new(BTreeMap::new()),
            reads: Arc::new(Semaphore::new(8)),
            controls: Arc::new(Semaphore::new(8)),
        })))
    }

    /// Execute an authenticated application's closed request. The host must enforce
    /// `terminals:control` before calling; owner partitions remain beneath that app.
    pub async fn execute(
        &self,
        application: &str,
        request: HumanTerminalRequest,
    ) -> ApiResult<Reply> {
        if !request.validate() {
            return Err(invalid());
        }
        // Quiet long polls cannot consume cancellation, input or close admission.
        let admission = if matches!(&request.operation, Op::Read { .. }) {
            &self.0.reads
        } else {
            &self.0.controls
        };
        let permit = admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| capacity())?;
        let HumanTerminalRequest { owner, operation } = request;
        if matches!(operation, Op::Context | Op::Open { .. }) {
            let service = self.clone();
            let application = application.to_owned();
            return tokio::task::spawn_blocking(move || {
                let _permit = permit;
                service.admit(&application, &owner, operation)
            })
            .await
            .map_err(|_| native::failure())?;
        }
        let id = terminal_id(&operation).ok_or_else(invalid)?;
        let session = self.session(application, &owner, id)?;
        match operation {
            Op::Read {
                after_sequence,
                max_bytes,
                wait_ms,
                ..
            } => {
                let _permit = permit;
                let changed = session.changed.notified();
                let reply = self.read(&session, after_sequence, max_bytes as usize)?;
                if wait_ms != 0
                    && reply.frames.is_empty()
                    && !reply.gap
                    && reply
                        .session
                        .as_ref()
                        .is_some_and(|s| s.status == HumanTerminalStatus::Running)
                {
                    let _ =
                        tokio::time::timeout(Duration::from_millis(u64::from(wait_ms)), changed)
                            .await;
                    self.read(&session, after_sequence, max_bytes as usize)
                } else {
                    Ok(reply)
                }
            }
            Op::Write {
                writer_epoch,
                input_sequence,
                data_base64,
                ..
            } => {
                let _permit = permit;
                self.write(&session, writer_epoch, input_sequence, &data_base64)
                    .await
            }
            operation => {
                let service = self.clone();
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    service.control(&session, operation)
                })
                .await
                .map_err(|_| native::failure())?
            }
        }
    }

    fn base(&self) -> Reply {
        Reply {
            available: true,
            incarnation: self.0.incarnation.clone(),
            ..Reply::default()
        }
    }
    fn session(&self, application: &str, owner: &str, id: &str) -> ApiResult<Arc<Session>> {
        self.0
            .sessions
            .lock()
            .map_err(|_| native::failure())?
            .get(id)
            .filter(|s| s.application == application && s.owner == owner)
            .cloned()
            .ok_or_else(absent)
    }
    fn admit(&self, application: &str, owner: &str, operation: Op) -> ApiResult<Reply> {
        let mut sessions = self.0.sessions.lock().map_err(|_| native::failure())?;
        sessions.retain(|_, s| {
            s.state.lock().is_ok_and(|v| {
                v.summary.status == HumanTerminalStatus::Running
                    || v.touched.elapsed() < RETAIN_EXITED
            })
        });
        if let Op::Open {
            request_id,
            controller_id,
            rows,
            cols,
        } = operation
        {
            if let Some(existing) = sessions.values().find(|s| {
                s.application == application && s.owner == owner && s.request_id == request_id
            }) {
                if existing.initial_size != (rows, cols)
                    || existing
                        .state
                        .lock()
                        .map_err(|_| native::failure())?
                        .summary
                        .controller_id
                        != controller_id
                {
                    return Err(conflict());
                }
                return self.snapshot(existing);
            }
            if sessions.len() >= MAX_SESSIONS
                || sessions
                    .values()
                    .filter(|s| s.application == application && s.owner == owner)
                    .count()
                    >= MAX_OWNER_SESSIONS
            {
                return Err(capacity());
            }
            let spawned = self.0.workspace.spawn(rows, cols)?;
            let id = uuid::Uuid::new_v4().simple().to_string();
            let now = Instant::now();
            let (sender, receiver) = mpsc::sync_channel(1);
            let session = Arc::new(Session {
                application: application.into(),
                owner: owner.into(),
                request_id,
                initial_size: (rows, cols),
                state: Mutex::new(State {
                    summary: HumanTerminalSession {
                        terminal_id: id.clone(),
                        writer_epoch: 1,
                        controller_id,
                        rows,
                        cols,
                        status: HumanTerminalStatus::Running,
                        first_sequence: 1,
                        next_sequence: 1,
                        complete_history: true,
                        exit_code: None,
                    },
                    output: Output::new(rows, cols),
                    last_input: 0,
                    last_digest: None,
                    pending: None,
                    created: now,
                    touched: now,
                }),
                control: spawned.control,
                input: sender,
                changed: Notify::new(),
            });
            sessions.insert(id.clone(), session.clone());
            let weak = Arc::downgrade(&session);
            if spawn_io(
                &session,
                spawned.reader,
                spawned.writer,
                spawned.wait,
                receiver,
            )
            .is_err()
            {
                session.control.close();
                sessions.remove(&id);
                return Err(native::failure());
            }
            if std::thread::Builder::new()
                .name("human-terminal-lease".into())
                .spawn(move || supervise(weak))
                .is_err()
            {
                session.control.close();
                sessions.remove(&id);
                return Err(native::failure());
            }
            return self.snapshot(&session);
        }
        let mut reply = self.base();
        for session in sessions
            .values()
            .filter(|s| s.application == application && s.owner == owner)
        {
            let state = session.state.lock().map_err(|_| native::failure())?;
            reply.sessions.push(summary(&state));
        }
        Ok(reply)
    }
    fn snapshot(&self, session: &Session) -> ApiResult<Reply> {
        let state = session.state.lock().map_err(|_| native::failure())?;
        let mut reply = self.base();
        reply.session = Some(summary(&state));
        reply.next_sequence = state.output.sequence;
        reply.input_sequence = Some(state.last_input);
        Ok(reply)
    }
    fn read(&self, session: &Session, after: u64, max: usize) -> ApiResult<Reply> {
        let mut state = session.state.lock().map_err(|_| native::failure())?;
        if state.summary.status == HumanTerminalStatus::Running {
            state.touched = Instant::now();
        }
        let (frames, next_sequence, gap) = state.output.read(after, max);
        let mut reply = self.base();
        reply.session = Some(summary(&state));
        reply.frames = frames;
        reply.next_sequence = next_sequence;
        reply.gap = gap;
        reply.input_sequence = Some(state.last_input);
        Ok(reply)
    }
    async fn write(
        &self,
        session: &Arc<Session>,
        epoch: u64,
        sequence: u64,
        encoded: &str,
    ) -> ApiResult<Reply> {
        let data = STANDARD.decode(encoded).map_err(|_| invalid())?;
        if data.is_empty() || data.len() > 64 * 1024 {
            return Err(invalid());
        }
        let digest: [u8; 32] = Sha256::digest(&data).into();
        let (answer, response) = oneshot::channel();
        {
            let mut state = session.state.lock().map_err(|_| native::failure())?;
            writer(&state, epoch)?;
            if sequence == state.last_input {
                if state.last_digest != Some(digest) {
                    return Err(conflict());
                }
                drop(state);
                return self.snapshot(session);
            }
            if state.pending.is_some() || sequence != state.last_input + 1 {
                return Err(conflict());
            }
            state.pending = Some((sequence, digest));
            if session
                .input
                .try_send(Input {
                    sequence,
                    digest,
                    data,
                    answer,
                })
                .is_err()
            {
                state.pending = None;
                return Err(capacity());
            }
            state.touched = Instant::now();
        }
        match tokio::time::timeout(Duration::from_secs(2), response).await {
            Ok(Ok(result)) => {
                result?;
                self.snapshot(session)
            }
            _ => Err(unknown()),
        }
    }
    fn control(&self, session: &Session, operation: Op) -> ApiResult<Reply> {
        let mut state = session.state.lock().map_err(|_| native::failure())?;
        match operation {
            Op::Attach {
                controller_id,
                takeover,
                ..
            } => {
                if state.summary.status != HumanTerminalStatus::Running {
                    drop(state);
                    return self.snapshot(session);
                }
                if state.summary.controller_id != controller_id {
                    if !takeover && state.touched.elapsed() < DETACHED_TIMEOUT {
                        return Err(conflict());
                    }
                    if state.pending.is_some() {
                        return Err(conflict());
                    }
                    state.summary.writer_epoch = state
                        .summary
                        .writer_epoch
                        .checked_add(1)
                        .ok_or_else(conflict)?;
                    state.summary.controller_id = controller_id;
                    state.last_input = 0;
                    state.last_digest = None;
                }
                state.touched = Instant::now();
            }
            Op::Resize {
                writer_epoch,
                rows,
                cols,
                ..
            } => {
                writer(&state, writer_epoch)?;
                session.control.resize(rows, cols)?;
                state.summary.rows = rows;
                state.summary.cols = cols;
                state
                    .output
                    .push(HumanTerminalFrameKind::Resize { rows, cols });
                state.touched = Instant::now();
            }
            Op::Signal {
                writer_epoch,
                signal,
                ..
            } => {
                writer(&state, writer_epoch)?;
                session.control.signal(signal)?;
                state.touched = Instant::now();
            }
            Op::Close { writer_epoch, .. } => {
                if writer_epoch != state.summary.writer_epoch {
                    return Err(conflict());
                }
                session.control.close();
                if state.summary.status == HumanTerminalStatus::Running {
                    state.summary.status = HumanTerminalStatus::Interrupted;
                }
                state.touched = Instant::now();
            }
            _ => return Err(invalid()),
        }
        drop(state);
        session.changed.notify_waiters();
        self.snapshot(session)
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(sessions) = self.sessions.lock() {
            for session in sessions.values() {
                session.control.close();
            }
        }
    }
}

fn spawn_io(
    session: &Arc<Session>,
    mut reader: Box<dyn std::io::Read + Send>,
    mut writer: Box<dyn std::io::Write + Send>,
    wait: Box<dyn FnOnce() -> Option<u32> + Send>,
    receiver: mpsc::Receiver<Input>,
) -> std::io::Result<()> {
    // Retain the reaper outside the spawn closure until thread creation succeeds.
    // A failed thread allocation must still kill and reap the already-started child.
    let waiter = Arc::new(Mutex::new(Some(wait)));
    let weak = Arc::downgrade(session);
    let input_thread = std::thread::Builder::new()
        .name("human-terminal-input".into())
        .spawn(move || {
            while let Ok(input) = receiver.recv() {
                let Some(session) = weak.upgrade() else {
                    break;
                };
                let result = writer.write_all(&input.data).map_err(|_| unknown());
                if let Ok(mut state) = session.state.lock() {
                    if result.is_ok() {
                        state.last_input = input.sequence;
                        state.last_digest = Some(input.digest);
                    } else {
                        state.summary.status = HumanTerminalStatus::Interrupted;
                        session.control.close();
                    }
                    state.pending = None;
                }
                session.changed.notify_waiters();
                let failed = result.is_err();
                let _ = input.answer.send(result);
                if failed {
                    break;
                }
            }
        });
    if let Err(error) = input_thread {
        cleanup_failed_start(session, &waiter);
        return Err(error);
    }
    let output_session = session.clone();
    let output_waiter = waiter.clone();
    let output_thread = std::thread::Builder::new()
        .name("human-terminal-output".into())
        .spawn(move || {
            let session = output_session;
            let mut buffer = [0u8; 8192];
            let mut filter = OscFilter::default();
            let mut failed = false;
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        let output = filter.filter(&buffer[..count]);
                        if let Ok(mut state) = session.state.lock() {
                            state.output.bytes(&output);
                        } else {
                            failed = true;
                            break;
                        }
                        session.changed.notify_waiters();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    #[cfg(unix)]
                    Err(error) if error.raw_os_error() == Some(5) => break,
                    Err(_) if session.control.leader_exited() => break,
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
            // Keep the direct child unreaped until process-group cleanup pins its ID.
            session.control.close();
            let exit_code = output_waiter
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
                .and_then(|wait| wait());
            if let Ok(mut state) = session.state.lock() {
                state.summary.status = if !failed && exit_code.is_some() {
                    HumanTerminalStatus::Exited
                } else {
                    HumanTerminalStatus::Interrupted
                };
                state.summary.exit_code = exit_code;
                state.touched = Instant::now();
            }
            session.changed.notify_waiters();
        });
    if let Err(error) = output_thread {
        cleanup_failed_start(session, &waiter);
        return Err(error);
    }
    Ok(())
}
type Waiter = Arc<Mutex<Option<Box<dyn FnOnce() -> Option<u32> + Send>>>>;
fn cleanup_failed_start(session: &Session, waiter: &Waiter) {
    session.control.close();
    if let Some(wait) = waiter.lock().ok().and_then(|mut slot| slot.take()) {
        let _ = wait();
    }
}
fn supervise(session: Weak<Session>) {
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let Some(session) = session.upgrade() else {
            break;
        };
        // A background job can retain the PTY after its shell exits. Observe the
        // leader independently of output EOF, preserving its unreaped Unix identity.
        if session.control.leader_exited() {
            session.control.close();
            session.changed.notify_waiters();
            break;
        }
        let expired = session.state.lock().is_ok_and(|s| {
            s.summary.status == HumanTerminalStatus::Running
                && (s.touched.elapsed() >= DETACHED_TIMEOUT || s.created.elapsed() >= MAX_LIFETIME)
        });
        if expired {
            session.control.close();
            if let Ok(mut state) = session.state.lock() {
                state.summary.status = HumanTerminalStatus::Interrupted;
                state.touched = Instant::now();
            }
            session.changed.notify_waiters();
            break;
        }
        if session
            .state
            .lock()
            .is_ok_and(|s| s.summary.status != HumanTerminalStatus::Running)
        {
            break;
        }
    }
}
fn summary(state: &State) -> HumanTerminalSession {
    let mut value = state.summary.clone();
    value.first_sequence = state.output.first();
    value.next_sequence = state.output.sequence;
    value.complete_history = value.first_sequence == 1;
    value
}
fn writer(state: &State, epoch: u64) -> ApiResult<()> {
    if state.summary.writer_epoch != epoch || state.summary.status != HumanTerminalStatus::Running {
        Err(conflict())
    } else {
        Ok(())
    }
}
fn terminal_id(op: &Op) -> Option<&str> {
    match op {
        Op::Attach { terminal_id, .. }
        | Op::Read { terminal_id, .. }
        | Op::Write { terminal_id, .. }
        | Op::Resize { terminal_id, .. }
        | Op::Signal { terminal_id, .. }
        | Op::Close { terminal_id, .. } => Some(terminal_id),
        _ => None,
    }
}
fn invalid() -> ApiError {
    ApiError::invalid(
        ApiErrorReason::InvalidArgument,
        "terminal",
        "terminal fields are invalid or exceed their bounds",
    )
}
fn absent() -> ApiError {
    ApiError::not_found(ApiErrorReason::RunNotFound, "the terminal is unavailable")
}
fn conflict() -> ApiError {
    ApiError::conflict(
        ApiErrorReason::ConcurrentModification,
        "the terminal controller or input sequence changed; attach again",
    )
}
fn capacity() -> ApiError {
    ApiError::resource_exhausted(
        ApiErrorReason::CapacityExceeded,
        "the terminal capacity is active",
    )
}
fn unknown() -> ApiError {
    ApiError {
        code: ApiErrorCode::OutcomeUnknown,
        reason: ApiErrorReason::OutcomeUnknown,
        message: "terminal input delivery is uncertain; do not resend automatically".into(),
        correlation_id: None,
        retryable: false,
        outcome: OutcomeCertainty::Unknown,
        violations: Vec::new(),
    }
}

#[cfg(test)]
#[path = "service_cancellation_tests.rs"]
mod cancellation_tests;
