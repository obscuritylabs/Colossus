use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<DictationAction>>,
    cancelled: AtomicUsize,
}
#[async_trait]
impl LocalDictation for Recorder {
    async fn control(&self, action: DictationAction) -> Result<Vec<DictationUpdate>, String> {
        self.calls.lock().unwrap().push(action);
        let transcript = |segment, text: &str| DictationUpdate::Transcript {
            segment,
            revision: 2,
            settled: true,
            text: text.into(),
        };
        Ok(match action {
            DictationAction::Start => vec![DictationUpdate::Listening],
            DictationAction::Pause => vec![transcript(1, "spoken"), DictationUpdate::Paused],
            DictationAction::Resume => vec![DictationUpdate::Listening],
            DictationAction::Stop => {
                vec![transcript(2, "final words"), DictationUpdate::Stopped]
            }
        })
    }
    async fn poll(&self) -> Result<Vec<DictationUpdate>, String> {
        Ok(vec![])
    }
    async fn configure(&self, _: &str) -> Result<String, String> {
        Ok("Saved".into())
    }
    fn cancel(&self) {
        self.cancelled.fetch_add(1, Ordering::Relaxed);
    }
}
async fn receive(state: &mut TuiState, receiver: &mut mpsc::Receiver<HostEvent>) {
    let event = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    crate::app::handle_host_event(state, event);
}
#[tokio::test]
async fn pause_edits_resume_and_enter_waits_for_final_speech_exactly_once() {
    let mut state = TuiState::from_snapshot(crate::tests::snapshot());
    state.composer.insert("Typed");
    let recorder = Arc::new(Recorder::default());
    state.dictation.port = Some(recorder.clone());
    let (events, mut receiver) = mpsc::channel(16);
    assert!(key(
        &mut state,
        KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE),
        events.clone()
    ));
    receive(&mut state, &mut receiver).await;
    assert!(key(
        &mut state,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        events.clone()
    ));
    assert_eq!(state.composer.draft, "Typed");
    control(&mut state, DictationAction::Pause, events.clone());
    receive(&mut state, &mut receiver).await;
    assert!(state.dictation.paused);
    state.composer.insert(" edited");
    control(&mut state, DictationAction::Resume, events.clone());
    receive(&mut state, &mut receiver).await;
    assert!(key(
        &mut state,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        events
    ));
    assert!(take_send(&mut state).is_none());
    receive(&mut state, &mut receiver).await;
    assert_eq!(
        take_send(&mut state).as_deref(),
        Some("Typed spoken edited final words")
    );
    assert!(take_send(&mut state).is_none());
    assert_eq!(
        *recorder.calls.lock().unwrap(),
        [
            DictationAction::Start,
            DictationAction::Pause,
            DictationAction::Resume,
            DictationAction::Stop
        ]
    );
}
#[tokio::test]
async fn pause_waits_for_an_in_flight_poll_before_releasing_draft_edits() {
    let mut state = TuiState::from_snapshot(crate::tests::snapshot());
    state.composer.insert("Typed");
    let recorder = Arc::new(Recorder::default());
    state.dictation.port = Some(recorder.clone());
    let (events, mut receiver) = mpsc::channel(16);
    control(&mut state, DictationAction::Start, events.clone());
    receive(&mut state, &mut receiver).await;
    state.dictation.polling = true;
    control(&mut state, DictationAction::Pause, events.clone());
    assert!(state.dictation.pending);
    assert!(!state.dictation.paused);
    assert_eq!(*recorder.calls.lock().unwrap(), [DictationAction::Start]);
    apply(
        &mut state,
        1,
        None,
        Ok(vec![DictationUpdate::Transcript {
            segment: 0,
            revision: 1,
            settled: true,
            text: "earlier speech".into(),
        }]),
    );
    tick(&mut state, events.clone());
    receive(&mut state, &mut receiver).await;
    assert!(state.dictation.paused);
    assert_eq!(state.composer.draft, "Typed earlier speech spoken");
    state.composer.insert(" edited");
    tick(&mut state, events);
    assert!(!state.dictation.polling);
    assert_eq!(state.composer.draft, "Typed earlier speech spoken edited");
}
#[test]
fn navigation_revokes_capture_and_rejects_old_session_updates() {
    let mut state = TuiState::from_snapshot(crate::tests::snapshot());
    let recorder = Arc::new(Recorder::default());
    state.dictation.port = Some(recorder.clone());
    state.dictation.active = true;
    state.dictation.session.clone_from(&state.session_id);
    state.session_id = "another session".into();
    let (events, _) = mpsc::channel(1);
    tick(&mut state, events);
    assert!(!state.dictation.active);
    assert_eq!(recorder.cancelled.load(Ordering::Relaxed), 1);
    apply(
        &mut state,
        0,
        None,
        Ok(vec![DictationUpdate::Transcript {
            segment: 1,
            revision: 1,
            settled: true,
            text: "old draft".into(),
        }]),
    );
    assert!(state.composer.draft.is_empty());
}
#[test]
fn revisions_replace_and_final_empty_clears_without_touching_typed_prefix() {
    let mut state = TuiState::from_snapshot(crate::tests::snapshot());
    state.dictation.base = "Typed prefix".into();
    state.dictation.active = true;
    let update = |revision, settled, text: &str| DictationUpdate::Transcript {
        segment: 0,
        revision,
        settled,
        text: text.into(),
    };
    apply(&mut state, 0, None, Ok(vec![update(1, false, "partial")]));
    assert_eq!(state.composer.draft, "Typed prefix partial");
    apply(&mut state, 0, None, Ok(vec![update(2, true, "")]));
    apply(&mut state, 0, None, Ok(vec![update(3, true, "stale")]));
    assert_eq!(state.composer.draft, "Typed prefix");
    apply(
        &mut state,
        99,
        None,
        Ok(vec![update(4, true, "wrong session")]),
    );
    assert_eq!(state.composer.draft, "Typed prefix");
}
