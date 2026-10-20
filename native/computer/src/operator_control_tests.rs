use super::*;
use colossus_contracts::{ComputerSessionId, ComputerWindowId};

fn active() -> ComputerSessionSummary {
    ComputerSessionSummary {
        session_id: ComputerSessionId::parse(format!("cu_s_{}", "1".repeat(32))).unwrap(),
        window_id: ComputerWindowId::parse(format!("cu_w_{}", "2".repeat(32))).unwrap(),
        control_generation: 1,
        state: State::Active,
        reason: None,
    }
}

#[test]
fn physical_escape_stops_irreversibly_and_repeat_cannot_resume() {
    let keyboard = Keyboard::new();
    let route = keyboard.route(true, Key::Escape, true, false);
    assert_eq!(route.action, Some(Action::Stop));
    assert!(route.suppress);
    let mut session = active();
    assert!(
        decide(session.state, route.action, true)
            .unwrap()
            .apply(&mut session)
    );
    assert_eq!(session.state, State::Stopped);
    assert_eq!(session.reason, Some(Reason::UserStopped));
    assert_eq!(session.control_generation, 2);
    assert!(keyboard.route(true, Key::Escape, true, false).suppress);
    assert_eq!(keyboard.route(true, Key::Escape, true, false).action, None);
    for action in [
        Action::Resume,
        Action::Pause,
        Action::Stop,
        Action::WindowClosed,
    ] {
        assert_eq!(decide(session.state, Some(action), true), None);
    }
    assert!(!Transition::Resume.apply(&mut session));
    assert!(!Transition::Stop(Reason::Expired).apply(&mut session));
    assert_eq!(session.reason, Some(Reason::UserStopped));
}

#[test]
fn physical_chord_pauses_only_and_native_button_alone_resumes() {
    let keyboard = Keyboard::new();
    keyboard.route(true, Key::ControlLeft, true, false);
    keyboard.route(true, Key::AltRight, true, false);
    let route = keyboard.route(true, Key::P, true, false);
    assert_eq!(route.action, Some(Action::Pause));
    assert!(route.suppress);
    let mut session = active();
    let transition = decide(session.state, route.action, true).unwrap();
    assert!(transition.apply(&mut session));
    assert_eq!(session.state, State::Paused);
    assert_eq!(session.reason, Some(Reason::UserPaused));
    assert_eq!(session.control_generation, 2);
    assert!(
        !decide(session.state, Some(Action::Pause), true)
            .unwrap()
            .apply(&mut session)
    );
    assert_eq!(session.state, State::Paused);
    assert_eq!(session.control_generation, 2);
    assert!(keyboard.route(true, Key::P, false, false).suppress);
    assert_eq!(
        keyboard.route(true, Key::P, true, false).action,
        Some(Action::Pause)
    );
    assert!(
        decide(session.state, Some(Action::Resume), true)
            .unwrap()
            .apply(&mut session)
    );
    assert_eq!(session.state, State::Active);
    assert_eq!(session.reason, None);
    assert_eq!(session.control_generation, 3);
}

#[test]
fn stopped_or_closed_request_cannot_be_overwritten_by_a_later_resume_or_pause() {
    for terminal in [Action::Stop, Action::WindowClosed] {
        let pending = AtomicU8::new(0);
        terminal.publish(&pending);
        Action::Resume.publish(&pending);
        Action::Pause.publish(&pending);
        assert_eq!(Action::take(&pending), Some(terminal));
        assert_eq!(Action::take(&pending), None);
    }
    let pending = AtomicU8::new(0);
    Action::Pause.publish(&pending);
    Action::Resume.publish(&pending);
    assert_eq!(Action::take(&pending), Some(Action::Pause));
}

#[test]
fn unarmed_or_injected_keys_neither_trigger_nor_supply_physical_modifiers() {
    let keyboard = Keyboard::new();
    assert_eq!(
        keyboard.route(false, Key::Escape, true, false),
        Route::default()
    );
    assert_eq!(
        keyboard.route(true, Key::Escape, true, true),
        Route::default()
    );
    keyboard.route(true, Key::ControlLeft, true, true);
    keyboard.route(true, Key::AltLeft, true, true);
    assert_eq!(keyboard.route(true, Key::P, true, false), Route::default());
    keyboard.reset();
    keyboard.route(true, Key::ControlRight, true, false);
    keyboard.route(true, Key::AltLeft, true, false);
    keyboard.route(true, Key::ControlRight, false, true);
    assert_eq!(
        keyboard.route(true, Key::P, true, false).action,
        Some(Action::Pause)
    );
    keyboard.route(false, Key::Other, false, false);
    assert_eq!(keyboard.route(true, Key::P, true, false), Route::default());
}

