use super::*;

impl Runtime {
    /// Load one canonical durable child-agent job.
    pub fn get_subagent(&self, id: &str) -> Result<Option<SubagentJob>, RuntimeError> {
        self.work.get_subagent(id).map_err(Into::into)
    }

    /// List bounded durable child-agent jobs.
    pub fn list_subagents(
        &self,
        session_id: Option<&str>,
        status: Option<SubagentStatus>,
        limit: usize,
    ) -> Result<Vec<SubagentJob>, RuntimeError> {
        self.work
            .list_subagents(session_id, status, limit)
            .map_err(Into::into)
    }

    /// Queue a durable child-agent job from an embedded or terminal caller.
    pub async fn queue_subagent(
        &self,
        session_id: &str,
        task: &str,
        role: &str,
    ) -> Result<SubagentJob, RuntimeError> {
        let lineage = format!("manual-{}", Uuid::now_v7());
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::SubagentCreate {
                session_id: session_id.into(),
                parent_run_id: lineage.clone(),
                parent_call_id: lineage,
                task: task.into(),
                role: role.into(),
                allowed_tools: None,
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Return scheduler counts without changing queued state.
    pub fn subagent_queue_status(
        &self,
        session_id: Option<&str>,
    ) -> Result<SubagentQueueStatus, RuntimeError> {
        let jobs = self.work.list_subagents(session_id, None, 1_000)?;
        let count = |status| jobs.iter().filter(|job| job.status == status).count();
        let running = count(SubagentStatus::Running);
        Ok(SubagentQueueStatus {
            total: jobs.len(),
            queued: count(SubagentStatus::Queued),
            running,
            completed: count(SubagentStatus::Completed),
            failed: count(SubagentStatus::Failed),
            cancelled: count(SubagentStatus::Cancelled),
            interrupted: count(SubagentStatus::Interrupted),
            max_concurrent: self.subagent_max_concurrent,
            available_slots: self.subagent_max_concurrent.saturating_sub(running),
        })
    }

    /// Cancel one queued or running child job. Late child output is never committed.
    pub async fn cancel_subagent(&self, id: &str) -> Result<SubagentJob, RuntimeError> {
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::SubagentStop {
                id: id.into(),
                status: SubagentStatus::Cancelled,
                error: "Subagent job was cancelled.".into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Requeue one failed, cancelled, or interrupted child job.
    pub async fn requeue_subagent(&self, id: &str) -> Result<SubagentJob, RuntimeError> {
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::SubagentRequeue { id: id.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Drain queued jobs with bounded local concurrency using normal child agent runs.
    pub async fn drain_subagents(&self) -> Result<SubagentQueueStatus, RuntimeError> {
        let _drain_guard = self.subagent_drain_lock.lock().await;
        loop {
            let queued = self
                .work
                .list_subagents(None, Some(SubagentStatus::Queued), 1_000)?;
            if queued.is_empty() {
                break;
            }
            let batch = queued
                .into_iter()
                .take(self.subagent_max_concurrent)
                .collect::<Vec<_>>();
            let mut running = Vec::with_capacity(batch.len());
            for job in batch {
                let started: SubagentJob = serde_json::from_value(
                    self.execute_work_operation(WorkOperation::SubagentStart { id: job.id })
                        .await?,
                )
                .map_err(|error| RuntimeError::Config(error.to_string()))?;
                running.push(started);
            }
            let mut set = JoinSet::new();
            for job in running {
                let agent = Arc::clone(&self.agent);
                let max_turns = self.agent_max_turns;
                set.spawn(
                    async move {
                        let instructions = format!(
                            "You are a durable Colossus child agent for job {}. Complete only the assigned task. Nested delegation is disabled. Return a concise result for the parent.",
                            job.id
                        );
                        let result = agent
                            .run_subagent(
                                &job.role,
                                &instructions,
                                &job.task,
                                max_turns,
                                &job.child_session_id,
                                &job.id,
                                job.allowed_tools.as_deref(),
                            )
                            .await;
                        (job.id, result)
                    }
                    .instrument(tracing::Span::current()),
                );
            }
            while let Some(joined) = set.join_next().await {
                let (id, result) = joined.map_err(|error| {
                    RuntimeError::Config(format!("subagent scheduler join failed: {error}"))
                })?;
                let current = self
                    .work
                    .get_subagent(&id)?
                    .ok_or_else(|| StoreError::NotFound(format!("subagent {id}")))?;
                if current.status == SubagentStatus::Cancelled {
                    continue;
                }
                match result {
                    Ok(result) => {
                        let completion = self
                            .execute_work_operation(WorkOperation::SubagentComplete {
                                id: id.clone(),
                                child_run_id: result.run_id,
                                output: bounded_tool_text(&result.output, 64 * 1024),
                            })
                            .await;
                        if let Err(error) = completion {
                            let cancelled = self
                                .work
                                .get_subagent(&id)?
                                .is_some_and(|job| job.status == SubagentStatus::Cancelled);
                            if !cancelled {
                                return Err(error);
                            }
                        }
                    }
                    Err(error) => {
                        let failure = self
                            .execute_work_operation(WorkOperation::SubagentStop {
                                id: id.clone(),
                                status: SubagentStatus::Failed,
                                error: bounded_tool_text(&error.to_string(), 64 * 1024),
                            })
                            .await;
                        if let Err(error) = failure {
                            let cancelled = self
                                .work
                                .get_subagent(&id)?
                                .is_some_and(|job| job.status == SubagentStatus::Cancelled);
                            if !cancelled {
                                return Err(error);
                            }
                        }
                    }
                }
            }
        }
        self.subagent_queue_status(None)
    }
}
