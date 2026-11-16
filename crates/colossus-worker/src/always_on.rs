//! Local host admission and dispatch for owner-enrolled Always On wakes.

use std::{sync::Arc, time::Duration};

use colossus_api::{
    AgentRunApi, ApplicationKind, ApplicationPrincipal, CallerContext, ContentPart,
    CreateRunRequest, IdempotencyKey, RequestId, RunMode, RunStatus, scopes,
};
use colossus_api_runtime::RuntimeAgentRunApi;
use colossus_contracts::{
    Actor, ActorType,
    always_on::{AlwaysOnProfile, AlwaysOnWakeStatus},
};
use colossus_grpc::{CredentialRepository, JournalCredentialRepository};
use colossus_runtime::Runtime;
use colossus_worker_protocol::AlwaysOnCommand;

use crate::WorkerError;

pub(super) const MANAGED_APPLICATION: &str = "app:colossus-desktop-managed";

pub(super) enum AlwaysOnHost {
    Terminal,
    Managed {
        runs: Arc<RuntimeAgentRunApi>,
        credential_id: String,
    },
}

impl AlwaysOnHost {
    fn caller(&self, runtime: &Runtime) -> Result<Option<CallerContext>, WorkerError> {
        let Self::Managed { credential_id, .. } = self else {
            return Ok(None);
        };
        let repository = JournalCredentialRepository::new(runtime.journal());
        let record = repository
            .get(credential_id)
            .map_err(|_| authority_unavailable())?
            .ok_or_else(authority_unavailable)?;
        let principal = record
            .active_host_principal()
            .map_err(|_| authority_unavailable())?
            .filter(|principal| {
                principal.application_id() == MANAGED_APPLICATION
                    && principal.kind() == ApplicationKind::Sidecar
                    && principal.has_scope(scopes::RUNS_READ)
                    && principal.has_scope(scopes::RUNS_EXECUTE)
                    && principal.allows_role("primary")
            })
            .ok_or_else(authority_unavailable)?;
        // Retain the activated application's stable owner and intersect its grant
        // with the exact role and empty tool ceiling for background checks.
        let principal = ApplicationPrincipal::authenticated(
            principal.application_id(),
            principal.credential_id(),
            principal.kind(),
            principal.scopes().cloned(),
            ["primary".to_owned()],
            [],
        )?;
        let request_id = RequestId::new(format!("always-on-{}", uuid::Uuid::now_v7()))?;
        Ok(Some(CallerContext::authenticated(principal, request_id)))
    }

    pub(super) fn command(
        &self,
        runtime: &Runtime,
        command: AlwaysOnCommand,
    ) -> Result<Option<AlwaysOnProfile>, WorkerError> {
        let owner = owner(self.caller(runtime)?.as_ref());
        let profile = match command {
            AlwaysOnCommand::Status => return Ok(runtime.always_on_profile(&owner)?),
            AlwaysOnCommand::Enable {
                session_id,
                expected_revision,
            } => runtime.enable_always_on(&owner, &session_id, expected_revision)?,
            AlwaysOnCommand::Pause { expected_revision } => {
                runtime.pause_always_on(&owner, expected_revision)?
            }
            AlwaysOnCommand::Wake { expected_revision } => {
                runtime.request_always_on_wake(&owner, expected_revision)?
            }
        };
        Ok(Some(profile))
    }

    pub(super) async fn serve(
        &self,
        runtime: &Runtime,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        let mut cadence = tokio::time::interval(Duration::from_secs(2));
        cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = shutdown.changed() => break,
                _ = cadence.tick() => {}
            }
            tokio::select! {
                biased;
                _ = shutdown.changed() => break,
                _ = self.drain_once(runtime) => {}
            }
        }
    }

    async fn drain_once(&self, runtime: &Runtime) -> Result<(), WorkerError> {
        let caller = self.caller(runtime)?;
        let owner = owner(caller.as_ref());
        let Some(claim) = runtime.claim_always_on_wake(&owner)? else {
            return Ok(());
        };
        let status = match (self, caller) {
            (Self::Terminal, None) => runtime.run_always_on_local_wake(&claim).await?,
            (Self::Managed { runs, .. }, Some(caller)) => {
                let idempotency_key = IdempotencyKey::new(claim.wake.idempotency_key())?;
                let request = CreateRunRequest {
                    model_profile: None,
                    reasoning_effort: None,
                    input: vec![ContentPart::Text { text: claim.prompt }],
                    session_id: Some(claim.session_id),
                    end_user_id: None,
                    role: Some("primary".into()),
                    mode: RunMode::Execute,
                    goal_max_iterations: 0,
                    research_depth: None,
                    research_sources: Vec::new(),
                    skill_ids: Vec::new(),
                    plan_action: None,
                    branch: None,
                    max_turns: 1,
                    idempotency_key,
                };
                // A resumed claim is observation-only. If the worker stopped
                // before API allocation, it remains interrupted.
                let status = if claim.resumed {
                    runs.resolve_host_wake(&caller, &request)?
                } else if runtime
                    .always_on_profile(&owner)?
                    .is_some_and(|profile| profile.enabled)
                {
                    Some(runs.create_run(&caller, request).await?.run.status)
                } else {
                    None
                };
                match status {
                    Some(RunStatus::Completed) => AlwaysOnWakeStatus::Completed,
                    Some(RunStatus::Failed) => AlwaysOnWakeStatus::Failed,
                    None | Some(RunStatus::Cancelled | RunStatus::Interrupted) => {
                        AlwaysOnWakeStatus::Interrupted
                    }
                    Some(RunStatus::OutcomeUnknown) => AlwaysOnWakeStatus::OutcomeUnknown,
                    Some(_) => return Ok(()),
                }
            }
            _ => return Err(authority_unavailable()),
        };
        runtime.finish_always_on_wake(&owner, &claim.wake.id, status)?;
        Ok(())
    }
}

fn owner(caller: Option<&CallerContext>) -> Actor {
    caller.map_or_else(
        || Actor {
            actor_type: ActorType::User,
            id: "terminal-user".into(),
        },
        CallerContext::actor,
    )
}

fn authority_unavailable() -> WorkerError {
    WorkerError::Protocol("Always On requires the active managed Desktop grant".into())
}
