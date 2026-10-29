use crate::TelemetryService;
use colossus_contracts::{
    Actor, ActorType, EventClassification, EventEnvelope, ExecutionContext, NewEvent,
    ProjectionWorkItem, ProviderUsage, RunTokenUsage, SignedCheckpoint,
};
use colossus_ports::{EventJournal, MAX_STREAM_READ_BATCH, StoreError, VerificationReport};
use colossus_testkit::InMemoryEventJournal;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

const RUN_ID: &str = "usage-run";
const SESSION_ID: &str = "usage-session";

type ReadTransform = Box<dyn Fn(&mut Vec<EventEnvelope>) + Send + Sync>;
type EventMutation = fn(&mut EventEnvelope);
type PageMutation = fn(&mut Vec<EventEnvelope>);

struct UsageJournal {
    inner: InMemoryEventJournal,
    ranged_reads: Mutex<Vec<(u64, usize)>>,
    page_transform: Option<ReadTransform>,
    tail_transform: Option<ReadTransform>,
    append_after_tail: AtomicBool,
}

impl Default for UsageJournal {
    fn default() -> Self {
        let inner = InMemoryEventJournal::default();
        inner.require_bounded_reads();
        Self {
            inner,
            ranged_reads: Mutex::new(Vec::new()),
            page_transform: None,
            tail_transform: None,
            append_after_tail: AtomicBool::new(false),
        }
    }
}

impl EventJournal for UsageJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.inner.append(event)
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.append_batch(events)
    }

    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_stream(stream_id)
    }

    fn read_stream_from(
        &self,
        stream_id: &str,
        after_version: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.ranged_reads
            .lock()
            .expect("ranged read log")
            .push((after_version, limit));
        let mut page = self
            .inner
            .read_stream_from(stream_id, after_version, limit)?;
        if let Some(transform) = &self.page_transform {
            transform(&mut page);
        }
        Ok(page)
    }

    fn read_stream_backwards(
        &self,
        stream_id: &str,
        before_version: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let mut page = self
            .inner
            .read_stream_backwards(stream_id, before_version, limit)?;
        if self.append_after_tail.swap(false, Ordering::AcqRel) {
            let prior_version = page
                .first()
                .expect("tail before concurrent append")
                .stream_version;
            self.inner.append(new_event(
                prior_version,
                "provider.usage.v1",
                usage_payload(&ProviderUsage {
                    input_tokens: 100,
                    output_tokens: 200,
                    total_tokens: 400,
                    cached_input_tokens: Some(0),
                    reasoning_tokens: Some(0),
                }),
            ))?;
        }
        if let Some(transform) = &self.tail_transform {
            transform(&mut page);
        }
        Ok(page)
    }

    fn list_stream_ids(
        &self,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        self.inner.list_stream_ids(prefix, after, limit)
    }

    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        self.inner.read_global(from_sequence, limit)
    }

    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        self.inner.read_projection_work(from_sequence, limit)
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        self.inner.head()
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        self.inner.decrypt_payload(event)
    }

    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.inner.verify()
    }

    fn is_recovery_mode(&self) -> bool {
        self.inner.is_recovery_mode()
    }

    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        self.inner.checkpoint()
    }
}

fn new_event(version: u64, event_type: &str, payload: Value) -> NewEvent {
    NewEvent {
        event_version: 1,
        stream_id: format!("run:{RUN_ID}"),
        expected_stream_version: version,
        classification: EventClassification::Domain,
        event_type: event_type.into(),
        actor: Actor {
            actor_type: ActorType::System,
            id: "usage-test".into(),
        },
        context: ExecutionContext {
            correlation_id: RUN_ID.into(),
            session_id: Some(SESSION_ID.into()),
            run_id: Some(RUN_ID.into()),
            ..ExecutionContext::default()
        },
        payload,
    }
}

fn append(journal: &dyn EventJournal, version: u64, event_type: &str, payload: Value) {
    journal
        .append(new_event(version, event_type, payload))
        .expect("append usage fixture");
}

fn started_journal() -> UsageJournal {
    let journal = UsageJournal::default();
    append(
        &journal,
        0,
        "run.started.v1",
        json!({"model": "fixture-model"}),
    );
    journal
}

fn known_zero() -> ProviderUsage {
    ProviderUsage {
        input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        cached_input_tokens: Some(0),
        reasoning_tokens: Some(0),
    }
}

fn usage_payload(usage: &ProviderUsage) -> Value {
    serde_json::to_value(usage).expect("usage JSON")
}

fn query(journal: UsageJournal) -> Result<RunTokenUsage, StoreError> {
    TelemetryService::new(Arc::new(journal)).run_token_usage(SESSION_ID, RUN_ID)
}

fn assert_verification(journal: UsageJournal, case: &str) {
    let result = query(journal);
    assert!(
        matches!(result, Err(StoreError::Verification(_))),
        "{case}: {result:?}"
    );
}

