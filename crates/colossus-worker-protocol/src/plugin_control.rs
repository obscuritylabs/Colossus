//! Duplex plugin management reuses the worker's authenticated approval connection.

use crate::{
    WorkerApprovalMode, WorkerControlClient, WorkerControlError,
    client::{client_handshake, signed_request},
    wire::*,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{future::Future, time::Duration};

/// Policy-released, one-use prompt bound to a native management connection.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManagementPrompt {
    /// Opaque prompt identity; answers cannot be reused for another request.
    pub prompt_id: String,
    /// Prompt purpose, currently approval.
    pub kind: String,
    /// Bounded native dialog title.
    pub title: String,
    /// Policy reason displayed to the operator.
    pub question: String,
    /// Exact supported answers.
    pub choices: Vec<String>,
    /// Whether free-form answers are permitted.
    pub allow_free_form: bool,
    /// Policy-released action/resource/risk details, never credentials.
    pub details: Value,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Content {
    Prompt { prompt: PluginManagementPrompt },
    Notice { notice: Value },
    Complete { result: Value },
    Error { message: String },
}

#[derive(Serialize)]
struct ClientFrame<'a> {
    version: u16,
    request_id: &'a str,
    connection_nonce: &'a str,
    sequence: u64,
    timestamp_ms: i128,
    content_base64: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    authentication_tag: Option<String>,
}

impl WorkerControlClient {
    /// Perform management with native-owned approval and cooperative cancellation.
    /// `on_prompt` must present the authenticated request and obtain fresh operator consent.
    /// Cancellation never claims to roll back already committed lifecycle changes.
    pub async fn manage_plugin_interactive<F, Fut>(
        &self,
        operation: colossus_contracts::PluginManagementRequest,
        mut cancelled: tokio::sync::watch::Receiver<bool>,
        mut on_prompt: F,
    ) -> Result<Value, WorkerControlError>
    where
        F: FnMut(PluginManagementPrompt) -> Fut + Send,
        Fut: Future<Output = Option<String>> + Send,
    {
        if *cancelled.borrow() {
            return Ok(json!({"cancelled": true}));
        }
        let mut stream = self.connect().await?;
        let nonce = tokio::time::timeout(
            Duration::from_secs(60),
            client_handshake(&mut stream, self.authentication_key.as_ref()),
        )
        .await
        .map_err(|_| WorkerControlError::Busy)??;
        let request = signed_request(
            self.authentication_key.as_ref(),
            ControlOperation::RunInteractive {
                request: PluginInteractiveRequest::PluginManage { request: operation },
                approval_mode: WorkerApprovalMode::Ask,
            },
            &nonce,
        )?;
        write_message(&mut stream, &request, MAX_REQUEST_BYTES).await?;
        let (mut reader, mut writer) = tokio::io::split(stream);
        let mut server_sequence = 0_u64;
        let mut client_sequence = 0_u64;
        let mut cancellation_sent = false;
        loop {
            if !cancellation_sent && *cancelled.borrow() {
                client_sequence += 1;
                send_control(
                    &mut writer,
                    self.authentication_key.as_ref(),
                    &request.request_id,
                    &nonce,
                    client_sequence,
                    json!({"kind": "cancel"}),
                )
                .await?;
                cancellation_sent = true;
            }
            // Keep this read pinned while processing cancellation: restarting a partially
            // read length-prefixed frame would desynchronize the authenticated stream.
            let read = tokio::time::timeout(
                Duration::from_secs(30 * 60),
                read_message::<_, WorkerFrame>(&mut reader, MAX_FRAME_BYTES),
            );
            tokio::pin!(read);
            let frame = loop {
                tokio::select! {
                    frame = &mut read => break frame.map_err(|_| WorkerControlError::Busy)??,
                    _ = cancelled.changed(), if !cancellation_sent => {
                        if *cancelled.borrow() {
                            client_sequence += 1;
                            send_control(&mut writer, self.authentication_key.as_ref(), &request.request_id, &nonce, client_sequence, json!({"kind": "cancel"})).await?;
                            cancellation_sent = true;
                        } else if cancelled.has_changed().is_err() { cancellation_sent = true; }
                    }
                }
            };
            server_sequence += 1;
            if frame.version != PROTOCOL_VERSION
                || frame.request_id != request.request_id
                || frame.sequence != server_sequence
                || (now_ms() - frame.timestamp_ms).abs() > MAX_CLOCK_SKEW_MS
            {
                return Err(WorkerControlError::Protocol(
                    "plugin response sequence or request binding is invalid".into(),
                ));
            }
            verify_tag(
                self.authentication_key.as_ref(),
                &UnsignedFrame {
                    version: frame.version,
                    request_id: &frame.request_id,
                    sequence: frame.sequence,
                    timestamp_ms: frame.timestamp_ms,
                    content_base64: &frame.content_base64,
                },
                &frame.authentication_tag,
                "plugin management response",
            )?;
            let bytes = STANDARD.decode(&frame.content_base64).map_err(|_| {
                WorkerControlError::Protocol("invalid plugin response encoding".into())
            })?;
            let content: Content = serde_json::from_slice(&bytes).map_err(|_| {
                WorkerControlError::Protocol("invalid plugin management response".into())
            })?;
            match content {
                Content::Complete { result } => return Ok(result),
                Content::Error { message } => return Err(WorkerControlError::Remote(message)),
                Content::Notice { notice } => {
                    let _ = notice;
                }
                Content::Prompt { prompt } => {
                    if prompt.kind != "approval"
                        || prompt.choices != ["Allow once", "Deny"]
                        || prompt.allow_free_form
                        || prompt.prompt_id.len() > 128
                        || prompt.title.len() > 1024
                        || prompt.question.len() > 16 * 1024
                        || bytes.len() > 64 * 1024
                    {
                        return Err(WorkerControlError::Protocol(
                            "invalid plugin approval prompt".into(),
                        ));
                    }
                    let id = prompt.prompt_id.clone();
                    let choices = prompt.choices.clone();
                    let answer = if cancellation_sent {
                        None
                    } else {
                        tokio::select! {
                            answer = on_prompt(prompt) => answer.filter(|answer| choices.contains(answer)),
                            _ = cancelled.changed() => None,
                        }
                    };
                    client_sequence += 1;
                    send_control(
                        &mut writer,
                        self.authentication_key.as_ref(),
                        &request.request_id,
                        &nonce,
                        client_sequence,
                        json!({"kind": "prompt_response", "prompt_id": id, "answer": answer}),
                    )
                    .await?;
                }
            }
        }
    }
}

async fn send_control<S: tokio::io::AsyncWrite + Unpin>(
    stream: &mut S,
    key: &[u8; 32],
    request_id: &str,
    connection_nonce: &str,
    sequence: u64,
    content: Value,
) -> Result<(), WorkerControlError> {
    let mut frame = ClientFrame {
        version: PROTOCOL_VERSION,
        request_id,
        connection_nonce,
        sequence,
        timestamp_ms: now_ms(),
        content_base64: STANDARD.encode(
            serde_json::to_vec(&content)
                .map_err(|error| WorkerControlError::Protocol(error.to_string()))?,
        ),
        authentication_tag: None,
    };
    frame.authentication_tag = Some(request_tag(key, &frame)?);
    write_message(stream, &frame, MAX_REQUEST_BYTES).await
}
