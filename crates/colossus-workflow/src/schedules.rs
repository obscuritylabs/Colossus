use super::*;

impl WorkflowService {
    /// Create one bounded, hash-pinned cadence schedule.
    #[allow(clippy::too_many_arguments)]
    pub fn create_schedule(
        &self,
        schedule_id: &str,
        workflow_name: &str,
        workflow_version: &str,
        inputs: Value,
        cadence_seconds: u64,
        misfire_policy: WorkflowScheduleMisfirePolicy,
        enabled: bool,
        starts_at: Option<&str>,
    ) -> Result<WorkflowSchedule, WorkflowError> {
        let now = OffsetDateTime::now_utc();
        self.create_schedule_at(
            schedule_id,
            workflow_name,
            workflow_version,
            inputs,
            cadence_seconds,
            misfire_policy,
            enabled,
            starts_at,
            now,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn create_schedule_at(
        &self,
        schedule_id: &str,
        workflow_name: &str,
        workflow_version: &str,
        inputs: Value,
        cadence_seconds: u64,
        misfire_policy: WorkflowScheduleMisfirePolicy,
        enabled: bool,
        starts_at: Option<&str>,
        now: OffsetDateTime,
    ) -> Result<WorkflowSchedule, WorkflowError> {
        if schedule_id.is_empty()
            || schedule_id.len() > MAX_SCHEDULE_ID_BYTES
            || !valid_name(schedule_id)
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "schedule id must contain 1..={MAX_SCHEDULE_ID_BYTES} lowercase letters, digits, dots, or hyphens"
            )));
        }
        if !(MIN_SCHEDULE_CADENCE_SECONDS..=MAX_SCHEDULE_CADENCE_SECONDS).contains(&cadence_seconds)
        {
            return Err(WorkflowError::InvalidDefinition(format!(
                "schedule cadence must be between {MIN_SCHEDULE_CADENCE_SECONDS} and {MAX_SCHEDULE_CADENCE_SECONDS} seconds"
            )));
        }
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        if self.repository.schedule(schedule_id)?.is_some() {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow schedule {schedule_id} already exists"
            )));
        }
        if self.repository.schedules(MAX_WORKFLOW_SCHEDULES)?.len() >= MAX_WORKFLOW_SCHEDULES {
            return Err(WorkflowError::InvalidTransition(format!(
                "workflow schedule limit {MAX_WORKFLOW_SCHEDULES} is exhausted"
            )));
        }
        let (definition, workflow_hash) = self
            .repository
            .definition(workflow_name, workflow_version)?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!("{workflow_name}:{workflow_version}"))
            })?;
        validate_call_graph(self.repository.as_ref(), &definition, true)?;
        validate_instance(&definition.inputs, &inputs, "schedule input")?;
        let first_fire = match starts_at {
            Some(starts_at) => parse_schedule_time(starts_at, "schedule start")?,
            None => add_schedule_occurrences(now, cadence_seconds, 1)?,
        };
        let now = format_schedule_time(now)?;
        let starts_at = format_schedule_time(first_fire)?;
        let schedule = WorkflowSchedule {
            schedule_id: schedule_id.into(),
            workflow_name: workflow_name.into(),
            workflow_version: workflow_version.into(),
            workflow_hash,
            inputs,
            cadence_seconds,
            misfire_policy,
            enabled,
            starts_at: starts_at.clone(),
            next_fire_at: starts_at,
            last_scheduled_at: None,
            last_run_id: None,
            blocked_reason: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.repository.create_schedule(
            &schedule,
            Actor {
                actor_type: ActorType::User,
                id: "workflow-schedule-registrar".into(),
            },
        )?;
        Ok(schedule)
    }

    /// Reconstruct one canonical workflow schedule.
    pub fn get_schedule(&self, schedule_id: &str) -> Result<WorkflowSchedule, WorkflowError> {
        self.repository
            .schedule(schedule_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow schedule {schedule_id}")))
    }

    /// List bounded schedules in deterministic identifier order.
    pub fn list_schedules(&self, limit: usize) -> Result<Vec<WorkflowSchedule>, WorkflowError> {
        self.repository
            .schedules(limit.min(MAX_WORKFLOW_SCHEDULES))
            .map_err(Into::into)
    }

    /// Explicitly enable or disable one schedule after rechecking pinned trust.
    pub fn set_schedule_enabled(
        &self,
        schedule_id: &str,
        enabled: bool,
    ) -> Result<WorkflowSchedule, WorkflowError> {
        let now = format_schedule_time(OffsetDateTime::now_utc())?;
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let schedule = self
            .repository
            .schedule(schedule_id)?
            .ok_or_else(|| WorkflowError::NotFound(format!("workflow schedule {schedule_id}")))?;
        if enabled {
            let (definition, current_hash) = self
                .repository
                .definition(&schedule.workflow_name, &schedule.workflow_version)?
                .ok_or_else(|| WorkflowError::NotFound(schedule.workflow_name.clone()))?;
            if current_hash != schedule.workflow_hash {
                return Err(WorkflowError::InvalidTransition(
                    "schedule cannot be enabled because its pinned workflow definition changed"
                        .into(),
                ));
            }
            validate_call_graph(self.repository.as_ref(), &definition, true)?;
            validate_instance(&definition.inputs, &schedule.inputs, "schedule input")?;
        }
        self.repository
            .set_schedule_enabled(
                schedule_id,
                enabled,
                &now,
                Actor {
                    actor_type: ActorType::User,
                    id: "workflow-schedule-operator".into(),
                },
            )
            .map_err(Into::into)
    }
}
