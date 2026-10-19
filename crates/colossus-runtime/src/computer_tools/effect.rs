use super::{
    arguments::ComputerInvocation,
    service::{ComputerRun, RuntimeComputerTools},
};
use crate::prelude::*;
use colossus_contracts::{
    ComputerControlState, ComputerObservation, ComputerScreenshotDescriptor, ComputerSessionId,
    ComputerSessionSummary, ComputerWindowSummary,
};
use colossus_ports::{
    AgentRunLifecycle, ComputerCommand, ComputerDriverError, ComputerDriverReply,
    ComputerDriverRequest,
};

pub(super) enum StagedComputerReply {
    Empty,
    Windows(Vec<ComputerWindowSummary>),
    Session(ComputerSessionSummary),
    Observation(ComputerObservation, Instant),
    Screenshot(ComputerScreenshotDescriptor, Instant),
    Closed,
}

pub(super) struct ComputerEffect<'a> {
    pub(super) computer: &'a RuntimeComputerTools,
    pub(super) staged: &'a StdMutex<Option<StagedComputerReply>>,
}

#[async_trait]
impl EffectExecutor for ComputerEffect<'_> {
    async fn execute(
        &self,
        request: &EffectRequest,
        permit: ExecutionPermit,
    ) -> Result<QuarantinedEffectResult, ExecutionError> {
        if !permit.obligations().require_post_effect {
            return Err(failed());
        }
        let invocation: ComputerInvocation =
            serde_json::from_value(request.content.clone()).map_err(|_| failed())?;
        if request.action != invocation.action()
            || request.resource != invocation.resource()
            || request.capabilities != [invocation.action()]
            || !request
                .context
                .offered_tools
                .iter()
                .any(|name| name == invocation.action())
        {
            return Err(failed());
        }
        let run = self.computer.run(&request.context).map_err(|_| failed())?;
        let ComputerInvocation::Command { mut command } = invocation else {
            *self.staged.lock().map_err(|_| failed())? = Some(StagedComputerReply::Empty);
            return json_result(
                json!({"session":null}),
                permit.obligations().max_output_bytes,
            );
        };
        admit_command(self.computer, &run, &command)?;
        if let ComputerCommand::Screenshot { max_bytes, .. } = &mut command {
            *max_bytes = (*max_bytes)
                .min(u32::try_from(permit.obligations().max_output_bytes).unwrap_or(u32::MAX));
            if *max_bytes == 0 {
                return Err(failed());
            }
        }
        let native = ComputerDriverRequest {
            binding: run.binding.clone(),
            command: command.clone(),
        };
        let observed_at = Instant::now();
        let reply = tokio::time::timeout(
            Duration::from_millis(u64::from(self.computer.limits.action_timeout_ms)),
            self.computer.driver.execute(native, &run.control),
        )
        .await;
        let reply = match reply {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) => {
                if matches!(
                    error,
                    ComputerDriverError::OutcomeUnknown | ComputerDriverError::UserStopped
                ) {
                    self.computer.cancel_run(&run.binding.run_id);
                }
                return Err(driver_error(error));
            }
            Err(_) => {
                self.computer.cancel_run(&run.binding.run_id);
                return Err(if mutation(&command) {
                    unknown()
                } else {
                    failed()
                });
            }
        };
        self.computer.run(&request.context).map_err(|_| unknown())?;
        let result = stage_reply(
            self.computer,
            &run,
            &command,
            reply,
            permit.obligations().max_output_bytes,
            self.staged,
            observed_at,
        );
        if result.is_err() {
            self.computer.cancel_run(&run.binding.run_id);
        }
        result
    }
}

