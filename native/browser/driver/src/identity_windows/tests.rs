use super::*;
use colossus_ports::RunControl;

fn request() -> Request {
    Request {
        tab: 7,
        generation: 1,
        request_id: 11,
        control_generation: 3,
        origin: BrowserOrigin::parse("https://example.com").unwrap(),
        candidates: vec![Candidate {
            index: 2,
            fingerprint: "a".repeat(64),
            not_before: 0,
            not_after: i64::MAX,
        }],
        deadline: Instant::now() + TIMEOUT,
        cancel: Arc::new(AtomicBool::new(false)),
        control: BrowserDriverControl::new(RunControl::default(), RunControl::default()),
    }
}

#[test]
fn exact_native_selection_rejects_unknown_expired_and_cancelled_choices() {
    let mut request = request();
    assert_eq!(request.selected(Some(&"a".repeat(64))), 2);
    assert_eq!(request.selected(None), -1);
    assert_eq!(request.selected(Some(&"b".repeat(64))), -1);
    request.candidates[0].not_after = 1;
    assert_eq!(request.selected(Some(&"a".repeat(64))), -1);
    request.candidates[0].not_after = i64::MAX;
    request.cancel.store(true, Ordering::Release);
    assert_eq!(request.selected(Some(&"a".repeat(64))), -1);
}

fn pending(review: &mut Review, request: Request) -> mpsc::SyncSender<Option<String>> {
    let (send, result) = mpsc::sync_channel(1);
    review.pending = Some(Pending { request, result });
    send
}

#[test]
fn revoked_run_changed_writer_and_native_request_cancel_cannot_consume_a_selection() {
    for cause in 0..4 {
        let mut review = Review::default();
        let control = RunControl::default();
        let mut request = request();
        request.control = BrowserDriverControl::new(control.clone(), RunControl::default());
        review.operation = Some((3, request.control.clone()));
        let send = pending(&mut review, request);
        send.send(Some("a".repeat(64))).unwrap();
        match cause {
            0 => {
                control.cancel();
            }
            1 => review.generation_changed(4),
            2 => review.cancel(Some(11)),
            _ => review.pending.as_mut().unwrap().request.deadline = Instant::now(),
        }
        let (request, index) = review.ready().unwrap();
        assert!(request.cancelled());
        assert_eq!(index, -1);
        assert!(
            review.ready().is_none(),
            "native decisions are consumed once"
        );
    }
}

#[test]
fn unrelated_native_request_cannot_cancel_or_replay_the_current_review() {
    let mut review = Review::default();
    let send = pending(&mut review, request());
    review.cancel(Some(12));
    send.send(Some("a".repeat(64))).unwrap();
    let (request, index) = review.ready().unwrap();
    assert_eq!(request.request_id, 11);
    assert_eq!(index, 2);
    assert!(review.ready().is_none());
}
