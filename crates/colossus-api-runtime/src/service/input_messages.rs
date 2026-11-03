use super::*;
use colossus_api::{
    ReleasedArtifactPurpose, ReleasedArtifactReference, ReleasedArtifactState, ReleasedContentPart,
    ReleasedMessageRole, ReleasedSessionMessage,
};
use colossus_contracts::{ModelMessageRole, SessionMessage};

const INPUT_MESSAGE_SCAN_LIMIT: usize = 1_024;
const INPUT_MESSAGE_PAGE_SIZE: usize = 64;

pub(super) struct InputMessageRelease {
    runtime: Arc<Runtime>,
    artifacts: Arc<dyn ArtifactApi>,
    caller: CallerContext,
    run_id: String,
    session_id: String,
    input: Vec<ContentPart>,
    expected: Option<ModelContent>,
    branch: bool,
    lower_bound: Option<u64>,
    released: bool,
}

impl InputMessageRelease {
    pub(super) fn matches_parent(&self, envelope: &RunEventEnvelope) -> bool {
        envelope.run_id == self.run_id && envelope.session_id == self.session_id
    }
    pub(super) fn new(
        service: &RuntimeAgentRunApi,
        caller: &CallerContext,
        run: &Run,
        request: &CreateRunRequest,
        expected: Option<ModelContent>,
    ) -> Option<Self> {
        // These paths do not append the accepted request as a parent user message.
        if request.mode == RunMode::Goal
            || matches!(request.plan_action, Some(PlanRunAction::Execute { .. }))
            || colossus_ports::AgentInbox::initial_origin(
                service.runtime.communication().as_ref(),
                &colossus_contracts::ExecutionContext {
                    run_id: Some(run.id.clone()),
                    session_id: Some(run.session_id.clone()),
                    ..Default::default()
                },
            )
            .is_ok_and(|origin| origin.is_some())
        {
            return None;
        }
        Some(Self {
            runtime: Arc::clone(&service.runtime),
            artifacts: Arc::clone(&service.artifacts),
            caller: caller.clone(),
            run_id: run.id.clone(),
            session_id: run.session_id.clone(),
            input: request.input.clone(),
            expected,
            branch: request.branch.is_some(),
            lower_bound: None,
            released: false,
        })
    }

    pub(super) fn after(mut self, lower_bound: Option<u64>) -> Self {
        self.lower_bound = lower_bound;
        self
    }

    pub(super) async fn publish(&mut self, writer: &RunWriter) -> ApiResult<bool> {
        if self.released {
            return Ok(true);
        }
        if writer.user_message_published(&self.input)? {
            self.released = true;
            return Ok(true);
        }
        // Copied context carries the new branch run ID too. Prove its exact
        // private boundary before searching for the newly accepted input.
        if self.branch
            && self
                .runtime
                .get_session(&self.session_id)
                .map_err(|_| input_error(&self.caller))?
                .is_some()
        {
            let copied = self
                .runtime
                .session_branch_input_boundary(&self.session_id, &self.run_id)
                .map_err(|_| input_error(&self.caller))?
                .ok_or_else(|| input_error(&self.caller))?;
            self.lower_bound = Some(self.lower_bound.unwrap_or(0).max(copied));
        }
        let Some(canonical) = self.canonical()? else {
            return Ok(false);
        };
        if canonical.message.agent_message_origin.is_some() {
            self.released = true;
            return Ok(true);
        }
        if self
            .expected
            .as_ref()
            .is_some_and(|expected| expected != &canonical.message.content)
        {
            return Err(input_error(&self.caller));
        }
        let mut content = Vec::with_capacity(self.input.len());
        for part in &self.input {
            content.push(match part {
                ContentPart::Text { text } => ReleasedContentPart::Text { text: text.clone() },
                ContentPart::Artifact { artifact_id } => {
                    let artifact = self.artifacts.get(&self.caller, artifact_id).await?;
                    if artifact.state != ArtifactState::Available
                        || artifact.purpose != ArtifactPurpose::RunInput
                    {
                        return Err(input_error(&self.caller));
                    }
                    ReleasedContentPart::Artifact {
                        artifact: ReleasedArtifactReference {
                            artifact_id: artifact.artifact_id,
                            file_name: artifact.file_name,
                            media_type: artifact.media_type,
                            size_bytes: artifact.size_bytes,
                            sha256: artifact.sha256,
                            purpose: ReleasedArtifactPurpose::RunInput,
                            state: ReleasedArtifactState::Available,
                            created_at: artifact.created_at,
                        },
                    }
                }
            });
        }
        verify_media(&canonical.message.content, &content)
            .map_err(|()| input_error(&self.caller))?;
        writer.append_user_message_once(ReleasedSessionMessage {
            session_id: canonical.session_id,
            run_id: canonical.run_id,
            sequence: canonical.sequence,
            role: ReleasedMessageRole::User,
            content,
            created_at: canonical.created_at,
        })?;
        self.released = true;
        Ok(true)
    }

