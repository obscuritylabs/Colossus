use super::*;

fn transport(
    worker: impl FnOnce(Receiver<Command>, SyncSender<SessionEvent>) -> Result<(), DictationError>
    + Send
    + 'static,
) -> Session {
    let (commands, receiver) = mpsc::sync_channel(4);
    let (sender, events) = mpsc::sync_channel(MAX_EVENTS);
    let outcome = Arc::new(Mutex::new(None));
    let thread_outcome = outcome.clone();
    let thread = thread::spawn(move || {
        *thread_outcome.lock().unwrap() = Some(worker(receiver, sender));
    });
    Session {
        commands,
        events,
        cancelled: Arc::new(AtomicBool::new(false)),
        outcome,
        thread: Some(thread),
        reported_end: false,
    }
}

fn final_segment(segment_id: u64) -> SessionEvent {
    SessionEvent::Transcript {
        turn_id: 1,
        update: TranscriptUpdate {
            segment_id,
            revision: 1,
            is_final: true,
            text: "settled".into(),
            audio_ms: 1000,
            inference_ms: 1,
        },
    }
}

#[test]
fn control_drains_ordered_finals_before_acknowledging_the_next_draft() {
    let mut session = transport(|commands, events| {
        let command = commands.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(command.action, SessionAction::FinishTurn);
        for segment in 1..=50 {
            events.send(final_segment(segment)).unwrap();
        }
        events.send(SessionEvent::Boundary { turn_id: 2 }).unwrap();
        command.reply.send(Ok(())).unwrap();
        Ok(())
    });
    let events = session.command(SessionAction::FinishTurn).unwrap();
    let segments: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            SessionEvent::Transcript { update, .. } => Some(update.segment_id),
            _ => None,
        })
        .collect();
    assert_eq!(segments, (1..=50).collect::<Vec<_>>());
    assert!(
        events
            .iter()
            .any(|event| matches!(event, SessionEvent::Boundary { turn_id: 2 }))
    );
    assert!(events.len() <= MAX_COMMAND_EVENTS);
}

#[test]
fn a_terminal_state_cannot_discard_updates_remaining_in_the_bounded_queue() {
    let mut session = transport(|_, events| {
        for segment in 1..=9 {
            events.send(final_segment(segment)).unwrap();
        }
        Ok(())
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while session.events.try_recv().is_err() {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    // The first update above is deliberately consumed. The remaining queue can
    // contain a full eight updates when the producer terminates.
    while !session.thread.as_ref().unwrap().is_finished() {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    let first = session.poll();
    assert_eq!(first.len(), 8);
    assert!(!session.is_finished());
    assert!(!first.iter().any(|event| matches!(
        event,
        SessionEvent::State {
            phase: SessionPhase::Stopped
        }
    )));
    assert!(matches!(
        session.poll().as_slice(),
        [SessionEvent::State {
            phase: SessionPhase::Stopped
        }]
    ));
    assert!(session.is_finished());
}

#[test]
fn a_stalled_consumer_reports_failure_after_delivering_its_retained_updates() {
    let mut session = transport(|_, events| {
        for segment in 1..=9 {
            emit(&events, final_segment(segment))?;
        }
        Ok(())
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !session.thread.as_ref().unwrap().is_finished() {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    assert_eq!(session.poll().len(), MAX_EVENTS);
    assert!(matches!(
        session.poll().as_slice(),
        [SessionEvent::Failure {
            error: DictationError::CaptureOverrun
        }]
    ));
}
