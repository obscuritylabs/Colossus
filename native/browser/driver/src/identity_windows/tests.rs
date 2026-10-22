use super::*;
use colossus_ports::RunControl;
use rcgen::{CertificateParams, DnType, ExtendedKeyUsagePurpose, KeyPair};

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
            label: "Client certificate".into(),
            not_before: 0,
            not_after: i64::MAX,
        }],
        deadline: Instant::now() + TIMEOUT,
        cancel: Arc::new(AtomicBool::new(false)),
        control: BrowserDriverControl::new(RunControl::default(), RunControl::default()),
    }
}
fn pending(review: &mut Review, request: Request) {
    review.pending = Some(Pending {
        request,
        decision: Decision::Pending,
    });
}
fn certificate(name: &str, eligible: bool) -> Vec<u8> {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.distinguished_name.push(DnType::CommonName, name);
    params.extended_key_usages = vec![if eligible {
        ExtendedKeyUsagePurpose::ClientAuth
    } else {
        ExtendedKeyUsagePurpose::ServerAuth
    }];
    params
        .self_signed(&KeyPair::generate().unwrap())
        .unwrap()
        .der()
        .to_vec()
}
fn enabled() -> Review {
    let mut review = Review::default();
    review.enable(true);
    review.operation(
        3,
        BrowserDriverControl::new(RunControl::default(), RunControl::default()),
    );
    review
}
fn start(review: &mut Review, certificates: &[&[u8]]) -> bool {
    review.start(
        7,
        1,
        11,
        BrowserOrigin::parse("https://example.com").unwrap(),
        certificates,
    )
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

#[test]
fn sole_eligible_certificate_keeps_native_index_and_needs_explicit_auto_policy() {
    let ineligible = certificate("Wrong EKU", false);
    let eligible = certificate("Reviewed identity", true);
    let mut review = enabled();
    // An earlier poll cannot grant key use to a future request.
    assert!(
        review
            .pending(ClientIdentityPolicy::AutomaticIfUnique)
            .unwrap()
            .is_none()
    );
    assert!(start(&mut review, &[&ineligible, &eligible]));
    assert!(
        review.ready().is_none(),
        "capability alone never selects a key"
    );
    let challenge = review
        .pending(ClientIdentityPolicy::Review)
        .unwrap()
        .unwrap();
    assert_eq!(challenge.candidates.len(), 1);
    assert_eq!(
        challenge.candidates[0].fingerprint_sha256,
        fingerprint(&eligible)
    );
    assert_eq!(challenge.candidates[0].label, "Reviewed identity");
    assert!(review.ready().is_none());
    assert!(
        review
            .pending(ClientIdentityPolicy::AutomaticIfUnique)
            .unwrap()
            .is_none()
    );
    let (request, index) = review.ready().unwrap();
    assert_eq!(index, 1, "filtering does not renumber native certificates");
    assert_eq!(request.key(), challenge.key);
    assert!(review.ready().is_none(), "native request is consumed once");
}

#[test]
fn multiple_eligible_identities_require_an_exact_single_use_choice() {
    let a = certificate("Identity A", true);
    let b = certificate("Identity B", true);
    let mut review = enabled();
    assert!(start(&mut review, &[&a, &b]));
    let challenge = review
        .pending(ClientIdentityPolicy::AutomaticIfUnique)
        .unwrap()
        .unwrap();
    assert_eq!(challenge.candidates.len(), 2);
    assert!(review.ready().is_none());
    for cause in 0..4 {
        let mut wrong = challenge.key;
        match cause {
            0 => wrong.tab += 1,
            1 => wrong.generation += 1,
            2 => wrong.request_id += 1,
            _ => wrong.control_generation += 1,
        }
        assert_eq!(
            review.select(wrong, Some(fingerprint(&a))),
            Err(PresentationError::Stale)
        );
        assert!(review.ready().is_none());
    }
    assert_eq!(
        review.select(challenge.key, Some("f".repeat(64))),
        Err(PresentationError::Invalid)
    );
    review.select(challenge.key, Some(fingerprint(&b))).unwrap();
    assert_eq!(
        review.select(challenge.key, None),
        Err(PresentationError::Stale)
    );
    assert_eq!(review.ready().unwrap().1, 1);
    assert!(review.ready().is_none());
    assert_eq!(
        review.select(challenge.key, Some(fingerprint(&b))),
        Err(PresentationError::Stale)
    );
}

#[test]
fn zero_duplicate_malformed_and_disabled_native_lists_cannot_auto_select() {
    let eligible = certificate("Eligible", true);
    let wrong = certificate("Wrong", false);
    for certificates in [
        vec![],
        vec![wrong.as_slice()],
        vec![eligible.as_slice(), eligible.as_slice()],
        vec![eligible.as_slice(), b"malformed".as_slice()],
    ] {
        let mut review = enabled();
        assert!(!start(&mut review, &certificates));
        assert!(
            review
                .pending(ClientIdentityPolicy::AutomaticIfUnique)
                .unwrap()
                .is_none()
        );
        assert!(review.ready().is_none());
    }
    let mut review = Review::default();
    review.operation(3, request().control);
    assert!(!start(&mut review, &[&eligible]));
    let mut review = enabled();
    assert!(!start(&mut review, &vec![eligible.as_slice(); 65]));
}

#[test]
fn revocation_expiry_and_changed_writer_cancel_already_queued_auto_selection() {
    for cause in 0..6 {
        let mut review = enabled();
        let control = RunControl::default();
        let mut request = request();
        request.control = BrowserDriverControl::new(control.clone(), RunControl::default());
        review.operation = Some((3, request.control.clone()));
        pending(&mut review, request);
        assert!(
            review
                .pending(ClientIdentityPolicy::AutomaticIfUnique)
                .unwrap()
                .is_none()
        );
        match cause {
            0 => {
                control.cancel();
            }
            1 => review.generation_changed(4),
            2 => review.cancel(Some(11)),
            3 => review.pending.as_mut().unwrap().request.deadline = Instant::now(),
            4 => review.pending.as_mut().unwrap().request.candidates[0].not_after = 1,
            _ => review.enable(false),
        }
        let (_, index) = review.ready().unwrap();
        assert_eq!(index, -1, "case {cause}");
        assert!(review.ready().is_none());
    }
}

#[test]
fn stale_origin_request_cannot_replace_cancel_or_replay_current_choice() {
    let mut review = enabled();
    let prior = request();
    let old_key = prior.key();
    pending(&mut review, prior);
    review.select(old_key, None).unwrap();
    assert_eq!(review.ready().unwrap().1, -1);
    let mut current = request();
    current.request_id += 1;
    current.origin = BrowserOrigin::parse("https://other.example").unwrap();
    let key = current.key();
    pending(&mut review, current);
    assert_eq!(
        review.select(old_key, Some("a".repeat(64))),
        Err(PresentationError::Stale)
    );
    review.cancel(Some(old_key.request_id));
    let challenge = review
        .pending(ClientIdentityPolicy::Review)
        .unwrap()
        .unwrap();
    assert_eq!(challenge.origin.as_str(), "https://other.example");
    review.select(key, Some("a".repeat(64))).unwrap();
    assert_eq!(review.ready().unwrap().1, 2);
}

#[test]
fn expired_candidates_are_recounted_and_review_deadline_is_never_extended() {
    let mut review = enabled();
    let mut expired = request();
    expired.candidates[0].not_after = 1;
    pending(&mut review, expired);
    assert!(
        review
            .pending(ClientIdentityPolicy::AutomaticIfUnique)
            .unwrap()
            .is_none()
    );
    assert_eq!(review.ready().unwrap().1, -1);
    pending(&mut review, request());
    let deadline = review.pending.as_ref().unwrap().request.deadline;
    for _ in 0..3 {
        review
            .pending(ClientIdentityPolicy::Review)
            .unwrap()
            .unwrap();
    }
    assert_eq!(review.pending.as_ref().unwrap().request.deadline, deadline);
    review.finish();
    assert!(
        review
            .pending(ClientIdentityPolicy::AutomaticIfUnique)
            .unwrap()
            .is_none()
    );
}

#[test]
fn native_review_attempt_budget_survives_auto_and_explicit_decisions() {
    let der = certificate("Eligible", true);
    let mut review = enabled();
    for _ in 0..8 {
        assert!(start(&mut review, &[&der]));
        review
            .select(review.request().unwrap().key(), None)
            .unwrap();
        assert_eq!(review.ready().unwrap().1, -1);
    }
    assert!(!start(&mut review, &[&der]));
    review.operation(
        4,
        BrowserDriverControl::new(RunControl::default(), RunControl::default()),
    );
    assert!(start(&mut review, &[&der]));
}
