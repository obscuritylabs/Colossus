//! Artifact authority comes from registered bindings and canonical host provenance.
use super::*;
use colossus_contracts::{Actor, ActorType, BrowserScope, BrowserSessionBinding, WorkflowOrigin};
use std::collections::BTreeSet;

pub(super) fn resolve(
    journal: &dyn EventJournal,
    binding: &BrowserSessionBinding,
) -> Result<String, BrowserArtifactError> {
    if let BrowserScope::Workflow { id } = &binding.scope {
        if binding.application_id != *id {
            return Err(BrowserArtifactError::Invalid);
        }
        return workflow(journal, id, &mut BTreeSet::new(), 0);
    }
    if binding.application_id == "terminal-user" {
        return Ok("app:colossus-cli".into());
    }
    if binding.application_id.starts_with("app:") {
        return application(&Actor {
            actor_type: ActorType::Application,
            id: binding.application_id.clone(),
        });
    }
    Err(BrowserArtifactError::Invalid)
}

fn bounded(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
fn application(actor: &Actor) -> Result<String, BrowserArtifactError> {
    match actor.actor_type {
        ActorType::Application
            if actor.id.starts_with("app:") && bounded(&actor.id) && actor.id.len() > 4 =>
        {
            Ok(actor.id.clone())
        }
        ActorType::User if actor.id == "terminal-user" => Ok("app:colossus-cli".into()),
        _ => Err(BrowserArtifactError::Invalid),
    }
}
fn origin(payload: &serde_json::Value) -> Result<Option<Actor>, BrowserArtifactError> {
    match payload.get("origin") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => serde_json::from_value::<WorkflowOrigin>(value.clone())
            .map(|origin| Some(origin.owner))
            .map_err(|_| BrowserArtifactError::Invalid),
    }
}
fn workflow(
    journal: &dyn EventJournal,
    id: &str,
    seen: &mut BTreeSet<String>,
    depth: usize,
) -> Result<String, BrowserArtifactError> {
    if depth >= 8 || !bounded(id) || !seen.insert(id.into()) {
        return Err(BrowserArtifactError::Invalid);
    }
    let stream = format!("workflow-run:{id}");
    let events = journal
        .read_stream_from(&stream, 0, 1)
        .map_err(|_| BrowserArtifactError::Invalid)?;
    let event = events
        .first()
        .filter(|event| {
            event.stream_id == stream
                && event.stream_version == 1
                && event.event_version == 1
                && event.classification == colossus_contracts::EventClassification::Workflow
                && matches!(
                    event.event_type.as_str(),
                    "workflow.run.queued.v1" | "workflow.run.started.v1"
                )
        })
        .ok_or(BrowserArtifactError::Invalid)?;
    let payload = journal
        .decrypt_payload(event)
        .map_err(|_| BrowserArtifactError::Invalid)?;
    if let Some(owner) = origin(&payload)? {
        if event.actor.actor_type == ActorType::Application && event.actor != owner {
            return Err(BrowserArtifactError::Invalid);
        }
        return application(&owner);
    }
    if event.actor.actor_type == ActorType::Application
        || (event.actor.actor_type == ActorType::User && event.actor.id == "terminal-user")
    {
        return application(&event.actor);
    }
    if let Some(parent) = payload
        .get("parent_run_id")
        .and_then(serde_json::Value::as_str)
    {
        return workflow(journal, parent, seen, depth + 1);
    }
    if payload
        .get("trigger_kind")
        .and_then(serde_json::Value::as_str)
        != Some("schedule")
    {
        return Err(BrowserArtifactError::Invalid);
    }
    let schedule = payload
        .get("trigger_id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| bounded(id))
        .ok_or(BrowserArtifactError::Invalid)?;
    let stream = format!("workflow-schedule:{schedule}");
    let events = journal
        .read_stream_from(&stream, 0, 1)
        .map_err(|_| BrowserArtifactError::Invalid)?;
    let event = events
        .first()
        .filter(|event| {
            event.stream_id == stream
                && event.stream_version == 1
                && event.event_version == 1
                && event.classification == colossus_contracts::EventClassification::Workflow
                && event.event_type == "workflow.schedule.registered.v1"
        })
        .ok_or(BrowserArtifactError::Invalid)?;
    let payload = journal
        .decrypt_payload(event)
        .map_err(|_| BrowserArtifactError::Invalid)?;
    let owner = origin(&payload)?.ok_or(BrowserArtifactError::Invalid)?;
    if event.actor.actor_type == ActorType::Application && event.actor != owner {
        return Err(BrowserArtifactError::Invalid);
    }
    application(&owner)
}
