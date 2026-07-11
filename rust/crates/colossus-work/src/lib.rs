//! Canonical event-sourced tasks and future-facing key decisions.

#![allow(clippy::missing_errors_doc)]

use colossus_contracts::{
    Actor, DecisionPriority, DecisionSource, DecisionStatus, EventClassification, ExecutionContext,
    KeyDecision, NewEvent, TaskRecord, TaskStatus,
};
use colossus_ports::{EventJournal, SessionRepository, StoreError, WorkRepository};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const TASK_CREATED: &str = "task.created.v1";
const TASK_UPDATED: &str = "task.updated.v1";
const DECISION_CREATED: &str = "decision.created.v1";
const DECISION_UPDATED: &str = "decision.updated.v1";
const DECISION_ARCHIVED: &str = "decision.archived.v1";
const DECISION_SUPERSEDED: &str = "decision.superseded.v1";
const MAX_TITLE_BYTES: usize = 512;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_LIST: usize = 1_000;

fn adapter(error: impl std::fmt::Display) -> StoreError {
    StoreError::Adapter(error.to_string())
}

fn now() -> Result<String, StoreError> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(adapter)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_task(task: &TaskRecord) -> Result<(), StoreError> {
    if !valid_id(&task.id)
        || !valid_id(&task.session_id)
        || task.title.trim().is_empty()
        || task.title.len() > MAX_TITLE_BYTES
        || task.description.len() > MAX_TEXT_BYTES
        || task.created_at.is_empty()
        || task.updated_at.is_empty()
    {
        return Err(StoreError::Adapter(
            "invalid task identity, title, description, or timestamp".into(),
        ));
    }
    Ok(())
}

fn validate_decision(decision: &KeyDecision) -> Result<(), StoreError> {
    let bounded = [
        &decision.decision,
        &decision.intent,
        &decision.applies_when,
        &decision.rationale,
        &decision.source_excerpt,
    ];
    if !valid_id(&decision.id)
        || !valid_id(&decision.session_id)
        || decision.title.trim().is_empty()
        || decision.title.len() > MAX_TITLE_BYTES
        || decision.decision.trim().is_empty()
        || bounded.iter().any(|value| value.len() > MAX_TEXT_BYTES)
        || decision.created_at.is_empty()
        || decision.updated_at.is_empty()
    {
        return Err(StoreError::Adapter(
            "invalid key-decision identity, content, or timestamp".into(),
        ));
    }
    Ok(())
}

/// Immutable-journal implementation of the work lifecycle port.
pub struct EventSourcedWorkRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedWorkRepository {
    /// Bind canonical task and decision streams to the authoritative journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn task_stream(id: &str) -> String {
        format!("task:{id}")
    }

    fn decision_stream(id: &str) -> String {
        format!("decision:{id}")
    }

    fn event(
        stream_id: String,
        expected_stream_version: u64,
        event_type: &str,
        actor: Actor,
        session_id: &str,
        payload: Value,
    ) -> NewEvent {
        NewEvent {
            event_version: 1,
            stream_id: stream_id.clone(),
            expected_stream_version,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: ExecutionContext {
                correlation_id: stream_id,
                session_id: Some(session_id.into()),
                ..ExecutionContext::default()
            },
            payload,
        }
    }

    fn ids(&self, prefix: &str, created_event: &str) -> Result<BTreeSet<String>, StoreError> {
        let mut ids = BTreeSet::new();
        let mut from = 1_u64;
        loop {
            let events = self.journal.read_global(from, 1_024)?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                if event.event_type == created_event
                    && let Some(id) = event.stream_id.strip_prefix(prefix)
                {
                    ids.insert(id.into());
                }
            }
            from = events
                .last()
                .map_or(from, |event| event.global_sequence.saturating_add(1));
            if events.len() < 1_024 {
                break;
            }
        }
        Ok(ids)
    }

    fn record<T: serde::de::DeserializeOwned>(
        &self,
        stream_id: &str,
        expected_created_event: &str,
    ) -> Result<Option<T>, StoreError> {
        let events = self.journal.read_stream(stream_id)?;
        let Some(first) = events.first() else {
            return Ok(None);
        };
        if first.event_type != expected_created_event {
            return Err(StoreError::Verification(format!(
                "work stream {stream_id} has no valid creation event"
            )));
        }
        let last = events
            .last()
            .ok_or_else(|| StoreError::Verification("work stream disappeared".into()))?;
        let payload = self.journal.decrypt_payload(last)?;
        serde_json::from_value(
            payload
                .get("record")
                .cloned()
                .ok_or_else(|| StoreError::Verification("work record is absent".into()))?,
        )
        .map(Some)
        .map_err(|error| StoreError::Verification(error.to_string()))
    }
}

