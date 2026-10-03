use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::{
    DictationError, InstalledModel, TranscriptUpdate, capture::Capture, decoder::WhisperDecoder,
    meter::InputMeter, pipeline::Pipeline,
};

const MAX_EVENTS: usize = 8;
const MAX_COMMAND_EVENTS: usize = 64;

/// Independently cancellable ownership handle for window close and reload.
#[derive(Clone)]
pub struct SessionCancellation(Arc<AtomicBool>);

impl SessionCancellation {
    /// Abort capture and any active helper decode without taking the transport lock.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Explicit human recording controls. Finishing a turn retains the microphone.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionAction {
    Pause,
    Resume,
    FinishTurn,
    Stop,
}

/// Recording state, including initialization before microphone access.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Starting,
    Recording,
    Paused,
    Stopped,
}

/// Bounded updates for one native-owned session; no audio or model data.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    Level {
        level: u8,
    },
    State {
        phase: SessionPhase,
    },
    Transcript {
        turn_id: u64,
        update: TranscriptUpdate,
    },
    Boundary {
        turn_id: u64,
    },
    Failure {
        error: DictationError,
    },
}

struct Command {
    action: SessionAction,
    reply: SyncSender<Result<(), DictationError>>,
}

/// Owns a recording thread and its bounded transport. Dropping it cancels local
/// inference, releases capture, and joins the recording thread.
pub struct Session {
    commands: SyncSender<Command>,
    events: Receiver<SessionEvent>,
    cancelled: Arc<AtomicBool>,
    outcome: Arc<Mutex<Option<Result<(), DictationError>>>>,
    thread: Option<JoinHandle<()>>,
    reported_end: bool,
    meter: Arc<InputMeter>,
    last_level: u8,
}

impl Session {
    /// Start model initialization off the UI thread. Capture begins only after
    /// the exact model bytes have been verified and the helper is ready.
    ///
    /// # Errors
    /// Returns a categorical error if the recording thread cannot start.
    pub fn start(model: InstalledModel) -> Result<Self, DictationError> {
        let (commands, receiver) = mpsc::sync_channel(4);
        let (sender, events) = mpsc::sync_channel(MAX_EVENTS);
        let cancelled = Arc::new(AtomicBool::new(false));
        let outcome = Arc::new(Mutex::new(None));
        let thread_cancelled = cancelled.clone();
        let thread_outcome = outcome.clone();
        let meter = Arc::new(InputMeter::default());
        let thread_meter = meter.clone();
        let thread = thread::Builder::new()
            .name("offline-dictation".into())
            .spawn(move || {
                let result = record(&model, &receiver, &sender, &thread_cancelled, &thread_meter);
                if let Ok(mut outcome) = thread_outcome.lock() {
                    *outcome = Some(result);
                }
            })
            .map_err(|_| DictationError::CaptureUnavailable)?;
        Ok(Self {
            commands,
            events,
            cancelled,
            outcome,
            thread: Some(thread),
            reported_end: false,
            meter,
            last_level: 0,
        })
    }

    /// Drain at most eight ordered updates plus one coalesced input level. A stalled renderer
    /// cannot retain an unbounded transcript or keep capture alive indefinitely.
    #[must_use]
    pub fn poll(&mut self) -> Vec<SessionEvent> {
        let mut events = self.drain();
        let level = if self.reported_end {
            self.meter.clear();
            0
        } else {
            self.meter.take()
        };
        if level > 0 || self.last_level > 0 {
            events.push(SessionEvent::Level { level });
            self.last_level = level;
        }
        events
    }

    fn drain(&mut self) -> Vec<SessionEvent> {
        let mut events: Vec<_> = self.events.try_iter().take(MAX_EVENTS).collect();
        if !self.reported_end
            && events.len() < MAX_EVENTS
            && self.thread.as_ref().is_some_and(JoinHandle::is_finished)
        {
            self.reported_end = true;
            let result = self.outcome.lock().ok().and_then(|mut value| value.take());
            events.push(match result {
                Some(Ok(())) => SessionEvent::State {
                    phase: SessionPhase::Stopped,
                },
                Some(Err(error)) => SessionEvent::Failure { error },
                None => SessionEvent::Failure {
                    error: DictationError::Inference,
                },
            });
        }
        events
    }