#[test]
fn exact_run_usage_pages_beyond_one_batch_without_global_or_unbounded_reads() {
    let journal = Arc::new(started_journal());
    let report = ProviderUsage {
        input_tokens: 2,
        output_tokens: 3,
        total_tokens: 11,
        cached_input_tokens: Some(0),
        reasoning_tokens: None,
    };
    let report_count = u64::try_from(MAX_STREAM_READ_BATCH).expect("batch fits") + 3;
    for version in 1..=report_count {
        append(
            journal.as_ref(),
            version,
            "provider.usage.v1",
            usage_payload(&report),
        );
    }
    let mut unrelated = new_event(0, "provider.usage.v1", usage_payload(&report));
    unrelated.stream_id = "run:another-run".into();
    unrelated.context.run_id = Some("another-run".into());
    journal.append(unrelated).expect("unrelated stream");

    let summary = TelemetryService::new(journal.clone())
        .run_token_usage(SESSION_ID, RUN_ID)
        .expect("complete exact usage");
    assert_eq!(summary.run_id, RUN_ID);
    assert_eq!(summary.session_id, SESSION_ID);
    assert_eq!(summary.through_version, report_count + 1);
    assert_eq!(summary.reports, report_count);
    assert_eq!(
        summary.usage,
        Some(ProviderUsage {
            input_tokens: report_count * 2,
            output_tokens: report_count * 3,
            total_tokens: report_count * 11,
            cached_input_tokens: Some(0),
            reasoning_tokens: None,
        })
    );
    let reads = journal.ranged_reads.lock().expect("read log");
    assert!(reads.len() > 1);
    assert_eq!(reads.first().expect("first read").0, 0);
    assert!(
        reads
            .iter()
            .all(|(_, limit)| (1..=MAX_STREAM_READ_BATCH).contains(limit))
    );
}

#[test]
fn zero_reports_remain_absent_while_reported_zero_is_explicit() {
    let journal = started_journal();
    let absent = query(journal).expect("run without accounting");
    assert_eq!(absent.through_version, 1);
    assert_eq!(absent.reports, 0);
    assert_eq!(absent.usage, None);

    let journal = started_journal();
    append(
        &journal,
        1,
        "provider.usage.v1",
        usage_payload(&known_zero()),
    );
    let zero = query(journal).expect("reported zero");
    assert_eq!(zero.reports, 1);
    assert_eq!(zero.usage, Some(known_zero()));
}

#[test]
fn missing_optional_details_propagate_independently_across_reports() {
    for (cached_input_tokens, reasoning_tokens) in [(None, Some(0)), (Some(0), None), (None, None)]
    {
        let journal = started_journal();
        append(
            &journal,
            1,
            "provider.usage.v1",
            usage_payload(&known_zero()),
        );
        let unknown = ProviderUsage {
            cached_input_tokens,
            reasoning_tokens,
            ..known_zero()
        };
        append(&journal, 2, "provider.usage.v1", usage_payload(&unknown));
        append(
            &journal,
            3,
            "provider.usage.v1",
            usage_payload(&known_zero()),
        );
        let summary = query(journal).expect("optional accounting");
        assert_eq!(summary.reports, 3);
        assert_eq!(summary.usage, Some(unknown));
    }
}

#[test]
fn failure_and_cancellation_keep_already_reported_usage() {
    for event_type in ["error.v1", "run.cancelled.v1"] {
        let journal = started_journal();
        let report = ProviderUsage {
            input_tokens: 5,
            output_tokens: 2,
            total_tokens: 9,
            ..known_zero()
        };
        append(&journal, 1, "provider.usage.v1", usage_payload(&report));
        append(
            &journal,
            2,
            event_type,
            json!({"turn": 1, "recoverable": false}),
        );
        let summary = query(journal).expect("reported usage survives terminal outcome");
        assert_eq!(summary.through_version, 3);
        assert_eq!(summary.reports, 1);
        assert_eq!(summary.usage, Some(report));
    }
}

#[test]
fn absent_and_invalid_start_markers_do_not_become_zero_usage() {
    assert!(matches!(
        query(UsageJournal::default()),
        Err(StoreError::NotFound(_))
    ));
    for first_type in ["provider.usage.v1", "model.delta.v1"] {
        let journal = UsageJournal::default();
        append(&journal, 0, first_type, usage_payload(&known_zero()));
        assert_verification(journal, first_type);
    }
    let journal = started_journal();
    append(&journal, 1, "run.started.v1", json!({}));
    assert_verification(journal, "repeated start marker");
}

#[test]
fn both_tail_and_ranged_events_require_exact_identity_and_versions() {
    let mutations: [(&str, EventMutation); 8] = [
        ("session", |event| {
            event.context.session_id = Some("other-session".into())
        }),
        ("missing session", |event| event.context.session_id = None),
        ("run", |event| {
            event.context.run_id = Some("other-run".into())
        }),
        ("missing run", |event| event.context.run_id = None),
        ("stream", |event| event.stream_id = "run:other-run".into()),
        ("schema", |event| event.schema_version = 2),
        ("event version", |event| event.event_version = 2),
        ("zero version", |event| event.stream_version = 0),
    ];
    for tail_read in [false, true] {
        for (case, mutate) in mutations {
            let mut journal = started_journal();
            append(
                &journal,
                1,
                "provider.usage.v1",
                usage_payload(&known_zero()),
            );
            let transform: ReadTransform = Box::new(move |page| {
                mutate(page.first_mut().expect("event to corrupt"));
            });
            if tail_read {
                journal.tail_transform = Some(transform);
            } else {
                journal.page_transform = Some(transform);
            }
            assert_verification(journal, case);
        }
    }
}

