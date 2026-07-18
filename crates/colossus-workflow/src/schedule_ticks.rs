use super::*;

impl WorkflowService {
    /// Evaluate every due schedule against an explicit UTC clock value.
    pub fn tick_schedules_at(
        &self,
        now: &str,
    ) -> Result<Vec<WorkflowScheduleDispatch>, WorkflowError> {
        let now = parse_schedule_time(now, "scheduler clock")?;
        self.tick_schedules(now)
    }

    /// Evaluate every due schedule using the current UTC clock.
    pub fn tick_schedules_now(&self) -> Result<Vec<WorkflowScheduleDispatch>, WorkflowError> {
        self.tick_schedules(OffsetDateTime::now_utc())
    }

    pub(super) fn tick_schedules(
        &self,
        now: OffsetDateTime,
    ) -> Result<Vec<WorkflowScheduleDispatch>, WorkflowError> {
        let now_text = format_schedule_time(now)?;
        let _guard = self
            .event_writer
            .lock()
            .map_err(|error| StoreError::Adapter(error.to_string()))?;
        let schedules = self.repository.schedules(MAX_WORKFLOW_SCHEDULES)?;
        let mut dispatches = Vec::new();
        for mut schedule in schedules.into_iter().filter(|schedule| schedule.enabled) {
            let next_fire = parse_schedule_time(&schedule.next_fire_at, "next schedule fire")?;
            if now < next_fire {
                continue;
            }
            let elapsed_seconds = (now - next_fire).whole_seconds();
            let cadence = i64::try_from(schedule.cadence_seconds)
                .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?;
            let due_count = u64::try_from(elapsed_seconds / cadence + 1)
                .map_err(|error| WorkflowError::InvalidTransition(error.to_string()))?;
            let latest_due = add_schedule_occurrences(
                next_fire,
                schedule.cadence_seconds,
                due_count.saturating_sub(1),
            )?;
            let latest_due_text = format_schedule_time(latest_due)?;

            let definition = self
                .repository
                .definition(&schedule.workflow_name, &schedule.workflow_version)?;
            let trust_failure = match definition.as_ref() {
                None => Some("pinned workflow definition is missing"),
                Some((_, current_hash)) if current_hash != &schedule.workflow_hash => {
                    Some("pinned workflow definition hash changed")
                }
                Some((definition, _)) => {
                    match validate_call_graph(self.repository.as_ref(), definition, true) {
                        Err(WorkflowError::Store(error)) => return Err(error.into()),
                        Err(_) => Some("pinned workflow call graph is no longer valid"),
                        Ok(()) => match validate_instance(
                            &definition.inputs,
                            &schedule.inputs,
                            "schedule input",
                        ) {
                            Err(WorkflowError::Store(error)) => return Err(error.into()),
                            Err(_) => Some("pinned workflow input is no longer valid"),
                            Ok(()) => None,
                        },
                    }
                }
            };
            if let Some(reason) = trust_failure {
                schedule.enabled = false;
                schedule.blocked_reason = Some(reason.into());
                schedule.updated_at = now_text.clone();
                let expected_version = self.schedule_version(&schedule.schedule_id)?;
                self.journal.append(schedule_event(
                    &schedule,
                    expected_version,
                    "workflow.schedule.blocked.v1",
                    json!({
                        "record": &schedule,
                        "reason": reason,
                        "scheduled_at": latest_due_text,
                    }),
                ))?;
                dispatches.push(WorkflowScheduleDispatch {
                    schedule_id: schedule.schedule_id.clone(),
                    status: WorkflowScheduleDispatchStatus::Blocked,
                    scheduled_at: Some(latest_due_text),
                    next_fire_at: schedule.next_fire_at.clone(),
                    missed_occurrences: 0,
                    run_id: None,
                    reason: Some(reason.into()),
                });
                continue;
            }

            let next_fire =
                add_schedule_occurrences(next_fire, schedule.cadence_seconds, due_count)?;
            let next_fire_text = format_schedule_time(next_fire)?;
            let skip =
                due_count > 1 && schedule.misfire_policy == WorkflowScheduleMisfirePolicy::Skip;
            schedule.next_fire_at = next_fire_text.clone();
            schedule.last_scheduled_at = Some(latest_due_text.clone());
            schedule.updated_at = now_text.clone();
            schedule.blocked_reason = None;
            let expected_schedule_version = self.schedule_version(&schedule.schedule_id)?;
            if skip {
                self.journal.append(schedule_event(
                    &schedule,
                    expected_schedule_version,
                    "workflow.schedule.skipped.v1",
                    json!({
                        "record": &schedule,
                        "scheduled_at": latest_due_text,
                        "due_occurrences": due_count,
                        "missed_occurrences": due_count,
                    }),
                ))?;
                dispatches.push(WorkflowScheduleDispatch {
                    schedule_id: schedule.schedule_id.clone(),
                    status: WorkflowScheduleDispatchStatus::Skipped,
                    scheduled_at: Some(latest_due_text),
                    next_fire_at: next_fire_text,
                    missed_occurrences: due_count,
                    run_id: None,
                    reason: None,
                });
                continue;
            }

            let run_id = scheduled_run_id(&schedule.schedule_id, &latest_due_text);
            if self.repository.run(&run_id)?.is_some() {
                return Err(WorkflowError::InvalidTransition(format!(
                    "deterministic scheduled run {run_id} already exists before its schedule transition"
                )));
            }
            schedule.last_run_id = Some(run_id.clone());
            let schedule_id = schedule.schedule_id.clone();
            let run_event = scheduled_run_event(&schedule, &run_id, &latest_due_text);
            self.journal.append_batch(vec![
                schedule_event(
                    &schedule,
                    expected_schedule_version,
                    "workflow.schedule.fired.v1",
                    json!({
                        "record": &schedule,
                        "scheduled_at": latest_due_text,
                        "due_occurrences": due_count,
                        "missed_occurrences": due_count.saturating_sub(1),
                        "run_id": run_id,
                    }),
                ),
                run_event,
            ])?;
            dispatches.push(WorkflowScheduleDispatch {
                schedule_id,
                status: WorkflowScheduleDispatchStatus::Queued,
                scheduled_at: Some(latest_due_text),
                next_fire_at: next_fire_text,
                missed_occurrences: due_count.saturating_sub(1),
                run_id: Some(run_id),
                reason: None,
            });
        }
        Ok(dispatches)
    }

    pub(super) fn schedule_version(&self, schedule_id: &str) -> Result<u64, StoreError> {
        u64::try_from(
            self.journal
                .read_stream(&schedule_stream(schedule_id))?
                .len(),
        )
        .map_err(|error| StoreError::Adapter(error.to_string()))
    }
}
