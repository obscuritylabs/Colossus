//! Persistent native custody with disposable bounded read workers.
use super::{
    Error,
    identity::{self, Window},
    indicator::{self, Indicator},
    lease::Writer,
    read_worker::ReadWorker,
};
use crate::{
    protocol::{self, Bootstrap, ReadQuery, Reply, Request, Response, Role},
    wire,
};
use colossus_contracts::*;
use colossus_ports::{ComputerCommand, ComputerDriverRequest};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId},
};

pub(crate) fn entry() -> Result<(), Error> {
    let (bootstrap, binary) =
        wire::read::<Bootstrap>(&mut std::io::stdin()).map_err(|_| Error::InvalidInput)?;
    if bootstrap.version != protocol::VERSION
        || !binary.is_empty()
        || bootstrap.nonce.len() != 37
        || !bootstrap.nonce.starts_with("cu_p_")
        || !bootstrap.nonce.as_bytes()[5..]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
        || bootstrap.context.protected.is_empty()
        || bootstrap.context.protected.len() > 32
        || bootstrap.binding.validate().is_err()
    {
        return Err(Error::Denied);
    }
    identity::ordinary_host()?;
    // SAFETY: this helper process exclusively owns its DPI context before creating any window.
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        )
    };
    let mut output = std::io::stdout();
    wire::write(
        &mut output,
        &Response {
            nonce: bootstrap.nonce.clone(),
            sequence: 0,
            result: Ok(Reply::Ready),
        },
        &[],
    )
    .map_err(|_| Error::Failed)?;
    match &bootstrap.role {
        Role::Read { window, query } => {
            #[cfg(feature = "native-test-driver")]
            eprintln!("COMPUTER_NATIVE_READ phase=started");
            let result = match query {
                ReadQuery::Snapshot { max_nodes } => {
                    super::semantics::snapshot(window, &bootstrap.context, *max_nodes)
                        .map(|nodes| (Reply::Nodes(nodes), Vec::new()))
                }
                ReadQuery::Screenshot {
                    max_bytes,
                    max_pixels,
                } => super::capture::png(window, &bootstrap.context, *max_bytes, *max_pixels)
                    .map(|png| (Reply::Pixels, png)),
                ReadQuery::ProtectedFocus => {
                    super::semantics::protected_focus(window, &bootstrap.context)
                        .map(|value| (Reply::ProtectedFocus(value), Vec::new()))
                }
            };
            let (result, binary) = match result {
                Ok((meta, bytes)) => (Ok(meta), bytes),
                Err(error) => (Err(error), Vec::new()),
            };
            wire::write(
                &mut output,
                &Response {
                    nonce: bootstrap.nonce,
                    sequence: 1,
                    result,
                },
                &binary,
            )
            .map_err(|_| Error::Failed)
        }
        Role::Broker => run(bootstrap),
    }
}

struct Observation {
    id: ComputerObservationId,
    window: Window,
    allowed: Vec<(ComputerWindowId, Window)>,
    nodes: Vec<ComputerNode>,
    created: Instant,
    human: u64,
    native_epoch: u64,
    foreground: Option<(u64, crate::NativeProcessIdentity)>,
}
struct Session {
    summary: ComputerSessionSummary,
    root: Window,
    limits: ComputerLimits,
    started: Instant,
    writer: Option<Writer>,
    indicator: Option<Indicator>,
    observation: Option<Observation>,
    human: u64,
}
struct Selection {
    window_id: ComputerWindowId,
    window: Window,
    allowed: Vec<(ComputerWindowId, Window)>,
}
struct Broker {
    bootstrap: Bootstrap,
    windows: HashMap<ComputerWindowId, Remembered>,
    lifetimes: crate::lifetime::Tracker,
    session: Option<Session>,
    blocked: bool,
    cancel: Arc<AtomicBool>,
    worker: Option<ReadWorker>,
}
#[derive(Clone)]
struct Remembered {
    window: Window,
    life: crate::lifetime::Stamp,
}
struct ObservedWindow {
    window: Window,
    title: String,
    life: crate::lifetime::Stamp,
}

