use crate::error::{Failure, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

/// Deliberately closed human operation set; no generic URL or credential forwarding.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    /// Read the authenticated human identity.
    Me,
    /// List enrolled workspace agents.
    Fleet {
        /// Exact project identity.
        project: String,
        /// Exclusive server cursor, if continuing a list.
        after: Option<String>,
    },
    /// List host platform metadata.
    Hosts {
        /// Exact project identity.
        project: String,
        /// Exclusive server cursor, if continuing a list.
        after: Option<String>,
    },
    /// List conversations.
    Threads {
        /// Exact project identity.
        project: String,
        /// Exclusive server cursor, if continuing a list.
        after: Option<String>,
    },
    /// List execution requests.
    Tasks {
        /// Exact project identity.
        project: String,
        /// Exclusive server cursor, if continuing a list.
        after: Option<String>,
    },
    /// Read one conversation.
    Thread {
        /// Exact project identity.
        project: String,
        /// Exact conversation identity.
        thread: String,
        /// Exclusive retained message cursor.
        message_after: Option<String>,
        /// Exclusive task history cursor.
        task_after: Option<String>,
    },
    /// Read capability metadata for the exact connected runtime.
    RuntimeContext {
        /// Exact project identity.
        project: String,
        /// Enrolled execution target.
        node: String,
        /// Reviewed connection lease.
        connection_id: String,
    },
    /// Read execute-authorized model choices for the exact connected runtime.
    ModelCatalog {
        /// Exact project identity.
        project: String,
        /// Enrolled execution target.
        node: String,
        /// Reviewed connection lease.
        connection_id: String,
    },
    /// Answer one pending interaction under its reviewed concurrency token.
    Respond {
        /// Exact project identity.
        project: String,
        /// Exact execution request identity.
        task: String,
        /// Stable caller mutation identity for reconciliation.
        mutation_id: String,
        /// Existing typed response contract, independently validated by the server.
        request: Value,
    },
    /// Read one mutation or inspection receipt.
    Command {
        /// Exact project identity.
        project: String,
        /// Exact execution request identity.
        task: String,
        /// Exact receipt identity.
        command: String,
    },
    /// Inspect retained agent-session participants and messages.
    InspectInboxes {
        /// Exact project identity.
        project: String,
        /// Exact execution request identity.
        task: String,
        /// Stable caller inspection identity.
        request_id: String,
        /// Optional exact participant identity.
        participant_id: Option<String>,
        /// Exclusive message cursor.
        after_sequence: u64,
    },
    /// Read one task's current runtime snapshot and dispatch outcome.
    Task {
        /// Exact project identity.
        project: String,
        /// Exact task identity.
        task: String,
    },
    /// Read a bounded page of retained runtime events.
    TaskUpdates {
        /// Exact project identity.
        project: String,
        /// Exact task identity.
        task: String,
        /// Exclusive retained-event cursor.
        after: u64,
    },
    /// Allocate a new conversation and its first task.
    CreateThread {
        /// Exact project identity.
        project: String,
        /// Enrolled execution target.
        node: String,
        /// Existing control-plane run request, validated by the server.
        request: Value,
    },
    /// Continue a conversation under its reviewed revision.
    SendMessage {
        /// Exact project identity.
        project: String,
        /// Exact conversation identity.
        thread: String,
        /// Reviewed conversation revision.
        revision: u64,
        /// Existing control-plane run request, validated by the server.
        request: Value,
    },
    /// Request cooperative cancellation.
    Cancel {
        /// Exact project identity.
        project: String,
        /// Exact execution request identity.
        task: String,
        /// Stable caller mutation identity for reconciliation.
        mutation_id: String,
    },
}