impl WorkRepository for EventSourcedWorkRepository {
    fn create_task(&self, task: TaskRecord, actor: Actor) -> Result<TaskRecord, StoreError> {
        validate_task(&task)?;
        self.journal.append(Self::event(
            Self::task_stream(&task.id),
            0,
            TASK_CREATED,
            actor,
            &task.session_id,
            json!({"record": &task}),
        ))?;
        Ok(task)
    }

    fn update_task(&self, task: TaskRecord, actor: Actor) -> Result<TaskRecord, StoreError> {
        validate_task(&task)?;
        let current = self
            .get_task(&task.id)?
            .ok_or_else(|| StoreError::NotFound(format!("task {}", task.id)))?;
        if current.session_id != task.session_id || current.created_at != task.created_at {
            return Err(StoreError::Adapter(
                "task session and creation timestamp are immutable".into(),
            ));
        }
        let stream = Self::task_stream(&task.id);
        let expected = u64::try_from(self.journal.read_stream(&stream)?.len()).map_err(adapter)?;
        self.journal.append(Self::event(
            stream,
            expected,
            TASK_UPDATED,
            actor,
            &task.session_id,
            json!({"record": &task}),
        ))?;
        Ok(task)
    }

    fn get_task(&self, id: &str) -> Result<Option<TaskRecord>, StoreError> {
        self.record(&Self::task_stream(id), TASK_CREATED)
    }

