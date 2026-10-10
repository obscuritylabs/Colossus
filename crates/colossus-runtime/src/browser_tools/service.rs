use super::arguments::BrowserInvocation;
use crate::{prelude::*, workspace_lease::WorkspaceIdentity};
use colossus_browser::{BrowserCoordinator, BrowserError};
use colossus_contracts::{
    BrowserActor, BrowserControlLease, BrowserLimits, BrowserOrigin, BrowserScope,
    BrowserSessionBinding, BrowserSessionId,
};
use colossus_ports::{AgentRunLifecycle, RunControl};
use std::sync::atomic::AtomicBool;

/// Registered initiating provenance is never reconstructed from model arguments.
#[derive(Clone)]
pub(super) struct BrowserRun {
    pub(super) actor: BrowserActor,
    pub(super) context: ExecutionContext,
    pub(super) control: RunControl,
    pub(super) sessions: BTreeMap<BrowserSessionId, BrowserSessionAuthority>,
    pub(super) cleanup_scheduled: Arc<AtomicBool>,
    pub(super) cleanup: Arc<TokioMutex<()>>,
}

#[derive(Clone)]
pub(super) struct BrowserSessionAuthority {
    pub(super) lease: BrowserControlLease,
    pub(super) origins: Vec<BrowserOrigin>,
}

pub(crate) struct RuntimeBrowserTools {
    pub(crate) coordinator: Arc<BrowserCoordinator>,
    pub(super) identity: WorkspaceIdentity,
    pub(super) runtime_id: String,
    pub(super) workspace_id: String,
    pub(super) runs: Arc<StdMutex<BTreeMap<String, BrowserRun>>>,
    pub(super) artifacts: StdMutex<Option<Arc<dyn colossus_ports::BrowserArtifactPublisher>>>,
    pub(super) native: Arc<super::native::NativeState>,
}

impl RuntimeBrowserTools {
    pub(crate) fn new(
        host: &super::RuntimeBrowserHost,
        identity: WorkspaceIdentity,
        workspace_id: String,
        limits: BrowserLimits,
        artifacts: Option<Arc<dyn colossus_ports::BrowserArtifactPublisher>>,
    ) -> Self {
        let native =
            super::native::NativeState::new(Arc::clone(&host.driver), host.presenter.clone());
        Self {
            coordinator: Arc::new(BrowserCoordinator::new(
                Arc::new(super::native::driver::Registrar(Arc::clone(&native))),
                limits,
            )),
            identity,
            runtime_id: Uuid::now_v7().to_string(),
            workspace_id,
            runs: Arc::new(StdMutex::new(BTreeMap::new())),
            artifacts: StdMutex::new(artifacts),
            native,
        }
    }

    pub(super) fn run(&self, context: &ExecutionContext) -> Result<BrowserRun, ToolError> {
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
        {
            return Err(denied());
        }
        Ok(run.clone())
    }

    pub(super) fn authority(
        &self,
        run: &BrowserRun,
        invocation: &BrowserInvocation,
    ) -> Result<Option<BrowserSessionAuthority>, ToolError> {
        let (session_id, generation) = match invocation {
            BrowserInvocation::Open { .. } => return Ok(None),
            BrowserInvocation::Status { session_id, .. }
            | BrowserInvocation::Close { session_id, .. } => (session_id, None),
            BrowserInvocation::Action {
                session_id,
                control_generation,
                ..
            } => (session_id, Some(*control_generation)),
        };
        let authority = run.sessions.get(session_id).ok_or_else(denied)?;
        if generation.is_some_and(|generation| generation != authority.lease.control_generation) {
            return Err(denied());
        }
        Ok(Some(authority.clone()))
    }

    pub(super) fn remember_session(
        &self,
        actor: &BrowserActor,
        authority: BrowserSessionAuthority,
    ) -> Result<(), ToolError> {
        let mut runs = self.runs.lock().map_err(|_| denied())?;
        let run = runs
            .get_mut(&actor.run_id)
            .filter(|run| run.actor == *actor && !run.control.is_cancelled())
            .ok_or_else(denied)?;
        run.sessions
            .insert(authority.lease.session_id.clone(), authority);
        Ok(())
    }

    pub(super) fn forget_session(&self, run_id: &str, session_id: &BrowserSessionId) {
        if let Ok(mut runs) = self.runs.lock()
            && let Some(run) = runs.get_mut(run_id)
        {
            run.sessions.remove(session_id);
        }
    }

    pub(super) fn cancel_all(&self) {
        self.native.stop();
        let run_ids = self
            .runs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in run_ids {
            self.cancel_run(&id);
        }
    }

    pub(super) async fn drain(&self) {
        self.cancel_all();
        let runs = self
            .runs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for run in runs {
            self.finish_run(&run.actor.run_id).await;
        }
        super::native::lifecycle::drain(self).await;
    }
}

impl Drop for RuntimeBrowserTools {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

pub(super) fn denied() -> ToolError {
    ToolError::Denied("browser ownership or active run authority is unavailable".into())
}

pub(super) fn execution_error(error: BrowserError) -> ExecutionError {
    // BrowserError diagnostics are categorical: they never contain page or PKI data.
    if matches!(
        error,
        BrowserError::OutcomeUnknown | BrowserError::InvalidEvidence
    ) {
        ExecutionError::OutcomeUnknown(error.to_string())
    } else {
        ExecutionError::Failed(error.to_string())
    }
}

pub(super) fn binding(
    service: &RuntimeBrowserTools,
    context: &ExecutionContext,
    initiator: &Actor,
) -> Result<BrowserSessionBinding, ToolError> {
    let scope = if let Some(workflow) = &context.workflow_id {
        BrowserScope::Workflow {
            id: workflow.clone(),
        }
    } else {
        BrowserScope::Conversation {
            id: context.session_id.clone().ok_or_else(denied)?,
        }
    };
    Ok(BrowserSessionBinding {
        runtime_id: service.runtime_id.clone(),
        workspace_id: service.workspace_id.clone(),
        application_id: initiator.id.clone(),
        scope,
    })
}
