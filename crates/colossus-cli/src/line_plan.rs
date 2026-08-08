use super::*;

pub(super) const DEFAULT_PLAN_GOAL_ITERATIONS: u16 = 5;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LinePlanCommand {
    Toggle,
    SetEnabled(bool),
    Status,
    New,
    List,
    Use(String),
    Show(Option<String>),
    Approve,
    Discard,
    Execute(Option<PlanExecutionStrategy>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum PlanExecutionPickerInput {
    Selected(PlanExecutionStrategy),
    Command(String),
    Cancelled,
}

#[derive(Debug, Default)]
pub(super) struct LinePlanState {
    enabled: bool,
    selected: Option<PlanRecord>,
}

impl LinePlanState {
    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(super) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub(super) fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    pub(super) fn start_new(&mut self) {
        self.enabled = true;
        self.selected = None;
    }

    pub(super) fn clear_selection(&mut self) {
        self.selected = None;
    }

    pub(super) fn select(&mut self, plan: PlanRecord, session_id: &str) -> Result<(), String> {
        self.refresh_selected(plan, session_id)?;
        self.enabled = true;
        Ok(())
    }

    pub(super) fn refresh_selected(
        &mut self,
        plan: PlanRecord,
        session_id: &str,
    ) -> Result<(), String> {
        if plan.session_id != session_id {
            return Err("only a Plan from the active session can be selected".into());
        }
        if !matches!(plan.status, PlanStatus::Draft | PlanStatus::Approved) {
            return Err("only a draft or approved Plan can be selected".into());
        }
        self.selected = Some(plan);
        Ok(())
    }

    pub(super) fn selected(&self) -> Option<&PlanRecord> {
        self.selected.as_ref()
    }

    pub(super) fn selected_with_status(&self, status: PlanStatus) -> Result<PlanRecord, String> {
        let plan = self
            .selected
            .as_ref()
            .ok_or_else(|| "no Plan is selected; use /plan use PLAN_ID".to_owned())?;
        if plan.status != status {
            return Err(format!(
                "selected Plan has status {:?}; expected {:?}",
                plan.status, status
            ));
        }
        Ok(plan.clone())
    }

    pub(super) fn agent_mode(&self) -> Result<AgentRunMode, String> {
        if !self.enabled {
            return Ok(AgentRunMode::Execute);
        }
        match self.selected.as_ref() {
            None => Ok(AgentRunMode::Plan(PlanDraftTarget::Create)),
            Some(plan) if plan.status == PlanStatus::Draft => {
                Ok(AgentRunMode::Plan(PlanDraftTarget::Update {
                    plan_id: plan.id.clone(),
                    revision: plan.revision,
                }))
            }
            Some(plan) if plan.status == PlanStatus::Approved => Err(
                "the selected Plan is approved; use /plan execute, /plan new, /plan discard, or /plan off"
                    .into(),
            ),
            Some(_) => Err("the selected Plan is no longer actionable; use /plan new".into()),
        }
    }

    pub(super) fn apply_run_outcome(
        &mut self,
        outcome: &AgentRunOutcome,
        session_id: &str,
    ) -> Result<(), String> {
        let plan = match outcome {
            AgentRunOutcome::Completed { result } => result.plan.as_ref(),
            AgentRunOutcome::Cancelled { result } => result.plan.as_ref(),
        };
        if let Some(plan) = plan {
            self.refresh_selected(plan.clone(), session_id)?;
        }
        Ok(())
    }

    pub(super) fn apply_execution_outcome(&mut self, outcome: &PlanExecutionOutcome) {
        match outcome {
            PlanExecutionOutcome::CancelledBeforeStart { plan } => {
                self.selected = Some(plan.clone());
            }
            PlanExecutionOutcome::Direct { .. } | PlanExecutionOutcome::Goal { .. } => {
                self.enabled = false;
                self.selected = None;
            }
        }
    }

    pub(super) fn status_line(&self) -> String {
        let mode = if self.enabled { "plan" } else { "execute" };
        match self.selected.as_ref() {
            Some(plan) => format!(
                "mode={mode}; plan={}; status={:?}; revision={}",
                plan.id, plan.status, plan.revision
            ),
            None => format!("mode={mode}; plan=none"),
        }
    }
}

pub(super) struct LinePlanEventObserver<'a> {
    inner: &'a mut dyn RunEventObserver,
    written_plan: Option<PlanRecord>,
}

impl<'a> LinePlanEventObserver<'a> {
    pub(super) fn new(inner: &'a mut dyn RunEventObserver) -> Self {
        Self {
            inner,
            written_plan: None,
        }
    }

    pub(super) fn into_written_plan(self) -> Option<PlanRecord> {
        self.written_plan
    }
}

#[async_trait]
impl RunEventObserver for LinePlanEventObserver<'_> {
    async fn observe(&mut self, envelope: RunEventEnvelope) -> Result<(), ModelProviderError> {
        if self.written_plan.is_none()
            && let RunEvent::PlanWritten { plan } = &envelope.event
        {
            self.written_plan = Some(plan.clone());
        }
        self.inner.observe(envelope).await
    }
}

