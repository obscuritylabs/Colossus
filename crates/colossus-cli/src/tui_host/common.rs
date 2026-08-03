use super::*;

pub(super) fn resumable_sessions(
    mut sessions: Vec<SessionSummary>,
    limit: usize,
) -> Vec<SessionSummary> {
    sessions.retain(|session| session.message_count > 0);
    sessions.truncate(limit);
    sessions
}

pub(super) fn session_picker_choice(session: &SessionSummary) -> String {
    let title = compact_text(session.title.as_deref().unwrap_or("Untitled"), 36);
    let preview = session
        .last_user_preview
        .as_deref()
        .map(|preview| compact_text(preview, 120))
        .filter(|preview| !preview.is_empty())
        .unwrap_or_else(|| "No user message preview".into());
    let short_id = session.id.chars().take(8).collect::<String>();
    let updated_at = compact_timestamp(&session.updated_at);
    format!(
        "{title} · {} msgs · {short_id} · {updated_at}\n{preview}",
        session.message_count
    )
}

pub(super) fn compact_text(value: &str, maximum_characters: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(maximum_characters)
        .collect()
}

pub(super) fn compact_timestamp(value: &str) -> String {
    value
        .get(..16)
        .map(|timestamp| format!("{}Z", timestamp.replace('T', " ")))
        .unwrap_or_else(|| value.to_owned())
}

/// One active TUI event destination shared by trusted approval and input providers.
#[derive(Default)]
pub(crate) struct TuiPromptRouter {
    pub(super) sender: Mutex<Option<mpsc::Sender<HostEvent>>>,
    next_id: AtomicU64,
}

impl TuiPromptRouter {
    fn next_prompt_id(&self, prefix: &str) -> String {
        format!("{prefix}:{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    pub(super) fn install(&self, sender: Option<mpsc::Sender<HostEvent>>) {
        if let Ok(mut current) = self.sender.lock() {
            *current = sender;
        }
    }

    fn notice(&self, document: PresentationDocument) {
        let sender = self.sender.lock().ok().and_then(|sender| sender.clone());
        if let Some(sender) = sender {
            // Approval-review notices are presentation-only and must never hold up
            // the permit path when the renderer is paused or disconnected.
            let _ = sender.try_send(HostEvent::Notice(document));
        }
    }

    async fn prompt(
        &self,
        id: String,
        title: String,
        document: PresentationDocument,
        choices: Vec<String>,
        allow_free_form: bool,
    ) -> Result<PromptResponse, String> {
        let sender = self
            .sender
            .lock()
            .map_err(|_| "interactive prompt router is poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "no interactive client is attached".to_owned())?;
        let (response_tx, response_rx) = oneshot::channel();
        sender
            .send(HostEvent::Prompt(InteractivePrompt {
                id,
                title,
                document,
                choices,
                initial_choice: None,
                allow_free_form,
                response: response_tx,
            }))
            .await
            .map_err(|_| "interactive client disconnected before the prompt".to_owned())?;
        tokio::time::timeout(INTERACTIVE_PROMPT_TIMEOUT, response_rx)
            .await
            .map_err(|_| "interactive prompt timed out".to_owned())?
            .map_err(|_| "interactive prompt was dropped".to_owned())
    }
}

/// Trusted approval provider that mints proof only after the TUI returns allow.
pub(crate) struct TuiApprovalProvider {
    pub(crate) router: Arc<TuiPromptRouter>,
    pub(crate) risk_auto: bool,
}

#[async_trait]
impl ApprovalProvider for TuiApprovalProvider {
    fn risk_auto_enabled(&self) -> bool {
        self.risk_auto
    }

    async fn automatic_approval_granted(&self, notice: AutomaticApprovalNotice) {
        self.router.notice(automatic_approval_document(&notice));
    }

    async fn risk_review_fallback(&self, notice: RiskReviewFallbackNotice) {
        self.router.notice(risk_review_fallback_document(&notice));
    }

    async fn request_approval(
        &self,
        request: &EffectRequest,
        request_hash: &str,
        decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        let content = serde_json::to_string_pretty(&request.content)
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let mut details = vec![
            ("Action".into(), request.action.clone()),
            ("Resource".into(), request.resource.clone()),
            ("Reason".into(), decision.reason.clone()),
        ];
        if let Some(reason) = request.risk.reason.as_deref() {
            let level = request.risk.level.as_deref().unwrap_or("unavailable");
            details.push(("Risk review".into(), format!("{level}: {reason}")));
        }
        let document = PresentationDocument::from_block(PresentationBlock::Card {
            title: "Approval required".into(),
            tone: PresentationTone::Warning,
            body: vec![
                PresentationBlock::KeyValue(details),
                PresentationBlock::Code {
                    language: Some("proposed content".into()),
                    content: content.chars().take(8_192).collect(),
                },
            ],
        });
        let response = self
            .router
            .prompt(
                format!("approval:{}", decision.decision_id),
                "Approval required".into(),
                document,
                vec!["Allow once".into(), "Deny".into()],
                false,
            )
            .await
            .map_err(PolicyError::Unavailable)?;
        if response != PromptResponse::Answer("Allow once".into()) {
            return Ok(None);
        }
        ApprovalProvider::request_approval(
            &AllowApproval {
                approved_by: "terminal-user".into(),
            },
            request,
            request_hash,
            decision,
        )
        .await
    }
}

/// Trusted `user.ask` provider bridged to the focused TUI overlay.
pub(crate) struct TuiUserPromptProvider {
    pub(crate) router: Arc<TuiPromptRouter>,
}

#[async_trait]
impl UserPromptProvider for TuiUserPromptProvider {
    async fn prompt(&self, request: UserPromptRequest) -> Result<UserPromptResponse, ToolError> {
        let response = self
            .router
            .prompt(
                self.router.next_prompt_id("user-ask"),
                "Input needed".into(),
                PresentationDocument::from_block(PresentationBlock::Markdown(
                    request.question.clone(),
                )),
                request.choices.clone(),
                request.allow_free_form,
            )
            .await
            .map_err(ToolError::Failed)?;
        let PromptResponse::Answer(answer) = response else {
            return Err(ToolError::Failed("user cancelled the question".into()));
        };
        let selected_index = request.choices.iter().position(|choice| choice == &answer);
        if selected_index.is_none() && !request.allow_free_form {
            return Err(ToolError::Failed(
                "user response did not match an allowed choice".into(),
            ));
        }
        Ok(UserPromptResponse {
            answer,
            selected_index,
        })
    }
}

pub(crate) struct ChannelRunObserver {
    pub(crate) sender: mpsc::Sender<HostEvent>,
}

#[async_trait]
impl RunEventObserver for ChannelRunObserver {
    async fn observe(&mut self, event: RunEventEnvelope) -> Result<(), ModelProviderError> {
        self.sender
            .send(HostEvent::Run(event))
            .await
            .map_err(|_| ModelProviderError::Failed("terminal event loop disconnected".into()))
    }
}
