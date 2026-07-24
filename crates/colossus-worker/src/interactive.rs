use super::*;

#[derive(Clone)]
pub(super) struct InteractiveRunBridge {
    pub(super) prompts: tokio::sync::mpsc::Sender<WorkerPrompt>,
    pub(super) notices: tokio::sync::mpsc::Sender<ApprovalReviewNotice>,
    pub(super) responses:
        Arc<tokio::sync::Mutex<BTreeMap<String, tokio::sync::oneshot::Sender<Option<String>>>>>,
}

impl InteractiveRunBridge {
    pub(super) async fn request(&self, prompt: WorkerPrompt) -> Result<Option<String>, String> {
        self.request_with_timeout(prompt, INTERACTIVE_PROMPT_TIMEOUT)
            .await
    }

    pub(super) async fn request_with_timeout(
        &self,
        prompt: WorkerPrompt,
        timeout: Duration,
    ) -> Result<Option<String>, String> {
        let prompt_id = prompt.prompt_id.clone();
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        if self
            .responses
            .lock()
            .await
            .insert(prompt_id.clone(), response_tx)
            .is_some()
        {
            return Err("duplicate interactive prompt id".into());
        }
        if self.prompts.send(prompt).await.is_err() {
            self.responses.lock().await.remove(&prompt_id);
            return Err("interactive worker client disconnected".into());
        }
        match tokio::time::timeout(timeout, response_rx).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(_)) => Err("interactive worker response channel closed".into()),
            Err(_) => {
                self.responses.lock().await.remove(&prompt_id);
                Err("interactive worker prompt timed out".into())
            }
        }
    }

    pub(super) async fn respond(
        &self,
        prompt_id: &str,
        answer: Option<String>,
    ) -> Result<(), WorkerError> {
        let response = self
            .responses
            .lock()
            .await
            .remove(prompt_id)
            .ok_or_else(|| WorkerError::Protocol("unknown, replayed, or wrong prompt id".into()))?;
        response
            .send(answer)
            .map_err(|_| WorkerError::Protocol("prompt response arrived after closure".into()))
    }

    pub(super) async fn cancel_all(&self) {
        let pending = std::mem::take(&mut *self.responses.lock().await);
        for (_, response) in pending {
            let _ = response.send(None);
        }
    }
}

tokio::task_local! {
    pub(super) static ACTIVE_INTERACTIVE_RUN: InteractiveRunBridge;
}

pub(super) struct WorkerInteractiveApproval {
    pub(super) mode: WorkerApprovalMode,
}

#[async_trait]
impl ApprovalProvider for WorkerInteractiveApproval {
    fn risk_auto_enabled(&self) -> bool {
        self.mode == WorkerApprovalMode::RiskAuto
    }

    async fn automatic_approval_granted(&self, notice: AutomaticApprovalNotice) {
        let Ok(bridge) = ACTIVE_INTERACTIVE_RUN.try_with(Clone::clone) else {
            return;
        };
        let _ = bridge
            .notices
            .try_send(ApprovalReviewNotice::AutomaticApproval { notice });
    }

    async fn risk_review_fallback(&self, notice: RiskReviewFallbackNotice) {
        let Ok(bridge) = ACTIVE_INTERACTIVE_RUN.try_with(Clone::clone) else {
            return;
        };
        let _ = bridge
            .notices
            .try_send(ApprovalReviewNotice::RiskReviewFallback { notice });
    }

    async fn request_approval(
        &self,
        request: &EffectRequest,
        request_hash: &str,
        decision: &PolicyDecision,
    ) -> Result<Option<ApprovalProof>, PolicyError> {
        match self.mode {
            WorkerApprovalMode::Deny => return Ok(None),
            WorkerApprovalMode::FullAccess => {
                return ApprovalProvider::request_approval(
                    &AllowApproval {
                        approved_by: "worker:full-access".into(),
                    },
                    request,
                    request_hash,
                    decision,
                )
                .await;
            }
            WorkerApprovalMode::Ask | WorkerApprovalMode::RiskAuto => {}
        }
        let bridge = ACTIVE_INTERACTIVE_RUN.try_with(Clone::clone).map_err(|_| {
            PolicyError::Unavailable("no interactive worker client attached".into())
        })?;
        let answer = bridge
            .request(WorkerPrompt {
                prompt_id: Uuid::now_v7().to_string(),
                kind: WorkerPromptKind::Approval,
                title: "Approval required".into(),
                question: decision.reason.clone(),
                choices: vec!["Allow once".into(), "Deny".into()],
                allow_free_form: false,
                details: json!({
                    "action": request.action,
                    "resource": request.resource,
                    "content": request.content,
                    "decision_id": decision.decision_id,
                }),
            })
            .await
            .map_err(PolicyError::Unavailable)?;
        if answer.as_deref() != Some("Allow once") {
            return Ok(None);
        }
        ApprovalProvider::request_approval(
            &AllowApproval {
                approved_by: "worker:interactive".into(),
            },
            request,
            request_hash,
            decision,
        )
        .await
    }
}

pub(super) struct WorkerInteractiveUserPrompt;

#[async_trait]
impl UserPromptProvider for WorkerInteractiveUserPrompt {
    async fn prompt(&self, request: UserPromptRequest) -> Result<UserPromptResponse, ToolError> {
        let bridge = ACTIVE_INTERACTIVE_RUN
            .try_with(Clone::clone)
            .map_err(|_| ToolError::Failed("no interactive worker client attached".into()))?;
        let answer = bridge
            .request(WorkerPrompt {
                prompt_id: Uuid::now_v7().to_string(),
                kind: WorkerPromptKind::UserInput,
                title: "Input needed".into(),
                question: request.question.clone(),
                choices: request.choices.clone(),
                allow_free_form: request.allow_free_form,
                details: Value::Null,
            })
            .await
            .map_err(ToolError::Failed)?
            .ok_or_else(|| ToolError::Failed("user cancelled the question".into()))?;
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