pub(super) fn parse_line_plan_command(line: &str) -> Result<Option<LinePlanCommand>, String> {
    let Some(rest) = line.strip_prefix("/plan") else {
        return Ok(None);
    };
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return Ok(None);
    }
    let arguments = rest.split_whitespace().collect::<Vec<_>>();
    let command = match arguments.as_slice() {
        [] => LinePlanCommand::Toggle,
        ["on"] => LinePlanCommand::SetEnabled(true),
        ["off"] => LinePlanCommand::SetEnabled(false),
        ["status"] => LinePlanCommand::Status,
        ["new"] => LinePlanCommand::New,
        ["list"] => LinePlanCommand::List,
        ["use", plan_id] => LinePlanCommand::Use((*plan_id).into()),
        ["show"] => LinePlanCommand::Show(None),
        ["show", plan_id] => LinePlanCommand::Show(Some((*plan_id).into())),
        ["approve"] => LinePlanCommand::Approve,
        ["discard"] => LinePlanCommand::Discard,
        ["execute"] => LinePlanCommand::Execute(None),
        ["execute", "direct"] => LinePlanCommand::Execute(Some(PlanExecutionStrategy::Direct)),
        ["execute", "goal"] => LinePlanCommand::Execute(Some(PlanExecutionStrategy::Goal {
            max_iterations: DEFAULT_PLAN_GOAL_ITERATIONS,
        })),
        ["execute", "goal", iterations] => {
            let max_iterations = iterations
                .parse::<u16>()
                .map_err(|_| "Goal iterations must be an integer in 1..=50".to_owned())?;
            if !(1..=50).contains(&max_iterations) {
                return Err("Goal iterations must be in 1..=50".into());
            }
            LinePlanCommand::Execute(Some(PlanExecutionStrategy::Goal { max_iterations }))
        }
        _ => {
            return Err(
                "usage: /plan [on|off|status|new|list|use PLAN_ID|show [PLAN_ID]|approve|discard|execute [direct|goal [ITERATIONS]]]"
                    .into(),
            );
        }
    };
    Ok(Some(command))
}

pub(super) fn choose_plan_execution(
    scripted_input: &mut dyn BufRead,
) -> Result<PlanExecutionPickerInput, Box<dyn Error>> {
    println!("Choose how to execute the selected Plan:");
    println!("  1. Direct");
    println!(
        "  2. Goal Mode ({} iterations)",
        DEFAULT_PLAN_GOAL_ITERATIONS
    );
    println!("  3. Cancel");
    println!("Enter a number (blank cancels; /command returns to the terminal).");
    loop {
        let mut choice = String::new();
        if scripted_input.read_line(&mut choice)? == 0 {
            return Ok(PlanExecutionPickerInput::Cancelled);
        }
        let choice = choice.trim();
        let parsed = match choice {
            "" | "3" | "cancel" => PlanExecutionPickerInput::Cancelled,
            "1" | "direct" => PlanExecutionPickerInput::Selected(PlanExecutionStrategy::Direct),
            "2" | "goal" => PlanExecutionPickerInput::Selected(PlanExecutionStrategy::Goal {
                max_iterations: DEFAULT_PLAN_GOAL_ITERATIONS,
            }),
            value if value.starts_with('/') => PlanExecutionPickerInput::Command(value.into()),
            _ => {
                println!("Enter 1, 2, or 3; leave it blank to cancel.");
                continue;
            }
        };
        return Ok(parsed);
    }
}

pub(super) fn completed_output(outcome: &AgentRunOutcome) -> Option<&str> {
    match outcome {
        AgentRunOutcome::Completed { result } => Some(&result.output),
        AgentRunOutcome::Cancelled { .. } => None,
    }
}

