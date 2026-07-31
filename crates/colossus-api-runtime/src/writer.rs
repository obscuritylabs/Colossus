use crate::feed::RunFeeds;
use colossus_api::{
    ApiError, ApiErrorCode, ApiErrorReason, ApiResult, CallerContext, IdempotencyKey, Interaction,
    InteractionResponse, OutcomeCertainty, Run, RunRepository, RunUpdate, RunUpdateKind,
};
use std::sync::{Arc, Mutex, MutexGuard};

pub(super) struct RunWriter {
    repository: Arc<dyn RunRepository>,
    feeds: Arc<RunFeeds>,
    caller: CallerContext,
    run_id: String,
    sequence: Mutex<u64>,
}

impl RunWriter {
    pub(super) fn new(
        repository: Arc<dyn RunRepository>,
        feeds: Arc<RunFeeds>,
        caller: CallerContext,
        run: &Run,
    ) -> Self {
        Self {
            repository,
            feeds,
            caller,
            run_id: run.id.clone(),
            sequence: Mutex::new(run.last_sequence),
        }
    }

    pub(super) fn append(&self, kind: RunUpdateKind) -> ApiResult<RunUpdate> {
        let mut sequence = lock(&self.sequence);
        let terminal = update_is_terminal(&kind);
        let update = self
            .repository
            .append_update(&self.caller, &self.run_id, *sequence, kind)?;
        *sequence = update.sequence;
        self.feeds.publish(&self.run_id, update.sequence, terminal);
        Ok(update)
    }

    pub(super) fn request_cancellation(
        &self,
        caller: &CallerContext,
        idempotency_key: &IdempotencyKey,
    ) -> ApiResult<Run> {
        let mut sequence = lock(&self.sequence);
        let result = self
            .repository
            .request_cancellation(caller, &self.run_id, idempotency_key)?;
        *sequence = result.value.last_sequence;
        self.feeds.publish(
            &self.run_id,
            result.value.last_sequence,
            result.value.status.is_terminal(),
        );

        Ok(result.value)
    }

