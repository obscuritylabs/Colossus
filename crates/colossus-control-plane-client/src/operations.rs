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
            Self::Thread { project, thread } => (
                vec![
                    "api".into(),
                    "projects".into(),
                    project,
                    "threads".into(),
                    thread,
                ],
                None,
                None,
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