pub(super) fn execution_output(outcome: &PlanExecutionOutcome) -> Option<&str> {
    match outcome {
        PlanExecutionOutcome::Direct {
            terminal: colossus_contracts::ControlledAgentTerminal::Completed { result },
            ..
        } => Some(&result.output),
        PlanExecutionOutcome::Goal { terminal, .. } => goal_output(terminal),
        PlanExecutionOutcome::CancelledBeforeStart { .. } | PlanExecutionOutcome::Direct { .. } => {
            None
        }
    }
}

pub(super) fn goal_output(outcome: &GoalRunOutcome) -> Option<&str> {
    let result = match outcome {
        GoalRunOutcome::Completed { result }
        | GoalRunOutcome::Cancelled { result, .. }
        | GoalRunOutcome::Failed { result, .. } => result,
    };
    result
        .iterations
        .last()
        .map(|iteration| iteration.output.as_str())
}

#[derive(Default)]
pub(super) struct LineWorkerPromptHandler {
    lock: Mutex<()>,
}

#[async_trait]
impl WorkerPromptHandler for LineWorkerPromptHandler {
    async fn notice(&self, notice: ApprovalReviewNotice) -> Result<(), WorkerError> {
        let document = match notice {
            ApprovalReviewNotice::AutomaticApproval { notice } => {
                automatic_approval_document(&notice)
            }
            ApprovalReviewNotice::RiskReviewFallback { notice } => {
                risk_review_fallback_document(&notice)
            }
        };
        write_stderr_document(&document).map_err(WorkerError::Io)
    }