fn admit_command(
    service: &RuntimeComputerTools,
    run: &ComputerRun,
    command: &ComputerCommand,
) -> Result<(), ExecutionError> {
    command.validate().map_err(|_| failed())?;
    if !service.capabilities().available {
        return Err(failed());
    }
    let mut state = run.state.lock().map_err(|_| failed())?;
    match command {
        ComputerCommand::Windows { max_windows } if *max_windows <= service.limits.max_windows => {
            return Ok(());
        }
        ComputerCommand::Open { window_id, limits }
            if state.session.is_none()
                && !state.paused
                && !state.stopped
                && state.windows.contains(window_id)
                && limits.is_within(&service.limits) =>
        {
            return Ok(());
        }
        ComputerCommand::Status { session_id }
            if state
                .session
                .as_ref()
                .is_some_and(|session| session.session_id == *session_id) =>
        {
            return Ok(());
        }
        ComputerCommand::Windows { .. }
        | ComputerCommand::Open { .. }
        | ComputerCommand::Status { .. } => return Err(failed()),
        _ => {}
    }
    let (id, generation) = match command {
        ComputerCommand::Snapshot {
            session_id,
            control_generation,
            max_nodes,
        } if *max_nodes <= service.limits.max_snapshot_nodes => (session_id, *control_generation),
        ComputerCommand::Screenshot {
            session_id,
            control_generation,
            max_bytes,
        } if *max_bytes <= service.limits.max_screenshot_bytes => (session_id, *control_generation),
        ComputerCommand::Input {
            session_id,
            control_generation,
            ..
        }
        | ComputerCommand::Close {
            session_id,
            control_generation,
        } => (session_id, *control_generation),
        _ => return Err(failed()),
    };
    let session = state.session.as_ref().ok_or_else(failed)?;
    if session.session_id != *id {
        return Err(failed());
    }
    if session.control_generation != generation {
        return Err(driver_error(ComputerDriverError::Stale));
    }
    if matches!(command, ComputerCommand::Close { .. }) {
        return Ok(());
    }
    if session.state != ComputerControlState::Active || state.paused || state.stopped {
        return Err(driver_error(ComputerDriverError::Denied));
    }
    if let ComputerCommand::Input {
        observation_id,
        input,
        ..
    } = command
    {
        let observation = state
            .observation
            .take()
            .ok_or_else(|| driver_error(ComputerDriverError::Stale))?;
        if observation.0 != *observation_id
            || observation.1.elapsed()
                > Duration::from_millis(u64::from(service.limits.observation_max_age_ms))
        {
            return Err(driver_error(ComputerDriverError::Stale));
        }
        if let colossus_contracts::ComputerInput::Focus { window_id } = input
            && !state.observed_windows.contains(window_id)
        {
            return Err(failed());
        }
    }
    Ok(())
}

fn stage_reply(
    service: &RuntimeComputerTools,
    run: &ComputerRun,
    command: &ComputerCommand,
    reply: ComputerDriverReply,
    bound: u64,
    staged: &StdMutex<Option<StagedComputerReply>>,
    observed_at: Instant,
) -> Result<QuarantinedEffectResult, ExecutionError> {
    let (receipt, value) = match (command, reply) {
        (ComputerCommand::Windows { max_windows }, ComputerDriverReply::Windows(windows)) => {
            stage_windows(windows, *max_windows)?
        }
        (ComputerCommand::Open { window_id, .. }, ComputerDriverReply::Opened(session)) => {
            if session.window_id != *window_id
                || session.validate().is_err()
                || session.state != ComputerControlState::Active
                || session.reason.is_some()
            {
                return Err(unknown());
            }
            let value = json!({"session":session});
            (StagedComputerReply::Session(session), value)
        }
        (ComputerCommand::Status { session_id }, ComputerDriverReply::Status(session)) => {
            if session.session_id != *session_id || !owned_summary(run, &session, false)? {
                return Err(unknown());
            }
            let value = json!({"session":session});
            (StagedComputerReply::Session(session), value)
        }
        (ComputerCommand::Input { .. }, ComputerDriverReply::Applied(session)) => {
            if !owned_summary(run, &session, true)? {
                return Err(unknown());
            }
            let value = json!({"session":session});
            (StagedComputerReply::Session(session), value)
        }
        (
            ComputerCommand::Snapshot {
                session_id,
                control_generation,
                max_nodes,
            },
            ComputerDriverReply::Snapshot(observation),
        ) => stage_observation(
            service,
            run,
            session_id,
            *control_generation,
            *max_nodes,
            observation,
            observed_at,
        )?,
        (
            ComputerCommand::Screenshot {
                session_id,
                control_generation,
                max_bytes,
            },
            ComputerDriverReply::Screenshot(capture),
        ) => {
            if capture.descriptor.session_id != *session_id
                || capture.descriptor.control_generation != *control_generation
                || capture.descriptor.size_bytes > *max_bytes
            {
                return Err(unknown());
            }
            return super::capture::quarantine(service, run, capture, bound, staged, observed_at);
        }
        (ComputerCommand::Close { .. }, ComputerDriverReply::Closed) => {
            (StagedComputerReply::Closed, json!({"closed":true}))
        }
        _ => return Err(unknown()),
    };
    *staged.lock().map_err(|_| unknown())? = Some(receipt);
    json_result(value, bound)
}

fn stage_windows(
    windows: Vec<ComputerWindowSummary>,
    maximum: u16,
) -> Result<(StagedComputerReply, Value), ExecutionError> {
    if windows.len() > usize::from(maximum)
        || windows.iter().any(|window| window.validate().is_err())
        || windows
            .iter()
            .map(|window| &window.window_id)
            .collect::<BTreeSet<_>>()
            .len()
            != windows.len()
    {
        return Err(unknown());
    }
    let value = json!({"windows":windows});
    Ok((StagedComputerReply::Windows(windows), value))
}