    fn list_tasks(
        &self,
        session_id: Option<&str>,
        status: Option<TaskStatus>,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let mut records = self
            .ids("task:", TASK_CREATED)?
            .into_iter()
            .filter_map(|id| self.get_task(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        records.retain(|record| {
            session_id.is_none_or(|id| record.session_id == id)
                && status.is_none_or(|status| record.status == status)
        });
        records.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }

    fn create_decision(
        &self,
        decision: KeyDecision,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError> {
        validate_decision(&decision)?;
        if decision.status != DecisionStatus::Active {
            return Err(StoreError::Adapter(
                "new key decisions must be active".into(),
            ));
        }
        self.journal.append(Self::event(
            Self::decision_stream(&decision.id),
            0,
            DECISION_CREATED,
            actor,
            &decision.session_id,
            json!({"record": &decision}),
        ))?;
        Ok(decision)
    }

    fn update_decision(
        &self,
        decision: KeyDecision,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError> {
        validate_decision(&decision)?;
        let current = self
            .get_decision(&decision.id)?
            .ok_or_else(|| StoreError::NotFound(format!("decision {}", decision.id)))?;
        if current.status != DecisionStatus::Active
            || decision.status != DecisionStatus::Active
            || current.session_id != decision.session_id
            || current.created_at != decision.created_at
            || current.source != decision.source
            || current.supersedes != decision.supersedes
        {
            return Err(StoreError::Adapter(
                "only active decision content may be updated; provenance is immutable".into(),
            ));
        }
        let stream = Self::decision_stream(&decision.id);
        let expected = u64::try_from(self.journal.read_stream(&stream)?.len()).map_err(adapter)?;
        self.journal.append(Self::event(
            stream,
            expected,
            DECISION_UPDATED,
            actor,
            &decision.session_id,
            json!({"record": &decision}),
        ))?;
        Ok(decision)
    }

    fn get_decision(&self, id: &str) -> Result<Option<KeyDecision>, StoreError> {
        self.record(&Self::decision_stream(id), DECISION_CREATED)
    }

    fn list_decisions(
        &self,
        session_id: Option<&str>,
        status: Option<DecisionStatus>,
        limit: usize,
    ) -> Result<Vec<KeyDecision>, StoreError> {
        let mut records = self
            .ids("decision:", DECISION_CREATED)?
            .into_iter()
            .filter_map(|id| self.get_decision(&id).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        records.retain(|record| {
            session_id.is_none_or(|id| record.session_id == id)
                && status.is_none_or(|status| record.status == status)
        });
        records.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        records.truncate(limit.clamp(1, MAX_LIST));
        Ok(records)
    }

    fn archive_decision(&self, id: &str, actor: Actor) -> Result<KeyDecision, StoreError> {
        let mut decision = self
            .get_decision(id)?
            .ok_or_else(|| StoreError::NotFound(format!("decision {id}")))?;
        if decision.status != DecisionStatus::Active {
            return Err(StoreError::Adapter(
                "only active decisions can be archived".into(),
            ));
        }
        decision.status = DecisionStatus::Archived;
        decision.updated_at = now()?;
        let stream = Self::decision_stream(id);
        let expected = u64::try_from(self.journal.read_stream(&stream)?.len()).map_err(adapter)?;
        self.journal.append(Self::event(
            stream,
            expected,
            DECISION_ARCHIVED,
            actor,
            &decision.session_id,
            json!({"record": &decision}),
        ))?;
        Ok(decision)
    }

    fn supersede_decision(
        &self,
        id: &str,
        replacement: KeyDecision,
        actor: Actor,
    ) -> Result<(KeyDecision, KeyDecision), StoreError> {
        let mut old = self
            .get_decision(id)?
            .ok_or_else(|| StoreError::NotFound(format!("decision {id}")))?;
        validate_decision(&replacement)?;
        if old.status != DecisionStatus::Active
            || replacement.status != DecisionStatus::Active
            || replacement.id == old.id
            || replacement.session_id != old.session_id
            || replacement.supersedes.as_deref() != Some(id)
            || self.get_decision(&replacement.id)?.is_some()
        {
            return Err(StoreError::Adapter(
                "decision supersession requires an active same-session replacement linked to the old id"
                    .into(),
            ));
        }
        old.status = DecisionStatus::Superseded;
        old.updated_at = now()?;
        let old_stream = Self::decision_stream(id);
        let expected =
            u64::try_from(self.journal.read_stream(&old_stream)?.len()).map_err(adapter)?;
        self.journal.append_batch(vec![
            Self::event(
                old_stream,
                expected,
                DECISION_SUPERSEDED,
                actor.clone(),
                &old.session_id,
                json!({"record": &old, "replacement_id": replacement.id}),
            ),
            Self::event(
                Self::decision_stream(&replacement.id),
                0,
                DECISION_CREATED,
                actor,
                &replacement.session_id,
                json!({"record": &replacement}),
            ),
        ])?;
        Ok((old, replacement))
    }
}

/// Validated application service shared by CLI, REPL, tools, and embedded callers.
pub struct WorkService {
    repository: Arc<dyn WorkRepository>,
    sessions: Arc<dyn SessionRepository>,
}

impl WorkService {
    /// Compose work operations from canonical repository ports.
    pub fn new(repository: Arc<dyn WorkRepository>, sessions: Arc<dyn SessionRepository>) -> Self {
        Self {
            repository,
            sessions,
        }
    }

    fn require_session(&self, session_id: &str) -> Result<(), StoreError> {
        self.sessions
            .get_session(session_id)?
            .ok_or_else(|| StoreError::NotFound(format!("session {session_id}")))?;
        Ok(())
    }

    /// Create a new task with a generated stable id.
    pub fn create_task(
        &self,
        session_id: &str,
        title: &str,
        description: &str,
        status: TaskStatus,
        actor: Actor,
    ) -> Result<TaskRecord, StoreError> {
        self.require_session(session_id)?;
        let timestamp = now()?;
        self.repository.create_task(
            TaskRecord {
                id: format!("task-{}", Uuid::now_v7()),
                session_id: session_id.into(),
                title: title.trim().into(),
                description: description.into(),
                status,
                created_at: timestamp.clone(),
                updated_at: timestamp,
            },
            actor,
        )
    }

    /// Update supplied task fields while preserving identity and creation time.
    pub fn update_task(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        status: Option<TaskStatus>,
        actor: Actor,
    ) -> Result<TaskRecord, StoreError> {
        let mut task = self
            .repository
            .get_task(id)?
            .ok_or_else(|| StoreError::NotFound(format!("task {id}")))?;
        if let Some(title) = title {
            task.title = title.trim().into();
        }
        if let Some(description) = description {
            task.description = description.into();
        }
        if let Some(status) = status {
            task.status = status;
        }
        task.updated_at = now()?;
        self.repository.update_task(task, actor)
    }

    /// Create one active future-facing decision.
    #[allow(clippy::too_many_arguments)]
    pub fn create_decision(
        &self,
        session_id: &str,
        title: &str,
        decision: &str,
        source: DecisionSource,
        priority: DecisionPriority,
        intent: &str,
        applies_when: &str,
        rationale: &str,
        source_excerpt: &str,
        goal_id: Option<String>,
        plan_id: Option<String>,
        supersedes: Option<String>,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError> {
        self.require_session(session_id)?;
        let timestamp = now()?;
        self.repository.create_decision(
            KeyDecision {
                id: format!("kd_{}", Uuid::now_v7()),
                session_id: session_id.into(),
                goal_id,
                plan_id,
                source,
                status: DecisionStatus::Active,
                priority,
                title: title.trim().into(),
                decision: decision.trim().into(),
                intent: intent.into(),
                applies_when: applies_when.into(),
                rationale: rationale.into(),
                source_excerpt: source_excerpt.into(),
                supersedes,
                created_at: timestamp.clone(),
                updated_at: timestamp,
            },
            actor,
        )
    }

    /// Update mutable decision content while leaving provenance and status intact.
    #[allow(clippy::too_many_arguments)]
    pub fn update_decision(
        &self,
        id: &str,
        title: Option<&str>,
        decision: Option<&str>,
        priority: Option<DecisionPriority>,
        intent: Option<&str>,
        applies_when: Option<&str>,
        rationale: Option<&str>,
        source_excerpt: Option<&str>,
        actor: Actor,
    ) -> Result<KeyDecision, StoreError> {
        let mut record = self
            .repository
            .get_decision(id)?
            .ok_or_else(|| StoreError::NotFound(format!("decision {id}")))?;
        if let Some(value) = title {
            record.title = value.trim().into();
        }
        if let Some(value) = decision {
            record.decision = value.trim().into();
        }
        if let Some(value) = priority {
            record.priority = value;
        }
        if let Some(value) = intent {
            record.intent = value.into();
        }
        if let Some(value) = applies_when {
            record.applies_when = value.into();
        }
        if let Some(value) = rationale {
            record.rationale = value.into();
        }
        if let Some(value) = source_excerpt {
            record.source_excerpt = value.into();
        }
        record.updated_at = now()?;
        self.repository.update_decision(record, actor)
    }

    /// Archive one active decision.
    pub fn archive_decision(&self, id: &str, actor: Actor) -> Result<KeyDecision, StoreError> {
        self.repository.archive_decision(id, actor)
    }

    /// Replace an active decision atomically and preserve lineage.
    #[allow(clippy::too_many_arguments)]
    pub fn supersede_decision(
        &self,
        id: &str,
        title: &str,
        decision: &str,
        source: DecisionSource,
        priority: DecisionPriority,
        intent: &str,
        applies_when: &str,
        rationale: &str,
        source_excerpt: &str,
        actor: Actor,
    ) -> Result<(KeyDecision, KeyDecision), StoreError> {
        let old = self
            .repository
            .get_decision(id)?
            .ok_or_else(|| StoreError::NotFound(format!("decision {id}")))?;
        let timestamp = now()?;
        let replacement = KeyDecision {
            id: format!("kd_{}", Uuid::now_v7()),
            session_id: old.session_id.clone(),
            goal_id: old.goal_id.clone(),
            plan_id: old.plan_id.clone(),
            source,
            status: DecisionStatus::Active,
            priority,
            title: title.trim().into(),
            decision: decision.trim().into(),
            intent: intent.into(),
            applies_when: applies_when.into(),
            rationale: rationale.into(),
            source_excerpt: source_excerpt.into(),
            supersedes: Some(old.id.clone()),
            created_at: timestamp.clone(),
            updated_at: timestamp,
        };
        self.repository.supersede_decision(id, replacement, actor)
    }

    /// Canonical repository for bounded query surfaces.
    pub fn repository(&self) -> Arc<dyn WorkRepository> {
        Arc::clone(&self.repository)
    }
}

#[cfg(test)]
fn user_actor() -> Actor {
    Actor {
        actor_type: colossus_contracts::ActorType::User,
        id: "terminal-user".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_session::EventSourcedSessionRepository;
    use colossus_testkit::InMemoryEventJournal;

    fn fixture() -> (Arc<dyn EventJournal>, Arc<dyn WorkRepository>, WorkService) {
        let journal: Arc<dyn EventJournal> = Arc::new(InMemoryEventJournal::default());
        let sessions: Arc<dyn SessionRepository> =
            Arc::new(EventSourcedSessionRepository::new(Arc::clone(&journal)));
        sessions
            .create_session("session-1", Some("work"), user_actor())
            .expect("session");
        let repository: Arc<dyn WorkRepository> =
            Arc::new(EventSourcedWorkRepository::new(Arc::clone(&journal)));
        let service = WorkService::new(Arc::clone(&repository), sessions);
        (journal, repository, service)
    }

    #[test]
    fn tasks_reconstruct_after_updates_and_repository_restart() {
        let (journal, repository, service) = fixture();
        let created = service
            .create_task(
                "session-1",
                "Implement work state",
                "Use immutable events",
                TaskStatus::Pending,
                user_actor(),
            )
            .expect("create");
        let updated = service
            .update_task(
                &created.id,
                None,
                Some("Repository and projections"),
                Some(TaskStatus::InProgress),
                user_actor(),
            )
            .expect("update");
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(updated.created_at, created.created_at);
        assert_eq!(
            repository
                .list_tasks(Some("session-1"), None, 10)
                .expect("list")
                .len(),
            1
        );

        let reopened = EventSourcedWorkRepository::new(journal);
        assert_eq!(
            reopened.get_task(&created.id).expect("get").expect("task"),
            updated
        );
    }

    #[test]
    fn active_decisions_update_archive_and_filter_without_deletion() {
        let (_journal, repository, service) = fixture();
        let created = service
            .create_decision(
                "session-1",
                "Audit first",
                "All durable mutations use immutable events.",
                DecisionSource::User,
                DecisionPriority::Critical,
                "Preserve evidence",
                "When changing canonical state",
                "Auditability is foundational",
                "I want auditing from the ground up",
                None,
                None,
                None,
                user_actor(),
            )
            .expect("create");
        let updated = service
            .update_decision(
                &created.id,
                None,
                Some("All state changes use immutable canonical events."),
                Some(DecisionPriority::High),
                None,
                None,
                None,
                None,
                user_actor(),
            )
            .expect("update");
        assert_eq!(updated.priority, DecisionPriority::High);
        assert_eq!(
            repository
                .list_decisions(Some("session-1"), Some(DecisionStatus::Active), 10,)
                .expect("active")
                .len(),
            1
        );
        let archived = service
            .archive_decision(&created.id, user_actor())
            .expect("archive");
        assert_eq!(archived.status, DecisionStatus::Archived);
        assert!(
            repository
                .list_decisions(Some("session-1"), Some(DecisionStatus::Active), 10,)
                .expect("active")
                .is_empty()
        );
        assert_eq!(
            repository
                .get_decision(&created.id)
                .expect("get")
                .expect("decision")
                .status,
            DecisionStatus::Archived
        );
    }

    #[test]
    fn supersession_is_atomic_and_preserves_lineage() {
        let (journal, repository, service) = fixture();
        let old = service
            .create_decision(
                "session-1",
                "Storage",
                "Use SQLite.",
                DecisionSource::User,
                DecisionPriority::Normal,
                "Keep state local",
                "During the Python implementation",
                "Legacy choice",
                "SQLite at first",
                None,
                None,
                None,
                user_actor(),
            )
            .expect("old");
        let (superseded, replacement) = service
            .supersede_decision(
                &old.id,
                "Storage",
                "Use replaceable repository ports with redb as the initial canonical adapter.",
                DecisionSource::User,
                DecisionPriority::Critical,
                "Keep storage replaceable",
                "For all Rust canonical state",
                "Supports redb, PostgreSQL, and indexes",
                "abstract layer like a Repo pattern",
                user_actor(),
            )
            .expect("supersede");
        assert_eq!(superseded.status, DecisionStatus::Superseded);
        assert_eq!(replacement.supersedes.as_deref(), Some(old.id.as_str()));
        assert_eq!(
            repository
                .list_decisions(Some("session-1"), Some(DecisionStatus::Active), 10,)
                .expect("active"),
            vec![replacement.clone()]
        );
        let stream = journal
            .read_stream(&format!("decision:{}", old.id))
            .expect("old stream");
        assert_eq!(stream.last().expect("last").event_type, DECISION_SUPERSEDED);
        assert_eq!(
            journal
                .read_stream(&format!("decision:{}", replacement.id))
                .expect("replacement stream")
                .len(),
            1
        );
    }
}
