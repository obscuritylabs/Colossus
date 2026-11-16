//! Durable state for a local, explicitly enrolled standing chat.

use crate::{Runtime, RuntimeError, tool_arguments::terminal_actor};
use colossus_contracts::{
    Actor, ActorType, AgentRunMode, AgentRunOutcome, EventClassification, ExecutionContext,
    IntegrationStatus, NewEvent, RunEventEnvelope, TaskStatus,
    always_on::{
        AlwaysOnProfile, AlwaysOnTrigger, AlwaysOnWake, AlwaysOnWakeResult, AlwaysOnWakeStatus,
    },
};
use colossus_ports::{EventJournal, ModelProviderError, RunControl, RunEventObserver, StoreError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const STREAM: &str = "always-on:workspace";
const EVENT: &str = "always_on.profile.updated.v1";
const INTERVAL: Duration = Duration::minutes(30);
const TASK_LIMIT: usize = 100;
const CONNECTION_LIMIT: usize = 100;

/// A canonical wake with its exact private input for server-side dispatch.
pub struct AlwaysOnWakeClaim {
    /// Released wake identity and timing.
    pub wake: AlwaysOnWake,
    /// Exact already-owned conversation to continue.
    pub session_id: String,
    /// Fixed instructions and bounded status captured with the claim.
    pub prompt: String,
    /// This wake was already claimed. The dispatcher must reconcile its existing
    /// execution and must not allocate a replacement when its outcome is uncertain.
    pub resumed: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    owner: Actor,
    profile: AlwaysOnProfile,
    schedule_revision: u64,
    claimed_prompt: Option<String>,
}

impl Runtime {
    /// Test whether a stored application run is the current owned wake.
    ///
    /// Foreign owners receive no match. Callers use this only to distinguish
    /// worker-created wakes from ordinary run idempotency keys.
    pub fn matches_pending_always_on_wake(
        &self,
        owner: &Actor,
        session_id: Option<&str>,
        idempotency_key: &str,
    ) -> Result<bool, RuntimeError> {
        Ok(read_record(self.journal.as_ref())?.is_some_and(|record| {
            &record.owner == owner
                && session_id == Some(record.profile.session_id.as_str())
                && record
                    .profile
                    .pending
                    .is_some_and(|wake| wake.idempotency_key() == idempotency_key)
        }))
    }

    /// Read the owned profile from this workspace's canonical journal.
    ///
    /// The host must derive the owner from the verified transport, never request data.
    pub fn always_on_profile(
        &self,
        owner: &Actor,
    ) -> Result<Option<AlwaysOnProfile>, RuntimeError> {
        Ok(self.always_on_record(owner)?.map(|record| record.profile))
    }

    /// Enroll one existing, owned, unarchived conversation or resume an enrollment.
    /// Revision zero means no enrollment exists. A paused profile can enroll another
    /// owned conversation once all previous wakes have settled.
    pub fn enable_always_on(
        &self,
        owner: &Actor,
        session_id: &str,
        expected_revision: u64,
    ) -> Result<AlwaysOnProfile, RuntimeError> {
        self.enable_always_on_at(
            owner,
            session_id,
            expected_revision,
            OffsetDateTime::now_utc(),
        )
    }

    fn enable_always_on_at(
        &self,
        owner: &Actor,
        session_id: &str,
        expected_revision: u64,
        now: OffsetDateTime,
    ) -> Result<AlwaysOnProfile, RuntimeError> {
        let existing = self.always_on_record(owner)?;
        check_revision(
            existing
                .as_ref()
                .map_or(0, |record| record.profile.revision),
            expected_revision,
        )?;
        if !self.always_on_session_available(owner, session_id)? {
            return Err(unavailable());
        }
        let mut record = match existing {
            Some(record) => record,
            None => Record {
                owner: owner.clone(),
                profile: AlwaysOnProfile {
                    session_id: session_id.into(),
                    enabled: false,
                    revision: 0,
                    next_wake_at: String::new(),
                    pending: None,
                    last_wake: None,
                },
                schedule_revision: 0,
                claimed_prompt: None,
            },
        };
        if record.profile.enabled {
            return if record.profile.session_id == session_id {
                Ok(record.profile)
            } else {
                Err(RuntimeError::Config(
                    "Pause Always On before enrolling another conversation".into(),
                ))
            };
        }
        if record.profile.pending.is_some() {
            return Err(RuntimeError::Config(
                "An Always On wake is still unresolved".into(),
            ));
        }
        record.profile.session_id = session_id.into();
        record.profile.enabled = true;
        record.schedule_revision = record
            .profile
            .revision
            .checked_add(1)
            .ok_or_else(invalid_record)?;
        record.profile.next_wake_at = timestamp(now + INTERVAL)?;
        self.write_always_on(record, owner)
    }

    /// Pause future wakes at the requested revision. An already claimed wake keeps
    /// its identity for reconciliation.
    pub fn pause_always_on(
        &self,
        owner: &Actor,
        expected_revision: u64,
    ) -> Result<AlwaysOnProfile, RuntimeError> {
        let mut record = self.always_on_record(owner)?.ok_or_else(unavailable)?;
        check_revision(record.profile.revision, expected_revision)?;
        if !record.profile.enabled {
            return Ok(record.profile);
        }
        record.profile.enabled = false;
        if record.claimed_prompt.is_none() && record.profile.pending.is_some() {
            record.settle(AlwaysOnWakeStatus::Interrupted, OffsetDateTime::now_utc())?;
        }
        self.write_always_on(record, owner)
    }

    /// Queue one explicit wake. A second request returns the existing unresolved
    /// occurrence after the caller refreshes its expected revision.
    pub fn request_always_on_wake(
        &self,
        owner: &Actor,
        expected_revision: u64,
    ) -> Result<AlwaysOnProfile, RuntimeError> {
        let mut record = self.always_on_record(owner)?.ok_or_else(unavailable)?;
        check_revision(record.profile.revision, expected_revision)?;
        if !record.profile.enabled
            || !self.always_on_session_available(owner, &record.profile.session_id)?
        {
            return Err(unavailable());
        }
        if record.profile.pending.is_some() {
            return Ok(record.profile);
        }
        record.profile.pending = Some(record.wake(
            AlwaysOnTrigger::Manual,
            timestamp(OffsetDateTime::now_utc())?,
            &record.profile.revision.to_string(),
        ));
        self.write_always_on(record, owner)
    }

    /// Atomically advance the due schedule and persist one claim with its immutable
    /// input. The worker passes its currently authorized local owner. A prior claim
    /// is returned only for reconciliation, including when the profile is paused.
    pub fn claim_always_on_wake(
        &self,
        owner: &Actor,
    ) -> Result<Option<AlwaysOnWakeClaim>, RuntimeError> {
        self.claim_always_on_at(owner, OffsetDateTime::now_utc())
    }

    fn claim_always_on_at(
        &self,
        owner: &Actor,
        now: OffsetDateTime,
    ) -> Result<Option<AlwaysOnWakeClaim>, RuntimeError> {
        let Some(mut record) = self.always_on_record(owner)? else {
            return Ok(None);
        };
        if let (Some(wake), Some(prompt)) = (&record.profile.pending, &record.claimed_prompt) {
            return Ok(Some(AlwaysOnWakeClaim {
                wake: wake.clone(),
                session_id: record.profile.session_id,
                prompt: prompt.clone(),
                resumed: true,
            }));
        }
        if !record.profile.enabled {
            return Ok(None);
        }
        if record.profile.pending.is_none() {
            let next = parse_timestamp(&record.profile.next_wake_at)?;
            if now < next {
                return Ok(None);
            }
            // Fire only the latest missed occurrence and preserve the initial phase.
            let elapsed = (now - next).whole_seconds();
            let cadence = INTERVAL.whole_seconds();
            let boundary = timestamp(next + Duration::seconds((elapsed / cadence) * cadence))?;
            record.profile.next_wake_at =
                timestamp(next + Duration::seconds((elapsed / cadence + 1) * cadence))?;
            let identity = format!("{}:{boundary}", record.schedule_revision);
            record.profile.pending =
                Some(record.wake(AlwaysOnTrigger::Scheduled, boundary, &identity));
        }
        let session_id = record.profile.session_id.clone();
        if self
            .native_session_lifecycle
            .has_live_primary_run(&session_id)?
        {
            return Ok(None);
        }
        // Collect bounded status before taking the archive/run admission fence.
        let prompt = self
            .always_on_session_available(owner, &session_id)?
            .then(|| self.always_on_prompt(&session_id))
            .transpose()?;
        self.native_session_lifecycle
            .with_always_on_admission(&session_id, |busy| {
                if !self.always_on_session_available(owner, &session_id)? {
                    record.settle(AlwaysOnWakeStatus::Blocked, now)?;
                    self.write_always_on(record, &scheduler_actor())?;
                    return Ok(None);
                }
                let Some(prompt) = prompt.filter(|_| !busy) else {
                    return Ok(None);
                };
                let wake = record.profile.pending.clone().ok_or_else(invalid_record)?;
                record.claimed_prompt = Some(prompt.clone());
                self.write_always_on(record, &scheduler_actor())?;
                Ok(Some(AlwaysOnWakeClaim {
                    wake,
                    session_id: session_id.clone(),
                    prompt,
                    resumed: false,
                }))
            })
    }

    /// Record the outcome of the exact claimed wake. Settlement is repeatable for
    /// the same result and releases only the matching unresolved occurrence.
    pub fn finish_always_on_wake(
        &self,
        owner: &Actor,
        wake_id: &str,
        status: AlwaysOnWakeStatus,
    ) -> Result<AlwaysOnProfile, RuntimeError> {
        let mut record = self.always_on_record(owner)?.ok_or_else(unavailable)?;
        if record
            .profile
            .last_wake
            .as_ref()
            .is_some_and(|last| last.id == wake_id && last.status == status)
        {
            return Ok(record.profile);
        }
        if record.claimed_prompt.is_none()
            || record
                .profile
                .pending
                .as_ref()
                .is_none_or(|wake| wake.id != wake_id)
        {
            return Err(unavailable());
        }
        record.settle(status, OffsetDateTime::now_utc())?;
        self.write_always_on(record, &scheduler_actor())
    }

    fn always_on_record(&self, owner: &Actor) -> Result<Option<Record>, RuntimeError> {
        if !valid_owner(owner) {
            return Err(unavailable());
        }
        let record = read_record(self.journal.as_ref())?;
        if record.as_ref().is_some_and(|record| &record.owner != owner) {
            return Err(unavailable());
        }
        Ok(record)
    }

    fn write_always_on(
        &self,
        mut record: Record,
        actor: &Actor,
    ) -> Result<AlwaysOnProfile, RuntimeError> {
        let expected_stream_version = record.profile.revision;
        record.profile.revision = expected_stream_version
            .checked_add(1)
            .ok_or_else(invalid_record)?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: STREAM.into(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: EVENT.into(),
            actor: actor.clone(),
            context: ExecutionContext {
                correlation_id: Uuid::now_v7().to_string(),
                session_id: Some(record.profile.session_id.clone()),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(&record).map_err(|_| invalid_record())?,
        })?;
        Ok(record.profile)
    }

    fn always_on_session_available(
        &self,
        owner: &Actor,
        session_id: &str,
    ) -> Result<bool, RuntimeError> {
        if !Uuid::parse_str(session_id).is_ok_and(|id| id.to_string() == session_id) {
            return Ok(false);
        }
        let created = self
            .journal
            .read_stream_from(&format!("session:{session_id}"), 0, 1)?;
        if !created
            .first()
            .is_some_and(|event| event.event_type == "session.created.v1" && &event.actor == owner)
        {
            return Ok(false);
        }
        Ok(self
            .session_lifecycle(session_id)?
            .is_some_and(|session| !session.archived))
    }

    fn always_on_prompt(&self, session_id: &str) -> Result<String, RuntimeError> {
        let tasks = self.list_tasks(Some(session_id), None, TASK_LIMIT)?;
        let open = tasks
            .iter()
            .filter(|task| !matches!(task.status, TaskStatus::Completed | TaskStatus::Cancelled))
            .count();
        let blocked = tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Blocked)
            .count();
        let connections = self.list_integrations(CONNECTION_LIMIT)?;
        let unavailable = connections
            .iter()
            .filter(|connection| connection.status != IntegrationStatus::Connected)
            .count();
        Ok(format!(
            concat!(
                "Scheduled or manually requested Always On status check. Summarize these observed ",
                "local counts briefly and flag work that may need attention. The counts are bounded ",
                "samples of recorded status. No live connection probe was performed. Tasks sampled: ",
                "{}. Open: {}. Blocked: {}. Connections sampled: {}. Recorded as ",
                "disconnected or needing authentication: {}.",
            ),
            tasks.len(),
            open,
            blocked,
            connections.len(),
            unavailable,
        ))
    }

    /// Execute the confirmed fresh wake of this workspace's ordinary local terminal
    /// owner. On a returned previous claim, inspect durable run evidence only.
    /// Tools and skills are unavailable during this status-only turn.
    pub async fn run_always_on_local_wake(
        &self,
        claim: &AlwaysOnWakeClaim,
    ) -> Result<AlwaysOnWakeStatus, RuntimeError> {
        let owner = terminal_actor();
        let record = self.always_on_record(&owner)?.ok_or_else(unavailable)?;
        if record.profile.session_id != claim.session_id
            || record.profile.pending.as_ref() != Some(&claim.wake)
            || record.claimed_prompt.as_deref() != Some(claim.prompt.as_str())
        {
            return Err(unavailable());
        }
        let previous = self
            .journal
            .read_stream_backwards(&format!("run:{}", claim.wake.id), None, 1)?
            .pop();
        if claim.resumed || previous.is_some() {
            return Ok(
                match previous.as_ref().map(|event| event.event_type.as_str()) {
                    Some("run.completed.v1") => AlwaysOnWakeStatus::Completed,
                    Some("run.cancelled.v1") | None => AlwaysOnWakeStatus::Interrupted,
                    Some("run.max_turns.v1") => AlwaysOnWakeStatus::Failed,
                    Some(_) => AlwaysOnWakeStatus::OutcomeUnknown,
                },
            );
        }
        if !record.profile.enabled {
            return Ok(AlwaysOnWakeStatus::Interrupted);
        }
        if !self.always_on_session_available(&owner, &claim.session_id)? {
            return Ok(AlwaysOnWakeStatus::Blocked);
        }
        let status = self
            .run_public_model_with_mode_and_skills_stream_controlled(
                "primary",
                "You are the workspace's Always On Agent performing a read-only status check.",
                &claim.prompt,
                Some(1),
                &claim.wake.id,
                &claim.session_id,
                false,
                &[],
                &[],
                AgentRunMode::Execute,
                None,
                None,
                owner,
                &mut SilentWakeObserver,
                &RunControl::default(),
            )
            .await;
        Ok(match status {
            Ok(AgentRunOutcome::Completed { .. }) => AlwaysOnWakeStatus::Completed,
            Ok(AgentRunOutcome::Cancelled { .. }) => AlwaysOnWakeStatus::Interrupted,
            Err(error) if error.outcome_unknown() => AlwaysOnWakeStatus::OutcomeUnknown,
            Err(_) => AlwaysOnWakeStatus::Failed,
        })
    }
}

