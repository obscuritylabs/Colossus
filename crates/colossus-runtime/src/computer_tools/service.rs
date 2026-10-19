use crate::{prelude::*, workspace_lease::WorkspaceIdentity};
use colossus_contracts::{
    ComputerBinding, ComputerCapabilities, ComputerLimits, ComputerObservationId, ComputerScope,
    ComputerSessionSummary, ComputerWindowId,
};
use colossus_ports::{AgentRunLifecycle, ComputerArtifactPublisher, ComputerDriver};
use std::sync::atomic::AtomicBool;

#[derive(Clone)]
pub(super) struct ComputerRun {
    pub(super) binding: ComputerBinding,
    pub(super) context: ExecutionContext,
    pub(super) control: RunControl,
    pub(super) state: Arc<StdMutex<ComputerRunState>>,
    pub(super) dispatch: Arc<TokioMutex<()>>,
    pub(super) cleanup: Arc<TokioMutex<()>>,
    pub(super) cleanup_scheduled: Arc<AtomicBool>,
}

#[derive(Default)]
pub(super) struct ComputerRunState {
    pub(super) windows: BTreeSet<ComputerWindowId>,
    pub(super) session: Option<ComputerSessionSummary>,
    pub(super) observation: Option<(ComputerObservationId, Instant)>,
    pub(super) observed_windows: BTreeSet<ComputerWindowId>,
    pub(super) paused: bool,
    pub(super) stopped: bool,
}

pub(crate) struct RuntimeComputerTools {
    pub(super) driver: Arc<dyn ComputerDriver>,
    pub(super) identity: WorkspaceIdentity,
    pub(super) runtime_id: String,
    pub(super) workspace_id: String,
    pub(super) limits: ComputerLimits,
    pub(super) runs: Arc<StdMutex<BTreeMap<String, ComputerRun>>>,
    pub(super) artifacts: StdMutex<Option<Arc<dyn ComputerArtifactPublisher>>>,
}

impl RuntimeComputerTools {
    pub(crate) fn new(
        host: &super::RuntimeComputerHost,
        identity: WorkspaceIdentity,
        workspace_id: String,
        config: &crate::ComputerConfig,
        artifacts: Option<Arc<dyn ComputerArtifactPublisher>>,
    ) -> Result<Self, crate::RuntimeError> {
        let native = host.capabilities();
        let limits = config.limits().intersect(&native.limits).map_err(|_| {
            crate::RuntimeError::Config("native computer resource bounds are invalid".into())
        })?;
        Ok(Self {
            driver: Arc::clone(&host.driver),
            identity,
            runtime_id: Uuid::now_v7().to_string(),
            workspace_id,
            limits,
            runs: Arc::new(StdMutex::new(BTreeMap::new())),
            artifacts: StdMutex::new(artifacts),
        })
    }

    pub(super) fn capabilities(&self) -> ComputerCapabilities {
        let mut capabilities = self.driver.capabilities();
        capabilities.limits = self.limits;
        capabilities
    }

    pub(super) fn run(&self, context: &ExecutionContext) -> Result<ComputerRun, ToolError> {
        self.identity.revalidate().map_err(|_| denied())?;
        let id = context.run_id.as_ref().ok_or_else(denied)?;
        let runs = self.runs.lock().map_err(|_| denied())?;
        let run = runs.get(id).ok_or_else(denied)?;
        if run.control.is_cancelled()
            || context.session_id != run.context.session_id
            || context.workflow_id != run.context.workflow_id
            || context.workflow_hash != run.context.workflow_hash
            || context.step_id != run.context.step_id
            || context.attempt != run.context.attempt
            || context.goal_id != run.context.goal_id
            || context.plan_id != run.context.plan_id
            || context.subagent_id != run.context.subagent_id
            || context.offered_tools != run.context.offered_tools
        {
            return Err(denied());
        }
        Ok(run.clone())
    }

    pub(super) fn cancel_all(&self) {
        let ids = self
            .runs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in ids {
            self.cancel_run(&id);
        }
    }

    pub(super) async fn drain(&self) -> Result<(), colossus_ports::ComputerDriverError> {
        self.cancel_all();
        let ids = self
            .runs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        tokio::time::timeout(Duration::from_secs(12), async {
            for id in ids {
                self.finish_run(&id).await;
            }
        })
        .await
        .map_err(|_| colossus_ports::ComputerDriverError::OutcomeUnknown)?;
        if self
            .runs
            .lock()
            .map_err(|_| colossus_ports::ComputerDriverError::OutcomeUnknown)?
            .is_empty()
        {
            Ok(())
        } else {
            Err(colossus_ports::ComputerDriverError::OutcomeUnknown)
        }
    }
}

impl Drop for RuntimeComputerTools {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

pub(super) fn denied() -> ToolError {
    ToolError::Denied("computer ownership or active run authority is unavailable".into())
}

pub(super) fn binding(
    service: &RuntimeComputerTools,
    context: &ExecutionContext,
    initiator: &Actor,
) -> Result<ComputerBinding, ToolError> {
    let scope = if let Some(id) = &context.workflow_id {
        ComputerScope::Workflow { id: id.clone() }
    } else {
        ComputerScope::Conversation {
            id: context.session_id.clone().ok_or_else(denied)?,
        }
    };
    let binding = ComputerBinding {
        runtime_id: service.runtime_id.clone(),
        workspace_id: service.workspace_id.clone(),
        application_id: initiator.id.clone(),
        scope,
        run_id: context.run_id.clone().ok_or_else(denied)?,
    };
    binding.validate().map_err(|_| denied())?;
    Ok(binding)
}