#[test]
fn wrong_requested_session_or_empty_identity_is_rejected() {
    let service = TelemetryService::new(Arc::new(started_journal()));
    for (session_id, run_id) in [("other-session", RUN_ID), ("", RUN_ID), (SESSION_ID, "")] {
        assert!(matches!(
            service.run_token_usage(session_id, run_id),
            Err(StoreError::Verification(_))
        ));
    }
    assert!(matches!(
        service.run_token_usage(SESSION_ID, "another-run"),
        Err(StoreError::NotFound(_))
    ));
}

#[test]
fn gaps_duplicates_truncation_and_oversized_pages_fail_closed() {
    let mutations: [(&str, PageMutation); 5] = [
        ("empty page", Vec::clear),
        ("gap", |page| {
            page.remove(1);
        }),
        ("duplicate", |page| page[1] = page[0].clone()),
        ("missing tail", |page| {
            page.pop();
        }),
        ("oversized page", |page| page.push(page[0].clone())),
    ];
    for (case, mutate) in mutations {
        let mut journal = started_journal();
        for version in 1..=3 {
            append(
                &journal,
                version,
                "provider.usage.v1",
                usage_payload(&known_zero()),
            );
        }
        journal.page_transform = Some(Box::new(mutate));
        assert_verification(journal, case);
    }
    let mut journal = started_journal();
    journal.tail_transform = Some(Box::new(|page| page.push(page[0].clone())));
    assert_verification(journal, "oversized frozen tail");
}

#[test]
fn malformed_typed_usage_counters_and_unknown_fields_fail_closed() {
    for (field, invalid) in [
        ("input_tokens", json!(-1)),
        ("output_tokens", json!("2")),
        ("total_tokens", json!(1.5)),
        ("cached_input_tokens", json!(-1)),
        ("reasoning_tokens", json!(true)),
        ("unexpected", json!(0)),
    ] {
        let journal = started_journal();
        let mut payload = usage_payload(&known_zero());
        payload[field] = invalid;
        append(&journal, 1, "provider.usage.v1", payload);
        assert_verification(journal, field);
    }
    let journal = started_journal();
    let mut payload = usage_payload(&known_zero());
    payload
        .as_object_mut()
        .expect("usage object")
        .remove("input_tokens");
    append(&journal, 1, "provider.usage.v1", payload);
    assert_verification(journal, "missing required input counter");
}

#[test]
fn required_and_known_optional_sum_overflow_fail_closed() {
    let increment = ProviderUsage {
        input_tokens: 1,
        output_tokens: 1,
        total_tokens: 1,
        cached_input_tokens: Some(1),
        reasoning_tokens: Some(1),
    };
    for field in [
        "input_tokens",
        "output_tokens",
        "total_tokens",
        "cached_input_tokens",
        "reasoning_tokens",
    ] {
        let journal = started_journal();
        let mut maximum = usage_payload(&increment);
        maximum[field] = json!(u64::MAX);
        append(&journal, 1, "provider.usage.v1", maximum);
        append(&journal, 2, "provider.usage.v1", usage_payload(&increment));
        assert_verification(journal, field);
    }
}

#[test]
fn frozen_tail_excludes_a_legitimate_append_until_the_next_query() {
    let journal = started_journal();
    append(
        &journal,
        1,
        "provider.usage.v1",
        usage_payload(&known_zero()),
    );
    journal.append_after_tail.store(true, Ordering::Release);
    let service = TelemetryService::new(Arc::new(journal));
    let frozen = service
        .run_token_usage(SESSION_ID, RUN_ID)
        .expect("frozen query");
    assert_eq!(frozen.through_version, 2);
    assert_eq!(frozen.reports, 1);
    assert_eq!(frozen.usage, Some(known_zero()));

    let next = service
        .run_token_usage(SESSION_ID, RUN_ID)
        .expect("next query");
    assert_eq!(next.through_version, 3);
    assert_eq!(next.reports, 2);
    assert_eq!(
        next.usage,
        Some(ProviderUsage {
            input_tokens: 100,
            output_tokens: 200,
            total_tokens: 400,
            cached_input_tokens: Some(0),
            reasoning_tokens: Some(0),
        })
    );
}

#[test]
fn ranged_read_cannot_replace_the_event_at_the_frozen_tail_version() {
    let mut journal = started_journal();
    append(
        &journal,
        1,
        "provider.usage.v1",
        usage_payload(&known_zero()),
    );
    journal.page_transform = Some(Box::new(|page| {
        page.last_mut().expect("frozen tail").record_hash.push('x');
    }));
    assert_verification(journal, "changed frozen tail identity");
}
