//! Native operator requests and physical keyboard routing, never model commands.
use colossus_contracts::{
    ComputerControlState as State, ComputerSessionSummary, ComputerStopReason as Reason,
};
use colossus_ports::{ComputerCommand, ComputerDriverError};
use std::sync::atomic::{AtomicU8, AtomicU16, Ordering};

/// Higher values dominate pending requests until the broker drains them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum Action {
    Resume = 1,
    Pause = 2,
    WindowClosed = 3,
    Stop = 4,
    Fault = 5,
}

impl Action {
    pub(crate) fn publish(self, pending: &AtomicU8) {
        pending.fetch_max(self as u8, Ordering::AcqRel);
    }

    pub(crate) fn take(pending: &AtomicU8) -> Option<Self> {
        match pending.swap(0, Ordering::AcqRel) {
            0 => None,
            1 => Some(Self::Resume),
            2 => Some(Self::Pause),
            3 => Some(Self::WindowClosed),
            4 => Some(Self::Stop),
            // All producers are private. An impossible pending value still
            // revokes authority, but cannot impersonate a positive human Stop.
            _ => Some(Self::Fault),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Transition {
    Resume,
    Pause(Reason),
    Stop(Reason),
}

/// Exact authenticated broker-owner tombstone, retained after session cleanup.
pub(crate) struct StopFence(bool);

impl StopFence {
    pub(crate) fn new() -> Self {
        Self(false)
    }

    pub(crate) fn record(&mut self, summary: &ComputerSessionSummary) {
        self.0 |= user_stopped(summary);
    }

    pub(crate) fn stopped(&self) -> bool {
        self.0
    }

    /// This check is used only before dispatch; a possibly applied operation
    /// must retain OutcomeUnknown instead of being relabelled by a later Stop.
    pub(crate) fn before_dispatch(
        &self,
        command: &ComputerCommand,
    ) -> Result<(), ComputerDriverError> {
        if self.0
            && !matches!(
                command,
                ComputerCommand::Status { .. } | ComputerCommand::Close { .. }
            )
        {
            Err(ComputerDriverError::UserStopped)
        } else {
            Ok(())
        }
    }
}

pub(crate) fn user_stopped(summary: &ComputerSessionSummary) -> bool {
    summary.state == State::Stopped && summary.reason == Some(Reason::UserStopped)
}

pub(crate) fn input_failure(error: ComputerDriverError, started: bool) -> ComputerDriverError {
    if started {
        ComputerDriverError::OutcomeUnknown
    } else {
        error
    }
}

/// Only callers that have proved no activation or input was dispatched may
/// classify a pending native Stop as a known, effect-free human cancellation.
pub(crate) fn undispatched_failure(error: ComputerDriverError, pending: u8) -> ComputerDriverError {
    if pending == Action::Stop as u8 {
        ComputerDriverError::UserStopped
    } else {
        error
    }
}

impl Transition {
    /// A terminal stop never changes reason or becomes resumable again.
    pub(crate) fn apply(self, summary: &mut ComputerSessionSummary) -> bool {
        if summary.state == State::Stopped {
            return false;
        }
        let (state, reason) = match self {
            Self::Resume => (State::Active, None),
            Self::Pause(reason) => (State::Paused, Some(reason)),
            Self::Stop(reason) => (State::Stopped, Some(reason)),
        };
        if summary.state == state && summary.reason == reason {
            return false;
        }
        summary.state = state;
        summary.reason = reason;
        summary.control_generation += 1;
        true
    }
}

pub(crate) fn decide(
    state: State,
    action: Option<Action>,
    human_changed: bool,
) -> Option<Transition> {
    if state == State::Stopped {
        return None;
    }
    match action {
        Some(Action::Stop) => Some(Transition::Stop(Reason::UserStopped)),
        Some(Action::WindowClosed) => Some(Transition::Stop(Reason::WindowClosed)),
        Some(Action::Fault) => Some(Transition::Stop(Reason::Disabled)),
        Some(Action::Pause) => Some(Transition::Pause(Reason::UserPaused)),
        Some(Action::Resume) if state == State::Paused => Some(Transition::Resume),
        Some(Action::Resume) => None,
        None if human_changed && state == State::Active => {
            Some(Transition::Pause(Reason::UserTakeover))
        }
        None => None,
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Key {
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    ShiftLeft,
    ShiftRight,
    WindowsLeft,
    WindowsRight,
    Escape,
    P,
    Other,
}

#[derive(Default, Debug, Eq, PartialEq)]
pub(crate) struct Route {
    pub action: Option<Action>,
    pub suppress: bool,
}

/// Only this helper's pumping thread routes events. Atomics keep callback
/// ownership explicit without a lock or any work on another application's thread.
pub(crate) struct Keyboard(AtomicU16);

impl Keyboard {
    pub(crate) fn new() -> Self {
        Self(AtomicU16::new(0))
    }

    pub(crate) fn reset(&self) {
        self.0.store(0, Ordering::Release);
    }

    pub(crate) fn route(&self, armed: bool, key: Key, down: bool, injected: bool) -> Route {
        if !armed {
            self.reset();
            return Route::default();
        }
        // Agent and foreign injected events cannot contribute a modifier,
        // trigger a shortcut, or clear a physically held modifier's state.
        if injected {
            return Route::default();
        }
        let bit = match key {
            Key::ControlLeft => 1,
            Key::ControlRight => 2,
            Key::AltLeft => 4,
            Key::AltRight => 8,
            Key::ShiftLeft => 16,
            Key::ShiftRight => 32,
            Key::WindowsLeft => 64,
            Key::WindowsRight => 128,
            Key::Escape => 256,
            Key::P => 512,
            Key::Other => 0,
        };
        let claimed = match key {
            Key::Escape => 1024,
            Key::P => 2048,
            _ => 0,
        };
        let before = self.0.load(Ordering::Acquire);
        let mut after = if down { before | bit } else { before & !bit };
        let action = if down && before & bit == 0 {
            match key {
                Key::Escape => Some(Action::Stop),
                Key::P if before & 3 != 0 && before & 12 != 0 && before & 240 == 0 => {
                    Some(Action::Pause)
                }
                _ => None,
            }
        } else {
            None
        };
        if action.is_some() {
            after |= claimed;
        }
        let suppress = (before | after) & claimed != 0;
        if !down {
            after &= !claimed;
        }
        self.0.store(after, Ordering::Release);
        Route { action, suppress }
    }
}

#[cfg(test)]
#[path = "operator_control_tests.rs"]
mod tests;