#[test]
fn modifier_releases_extra_modifiers_and_key_repeat_do_not_expand_the_shortcut() {
    for extra in [
        Key::ShiftLeft,
        Key::ShiftRight,
        Key::WindowsLeft,
        Key::WindowsRight,
    ] {
        let keyboard = Keyboard::new();
        keyboard.route(true, Key::ControlLeft, true, false);
        keyboard.route(true, Key::AltLeft, true, false);
        keyboard.route(true, extra, true, false);
        assert_eq!(keyboard.route(true, Key::P, true, false), Route::default());
    }
    let keyboard = Keyboard::new();
    keyboard.route(true, Key::ControlLeft, true, false);
    keyboard.route(true, Key::ControlRight, true, false);
    keyboard.route(true, Key::ControlLeft, false, false);
    keyboard.route(true, Key::AltLeft, true, false);
    assert_eq!(
        keyboard.route(true, Key::P, true, false).action,
        Some(Action::Pause)
    );
    assert_eq!(keyboard.route(true, Key::P, true, false).action, None);
    keyboard.route(true, Key::P, false, false);
    keyboard.route(true, Key::ControlRight, false, false);
    assert_eq!(keyboard.route(true, Key::P, true, false), Route::default());
}

#[test]
fn ordinary_human_input_still_pauses_and_stop_dominates_takeover() {
    let mut session = active();
    assert!(
        decide(session.state, None, true)
            .unwrap()
            .apply(&mut session)
    );
    assert_eq!(session.reason, Some(Reason::UserTakeover));
    assert_eq!(decide(session.state, None, true), None);
    assert!(
        decide(session.state, Some(Action::Stop), true)
            .unwrap()
            .apply(&mut session)
    );
    assert_eq!(session.state, State::Stopped);
    assert_eq!(session.reason, Some(Reason::UserStopped));
}

#[test]
fn pointer_motion_preserves_active_generation_until_click_or_explicit_escape() {
    let human = AtomicU64::new(0);
    let mut session = active();
    for _ in 0..100 {
        foreign_pointer(&human, true);
        assert_eq!(
            decide(session.state, None, human.load(Ordering::Acquire) != 0),
            None
        );
    }
    assert_eq!(session.control_generation, 1);
    let keyboard = Keyboard::new();
    let escape = keyboard.route(true, Key::Escape, true, false);
    decide(session.state, escape.action, false)
        .unwrap()
        .apply(&mut session);
    assert_eq!(session.state, State::Stopped);
    assert_eq!(session.reason, Some(Reason::UserStopped));

    let mut session = active();
    foreign_pointer(&human, false);
    decide(session.state, None, human.load(Ordering::Acquire) != 0)
        .unwrap()
        .apply(&mut session);
    assert_eq!(session.state, State::Paused);
    assert_eq!(session.reason, Some(Reason::UserTakeover));
}

#[test]
fn delayed_pause_after_takeover_cannot_resume_the_session() {
    let mut session = active();
    assert_eq!(decide(session.state, Some(Action::Resume), false), None);
    decide(session.state, None, true)
        .unwrap()
        .apply(&mut session);
    assert_eq!(session.reason, Some(Reason::UserTakeover));
    decide(session.state, Some(Action::Pause), true)
        .unwrap()
        .apply(&mut session);
    assert_eq!(session.state, State::Paused);
    assert_eq!(session.reason, Some(Reason::UserPaused));
    let generation = session.control_generation;
    assert!(
        !decide(session.state, Some(Action::Pause), true)
            .unwrap()
            .apply(&mut session)
    );
    assert_eq!(session.control_generation, generation);
    decide(session.state, Some(Action::Resume), true)
        .unwrap()
        .apply(&mut session);
    assert_eq!(session.state, State::Active);
    assert_eq!(session.control_generation, generation + 1);
}

