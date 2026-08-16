use super::*;

impl Runtime {
    /// Reconstruct the current canonical terminal presentation profile.
    pub fn presentation_preferences(&self) -> Result<TerminalPreferences, RuntimeError> {
        self.presentation.load().map_err(Into::into)
    }

    /// Persist a complete presentation profile through policy, permit, and audit boundaries.
    pub async fn save_presentation_preferences(
        &self,
        preferences: TerminalPreferences,
    ) -> Result<TerminalPreferences, RuntimeError> {
        self.save_presentation_preferences_with_session(preferences, None)
            .await
    }

    /// Persist a terminal preference snapshot for one acknowledged interactive session.
    pub async fn save_presentation_preferences_for_session(
        &self,
        session_id: &str,
        preferences: TerminalPreferences,
    ) -> Result<TerminalPreferences, RuntimeError> {
        self.save_presentation_preferences_with_session(preferences, Some(session_id))
            .await
    }

    async fn save_presentation_preferences_with_session(
        &self,
        preferences: TerminalPreferences,
        session_id: Option<&str>,
    ) -> Result<TerminalPreferences, RuntimeError> {
        let operation = PresentationOperation::Save { preferences };
        let action = operation.action();
        let mut request = effect_request(
            terminal_actor(),
            action,
            "presentation:repl",
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![action.into()];
        request.context.session_id = session_id.map(str::to_owned);
        let result = self
            .gateway
            .execute(request, self.presentation_executor.as_ref())
            .await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Reconstruct newest encrypted terminal-history entries in chronological order.
    pub fn terminal_history(&self, limit: usize) -> Result<Vec<String>, RuntimeError> {
        self.presentation.list_history(limit).map_err(Into::into)
    }

    /// Append one terminal-history entry through policy, permit, and audit boundaries.
    pub async fn append_terminal_history(&self, entry: &str) -> Result<String, RuntimeError> {
        self.append_terminal_history_with_session(entry, None).await
    }

    /// Append one terminal-history entry for one acknowledged interactive session.
    pub async fn append_terminal_history_for_session(
        &self,
        session_id: &str,
        entry: &str,
    ) -> Result<String, RuntimeError> {
        self.append_terminal_history_with_session(entry, Some(session_id))
            .await
    }

    async fn append_terminal_history_with_session(
        &self,
        entry: &str,
        session_id: Option<&str>,
    ) -> Result<String, RuntimeError> {
        let operation = PresentationOperation::AppendHistory {
            entry: entry.into(),
        };
        let action = operation.action();
        let resource = operation.resource();
        let mut request = effect_request(
            terminal_actor(),
            action,
            resource,
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![action.into()];
        request.context.session_id = session_id.map(str::to_owned);
        let result = self
            .gateway
            .execute(request, self.presentation_executor.as_ref())
            .await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Compatibility alias for callers using the original interactive surface name.
    #[deprecated(note = "use Runtime::terminal_history")]
    pub fn repl_history(&self, limit: usize) -> Result<Vec<String>, RuntimeError> {
        self.terminal_history(limit)
    }

    /// Compatibility alias for callers using the original interactive surface name.
    #[deprecated(note = "use Runtime::append_terminal_history")]
    pub async fn append_repl_history(&self, entry: &str) -> Result<String, RuntimeError> {
        self.append_terminal_history(entry).await
    }

    /// Create a durable empty session.
    pub fn create_session(&self, title: Option<&str>) -> Result<SessionSummary, RuntimeError> {
        let id = Uuid::now_v7().to_string();
        self.sessions
            .create_session(
                &id,
                title,
                Actor {
                    actor_type: ActorType::User,
                    id: "terminal-user".into(),
                },
            )
            .map_err(Into::into)
    }

    /// Materialize one caller-owned session whose identifier was durably allocated by
    /// an authenticated application coordinator.
    pub fn create_application_session(
        &self,
        id: &str,
        title: Option<&str>,
        actor: Actor,
    ) -> Result<SessionSummary, RuntimeError> {
        self.sessions
            .create_session(id, title, actor)
            .map_err(Into::into)
    }

    /// Append one visible application-owned message to a canonical session.
    ///
    /// Application coordinators use this when a workflow, such as Research, owns
    /// its orchestration outside the conversational agent loop but must preserve
    /// the user's question for subsequent turns.
    pub fn append_application_message(
        &self,
        session_id: &str,
        run_id: &str,
        message: ModelMessage,
        actor: Actor,
    ) -> Result<SessionMessage, RuntimeError> {
        self.sessions
            .append_message(session_id, run_id, message, actor)
            .map_err(Into::into)
    }

    /// Reconstruct one exact session summary.
    pub fn get_session(&self, id: &str) -> Result<Option<SessionSummary>, RuntimeError> {
        self.sessions.get_session(id).map_err(Into::into)
    }

    /// List recent sessions newest first.
    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, RuntimeError> {
        self.sessions.list_sessions(limit).map_err(Into::into)
    }

    /// Resolve the most recently updated session.
    pub fn latest_session(&self) -> Result<SessionSummary, RuntimeError> {
        self.sessions
            .list_sessions(1)?
            .into_iter()
            .next()
            .ok_or_else(|| RuntimeError::Store(StoreError::NotFound("no sessions exist".into())))
    }

    /// Reconstruct append-only messages for an exact session.
    pub fn session_messages(&self, id: &str) -> Result<Vec<SessionMessage>, RuntimeError> {
        self.sessions.list_messages(id).map_err(Into::into)
    }

    /// Idempotently materialize an exact canonical session prefix as a separate branch.
    pub fn ensure_session_branch(
        &self,
        source_session_id: &str,
        source_message_count: u64,
        context_mode: RunBranchContextMode,
        branch_session_id: &str,
        branch_run_id: &str,
        actor: Actor,
    ) -> Result<SessionSummary, RuntimeError> {
        const MAX_BRANCH_MESSAGES: usize = 512;
        const MAX_BRANCH_BYTES: usize = 2 * 1024 * 1024;

        let source = self
            .sessions
            .get_session(source_session_id)?
            .ok_or_else(|| StoreError::NotFound("branch source session".into()))?;
        if source_message_count > source.message_count
            || source_message_count > MAX_BRANCH_MESSAGES as u64
        {
            return Err(StoreError::Verification("invalid branch message boundary".into()).into());
        }
        let source_messages = self.sessions.list_messages(source_session_id)?;
        let prefix = source_messages
            .into_iter()
            .take(source_message_count as usize)
            .collect::<Vec<_>>();
        let prefix = match context_mode {
            RunBranchContextMode::Exact => prefix,
            RunBranchContextMode::Conversation | RunBranchContextMode::SourceRunConversation => {
                prefix
                    .into_iter()
                    .filter_map(|mut record| match record.message.role {
                        ModelMessageRole::User => {
                            record.message.tool_call_id = None;
                            record.message.tool_calls.clear();
                            Some(record)
                        }
                        ModelMessageRole::Assistant
                            if !record.message.content.trim().is_empty() =>
                        {
                            record.message.tool_call_id = None;
                            record.message.tool_calls.clear();
                            Some(record)
                        }
                        ModelMessageRole::System
                        | ModelMessageRole::Assistant
                        | ModelMessageRole::Tool => None,
                    })
                    .collect()
            }
        };
        let bytes = prefix.iter().try_fold(0_usize, |total, message| {
            serde_json::to_vec(&message.message)
                .map(|value| total.saturating_add(value.len()))
                .map_err(|error| StoreError::Adapter(error.to_string()))
        })?;
        if bytes > MAX_BRANCH_BYTES {
            return Err(StoreError::Verification("branch context is too large".into()).into());
        }
        validate_model_transcript(
            &prefix
                .iter()
                .map(|message| message.message.clone())
                .collect::<Vec<_>>(),
        )
        .map_err(|error| StoreError::Verification(error.to_string()))?;

        if let Some(existing) = self.sessions.get_session(branch_session_id)? {
            let existing_messages = self.sessions.list_messages(branch_session_id)?;
            if existing_messages.is_empty() && !prefix.is_empty() {
                self.sessions.append_messages(
                    branch_session_id,
                    branch_run_id,
                    prefix
                        .into_iter()
                        .map(|message| SessionMessageAppend {
                            message: message.message,
                            actor: actor.clone(),
                        })
                        .collect(),
                )?;
                return self
                    .sessions
                    .get_session(branch_session_id)?
                    .ok_or_else(|| StoreError::NotFound("branch session".into()).into());
            }
            let expected = prefix
                .iter()
                .map(|message| &message.message)
                .collect::<Vec<_>>();
            let actual = existing_messages
                .iter()
                .map(|message| &message.message)
                .collect::<Vec<_>>();
            if actual != expected {
                return Err(
                    StoreError::Verification("branch session context drifted".into()).into(),
                );
            }
            return Ok(existing);
        }

        self.sessions
            .create_session(branch_session_id, source.title.as_deref(), actor.clone())?;
        if !prefix.is_empty() {
            self.sessions.append_messages(
                branch_session_id,
                branch_run_id,
                prefix
                    .into_iter()
                    .map(|message| SessionMessageAppend {
                        message: message.message,
                        actor: actor.clone(),
                    })
                    .collect(),
            )?;
        }
        self.sessions
            .get_session(branch_session_id)?
            .ok_or_else(|| StoreError::NotFound("branch session".into()).into())
    }

    /// Reconstruct a bounded page of canonical session messages newest-first by cursor.
    pub fn session_messages_page(
        &self,
        id: &str,
        before_sequence: Option<u64>,
        limit: usize,
    ) -> Result<SessionMessagePage, RuntimeError> {
        self.sessions
            .list_messages_page(
                id,
                before_sequence,
                limit.clamp(1, SESSION_MESSAGE_PAGE_LIMIT),
                SESSION_MESSAGE_PAGE_MAX_BYTES,
            )
            .map_err(Into::into)
    }

    /// Show active context budget and canonical-history size for one session.
    pub async fn context_status(&self, session_id: &str) -> Result<ContextStatus, RuntimeError> {
        self.context_status_for_role(session_id, "primary").await
    }

    /// Show active context budget for one session and logical model role.
    pub async fn context_status_for_role(
        &self,
        session_id: &str,
        role: &str,
    ) -> Result<ContextStatus, RuntimeError> {
        serde_json::from_value(
            self.execute_context_operation(ContextOperation::Show {
                session_id: session_id.into(),
                role: role.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// List immutable context snapshots for one session.
    pub async fn context_snapshots(
        &self,
        session_id: &str,
    ) -> Result<Vec<ContextSnapshot>, RuntimeError> {
        serde_json::from_value(
            self.execute_context_operation(ContextOperation::Snapshots {
                session_id: session_id.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Force a new context snapshot while preserving every canonical message.
    pub async fn compact_context(&self, session_id: &str) -> Result<PreparedContext, RuntimeError> {
        self.compact_context_for_role(session_id, "primary").await
    }

    /// Force a context snapshot using one logical role's model budget.
    pub async fn compact_context_for_role(
        &self,
        session_id: &str,
        role: &str,
    ) -> Result<PreparedContext, RuntimeError> {
        serde_json::from_value(
            self.execute_context_operation(ContextOperation::Compact {
                session_id: session_id.into(),
                role: role.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Activate an existing snapshot for subsequent provider turns.
    pub async fn restore_context(
        &self,
        session_id: &str,
        snapshot_id: &str,
    ) -> Result<ContextSnapshot, RuntimeError> {
        serde_json::from_value(
            self.execute_context_operation(ContextOperation::Restore {
                session_id: session_id.into(),
                snapshot_id: snapshot_id.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    pub(super) async fn execute_context_operation(
        &self,
        operation: ContextOperation,
    ) -> Result<Value, RuntimeError> {
        let session_id = operation.session_id().to_owned();
        let output = execute_context_effect(
            self.gateway.as_ref(),
            self.context_executor.as_ref(),
            terminal_actor(),
            ExecutionContext {
                correlation_id: Uuid::now_v7().to_string(),
                session_id: Some(session_id),
                ..ExecutionContext::default()
            },
            operation,
        )
        .await?;
        serde_json::from_str(&output).map_err(|error| RuntimeError::Config(error.to_string()))
    }
}
