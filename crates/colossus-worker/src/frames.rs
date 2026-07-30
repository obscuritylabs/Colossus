use super::*;

#[derive(Serialize)]
pub(super) struct UnsignedRequest<'a> {
    pub(super) version: u16,
    pub(super) request_id: &'a str,
    pub(super) timestamp_ms: i128,
    pub(super) nonce: &'a str,
    pub(super) connection_nonce: &'a str,
    pub(super) operation: &'a WorkerOperation,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkerRequest {
    pub(super) version: u16,
    pub(super) request_id: String,
    pub(super) timestamp_ms: i128,
    pub(super) nonce: String,
    pub(super) connection_nonce: String,
    pub(super) operation: WorkerOperation,
    pub(super) authentication_tag: String,
}

/// Worker-side policy mode used by attached and headless clients.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerApprovalMode {
    /// Deny approval obligations without prompting.
    Deny,
    /// Ask an attached protocol-v6 interactive client.
    Ask,
    /// Preserve model-assisted low-risk auto-approval and ask otherwise.
    RiskAuto,
    /// Mint approval obligations without a prompt.
    FullAccess,
}

/// Kind of one authenticated worker-to-client prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerPromptKind {
    /// Policy approval obligation.
    Approval,
    /// Tool-requested operator input.
    UserInput,
}

/// Bounded authenticated prompt transported to an attached interactive client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerPrompt {
    /// One-use prompt identity bound to the request connection.
    pub prompt_id: String,
    /// Prompt purpose.
    pub kind: WorkerPromptKind,
    /// Short overlay title.
    pub title: String,
    /// Policy-released question or reason.
    pub question: String,
    /// Optional exact answer choices.
    pub choices: Vec<String>,
    /// Whether an answer outside the exact choices is valid.
    pub allow_free_form: bool,
    /// Bounded released details suitable for a semantic card.
    pub details: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum WorkerFrameContent {
    Event { event: RunEventEnvelope },
    Notice { notice: ApprovalReviewNotice },
    Prompt { prompt: WorkerPrompt },
    Complete { result: Value },
    Error { message: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ClientFrameContent {
    PromptResponse {
        prompt_id: String,
        answer: Option<String>,
    },
    Cancel,
}

#[derive(Serialize)]
pub(super) struct UnsignedClientFrame<'a> {
    pub(super) version: u16,
    pub(super) request_id: &'a str,
    pub(super) connection_nonce: &'a str,
    pub(super) sequence: u64,
    pub(super) timestamp_ms: i128,
    pub(super) content_base64: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkerClientFrame {
    pub(super) version: u16,
    pub(super) request_id: String,
    pub(super) connection_nonce: String,
    pub(super) sequence: u64,
    pub(super) timestamp_ms: i128,
    pub(super) content_base64: String,
    pub(super) authentication_tag: String,
}

#[derive(Serialize)]
pub(super) struct UnsignedFrame<'a> {
    pub(super) version: u16,
    pub(super) request_id: &'a str,
    pub(super) sequence: u64,
    pub(super) timestamp_ms: i128,
    pub(super) content_base64: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkerFrame {
    pub(super) version: u16,
    pub(super) request_id: String,
    pub(super) sequence: u64,
    pub(super) timestamp_ms: i128,
    pub(super) content_base64: String,
    pub(super) authentication_tag: String,
}

#[derive(Default)]
pub(super) struct ReplayGuard {
    pub(super) order: VecDeque<String>,
    pub(super) entries: BTreeSet<String>,
}

impl ReplayGuard {
    pub(super) fn accept(&mut self, nonce: &str) -> Result<(), WorkerError> {
        if nonce.is_empty() || nonce.len() > 128 || !self.entries.insert(nonce.into()) {
            return Err(WorkerError::Protocol(
                "empty, oversized, or replayed request nonce".into(),
            ));
        }
        self.order.push_back(nonce.into());
        while self.order.len() > REPLAY_WINDOW {
            if let Some(expired) = self.order.pop_front() {
                self.entries.remove(&expired);
            }
        }
        Ok(())
    }
}