fn stage_observation(
    service: &RuntimeComputerTools,
    run: &ComputerRun,
    session_id: &ComputerSessionId,
    generation: u64,
    maximum: u16,
    observation: ComputerObservation,
    observed_at: Instant,
) -> Result<(StagedComputerReply, Value), ExecutionError> {
    observation
        .validate(&service.limits)
        .map_err(|_| unknown())?;
    if observation.session_id != *session_id
        || observation.control_generation != generation
        || observation.nodes.len() > usize::from(maximum)
    {
        return Err(unknown());
    }
    let state = run.state.lock().map_err(|_| failed())?;
    let root = &state.session.as_ref().ok_or_else(failed)?.window_id;
    if !observation
        .windows
        .iter()
        .any(|window| window.window_id == *root)
        || !observation
            .windows
            .iter()
            .any(|window| window.window_id == observation.window_id)
        || state
            .observation
            .as_ref()
            .is_some_and(|(id, _)| id == &observation.observation_id)
    {
        return Err(unknown());
    }
    let value = json!({"observation":observation});
    Ok((
        StagedComputerReply::Observation(observation, observed_at),
        value,
    ))
}

fn owned_summary(
    run: &ComputerRun,
    summary: &ComputerSessionSummary,
    input: bool,
) -> Result<bool, ExecutionError> {
    let state = run.state.lock().map_err(|_| failed())?;
    let current = state.session.as_ref().ok_or_else(failed)?;
    Ok(summary.validate().is_ok()
        && summary.session_id == current.session_id
        && summary.window_id == current.window_id
        && summary.control_generation >= current.control_generation
        && (!input
            || (summary.control_generation == current.control_generation
                && summary.state == ComputerControlState::Active
                && summary.reason.is_none())))
}

pub(super) fn commit(run: &ComputerRun, receipt: StagedComputerReply) -> Result<(), ToolError> {
    if run.control.is_cancelled() {
        return Err(super::service::denied());
    }
    let mut state = run.state.lock().map_err(|_| super::service::denied())?;
    match receipt {
        StagedComputerReply::Empty => {}
        StagedComputerReply::Windows(windows) => {
            state.windows = windows
                .into_iter()
                .filter(|window| window.available)
                .map(|window| window.window_id)
                .collect();
        }
        StagedComputerReply::Session(session) => {
            if state
                .session
                .as_ref()
                .is_none_or(|current| current != &session)
                || session.state != ComputerControlState::Active
            {
                state.observation = None;
            }
            state.paused = session.state == ComputerControlState::Paused;
            state.stopped |= session.state == ComputerControlState::Stopped;
            state.observed_windows.insert(session.window_id.clone());
            state.session = Some(session);
        }
        StagedComputerReply::Observation(observation, observed_at) => {
            state.observation = Some((observation.observation_id, observed_at));
            state.observed_windows = observation
                .windows
                .into_iter()
                .map(|window| window.window_id)
                .collect();
        }
        StagedComputerReply::Screenshot(descriptor, observed_at) => {
            state.observed_windows.insert(descriptor.window_id);
            state.observation = Some((descriptor.observation_id, observed_at));
        }
        StagedComputerReply::Closed => {
            state.session = None;
            state.observation = None;
            state.observed_windows.clear();
        }
    }
    Ok(())
}

fn json_result(value: Value, bound: u64) -> Result<QuarantinedEffectResult, ExecutionError> {
    let bytes = serde_json::to_vec(&value).map_err(|_| unknown())?;
    if bytes.len() as u64 > bound.min(1024 * 1024) {
        return Err(unknown());
    }
    Ok(QuarantinedEffectResult {
        bytes,
        media_type: "application/json".into(),
        effect_succeeded: true,
    })
}

fn mutation(command: &ComputerCommand) -> bool {
    matches!(
        command,
        ComputerCommand::Open { .. }
            | ComputerCommand::Input { .. }
            | ComputerCommand::Close { .. }
    )
}

fn driver_error(error: ComputerDriverError) -> ExecutionError {
    if error == ComputerDriverError::OutcomeUnknown {
        unknown()
    } else {
        ExecutionError::Failed(error.to_string())
    }
}

fn failed() -> ExecutionError {
    ExecutionError::Failed("computer authority, target, or bounded evidence is unavailable".into())
}
fn unknown() -> ExecutionError {
    ExecutionError::OutcomeUnknown(
        "computer effect or evidence outcome is unknown; automatic retry is prohibited".into(),
    )
}