struct SilentWakeObserver;

#[async_trait::async_trait]
impl RunEventObserver for SilentWakeObserver {
    async fn observe(&mut self, _event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        Ok(())
    }
}

pub(super) fn is_enrolled_session(
    journal: &dyn EventJournal,
    session_id: &str,
) -> Result<bool, StoreError> {
    Ok(read_record(journal)?.is_some_and(|record| {
        record.profile.session_id == session_id
            && (record.profile.enabled || record.profile.pending.is_some())
    }))
}

pub(super) fn has_claimed_wake(
    journal: &dyn EventJournal,
    session_id: &str,
) -> Result<bool, StoreError> {
    Ok(read_record(journal)?.is_some_and(|record| {
        record.profile.session_id == session_id && record.claimed_prompt.is_some()
    }))
}

fn read_record(journal: &dyn EventJournal) -> Result<Option<Record>, StoreError> {
    let Some(last) = journal.read_stream_backwards(STREAM, None, 1)?.pop() else {
        return Ok(None);
    };
    let invalid = || StoreError::Verification("Always On record is invalid".into());
    if last.event_type != EVENT || last.event_version != 1 {
        return Err(invalid());
    }
    let record: Record =
        serde_json::from_value(journal.decrypt_payload(&last)?).map_err(|_| invalid())?;
    if record.profile.revision != last.stream_version
        || !(1..=record.profile.revision).contains(&record.schedule_revision)
        || (record.claimed_prompt.is_some() && record.profile.pending.is_none())
    {
        return Err(invalid());
    }
    Ok(Some(record))
}

