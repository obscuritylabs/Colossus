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
        let prepared = self.prepare_agent_instructions("", "")?;
        let instruction_snapshot_id = if let Some(snapshot) = prepared.snapshot {
            self.instruction_snapshots.persist(&snapshot)?;
            Some(snapshot.id().to_owned())
        } else {
            None
        };
        serde_json::from_value(
            self.execute_work_operation(WorkOperation::SubagentCreate {
                session_id: session_id.into(),
                parent_run_id: lineage.clone(),
                parent_call_id: lineage,
                task: task.into(),
                role: role.into(),
                allowed_tools: None,
                instruction_snapshot_id,
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
        self.drain_subagents_with_events().await
    }

    pub(super) async fn drain_subagents_with_events(
        &self,
    ) -> Result<SubagentQueueStatus, RuntimeError> {
        let _drain_guard = self.subagent_drain_lock.lock().await;
        let mut set = JoinSet::new();
        loop {
            let available = self.subagent_max_concurrent.saturating_sub(set.len());
            let queued = if available == 0 {
                Vec::new()
            } else {
                self.work
                    .list_subagents(None, Some(SubagentStatus::Queued), 1_000)?
                    .into_iter()
                    .take(available)
                    .collect::<Vec<_>>()
            };
            for job in queued {
                let started: SubagentJob = serde_json::from_value(
                    self.execute_work_operation(WorkOperation::SubagentStart { id: job.id })
                        .await?,
                )
                .map_err(|error| RuntimeError::Config(error.to_string()))?;
                self.emit_subagent_update(&started).await;
                let job = started;
                let agent = Arc::clone(&self.agent);
                let max_turns = self.agent_max_turns;
                let inherited_instructions =
                    match self.work.subagent_instruction_snapshot_id(&job.id)? {
                        Some(id) => self.instruction_snapshots.load(&id)?.compose(),
                        None => String::new(),
                    };
                set.spawn(
                    async move {
                        let child_instructions = format!(
                            "You are a durable Colossus child agent for job {}. Complete only the assigned task. Nested delegation is disabled. Return a concise result for the parent.",
                            job.id
                        );
                        let instructions = if inherited_instructions.is_empty() {
                            child_instructions
                        } else {
                            format!("{inherited_instructions}\n\n{child_instructions}")
                        };
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
            if set.is_empty() {
                break;
            }
            let joined = if set.len() < self.subagent_max_concurrent {
                tokio::select! {
                    joined = set.join_next() => joined,
                    _ = self.subagent_notify.notified() => continue,
                }
            } else {
                set.join_next().await
            };
            let Some(joined) = joined else {
                continue;
            };
            let (id, result) = joined.map_err(|error| {
                RuntimeError::Config(format!("subagent scheduler join failed: {error}"))
            })?;
            let current = self
                .work
                .get_subagent(&id)?
                .ok_or_else(|| StoreError::NotFound(format!("subagent {id}")))?;
            if current.status == SubagentStatus::Cancelled {
                self.emit_subagent_update(&current).await;
                continue;
            }
            let transition = match result {
                Ok(result) => {
                    self.execute_work_operation(WorkOperation::SubagentComplete {
                        id: id.clone(),
                        child_run_id: result.run_id,
                        output: bounded_tool_text(&result.output, 64 * 1024),
                    })
                    .await
                }
                Err(error) => {
                    self.execute_work_operation(WorkOperation::SubagentStop {
                        id: id.clone(),
                        status: SubagentStatus::Failed,
                        error: bounded_tool_text(&error.to_string(), 64 * 1024),
                    })
                    .await
                }
            };
            match transition {
                Ok(value) => {
                    let terminal: SubagentJob = serde_json::from_value(value)
                        .map_err(|error| RuntimeError::Config(error.to_string()))?;
                    self.emit_subagent_update(&terminal).await;
                }
                Err(error) => {
                    let latest = self
                        .work
                        .get_subagent(&id)?
                        .ok_or_else(|| StoreError::NotFound(format!("subagent {id}")))?;
                    if latest.status == SubagentStatus::Cancelled {
                        self.emit_subagent_update(&latest).await;
                    } else {
                        return Err(error);
                    }
                }
            }
        }
        self.subagent_queue_status(None)
    }

    async fn emit_subagent_update(&self, job: &SubagentJob) {
        let events = self
            .subagent_event_sinks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&job.parent_run_id)
            .cloned();
        if let Some(events) = events {
            let _ = events
                .send(RunEventEnvelope {
                    schema_version: 1,
                    run_id: job.parent_run_id.clone(),
                    session_id: job.session_id.clone(),
                    event: RunEvent::SubagentUpdated {
                        job: Box::new(job.clone()),
                    },
                })
                .await;
        }
    }
}