fn run(bootstrap: Bootstrap) -> Result<(), Error> {
    let cancel = Arc::new(AtomicBool::new(false));
    let read_cancel = Arc::clone(&cancel);
    let (sender, receiver) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        let mut input = std::io::stdin();
        loop {
            match wire::read::<Request>(&mut input) {
                Ok((request, binary)) if binary.is_empty() => {
                    if request.request.is_none() {
                        read_cancel.store(true, Ordering::Release);
                    }
                    if sender.send(request).is_err() {
                        break;
                    }
                }
                _ => {
                    read_cancel.store(true, Ordering::Release);
                    break;
                }
            }
        }
    });
    let mut broker = Broker {
        bootstrap,
        windows: HashMap::new(),
        lifetimes: crate::lifetime::Tracker::new()?,
        session: None,
        blocked: false,
        cancel,
        worker: None,
    };
    let mut sequence = 0u64;
    let started = Instant::now();
    let mut output = std::io::stdout();
    loop {
        broker.refresh();
        if started.elapsed() > Duration::from_secs(20 * 60) {
            break;
        }
        let request = match receiver.recv_timeout(Duration::from_millis(20)) {
            Ok(v) => v,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if broker.cancel.load(Ordering::Acquire) {
                    break;
                } else {
                    continue;
                }
            }
            Err(_) => break,
        };
        if request.nonce != broker.bootstrap.nonce
            || Some(request.sequence) != sequence.checked_add(1)
        {
            return Err(Error::Denied);
        }
        sequence = request.sequence;
        let stop = request.request.is_none();
        let result = if let Some(request) = request.request {
            if request.binding != broker.bootstrap.binding || request.validate().is_err() {
                Err(Error::Denied)
            } else {
                broker.execute(request)
            }
        } else {
            broker.session = None;
            Ok((Reply::Closed, Vec::new()))
        };
        let (result, binary) = match result {
            Ok((meta, bytes)) => (Ok(meta), bytes),
            Err(error) => (Err(error), Vec::new()),
        };
        wire::write(
            &mut output,
            &Response {
                nonce: broker.bootstrap.nonce.clone(),
                sequence,
                result,
            },
            &binary,
        )
        .map_err(|_| Error::Failed)?;
        if stop {
            break;
        }
    }
    Ok(())
}