    pub(super) fn respond_interaction(
        &self,
        caller: &CallerContext,
        interaction_id: &str,
        etag: &str,
        idempotency_key: &IdempotencyKey,
        response: InteractionResponse,
    ) -> ApiResult<Interaction> {
        let mut sequence = lock(&self.sequence);
        let replay_response = response.clone();
        let result = match self.repository.respond_interaction(
            caller,
            &self.run_id,
            interaction_id,
            etag,
            idempotency_key,
            response,
        ) {
            Ok(result) => result,
            Err(error) if error.reason == ApiErrorReason::OutcomeUnknown => {
                match self.repository.resolve_interaction_response(
                    caller,
                    &self.run_id,
                    interaction_id,
                    etag,
                    idempotency_key,
                    &replay_response,
                ) {
                    Ok(Some(interaction)) => {
                        self.synchronize_locked(&mut sequence)?;
                        return Ok(interaction);
                    }
                    Ok(None) | Err(_) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        if !result.replayed {
            *sequence = sequence.saturating_add(1);
            self.feeds.publish(&self.run_id, *sequence, false);
        } else {
            self.synchronize_locked(&mut sequence)?;
        }
        Ok(result.value)
    }

    pub(super) fn synchronize_durable_state(&self) -> ApiResult<Run> {
        let mut sequence = lock(&self.sequence);
        self.synchronize_locked(&mut sequence)
    }

    pub(super) fn run_id(&self) -> &str {
        &self.run_id
    }

    pub(super) fn caller(&self) -> &CallerContext {
        &self.caller
    }

    pub(super) fn current_run(&self) -> ApiResult<Option<Run>> {
        self.repository.get_run(&self.caller, &self.run_id)
    }

    pub(super) fn current_execution_run(&self) -> ApiResult<Option<Run>> {
        self.repository
            .recoverable_run(&self.caller, &self.run_id)
            .map(|current| current.map(|(run, _)| run))
    }

    fn synchronize_locked(&self, sequence: &mut u64) -> ApiResult<Run> {
        let current = self
            .repository
            .recoverable_run(&self.caller, &self.run_id)?
            .map(|(run, _)| run)
            .ok_or_else(|| {
                reconciliation_error(
                    &self.caller,
                    "the active run disappeared during durable reconciliation",
                )
            })?;
        if current.last_sequence < *sequence {
            return Err(reconciliation_error(
                &self.caller,
                "the durable run sequence regressed during reconciliation",
            ));
        }
        *sequence = current.last_sequence;
        self.feeds.publish(
            &self.run_id,
            current.last_sequence,
            current.status.is_terminal(),
        );
        Ok(current)
    }
}

fn reconciliation_error(caller: &CallerContext, message: &str) -> ApiError {
    ApiError {
        code: ApiErrorCode::Internal,
        reason: ApiErrorReason::InternalInvariant,
        message: message.into(),
        correlation_id: Some(caller.request_id().clone()),
        retryable: false,
        outcome: OutcomeCertainty::Known,
        violations: Vec::new(),
    }
}

fn update_is_terminal(kind: &RunUpdateKind) -> bool {
    matches!(
        kind,
        RunUpdateKind::Result { .. }
            | RunUpdateKind::Failure { .. }
            | RunUpdateKind::Cancellation { .. }
    )
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::RunFeeds;
    use colossus_api::{
        ApiErrorReason, ApiScope, ApplicationKind, ApplicationPrincipal, ContentPart,
        CreateRunRequest, EventSourcedRunRepository, IdempotencyKey, InteractionKind,
        InteractionStatus, NewRun, RequestId, RunMode, RunStatus, scopes,
    };
    use colossus_contracts::{EventEnvelope, NewEvent, ProjectionWorkItem, SignedCheckpoint};
    use colossus_ports::{EventJournal, StoreError, VerificationReport};
    use colossus_testkit::InMemoryEventJournal;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct CommitThenOutcomeUnknownJournal {
        inner: InMemoryEventJournal,
        fail_next_batch_after_commit: AtomicBool,
    }

    impl CommitThenOutcomeUnknownJournal {
        fn fail_next_batch_after_commit(&self) {
            self.fail_next_batch_after_commit
                .store(true, Ordering::Release);
        }
    }

    impl EventJournal for CommitThenOutcomeUnknownJournal {
        fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
            self.inner.append(event)
        }

        fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
            let persisted = self.inner.append_batch(events)?;
            if self
                .fail_next_batch_after_commit
                .swap(false, Ordering::AcqRel)
            {
                Err(StoreError::OutcomeUnknown(
                    "the test batch committed before acknowledgement".into(),
                ))
            } else {
                Ok(persisted)
            }
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
            self.inner.read_stream_from(stream_id, after_version, limit)
        }

        fn read_stream_backwards(
            &self,
            stream_id: &str,
            before_version: Option<u64>,
            limit: usize,
        ) -> Result<Vec<EventEnvelope>, StoreError> {
            self.inner
                .read_stream_backwards(stream_id, before_version, limit)
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

        fn decrypt_payload(&self, event: &EventEnvelope) -> Result<serde_json::Value, StoreError> {
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

    fn caller(application_id: &str, request_id: &str, granted: &[&str]) -> CallerContext {
        CallerContext::authenticated(
            ApplicationPrincipal::authenticated(
                application_id,
                format!("credential-{request_id}"),
                ApplicationKind::Enrolled,
                granted
                    .iter()
                    .map(|scope| ApiScope::new(*scope).expect("scope")),
                ["assistant".to_owned()],
                std::iter::empty(),
            )
            .expect("principal"),
            RequestId::new(request_id).expect("request id"),
        )
    }

    fn writer_fixture() -> (
        Arc<dyn RunRepository>,
        Arc<RunFeeds>,
        CallerContext,
        Run,
        RunWriter,
    ) {
        let durable: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let repository: Arc<dyn RunRepository> = Arc::new(EventSourcedRunRepository::new(durable));
        let creator = caller(
            "app:desktop-ui",
            "create-request",
            &[
                scopes::RUNS_EXECUTE,
                scopes::RUNS_READ,
                scopes::RUNS_CONTROL,
                scopes::PROMPTS_RESPOND,
            ],
        );
        let request = CreateRunRequest {
            input: vec![ContentPart::Text {
                text: "hello".into(),
            }],
            session_id: None,
            role: Some("assistant".into()),
            mode: RunMode::Execute,
            skill_ids: Vec::new(),
            plan_action: None,
            max_turns: 1,
            idempotency_key: IdempotencyKey::new("create-key").expect("key"),
        };
        let new_run =
            NewRun::from_request("run-1", "session-1", "assistant", &request).expect("new run");
        let run = repository
            .create_run(&creator, &request, &new_run)
            .expect("create")
            .value;
        let feeds = Arc::new(RunFeeds::default());
        let writer = RunWriter::new(
            Arc::clone(&repository),
            Arc::clone(&feeds),
            creator.clone(),
            &run,
        );
        (repository, feeds, creator, run, writer)
    }

    #[test]
    fn cancellation_authorizes_the_requesting_caller() {
        let (repository, _, creator, _, writer) = writer_fixture();
        let unprivileged = caller("app:desktop-ui", "unprivileged", &[scopes::RUNS_READ]);
        let error = writer
            .request_cancellation(
                &unprivileged,
                &IdempotencyKey::new("cancel-denied").expect("key"),
            )
            .expect_err("current caller lacks control scope");
        assert_eq!(error.reason, ApiErrorReason::ScopeDenied);
        assert_eq!(
            repository
                .get_run(&creator, "run-1")
                .expect("get")
                .expect("run")
                .status,
            RunStatus::Queued
        );

        let controller = caller("app:desktop-ui", "controller", &[scopes::RUNS_CONTROL]);
        let cancelled = writer
            .request_cancellation(
                &controller,
                &IdempotencyKey::new("cancel-allowed").expect("key"),
            )
            .expect("control scope");
        assert_eq!(cancelled.status, RunStatus::Cancelling);
    }

    #[test]
    fn interaction_response_authorizes_the_requesting_caller() {
        let (repository, _, creator, _, writer) = writer_fixture();
        writer
            .append(RunUpdateKind::State {
                status: RunStatus::Running,
            })
            .expect("running");
        writer
            .append(RunUpdateKind::Interaction {
                interaction: Interaction {
                    id: "interaction-1".into(),
                    kind: InteractionKind::Prompt,
                    status: InteractionStatus::Pending,
                    application_id: "app:desktop-ui".into(),
                    created_at: "2026-01-01T00:00:00Z".into(),
                    prompt: "Continue?".into(),
                    choices: Vec::new(),
                    allow_free_form: true,
                    request_hash: None,
                    action: None,
                    resource: None,
                    risk: None,
                    expires_at: "2999-01-01T00:00:00Z".into(),
                    response: None,
                    responded_at: None,
                },
            })
            .expect("interaction");
        let waiting = repository
            .get_run(&creator, "run-1")
            .expect("get")
            .expect("run");
        let unprivileged = caller(
            "app:desktop-ui",
            "unprivileged",
            std::slice::from_ref(&scopes::RUNS_READ),
        );
        let response = InteractionResponse::Prompt {
            answer: "yes".into(),
            selected_index: None,
        };
        let error = writer
            .respond_interaction(
                &unprivileged,
                "interaction-1",
                &waiting.etag,
                &IdempotencyKey::new("response-denied").expect("key"),
                response.clone(),
            )
            .expect_err("current caller lacks prompt scope");
        assert_eq!(error.reason, ApiErrorReason::ScopeDenied);

        let responder = caller("app:desktop-ui", "responder", &[scopes::PROMPTS_RESPOND]);
        let resolved = writer
            .respond_interaction(
                &responder,
                "interaction-1",
                &waiting.etag,
                &IdempotencyKey::new("response-allowed").expect("key"),
                response,
            )
            .expect("prompt scope");
        assert_eq!(resolved.status, InteractionStatus::Responded);
    }

    #[test]
    fn committed_outcome_unknown_response_reconciles_cursor_before_execution_resumes() {
        let journal = Arc::new(CommitThenOutcomeUnknownJournal::default());
        let durable: Arc<dyn EventJournal> = journal.clone();
        let repository: Arc<dyn RunRepository> = Arc::new(EventSourcedRunRepository::new(durable));
        let creator = caller(
            "app:desktop-ui",
            "outcome-unknown-create",
            &[
                scopes::RUNS_EXECUTE,
                scopes::RUNS_READ,
                scopes::PROMPTS_RESPOND,
            ],
        );
        let request = CreateRunRequest {
            input: vec![ContentPart::Text {
                text: "hello".into(),
            }],
            session_id: None,
            role: Some("assistant".into()),
            mode: RunMode::Execute,
            skill_ids: Vec::new(),
            plan_action: None,
            max_turns: 1,
            idempotency_key: IdempotencyKey::new("outcome-unknown-create-key").expect("key"),
        };
        let new_run =
            NewRun::from_request("run-outcome-unknown", "session-1", "assistant", &request)
                .expect("new run");
        let run = repository
            .create_run(&creator, &request, &new_run)
            .expect("create")
            .value;
        let writer = RunWriter::new(
            Arc::clone(&repository),
            Arc::new(RunFeeds::default()),
            creator.clone(),
            &run,
        );
        writer
            .append(RunUpdateKind::State {
                status: RunStatus::Running,
            })
            .expect("running");
        writer
            .append(RunUpdateKind::Interaction {
                interaction: Interaction {
                    id: "interaction-outcome-unknown".into(),
                    kind: InteractionKind::Prompt,
                    status: InteractionStatus::Pending,
                    application_id: "app:desktop-ui".into(),
                    created_at: "2026-01-01T00:00:00Z".into(),
                    prompt: "Continue?".into(),
                    choices: vec!["Yes".into()],
                    allow_free_form: false,
                    request_hash: None,
                    action: None,
                    resource: None,
                    risk: None,
                    expires_at: "2999-01-01T00:00:00Z".into(),
                    response: None,
                    responded_at: None,
                },
            })
            .expect("interaction");
        let waiting = repository
            .get_run(&creator, "run-outcome-unknown")
            .expect("get waiting run")
            .expect("run");

        journal.fail_next_batch_after_commit();
        let resolved = writer
            .respond_interaction(
                &creator,
                "interaction-outcome-unknown",
                &waiting.etag,
                &IdempotencyKey::new("outcome-unknown-response-key").expect("key"),
                InteractionResponse::Prompt {
                    answer: "Yes".into(),
                    selected_index: Some(0),
                },
            )
            .expect("committed response is reconciled");
        assert_eq!(resolved.status, InteractionStatus::Responded);

        writer
            .append(RunUpdateKind::State {
                status: RunStatus::Running,
            })
            .expect("writer cursor was synchronized before resuming");
        let resumed = repository
            .get_run(&creator, "run-outcome-unknown")
            .expect("get resumed run")
            .expect("run");
        assert_eq!(resumed.status, RunStatus::Running);
        assert_eq!(resumed.last_sequence, 5);
    }
}
