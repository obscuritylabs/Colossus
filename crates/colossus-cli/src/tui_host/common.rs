use super::*;

pub(super) fn resumable_sessions(
    mut sessions: Vec<SessionSummary>,
    limit: usize,
) -> Vec<SessionSummary> {
    sessions.retain(|session| session.message_count > 0);
    sessions.truncate(limit);
    sessions
}

pub(super) fn session_browser_entry(
    summary: SessionSummary,
    messages: Vec<SessionMessage>,
) -> InteractiveSessionBrowserEntry {
    let mut recent_messages = messages
        .into_iter()
        .rev()
        .filter(|message| {
            matches!(
                message.message.role,
                ModelMessageRole::User | ModelMessageRole::Assistant
            ) && !message.message.content.trim().is_empty()
        })
        .take(8)
        .map(|message| InteractiveSessionBrowserMessage {
            role: message.message.role,
            content: compact_text(&message.message.content, 2_000),
        })
        .collect::<Vec<_>>();
    recent_messages.reverse();
    InteractiveSessionBrowserEntry {
        summary,
        recent_messages,
    }
}

pub(super) async fn browse_sessions(
    events: &mpsc::Sender<HostEvent>,
    current_session_id: &str,
    sessions: Vec<InteractiveSessionBrowserEntry>,
) -> Result<Option<String>, String> {
    let (response_tx, response_rx) = oneshot::channel();
    events
        .send(HostEvent::SessionBrowser(InteractiveSessionBrowser {
            current_session_id: current_session_id.into(),
            sessions,
            response: response_tx,
        }))
        .await
        .map_err(|_| "terminal event loop disconnected".to_owned())?;
    match tokio::time::timeout(INTERACTIVE_PROMPT_TIMEOUT, response_rx)
        .await
        .map_err(|_| "interactive session browser timed out".to_owned())?
        .map_err(|_| "interactive session browser was dropped".to_owned())?
    {
        PromptResponse::Answer(session_id) => Ok(Some(session_id)),
        PromptResponse::Cancelled => Ok(None),
    }
}

pub(super) async fn browse_themes(
    events: &mpsc::Sender<HostEvent>,
    themes: &ThemeLibrary,
    preferences: &TerminalPreferences,
) -> Result<Option<String>, String> {
    let entries = themes
        .names()
        .into_iter()
        .map(|name| {
            themes
                .preview_preferences(&name, preferences)
                .map(|preferences| InteractiveThemePickerEntry { name, preferences })
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (response_tx, response_rx) = oneshot::channel();
    events
        .send(HostEvent::ThemePicker(InteractiveThemePicker {
            current_theme: preferences.theme_name().into(),
            themes: entries,
            response: response_tx,
        }))
        .await
        .map_err(|_| "terminal event loop disconnected".to_owned())?;
    match tokio::time::timeout(INTERACTIVE_PROMPT_TIMEOUT, response_rx)
        .await
        .map_err(|_| "interactive theme picker timed out".to_owned())?
        .map_err(|_| "interactive theme picker was dropped".to_owned())?
    {
        PromptResponse::Answer(theme) => Ok(Some(theme)),
        PromptResponse::Cancelled => Ok(None),
    }
}

pub(super) fn compact_text(value: &str, maximum_characters: usize) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '\u{200b}'
                        | '\u{200c}'
                        | '\u{200d}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2060}'..='\u{206f}'
                        | '\u{feff}'
                )
            {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(maximum_characters)
        .collect()
}

pub(super) fn bounded_approval_content(value: &Value) -> Result<String, serde_json::Error> {
    let content = serde_json::to_string_pretty(value)?;
    if content.chars().count() <= APPROVAL_CONTENT_PREVIEW_CHARACTERS {
        return Ok(content);
    }
    let mut preview = content
        .chars()
        .take(APPROVAL_CONTENT_PREVIEW_CHARACTERS)
        .collect::<String>();
    preview.push_str("\n… request display truncated at 65,536 characters");
    Ok(preview)
}

fn actor_type_label(actor_type: ActorType) -> &'static str {
    match actor_type {
        ActorType::User => "Operator",
        ActorType::Application => "Application",
        ActorType::Model => "Model",
        ActorType::Workflow => "Workflow",
        ActorType::Subagent => "Child agent",
        ActorType::System => "System service",
    }
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
        kind: InteractivePromptKind,
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
                kind,
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
        let content = bounded_approval_content(&request.content)
            .map_err(|error| PolicyError::Unavailable(error.to_string()))?;
        let mut details = vec![
            (
                "Requested by".into(),
                format!(
                    "{} · {}",
                    actor_type_label(request.actor.actor_type),
                    request.actor.id
                ),
            ),
            ("Action".into(), request.action.clone()),
            ("Resource".into(), request.resource.clone()),
            ("Reason".into(), decision.reason.clone()),
        ];
        if let Some(reason) = request.risk.reason.as_deref() {
            let level = request.risk.level.as_deref().unwrap_or("not assessed");
            details.push(("Risk review".into(), format!("{level}: {reason}")));
        }
        let document = PresentationDocument::from_block(PresentationBlock::Card {
            title: "Approval required".into(),
            tone: PresentationTone::Warning,
            body: vec![
                PresentationBlock::KeyValue(details),
                PresentationBlock::Code {
                    language: Some("exact prepared request".into()),
                    content,
                },
            ],
        });
        let response = self
            .router
            .prompt(
                format!("approval:{}", decision.decision_id),
                InteractivePromptKind::Approval,
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
                InteractivePromptKind::UserInput,
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