impl Record {
    fn wake(&self, trigger: AlwaysOnTrigger, requested_at: String, identity: &str) -> AlwaysOnWake {
        let kind = match trigger {
            AlwaysOnTrigger::Scheduled => "scheduled",
            AlwaysOnTrigger::Manual => "manual",
        };
        let input = format!("always-on:{kind}:{}:{identity}", self.profile.session_id);
        let id = format!("aow-{}", hex::encode(Sha256::digest(input)));
        AlwaysOnWake {
            id,
            requested_at,
            trigger,
        }
    }

    fn settle(
        &mut self,
        status: AlwaysOnWakeStatus,
        now: OffsetDateTime,
    ) -> Result<(), RuntimeError> {
        let wake = self.profile.pending.take().ok_or_else(invalid_record)?;
        self.claimed_prompt = None;
        self.profile.last_wake = Some(AlwaysOnWakeResult {
            id: wake.id,
            completed_at: timestamp(now)?,
            status,
        });
        if matches!(
            status,
            AlwaysOnWakeStatus::Blocked | AlwaysOnWakeStatus::OutcomeUnknown
        ) {
            self.profile.enabled = false;
        }
        Ok(())
    }
}

fn valid_owner(owner: &Actor) -> bool {
    owner == &terminal_actor()
        || (owner.actor_type == ActorType::Application
            && owner.id.starts_with("app:")
            && owner.id.len() > "app:".len()
            && owner.id.len() <= 128)
}

fn scheduler_actor() -> Actor {
    Actor {
        actor_type: ActorType::System,
        id: "always-on".into(),
    }
}

fn check_revision(actual: u64, expected: u64) -> Result<(), RuntimeError> {
    if actual != expected {
        return Err(StoreError::Conflict {
            stream_id: STREAM.into(),
            expected,
            actual,
        }
        .into());
    }
    Ok(())
}

fn timestamp(time: OffsetDateTime) -> Result<String, RuntimeError> {
    time.format(&Rfc3339).map_err(|_| invalid_record())
}

fn parse_timestamp(value: &str) -> Result<OffsetDateTime, RuntimeError> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| invalid_record())
}

fn unavailable() -> RuntimeError {
    StoreError::NotFound("eligible Always On conversation".into()).into()
}

fn invalid_record() -> RuntimeError {
    StoreError::Verification("Always On record is invalid".into()).into()
}

#[cfg(test)]
mod tests;