impl Broker {
    fn refresh(&mut self) {
        indicator::pump();
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let mut transition = None;
        if self.cancel.load(Ordering::Acquire) {
            transition = Some((ComputerControlState::Stopped, ComputerStopReason::Disabled));
        } else if session.started.elapsed()
            >= Duration::from_millis(u64::from(session.limits.session_timeout_ms))
        {
            transition = Some((ComputerControlState::Stopped, ComputerStopReason::Expired));
        } else if identity::desktop().is_err() {
            transition = Some((
                ComputerControlState::Stopped,
                ComputerStopReason::DesktopUnavailable,
            ));
        } else if identity::process(session.root.process.pid).ok().as_ref()
            != Some(&session.root.process)
        {
            transition = Some((
                ComputerControlState::Stopped,
                ComputerStopReason::WindowClosed,
            ));
        }
        if let Some(card) = session.indicator.as_ref() {
            let action = card.signals.action.swap(0, Ordering::AcqRel);
            if action == 4 {
                transition = Some((
                    ComputerControlState::Stopped,
                    ComputerStopReason::WindowClosed,
                ));
            } else if action == 2 {
                transition = Some((
                    ComputerControlState::Stopped,
                    ComputerStopReason::UserStopped,
                ));
            } else if action == 1 && session.summary.state != ComputerControlState::Stopped {
                if session.summary.state == ComputerControlState::Paused {
                    session.summary.state = ComputerControlState::Active;
                    session.summary.reason = None;
                    session.summary.control_generation += 1;
                    session.observation = None;
                    session.human = card.signals.human.load(Ordering::Acquire);
                    card.status(false);
                } else {
                    transition =
                        Some((ComputerControlState::Paused, ComputerStopReason::UserPaused));
                }
            } else if session.summary.state == ComputerControlState::Active
                && card.signals.human.load(Ordering::Acquire) != session.human
            {
                transition = Some((
                    ComputerControlState::Paused,
                    ComputerStopReason::UserTakeover,
                ));
            }
        }
        if let Some((state, reason)) = transition {
            if session.summary.state != state || session.summary.reason != Some(reason) {
                session.summary.control_generation += 1;
            }
            session.summary.state = state;
            session.summary.reason = Some(reason);
            session.observation = None;
            if state == ComputerControlState::Stopped {
                self.blocked = true;
                session.indicator = None;
                session.writer = None;
            } else if let Some(card) = &session.indicator {
                card.status(true);
            }
        }
    }
    fn active(&self, id: &ComputerSessionId, generation: u64) -> Result<&Session, Error> {
        let session = self
            .session
            .as_ref()
            .filter(|s| &s.summary.session_id == id)
            .ok_or(Error::Stale)?;
        if session.summary.state != ComputerControlState::Active
            || session.summary.control_generation != generation
            || self.cancel.load(Ordering::Acquire)
        {
            return Err(Error::Denied);
        }
        Ok(session)
    }
    fn enumerate(&self, max: u16, root: Option<&Window>) -> Result<Vec<ObservedWindow>, Error> {
        identity::desktop()?;
        indicator::pump();
        let mut result = Vec::new();
        for handle in enumerate_handles() {
            let admitted = self.lifetimes.admit(
                handle,
                || {
                    // All facts, including newly discovered modal ownership,
                    // are queried only after this HWND has its lifetime seal.
                    let window =
                        identity::inspect(handle as usize as HWND, &self.bootstrap.context)?;
                    if let Some(root) = root
                        && (window.handle == root.handle || !identity::owned_modal(root, &window))
                    {
                        return Err(Error::Denied);
                    }
                    let title = identity::title(window.hwnd());
                    Ok((window, title))
                },
                indicator::pump,
            );
            match admitted {
                Ok(admitted) => result.push(ObservedWindow {
                    window: admitted.value.0,
                    title: admitted.value.1,
                    life: admitted.stamp,
                }),
                Err(Error::Busy) => return Err(Error::Busy),
                // Unavailable/protected/destroyed candidates grant no identity.
                Err(_) => continue,
            }
            if result.len() == usize::from(max) {
                break;
            }
        }
        Ok(result)
    }
    fn remember(&mut self, observed: &ObservedWindow) -> Result<ComputerWindowId, Error> {
        indicator::pump();
        let window = &observed.window;
        if !self.lifetimes.alive(observed.life, window.handle) {
            return Err(Error::Stale);
        }
        if let Some((id, previous)) = self.windows.iter_mut().find(|(_, w)| {
            w.window.handle == window.handle
                && w.window.process == window.process
                && w.life == observed.life
        }) {
            previous.window = window.clone();
            return Ok(id.clone());
        }
        if self.windows.len() >= 512 {
            return Err(Error::Busy);
        }
        let id = ComputerWindowId::parse(protocol::opaque("cu_w_")?).map_err(|_| Error::Failed)?;
        self.windows.insert(
            id.clone(),
            Remembered {
                window: window.clone(),
                life: observed.life,
            },
        );
        Ok(id)
    }
    fn summaries(
        &mut self,
        windows: Vec<ObservedWindow>,
    ) -> Result<Vec<ComputerWindowSummary>, Error> {
        let mut result = Vec::new();
        for observed in windows {
            let id = self.remember(&observed)?;
            result.push(ComputerWindowSummary {
                window_id: id,
                title: observed.title,
                application_name: "Windows application".into(),
                rect: observed.window.rect,
                foreground: identity::foreground() == observed.window.hwnd(),
                available: true,
                unavailable_reason: None,
            });
        }
        Ok(result)
    }
    fn known_summaries(
        &self,
        windows: &[(ComputerWindowId, Window)],
    ) -> Result<Vec<ComputerWindowSummary>, Error> {
        let mut result = Vec::new();
        for (id, window) in windows {
            indicator::pump();
            let remembered = self.windows.get(id).ok_or(Error::Stale)?;
            if remembered.window.handle != window.handle
                || remembered.window.process != window.process
                || !self.lifetimes.alive(remembered.life, window.handle)
            {
                return Err(Error::Stale);
            }
            window.revalidate(&self.bootstrap.context)?;
            let title = identity::title(window.hwnd());
            indicator::pump();
            if !self.lifetimes.alive(remembered.life, window.handle) {
                return Err(Error::Stale);
            }
            result.push(ComputerWindowSummary {
                window_id: id.clone(),
                title,
                application_name: "Windows application".into(),
                rect: window.rect,
                foreground: identity::foreground() == window.hwnd(),
                available: true,
                unavailable_reason: None,
            });
        }
        Ok(result)
    }
    fn selected(&mut self, id: &ComputerSessionId, generation: u64) -> Result<Selection, Error> {
        let session = self.active(id, generation)?;
        let remembered = self
            .windows
            .get(&session.summary.window_id)
            .ok_or(Error::Stale)?;
        let root_life = (remembered.life, remembered.window.handle);
        if !self.lifetimes.alive(root_life.0, root_life.1) {
            return Err(Error::Stale);
        }
        let root = identity::inspect(session.root.hwnd(), &self.bootstrap.context)?;
        if root.process != session.root.process {
            return Err(Error::Stale);
        }
        let mut allowed = vec![(session.summary.window_id.clone(), root.clone())];
        for candidate in self.enumerate(128, Some(&root))? {
            let candidate_id = self.remember(&candidate)?;
            allowed.push((candidate_id, candidate.window));
        }
        let chosen = allowed
            .iter()
            .find(|(_, w)| w.hwnd() == identity::foreground())
            .cloned()
            .unwrap_or_else(|| allowed[0].clone());
        if !self.lifetimes.alive(root_life.0, root_life.1) {
            return Err(Error::Stale);
        }
        Ok(Selection {
            window_id: chosen.0,
            window: chosen.1,
            allowed,
        })
    }
    fn execute(&mut self, request: ComputerDriverRequest) -> Result<(Reply, Vec<u8>), Error> {
        self.refresh();
        if self.cancel.load(Ordering::Acquire) {
            return Err(Error::Denied);
        }
        match request.command {
            ComputerCommand::Windows { max_windows } => {
                let windows = self.enumerate(max_windows, None)?;
                Ok((Reply::Windows(self.summaries(windows)?), Vec::new()))
            }
            ComputerCommand::Open { window_id, limits } => {
                if self.blocked || self.session.is_some() {
                    return Err(Error::Denied);
                }
                indicator::pump();
                let remembered = self.windows.get(&window_id).cloned().ok_or(Error::Stale)?;
                if !self
                    .lifetimes
                    .alive(remembered.life, remembered.window.handle)
                {
                    return Err(Error::Stale);
                }
                let root = remembered.window;
                root.revalidate(&self.bootstrap.context)?;
                let writer = Writer::acquire()?;
                let indicator = Indicator::create(root.handle)?;
                indicator::pump();
                if !self.lifetimes.alive(remembered.life, root.handle) {
                    return Err(Error::Stale);
                }
                let human = indicator.signals.human.load(Ordering::Acquire);
                let summary = ComputerSessionSummary {
                    session_id: ComputerSessionId::parse(protocol::opaque("cu_s_")?)
                        .map_err(|_| Error::Failed)?,
                    control_generation: 1,
                    window_id,
                    state: ComputerControlState::Active,
                    reason: None,
                };
                self.session = Some(Session {
                    summary: summary.clone(),
                    root,
                    limits,
                    started: Instant::now(),
                    writer: Some(writer),
                    indicator: Some(indicator),
                    observation: None,
                    human,
                });
                Ok((Reply::Opened(summary), Vec::new()))
            }
            ComputerCommand::Status { session_id } => {
                let session = self
                    .session
                    .as_ref()
                    .filter(|s| s.summary.session_id == session_id)
                    .ok_or(Error::Stale)?;
                Ok((Reply::Status(session.summary.clone()), Vec::new()))
            }
            ComputerCommand::Snapshot {
                session_id,
                control_generation,
                max_nodes,
            } => {
                let limits = self.active(&session_id, control_generation)?.limits;
                if max_nodes > limits.max_snapshot_nodes {
                    return Err(Error::InvalidInput);
                }
                let Selection {
                    window_id,
                    window,
                    allowed,
                } = self.selected(&session_id, control_generation)?;
                let native_epoch = self.begin_observation(window.handle)?;
                let (reply, _) = self.read_worker(
                    &window,
                    ReadQuery::Snapshot { max_nodes },
                    Duration::from_millis(u64::from(limits.action_timeout_ms)),
                )?;
                let Reply::Nodes(nodes) = reply else {
                    return Err(Error::Failed);
                };
                let observation_id = ComputerObservationId::parse(protocol::opaque("cu_o_")?)
                    .map_err(|_| Error::Failed)?;
                let windows = self.known_summaries(&allowed)?;
                let observation = ComputerObservation {
                    session_id: session_id.clone(),
                    control_generation,
                    observation_id: observation_id.clone(),
                    window_id: window_id.clone(),
                    width: window.rect.width,
                    height: window.rect.height,
                    scale_milli: window.dpi * 1000 / 96,
                    nodes: nodes.clone(),
                    windows,
                };
                observation.validate(&limits).map_err(|_| Error::Failed)?;
                self.store_observation(
                    &session_id,
                    control_generation,
                    Observation {
                        id: observation_id,
                        window,
                        allowed,
                        nodes,
                        created: Instant::now(),
                        human: 0,
                        native_epoch,
                        foreground: None,
                    },
                )?;
                Ok((Reply::Snapshot(observation), Vec::new()))
            }
            ComputerCommand::Screenshot {
                session_id,
                control_generation,
                max_bytes,
            } => {
                let limits = self.active(&session_id, control_generation)?.limits;
                if max_bytes > limits.max_screenshot_bytes {
                    return Err(Error::InvalidInput);
                }
                let Selection {
                    window_id,
                    window,
                    allowed,
                } = self.selected(&session_id, control_generation)?;
                let native_epoch = self.begin_observation(window.handle)?;
                let (reply, png) = self.read_worker(
                    &window,
                    ReadQuery::Screenshot {
                        max_bytes,
                        max_pixels: limits.max_screenshot_pixels,
                    },
                    Duration::from_millis(u64::from(limits.action_timeout_ms)),
                )?;
                if !matches!(reply, Reply::Pixels) {
                    return Err(Error::Failed);
                }
                let observation_id = ComputerObservationId::parse(protocol::opaque("cu_o_")?)
                    .map_err(|_| Error::Failed)?;
                let descriptor = ComputerScreenshotDescriptor {
                    session_id: session_id.clone(),
                    control_generation,
                    observation_id: observation_id.clone(),
                    window_id: window_id.clone(),
                    width: window.rect.width,
                    height: window.rect.height,
                    size_bytes: png.len() as u32,
                    sha256: format!("{:x}", Sha256::digest(&png)),
                };
                descriptor.validate(&limits).map_err(|_| Error::Failed)?;
                self.store_observation(
                    &session_id,
                    control_generation,
                    Observation {
                        id: observation_id,
                        window,
                        allowed,
                        nodes: Vec::new(),
                        created: Instant::now(),
                        human: 0,
                        native_epoch,
                        foreground: None,
                    },
                )?;
                let root_window_id = self
                    .active(&session_id, control_generation)?
                    .summary
                    .window_id
                    .clone();
                Ok((
                    Reply::Screenshot {
                        descriptor,
                        root_window_id,
                    },
                    png,
                ))
            }
            ComputerCommand::Input {
                session_id,
                control_generation,
                observation_id,
                input,
            } => {
                let session = self.active(&session_id, control_generation)?;
                let age = Duration::from_millis(u64::from(session.limits.observation_max_age_ms));
                let timeout = Duration::from_millis(u64::from(session.limits.action_timeout_ms));
                let root_id = session.summary.window_id.clone();
                let root_identity = (session.root.handle, session.root.process.clone());
                let observation = self
                    .session
                    .as_mut()
                    .and_then(|s| s.observation.take())
                    .ok_or(Error::Stale)?;
                if observation.id != observation_id
                    || observation.created.elapsed() > age
                    || foreground_identity() != observation.foreground
                {
                    return Err(Error::Stale);
                }
                observation.window.revalidate(&self.bootstrap.context)?;
                // Use the current observation's placement, while retaining the
                // originally selected root's process/lifetime identity.
                let root = observation
                    .allowed
                    .iter()
                    .find(|(id, _)| id == &root_id)
                    .map(|(_, w)| w.clone())
                    .ok_or(Error::Stale)?;
                if (root.handle, root.process.clone()) != root_identity {
                    return Err(Error::Stale);
                }
                let target = if let ComputerInput::Focus { window_id } = &input {
                    observation
                        .allowed
                        .iter()
                        .find(|(id, _)| id == window_id)
                        .map(|(_, w)| w.clone())
                        .ok_or(Error::Denied)?
                } else {
                    observation.window.clone()
                };
                let root_record = self.windows.get(&root_id).ok_or(Error::Stale)?;
                let target_record = observation
                    .allowed
                    .iter()
                    .find(|(_, w)| w == &target)
                    .and_then(|(id, _)| self.windows.get(id))
                    .ok_or(Error::Stale)?;
                let lifetime = self.lifetimes.fence(
                    (root_record.life, root.handle),
                    (target_record.life, target.handle),
                );
                target.revalidate(&self.bootstrap.context)?;
                let signals = Arc::clone(
                    &self
                        .active(&session_id, control_generation)?
                        .indicator
                        .as_ref()
                        .ok_or(Error::Denied)?
                        .signals,
                );
                if signals.human.load(Ordering::Acquire) != observation.human {
                    return Err(Error::Denied);
                }
                if signals.native_epoch.load(Ordering::Acquire) != observation.native_epoch {
                    return Err(Error::Stale);
                }
                let context = self.bootstrap.context.clone();
                let cancel = Arc::clone(&self.cancel);
                let deadline = Instant::now() + timeout;
                super::input::apply(
                    super::input::InputScope {
                        root: &root,
                        lifetime,
                        window: &target,
                        context: &context,
                        nodes: &observation.nodes,
                        signals: &signals,
                        cancel: &cancel,
                        timeout,
                        native_epoch: observation.native_epoch,
                    },
                    &input,
                    || {
                        let remaining = deadline
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_secs(2));
                        if remaining.is_zero() {
                            return Err(Error::Unavailable);
                        }
                        let (reply, _) =
                            self.read_worker(&target, ReadQuery::ProtectedFocus, remaining)?;
                        match reply {
                            Reply::ProtectedFocus(value) => Ok(value),
                            _ => Err(Error::Denied),
                        }
                    },
                )?;
                self.refresh();
                Ok((
                    Reply::Applied(
                        self.active(&session_id, control_generation)
                            .map_err(|_| Error::OutcomeUnknown)?
                            .summary
                            .clone(),
                    ),
                    Vec::new(),
                ))
            }
            ComputerCommand::Close {
                session_id,
                control_generation,
            } => {
                let session = self
                    .session
                    .as_ref()
                    .filter(|s| {
                        s.summary.session_id == session_id
                            && s.summary.control_generation == control_generation
                    })
                    .ok_or(Error::Stale)?;
                if session.summary.state != ComputerControlState::Active {
                    self.blocked = true;
                }
                self.session = None;
                Ok((Reply::Closed, Vec::new()))
            }
        }
    }
    fn store_observation(
        &mut self,
        id: &ComputerSessionId,
        generation: u64,
        mut observation: Observation,
    ) -> Result<(), Error> {
        self.refresh();
        let session = self.active(id, generation)?;
        let root = self
            .windows
            .get(&session.summary.window_id)
            .ok_or(Error::Stale)?;
        let target = observation
            .allowed
            .iter()
            .find(|(_, w)| w == &observation.window)
            .and_then(|(id, _)| self.windows.get(id))
            .ok_or(Error::Stale)?;
        if !self.lifetimes.alive(root.life, root.window.handle)
            || !self.lifetimes.alive(target.life, target.window.handle)
        {
            return Err(Error::Stale);
        }
        observation.window.revalidate(&self.bootstrap.context)?;
        let signals = &session.indicator.as_ref().ok_or(Error::Denied)?.signals;
        if signals.native_epoch.load(Ordering::Acquire) != observation.native_epoch {
            return Err(Error::Stale);
        }
        observation.human = signals.human.load(Ordering::Acquire);
        observation.foreground = foreground_identity();
        self.session.as_mut().ok_or(Error::Stale)?.observation = Some(observation);
        Ok(())
    }
    fn begin_observation(&self, window: u64) -> Result<u64, Error> {
        let signals = &self
            .session
            .as_ref()
            .and_then(|s| s.indicator.as_ref())
            .ok_or(Error::Denied)?
            .signals;
        signals.tracked.store(window, Ordering::Release);
        Ok(signals.native_epoch.load(Ordering::Acquire))
    }
    fn read_worker(
        &mut self,
        window: &Window,
        query: ReadQuery,
        timeout: Duration,
    ) -> Result<(Reply, Vec<u8>), Error> {
        if let Some(worker) = self.worker.as_mut() {
            worker.cleanup()?;
        }
        self.worker = None;
        #[cfg(feature = "native-test-driver")]
        eprintln!("COMPUTER_NATIVE_READ phase=spawn");
        let path = std::env::current_exe().map_err(|_| Error::Failed)?;
        let image = colossus_windows_native::BoundPath::open_immutable_file(&path)
            .map_err(|_| Error::Failed)?;
        let identity = image.identity();
        let mut command = std::process::Command::new(&path);
        command
            .arg(protocol::HELPER_ARGUMENT)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(if cfg!(feature = "native-test-driver") {
                std::process::Stdio::inherit()
            } else {
                std::process::Stdio::null()
            });
        super::configure_environment(&mut command)?;
        colossus_windows_native::configure_suspended_process(&mut command);
        let nonce = protocol::opaque("cu_p_")?;
        self.worker = Some(ReadWorker {
            child: command.spawn().map_err(|_| Error::Failed)?,
            job: None,
            reader: None,
        });
        let result = (|| {
            let worker = self.worker.as_mut().ok_or(Error::Failed)?;
            let job = colossus_windows_native::KillOnCloseJob::assign_verify_and_resume(
                &worker.child,
                worker.child.id(),
                identity,
            )
            .map_err(|_| Error::Failed)?;
            worker.job = Some(job);
            #[cfg(feature = "native-test-driver")]
            eprintln!("COMPUTER_NATIVE_READ phase=enrolled");
            let mut input = worker.child.stdin.take().ok_or(Error::Failed)?;
            wire::write(
                &mut input,
                &Bootstrap {
                    version: protocol::VERSION,
                    nonce: nonce.clone(),
                    context: self.bootstrap.context.clone(),
                    binding: self.bootstrap.binding.clone(),
                    role: Role::Read {
                        window: window.clone(),
                        query,
                    },
                },
                &[],
            )
            .map_err(|_| Error::Failed)?;
            drop(input);
            #[cfg(feature = "native-test-driver")]
            eprintln!("COMPUTER_NATIVE_READ phase=bootstrapped");
            let mut output = worker.child.stdout.take().ok_or(Error::Failed)?;
            let (sender, receiver) = mpsc::sync_channel(1);
            worker.reader = Some(std::thread::spawn(move || {
                let result = (|| {
                    let (ready, binary) = wire::read::<Response>(&mut output)?;
                    if ready.nonce != nonce
                        || ready.sequence != 0
                        || !matches!(ready.result, Ok(Reply::Ready))
                        || !binary.is_empty()
                    {
                        return Err(wire::invalid());
                    }
                    #[cfg(feature = "native-test-driver")]
                    eprintln!("COMPUTER_NATIVE_READ phase=ready");
                    let (response, binary) = wire::read::<Response>(&mut output)?;
                    if response.nonce != nonce || response.sequence != 1 {
                        return Err(wire::invalid());
                    }
                    #[cfg(feature = "native-test-driver")]
                    eprintln!("COMPUTER_NATIVE_READ phase=acknowledged");
                    Ok((response, binary))
                })();
                let _ = sender.send(result);
            }));
            Ok(receiver)
        })();
        let result = match result {
            Err(error) => Err(error),
            Ok(receiver) => {
                let deadline = Instant::now() + timeout;
                loop {
                    self.refresh();
                    if self.cancel.load(Ordering::Acquire)
                        || self
                            .session
                            .as_ref()
                            .is_some_and(|s| s.summary.state != ComputerControlState::Active)
                    {
                        break Err(Error::Denied);
                    }
                    match receiver.try_recv() {
                        Ok(Ok((response, binary))) => {
                            break response.result.map(|meta| (meta, binary));
                        }
                        Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                            break Err(Error::Failed);
                        }
                        Err(mpsc::TryRecvError::Empty) => {}
                    }
                    if Instant::now() >= deadline {
                        break Err(Error::Unavailable);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        };
        // Failed enrollment/bootstrap/read retains the exact worker until its
        // exit is confirmed. A failed cleanup escalates to broker supervision.
        self.worker
            .as_mut()
            .ok_or(Error::OutcomeUnknown)?
            .cleanup()?;
        self.worker = None;
        result
    }
}

fn foreground_identity() -> Option<(u64, crate::NativeProcessIdentity)> {
    let hwnd = identity::foreground();
    let mut pid = 0;
    // SAFETY: native foreground query and stack process-id output.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    Some((hwnd as usize as u64, identity::process(pid).ok()?))
}

fn enumerate_handles() -> Vec<u64> {
    // Keep enumeration itself free of identity facts. A candidate can disappear
    // or be recycled before inspection; admission seals it before reading facts.
    const MAX_CANDIDATES: usize = 512;
    unsafe extern "system" fn callback(hwnd: HWND, data: LPARAM) -> i32 {
        // SAFETY: EnumWindows borrows the stack enumeration until returning.
        let handles = unsafe { &mut *(data as *mut Vec<u64>) };
        handles.push(hwnd as usize as u64);
        i32::from(handles.len() < MAX_CANDIDATES)
    }
    let mut handles = Vec::<u64>::new();
    // SAFETY: synchronous callback receives only this live bounded collection.
    unsafe { EnumWindows(Some(callback), (&mut handles as *mut Vec<u64>) as LPARAM) };
    handles
}
