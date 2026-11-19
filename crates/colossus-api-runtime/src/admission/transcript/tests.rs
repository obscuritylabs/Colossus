use super::*;

#[test]
fn transcript_rate_bursts_and_refill_are_independent_and_monotonic() {
    let now = Instant::now();
    let admission = TranscriptAdmission::new_at(now);
    for _ in 0..4 {
        drop(admission.try_acquire_at("app:a", now).unwrap());
    }
    assert!(admission.try_acquire_at("app:a", now).is_err());
    assert!(
        admission
            .try_acquire_at("app:a", now + Duration::from_millis(124))
            .is_err()
    );
    drop(
        admission
            .try_acquire_at("app:a", now + Duration::from_millis(125))
            .unwrap(),
    );
    assert!(admission.try_acquire_at("app:a", now).is_err());
    assert!(
        admission
            .try_acquire_at("app:a", now + Duration::from_millis(125))
            .is_err()
    );

    let admission = TranscriptAdmission::new_at(now);
    for index in 0..8 {
        drop(
            admission
                .try_acquire_at(&format!("app:{index}"), now)
                .unwrap(),
        );
    }
    assert!(admission.try_acquire_at("app:next", now).is_err());
    assert!(
        admission
            .try_acquire_at("app:next", now + Duration::from_micros(62_499))
            .is_err()
    );
    drop(
        admission
            .try_acquire_at("app:next", now + Duration::from_micros(62_500))
            .unwrap(),
    );
    assert!(
        admission
            .try_acquire_at("app:next", now + Duration::from_micros(62_500))
            .is_err()
    );
}

#[test]
fn transcript_active_and_waiting_capacity_is_bounded_per_application_and_globally() {
    let now = Instant::now();
    let admission = TranscriptAdmission::new_at(now);
    let first = admission.try_acquire_at("app:a", now).unwrap();
    let second = admission.try_acquire_at("app:b", now).unwrap();
    assert!(admission.try_acquire_at("app:a", now).is_err());
    assert!(admission.try_acquire_at("app:c", now).is_err());
    let mut waiting = vec![admission.enqueue("app:a").unwrap()];
    assert!(admission.enqueue("app:a").is_err());
    for app in ["app:b", "app:c", "app:d"] {
        assert!(admission.try_acquire_at(app, now).is_err());
        waiting.push(admission.enqueue(app).unwrap());
    }
    assert!(admission.try_acquire_at("app:e", now).is_err());
    assert!(admission.enqueue("app:e").is_err());
    assert_eq!(lock(&admission.state).waiting, 4);
    drop(waiting);
    assert_eq!(lock(&admission.state).waiting, 0);
    drop(first);
    drop(admission.try_acquire_at("app:c", now).unwrap());
    drop(second);
    assert_eq!(lock(&admission.state).active, 0);
}

#[tokio::test]
async fn cancelled_and_expired_waiters_release_capacity_without_consuming_tokens() {
    let admission = TranscriptAdmission::new();
    let active = admission.acquire("app:a").await.unwrap();
    let tokens = lock(&admission.state).applications["app:a"].bucket.tokens;
    let mut waiting = Box::pin(admission.acquire("app:a"));
    assert!(futures::poll!(waiting.as_mut()).is_pending());
    assert_eq!(lock(&admission.state).waiting, 1);
    drop(waiting);
    assert_eq!(lock(&admission.state).waiting, 0);
    assert!(
        admission
            .acquire_until("app:a", tokio::time::Instant::now())
            .await
            .is_err()
    );
    assert_eq!(lock(&admission.state).waiting, 0);
    // Waiting attempts may refill, but cannot consume additional page tokens.
    assert!(lock(&admission.state).applications["app:a"].bucket.tokens >= tokens);
    assert_eq!(WAIT_LIMIT, Duration::from_secs(2));
    drop(active);
    drop(admission.acquire("app:a").await.unwrap());
}