    /// Perform a serialized control and collect ordered updates through its
    /// acknowledgement. A Send boundary therefore includes its final transcript
    /// before the caller submits the draft. This method belongs off the UI thread.
    ///
    /// # Errors
    /// Cancels capture if a control fails or exceeds its bounded deadline.
    pub fn command(&mut self, action: SessionAction) -> Result<Vec<SessionEvent>, DictationError> {
        let (reply, acknowledgement) = mpsc::sync_channel(1);
        self.commands
            .try_send(Command { action, reply })
            .map_err(|_| DictationError::CaptureUnavailable)?;
        let deadline = Instant::now() + Duration::from_mins(1);
        let mut events = Vec::new();
        loop {
            events.extend(self.drain());
            if events.len() > MAX_COMMAND_EVENTS || Instant::now() >= deadline {
                self.cancel();
                return Err(DictationError::CaptureOverrun);
            }
            match acknowledgement.try_recv() {
                Ok(Ok(())) => {
                    events.extend(self.drain());
                    if events.len() > MAX_COMMAND_EVENTS {
                        self.cancel();
                        return Err(DictationError::CaptureOverrun);
                    }
                    if matches!(action, SessionAction::Pause | SessionAction::Stop) {
                        self.last_level = 0;
                    }
                    return Ok(events);
                }
                Ok(Err(error)) => {
                    self.cancel();
                    events.push(SessionEvent::Failure { error });
                    return Ok(events);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    events.push(SessionEvent::Failure {
                        error: DictationError::Inference,
                    });
                    return Ok(events);
                }
                Err(mpsc::TryRecvError::Empty) => thread::sleep(Duration::from_millis(2)),
            }
        }
    }

    /// Immediately request cancellation, including an in-progress decode. The
    /// supervisor polls this flag every 20 ms and kills/reaps its helper.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Obtain cancellation authority without exposing capture or inference internals.
    #[must_use]
    pub fn cancellation(&self) -> SessionCancellation {
        SessionCancellation(self.cancelled.clone())
    }

    /// Whether capture, inference, and delivery of pending updates have ended.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.reported_end
    }
}

#[cfg(test)]
mod tests;

impl Drop for Session {
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn emit(sender: &SyncSender<SessionEvent>, event: SessionEvent) -> Result<(), DictationError> {
    sender
        .try_send(event)
        .map_err(|_| DictationError::CaptureOverrun)
}

fn record(
    model: &InstalledModel,
    commands: &Receiver<Command>,
    events: &SyncSender<SessionEvent>,
    cancelled: &Arc<AtomicBool>,
    meter: &Arc<InputMeter>,
) -> Result<(), DictationError> {
    emit(
        events,
        SessionEvent::State {
            phase: SessionPhase::Starting,
        },
    )?;
    let decoder = WhisperDecoder::load_cancellable(&model.path, model.digest, cancelled.clone())?;
    if cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut pipeline = Pipeline::new(decoder);
    let mut capture = Some(Capture::start_with_meter(meter.clone())?);
    emit(
        events,
        SessionEvent::State {
            phase: SessionPhase::Recording,
        },
    )?;
    let mut turn_id = 1;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let current_turn = turn_id;
        let mut transcript = |update| {
            emit(
                events,
                SessionEvent::Transcript {
                    turn_id: current_turn,
                    update,
                },
            )
        };
        match commands.try_recv() {
            Ok(command) => {
                let result = (|| match command.action {
                    SessionAction::Pause | SessionAction::Stop => {
                        if let Some(active) = capture.take() {
                            active.finish(&mut pipeline, &mut transcript)?;
                        }
                        meter.clear();
                        emit(
                            events,
                            SessionEvent::State {
                                phase: if command.action == SessionAction::Stop {
                                    SessionPhase::Stopped
                                } else {
                                    SessionPhase::Paused
                                },
                            },
                        )
                    }
                    SessionAction::Resume => {
                        if capture.is_none() {
                            capture = Some(Capture::start_with_meter(meter.clone())?);
                        }
                        emit(
                            events,
                            SessionEvent::State {
                                phase: SessionPhase::Recording,
                            },
                        )
                    }
                    SessionAction::FinishTurn => {
                        if let Some(active) = &mut capture {
                            active.finish_turn(&mut pipeline, &mut transcript)?;
                        }
                        turn_id = turn_id
                            .checked_add(1)
                            .ok_or(DictationError::TranscriptLimit)?;
                        emit(events, SessionEvent::Boundary { turn_id })
                    }
                })();
                let _ = command.reply.send(result);
                result?;
                if command.action == SessionAction::Stop {
                    return Ok(());
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
            Err(mpsc::TryRecvError::Empty) => {
                if let Some(active) = &mut capture {
                    active.poll(&mut pipeline, &mut transcript)?;
                }
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}