impl Operation {
    /// Read the authenticated human identity.
    pub(crate) fn route(self, origin: &Url) -> Result<(Method, Url, Option<Value>)> {
        let mut additional_query = Vec::new();
        let (segments, after, body) = match self {
            Self::Me => (vec!["api".into(), "me".into()], None, None),
            Self::Fleet { project, after } => (
                vec!["api".into(), "projects".into(), project, "nodes".into()],
                after,
                None,
            ),
            Self::Hosts { project, after } => (
                vec!["api".into(), "projects".into(), project, "hosts".into()],
                after,
                None,
            ),
            Self::Threads { project, after } => (
                vec!["api".into(), "projects".into(), project, "threads".into()],
                after,
                None,
            ),
            Self::Tasks { project, after } => (
                vec!["api".into(), "projects".into(), project, "tasks".into()],
                after,
                None,
            ),
            Self::Thread {
                project,
                thread,
                message_after,
                task_after,
            } => {
                if let Some(cursor) = message_after {
                    additional_query.push(("message_after", cursor));
                }
                if let Some(cursor) = task_after {
                    additional_query.push(("task_after", cursor));
                }
                (
                    vec![
                        "api".into(),
                        "projects".into(),
                        project,
                        "threads".into(),
                        thread,
                    ],
                    None,
                    None,
                )
            }
            Self::RuntimeContext {
                project,
                node,
                connection_id,
            } => (
                vec![
                    "api".into(),
                    "projects".into(),
                    project,
                    "nodes".into(),
                    node,
                    "resources".into(),
                ],
                None,
                Some(json!({"connection_id":connection_id,"operation":{"operation":"context"}})),
            ),
            Self::ModelCatalog {
                project,
                node,
                connection_id,
            } => (
                vec![
                    "api".into(),
                    "projects".into(),
                    project,
                    "nodes".into(),
                    node,
                    "resources".into(),
                ],
                None,
                Some(
                    json!({"connection_id":connection_id,"operation":{"operation":"model_catalog"}}),
                ),
            ),
            Self::Respond {
                project,
                task,
                mutation_id,
                request,
            } => (
                vec![
                    "api".into(),
                    "projects".into(),
                    project,
                    "tasks".into(),
                    task,
                    "respond".into(),
                ],
                None,
                Some(json!({"mutation_id":mutation_id,"request":request})),
            ),
            Self::Command {
                project,
                task,
                command,
            } => (
                vec![
                    "api".into(),
                    "projects".into(),
                    project,
                    "tasks".into(),
                    task,
                    "commands".into(),
                    command,
                ],
                None,
                None,
            ),
            Self::InspectInboxes {
                project,
                task,
                request_id,
                participant_id,
                after_sequence,
            } => (
                vec![
                    "api".into(),
                    "projects".into(),
                    project,
                    "tasks".into(),
                    task,
                    "inboxes".into(),
                ],
                None,
                Some(
                    json!({"request_id":request_id,"participant_id":participant_id,"after_sequence":after_sequence}),
                ),
            ),
            Self::Task { project, task } => (
                vec![
                    "api".into(),
                    "projects".into(),
                    project,
                    "tasks".into(),
                    task,
                ],
                None,
                None,
            ),
            Self::TaskUpdates {
                project,
                task,
                after,
            } => (
                vec![
                    "api".into(),
                    "projects".into(),
                    project,
                    "tasks".into(),
                    task,
                    "updates".into(),
                ],
                Some(after.to_string()),
                None,
            ),
            Self::CreateThread {
                project,
                node,
                request,
            } => (
                vec!["api".into(), "projects".into(), project, "threads".into()],
                None,
                Some(json!({"node_id":node,"request":request})),
            ),
            Self::SendMessage {
                project,
                thread,
                revision,
                request,
            } => (
                vec![
                    "api".into(),
                    "projects".into(),
                    project,
                    "threads".into(),
                    thread,
                    "messages".into(),
                ],
                None,
                Some(json!({"expected_revision":revision,"request":request})),
            ),
            Self::Cancel {
                project,
                task,
                mutation_id,
            } => (
                vec![
                    "api".into(),
                    "projects".into(),
                    project,
                    "tasks".into(),
                    task,
                    "cancel".into(),
                ],
                None,
                Some(json!({"mutation_id":mutation_id})),
            ),
        };
        if segments.iter().any(|s| !valid_identity(s))
            || after.as_ref().is_some_and(|s| s.len() > 2048)
            || additional_query
                .iter()
                .any(|(_, cursor)| cursor.len() > 2048)
        {
            return Err(Failure::new(
                "invalid_argument",
                "Invalid resource address.",
            ));
        }
        let mut url = origin.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.clear().extend(segments);
        }
        if let Some(after) = after {
            url.query_pairs_mut().append_pair("after", &after);
        }
        for (key, value) in additional_query {
            url.query_pairs_mut().append_pair(key, &value);
        }
        Ok((
            if body.is_some() {
                Method::POST
            } else {
                Method::GET
            },
            url,
            body,
        ))
    }
}

pub(crate) fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