    async fn prompt(&self, prompt: WorkerPrompt) -> Result<Option<String>, WorkerError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| WorkerError::Protocol("worker prompt lock is poisoned".into()))?;
        let mut choices = PresentationTable::new(["#", "Choice"], "Enter a free-form answer.");
        for (index, choice) in prompt.choices.iter().enumerate() {
            choices.push_row([(index + 1).to_string(), choice.clone()]);
        }
        let mut body = vec![PresentationBlock::Markdown(prompt.question.clone())];
        if !prompt.choices.is_empty() {
            body.push(PresentationBlock::Table(choices));
        }
        if !prompt.details.is_null() {
            body.extend(document_from_json(&prompt.details, None).blocks);
        }
        let tone = match prompt.kind {
            WorkerPromptKind::Approval | WorkerPromptKind::SandboxBoundaryAcknowledgement => {
                colossus_presentation::PresentationTone::Warning
            }
            WorkerPromptKind::UserInput => colossus_presentation::PresentationTone::Neutral,
        };
        write_stderr_document(&PresentationDocument::from_block(PresentationBlock::Card {
            title: prompt.title,
            tone,
            body,
        }))
        .map_err(WorkerError::Io)?;
        for _ in 0..3 {
            eprint!("Answer (blank cancels): ");
            io::stderr().flush().map_err(WorkerError::Io)?;
            let mut answer = String::new();
            io::stdin()
                .read_line(&mut answer)
                .map_err(WorkerError::Io)?;
            let answer = answer.trim();
            if answer.is_empty() {
                return Ok(None);
            }
            if let Ok(index) = answer.parse::<usize>()
                && let Some(choice) = index
                    .checked_sub(1)
                    .and_then(|index| prompt.choices.get(index))
            {
                return Ok(Some(choice.clone()));
            }
            if let Some(choice) = prompt
                .choices
                .iter()
                .find(|choice| choice.as_str() == answer)
            {
                return Ok(Some(choice.clone()));
            }
            if prompt.allow_free_form {
                return Ok(Some(answer.into()));
            }
            eprintln!("Enter one of the numbered choices.");
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(event: RunEvent) -> RunEventEnvelope {
        RunEventEnvelope {
            schema_version: 1,
            run_id: "run".into(),
            session_id: "session".into(),
            event,
        }
    }

    fn plan(status: PlanStatus) -> PlanRecord {
        PlanRecord {
            id: "plan".into(),
            session_id: "session".into(),
            prompt: "objective".into(),
            status,
            revision: 3,
            content: "content".into(),
            steps: Vec::new(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            approved_at: None,
            executed_run_id: None,
        }
    }

    struct RejectingObserver;

    #[async_trait]
    impl RunEventObserver for RejectingObserver {
        async fn observe(&mut self, _envelope: RunEventEnvelope) -> Result<(), ModelProviderError> {
            Err(ModelProviderError::Failed("render failed".into()))
        }
    }

    #[derive(Default)]
    struct FailOnSecondEventObserver {
        events: usize,
    }

    #[async_trait]
    impl RunEventObserver for FailOnSecondEventObserver {
        async fn observe(&mut self, _envelope: RunEventEnvelope) -> Result<(), ModelProviderError> {
            self.events += 1;
            if self.events == 2 {
                Err(ModelProviderError::Failed("run failed after write".into()))
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn plan_write_is_captured_before_the_render_observer_fails() {
        let persisted = plan(PlanStatus::Draft);
        let mut inner = RejectingObserver;
        let mut observer = LinePlanEventObserver::new(&mut inner);

        assert!(
            observer
                .observe(envelope(RunEvent::PlanWritten {
                    plan: persisted.clone(),
                }))
                .await
                .is_err()
        );

        let mut state = LinePlanState::default();
        state.start_new();
        state
            .refresh_selected(
                observer.into_written_plan().expect("captured durable plan"),
                "session",
            )
            .expect("apply durable plan");
        assert_eq!(
            state.agent_mode().expect("refinement mode"),
            AgentRunMode::Plan(PlanDraftTarget::Update {
                plan_id: persisted.id,
                revision: persisted.revision,
            })
        );
    }

    #[tokio::test]
    async fn write_then_failure_refreshes_the_selected_draft_revision() {
        let mut stale = plan(PlanStatus::Draft);
        stale.revision = 2;
        let mut persisted = stale.clone();
        persisted.revision = 3;

        let mut state = LinePlanState::default();
        state.set_enabled(true);
        state.select(stale, "session").expect("select stale draft");

        let mut inner = FailOnSecondEventObserver::default();
        let mut observer = LinePlanEventObserver::new(&mut inner);
        observer
            .observe(envelope(RunEvent::PlanWritten {
                plan: persisted.clone(),
            }))
            .await
            .expect("render plan write");
        assert!(
            observer
                .observe(envelope(RunEvent::Error {
                    code: "provider_failed".into(),
                    message: "provider failed after the durable write".into(),
                    recoverable: false,
                    http_status: None,
                    retry_after_ms: None,
                    turn: Some(2),
                    elapsed_seconds: 1.0,
                }))
                .await
                .is_err()
        );

        state
            .refresh_selected(
                observer.into_written_plan().expect("captured durable plan"),
                "session",
            )
            .expect("refresh selected draft");
        assert_eq!(
            state.agent_mode().expect("updated refinement mode"),
            AgentRunMode::Plan(PlanDraftTarget::Update {
                plan_id: persisted.id,
                revision: persisted.revision,
            })
        );
    }

    #[test]
    fn plan_commands_parse_the_exact_line_contract() {
        assert_eq!(
            parse_line_plan_command("/plan").expect("parse"),
            Some(LinePlanCommand::Toggle)
        );
        assert_eq!(
            parse_line_plan_command("/plan on").expect("parse"),
            Some(LinePlanCommand::SetEnabled(true))
        );
        assert_eq!(
            parse_line_plan_command("/plan off").expect("parse"),
            Some(LinePlanCommand::SetEnabled(false))
        );
        assert_eq!(
            parse_line_plan_command("/plan status").expect("parse"),
            Some(LinePlanCommand::Status)
        );
        assert_eq!(
            parse_line_plan_command("/plan new").expect("parse"),
            Some(LinePlanCommand::New)
        );
        assert_eq!(
            parse_line_plan_command("/plan list").expect("parse"),
            Some(LinePlanCommand::List)
        );
        assert_eq!(
            parse_line_plan_command("/plan use 01plan").expect("parse"),
            Some(LinePlanCommand::Use("01plan".into()))
        );
        assert_eq!(
            parse_line_plan_command("/plan show").expect("parse"),
            Some(LinePlanCommand::Show(None))
        );
        assert_eq!(
            parse_line_plan_command("/plan show 01plan").expect("parse"),
            Some(LinePlanCommand::Show(Some("01plan".into())))
        );
        assert_eq!(
            parse_line_plan_command("/plan approve").expect("parse"),
            Some(LinePlanCommand::Approve)
        );
        assert_eq!(
            parse_line_plan_command("/plan discard").expect("parse"),
            Some(LinePlanCommand::Discard)
        );
        assert_eq!(
            parse_line_plan_command("/plan execute").expect("parse"),
            Some(LinePlanCommand::Execute(None))
        );
        assert_eq!(
            parse_line_plan_command("/plan execute direct").expect("parse"),
            Some(LinePlanCommand::Execute(Some(
                PlanExecutionStrategy::Direct
            )))
        );
        assert_eq!(
            parse_line_plan_command("/plan execute goal 12").expect("parse"),
            Some(LinePlanCommand::Execute(Some(
                PlanExecutionStrategy::Goal { max_iterations: 12 }
            )))
        );
        assert!(parse_line_plan_command("/plan execute goal 0").is_err());
        assert!(parse_line_plan_command("/plan execute goal 51").is_err());
        assert!(parse_line_plan_command("/plan approve extra").is_err());
        assert_eq!(parse_line_plan_command("/planner").expect("parse"), None);
        assert_eq!(
            parse_line_plan_command("/plans").expect("alias is separate"),
            None
        );
    }

    #[test]
    fn execution_picker_accepts_numbered_and_queued_command_input() {
        let mut direct = io::Cursor::new(b"1\n");
        assert_eq!(
            choose_plan_execution(&mut direct).expect("direct"),
            PlanExecutionPickerInput::Selected(PlanExecutionStrategy::Direct)
        );
        let mut goal = io::Cursor::new(b"invalid\n2\n");
        assert_eq!(
            choose_plan_execution(&mut goal).expect("goal"),
            PlanExecutionPickerInput::Selected(PlanExecutionStrategy::Goal {
                max_iterations: DEFAULT_PLAN_GOAL_ITERATIONS
            })
        );
        let mut command = io::Cursor::new(b"/plans\n");
        assert_eq!(
            choose_plan_execution(&mut command).expect("command"),
            PlanExecutionPickerInput::Command("/plans".into())
        );
    }

    #[test]
    fn lifecycle_refresh_and_pre_start_cancel_preserve_the_current_mode() {
        let mut state = LinePlanState::default();
        state
            .select(plan(PlanStatus::Draft), "session")
            .expect("select draft");
        state.set_enabled(false);
        state
            .refresh_selected(plan(PlanStatus::Approved), "session")
            .expect("refresh approved");
        assert!(!state.enabled());

        state.apply_execution_outcome(&PlanExecutionOutcome::CancelledBeforeStart {
            plan: plan(PlanStatus::Approved),
        });
        assert!(!state.enabled());
        assert_eq!(state.selected().map(|plan| plan.revision), Some(3));
    }

    #[test]
    fn run_outcome_cannot_select_a_plan_from_another_session() {
        let mut other = plan(PlanStatus::Draft);
        other.session_id = "other-session".into();
        let outcome = AgentRunOutcome::Cancelled {
            result: colossus_contracts::AgentRunCancellation {
                run_id: "run".into(),
                session_id: "session".into(),
                turn: 1,
                event_count: 1,
                elapsed_seconds: 0.1,
                plan: Some(other),
            },
        };
        let mut state = LinePlanState::default();

        assert!(state.apply_run_outcome(&outcome, "session").is_err());
        assert!(state.selected().is_none());
    }

    #[test]
    fn mode_and_selection_follow_process_local_lifecycle_rules() {
        let mut state = LinePlanState::default();
        assert_eq!(
            state.agent_mode().expect("default mode"),
            AgentRunMode::Execute
        );

        state
            .select(plan(PlanStatus::Draft), "session")
            .expect("select draft");
        assert_eq!(
            state
                .agent_mode()
                .expect("selection enters refinement mode"),
            AgentRunMode::Plan(PlanDraftTarget::Update {
                plan_id: "plan".into(),
                revision: 3,
            })
        );

        state.set_enabled(false);
        assert_eq!(
            state.agent_mode().expect("disabled mode"),
            AgentRunMode::Execute
        );
        assert_eq!(state.selected().map(|plan| plan.id.as_str()), Some("plan"));

        state.set_enabled(true);
        state.clear_selection();
        assert_eq!(
            state.agent_mode().expect("new session selection"),
            AgentRunMode::Plan(PlanDraftTarget::Create)
        );

        state
            .select(plan(PlanStatus::Approved), "session")
            .expect("select approved");
        assert!(state.agent_mode().is_err());
        state.start_new();
        assert!(state.enabled());
        assert!(state.selected().is_none());
    }

    #[test]
    fn selection_is_limited_to_actionable_plans_in_the_active_session() {
        let mut state = LinePlanState::default();
        let mut other_session = plan(PlanStatus::Draft);
        other_session.session_id = "other".into();
        assert!(state.select(other_session, "session").is_err());
        assert!(state.select(plan(PlanStatus::Executed), "session").is_err());
        assert!(
            state
                .select(plan(PlanStatus::Discarded), "session")
                .is_err()
        );
    }
}