#[test]
fn only_a_positive_user_stop_latches_and_status_and_close_remain_available() {
    use colossus_contracts::{ComputerInput, ComputerLimits, ComputerObservationId};
    let mut fence = StopFence::new();
    let mut session = active();
    let snapshot = ComputerCommand::Snapshot {
        session_id: session.session_id.clone(),
        control_generation: 1,
        max_nodes: 16,
    };
    for transition in [
        Transition::Pause(Reason::UserPaused),
        Transition::Pause(Reason::UserTakeover),
        Transition::Stop(Reason::Disabled),
    ] {
        let mut candidate = active();
        transition.apply(&mut candidate);
        fence.record(&candidate);
        assert_eq!(fence.before_dispatch(&snapshot), Ok(()));
        assert!(!fence.stopped());
    }
    Transition::Stop(Reason::UserStopped).apply(&mut session);
    fence.record(&session);
    assert!(fence.stopped());
    let commands = [
        ComputerCommand::Windows { max_windows: 1 },
        ComputerCommand::Open {
            window_id: session.window_id.clone(),
            limits: ComputerLimits::default(),
        },
        snapshot,
        ComputerCommand::Screenshot {
            session_id: session.session_id.clone(),
            control_generation: 1,
            max_bytes: 1024,
        },
        ComputerCommand::Input {
            session_id: session.session_id.clone(),
            control_generation: 1,
            observation_id: ComputerObservationId::parse(format!("cu_o_{}", "3".repeat(32)))
                .unwrap(),
            input: ComputerInput::Move { x: 20, y: 20 },
        },
    ];
    for command in commands {
        assert_eq!(
            fence.before_dispatch(&command),
            Err(ComputerDriverError::UserStopped)
        );
    }
    for command in [
        ComputerCommand::Status {
            session_id: session.session_id.clone(),
        },
        ComputerCommand::Close {
            session_id: session.session_id.clone(),
            control_generation: session.control_generation,
        },
    ] {
        assert_eq!(fence.before_dispatch(&command), Ok(()));
    }
    // Cleanup may remove the session, but cannot erase the owner tombstone or
    // make a later Open admissible. Recording another summary cannot clear it.
    fence.record(&active());
    assert_eq!(
        fence.before_dispatch(&ComputerCommand::Windows { max_windows: 1 }),
        Err(ComputerDriverError::UserStopped)
    );
}

#[test]
fn stop_is_positive_before_dispatch_but_never_relabels_possible_input() {
    assert_eq!(
        input_failure(ComputerDriverError::UserStopped, false),
        ComputerDriverError::UserStopped
    );
    assert_eq!(
        input_failure(ComputerDriverError::UserStopped, true),
        ComputerDriverError::OutcomeUnknown
    );
    assert_eq!(
        input_failure(ComputerDriverError::Denied, false),
        ComputerDriverError::Denied
    );
    assert_eq!(
        input_failure(ComputerDriverError::Stale, false),
        ComputerDriverError::Stale
    );
    assert_eq!(
        input_failure(ComputerDriverError::Denied, true),
        ComputerDriverError::OutcomeUnknown
    );
}

#[test]
fn pending_stop_is_categorical_only_before_the_first_input_effect() {
    for pending in [0, Action::Resume as u8, Action::WindowClosed as u8, 255] {
        assert_eq!(
            undispatched_failure(ComputerDriverError::Denied, pending),
            ComputerDriverError::Denied
        );
        assert_eq!(
            input_failure(
                undispatched_failure(ComputerDriverError::Denied, pending),
                true
            ),
            ComputerDriverError::OutcomeUnknown
        );
    }
    assert_eq!(
        undispatched_failure(ComputerDriverError::Denied, Action::Pause as u8),
        ComputerDriverError::Paused
    );
    assert_eq!(
        undispatched_failure(ComputerDriverError::Denied, Action::Stop as u8),
        ComputerDriverError::UserStopped
    );
    assert_eq!(
        input_failure(
            undispatched_failure(ComputerDriverError::Denied, Action::Stop as u8),
            true
        ),
        ComputerDriverError::OutcomeUnknown
    );
}

#[test]
fn stop_drained_during_focus_admission_prevents_activation_and_returns_user_stop() {
    use crate::focus_settle::{self, Conditions};
    use std::cell::Cell;
    let pending = AtomicU8::new(0);
    let activated = Cell::new(false);
    let result = focus_settle::activate(
        || Ok(()),
        || {
            Action::Stop.publish(&pending);
            (
                Conditions {
                    foreground: false,
                    human_unchanged: false,
                    cancelled: false,
                    control_pending: pending.load(Ordering::Acquire) != 0,
                    epoch_unchanged: true,
                    admission_valid: true,
                    lifetime_valid: true,
                },
                true,
            )
        },
        || {
            activated.set(true);
            true
        },
        ComputerDriverError::Denied,
    )
    .map_err(|error| undispatched_failure(error, pending.load(Ordering::Acquire)));
    assert!(!activated.get());
    assert_eq!(result, Err(ComputerDriverError::UserStopped));
}

#[test]
fn invalid_pending_request_stops_without_claiming_a_positive_user_stop() {
    let mut session = active();
    let mut fence = StopFence::new();
    let pending = AtomicU8::new(255);
    decide(session.state, Action::take(&pending), false)
        .unwrap()
        .apply(&mut session);
    fence.record(&session);
    assert_eq!(session.state, State::Stopped);
    assert_eq!(session.reason, Some(Reason::Disabled));
    assert!(!fence.stopped());
}