    fn canonical(&self) -> ApiResult<Option<SessionMessage>> {
        if self
            .runtime
            .get_session(&self.session_id)
            .map_err(|_| input_error(&self.caller))?
            .is_none()
        {
            return Ok(None);
        }
        let mut before = None;
        let mut scanned = 0;
        loop {
            let page = self
                .runtime
                .session_messages_page(&self.session_id, before, INPUT_MESSAGE_PAGE_SIZE)
                .map_err(|_| input_error(&self.caller))?;
            scanned += page.messages.len();
            let oldest = page.messages.first().map(|message| message.sequence);
            if let Some(message) = page.messages.into_iter().find(|message| {
                message.session_id == self.session_id
                    && message.run_id == self.run_id
                    && message.message.role == ModelMessageRole::User
                    && self
                        .lower_bound
                        .is_none_or(|lower| message.sequence > lower)
            }) {
                return Ok(Some(message));
            }
            if !page.has_more
                || self
                    .lower_bound
                    .is_some_and(|lower| oldest.is_some_and(|sequence| sequence <= lower))
            {
                return Ok(None);
            }
            if scanned >= INPUT_MESSAGE_SCAN_LIMIT || page.before_sequence == before {
                return Err(input_error(&self.caller));
            }
            before = page.before_sequence;
        }
    }
}

fn verify_media(model: &ModelContent, released: &[ReleasedContentPart]) -> Result<(), ()> {
    let expected = released
        .iter()
        .filter_map(|part| match part {
            ReleasedContentPart::Artifact { artifact }
                if artifact.media_type.starts_with("image/")
                    || colossus_runtime::supported_document_media_type(&artifact.media_type) =>
            {
                Some(artifact)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let ModelContent::Parts(parts) = model else {
        return if expected.is_empty() { Ok(()) } else { Err(()) };
    };
    let actual = parts
        .iter()
        .filter_map(|part| match part {
            ModelContentPart::Image { image } => Some((
                &image.artifact_id,
                &image.file_name,
                &image.media_type,
                image.size_bytes,
                &image.sha256,
            )),
            ModelContentPart::File { file } => Some((
                &file.artifact_id,
                &file.file_name,
                &file.media_type,
                file.size_bytes,
                &file.sha256,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    if actual.len() != expected.len()
        || actual.iter().zip(expected).any(|(actual, expected)| {
            *actual
                != (
                    &expected.artifact_id,
                    &expected.file_name,
                    &expected.media_type,
                    expected.size_bytes,
                    &expected.sha256,
                )
        })
    {
        return Err(());
    }
    Ok(())
}

fn input_error(caller: &CallerContext) -> ApiError {
    ApiError::from_store(
        &StoreError::Verification("the canonical user input release could not be verified".into()),
        caller.request_id(),
    )
}
