use crate::{
    Runtime, RuntimeConfig, RuntimeOpenOptions, computer_tools::*, prelude::*,
    test_support::private_tempdir,
};
use colossus_contracts::*;
use colossus_ports::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(super) const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGMQMgkDAAD4AJ3MaiF4AAAAAElFTkSuQmCC";

pub(super) fn window_id() -> ComputerWindowId {
    ComputerWindowId::parse(format!("cu_w_{}", "a".repeat(32))).unwrap()
}
pub(super) fn observation_id() -> ComputerObservationId {
    ComputerObservationId::parse(format!("cu_o_{}", Uuid::now_v7().simple())).unwrap()
}

pub(super) fn window() -> ComputerWindowSummary {
    ComputerWindowSummary {
        window_id: window_id(),
        title: "Disposable fixture".into(),
        application_name: "Fixture".into(),
        rect: ComputerRect {
            x: -100,
            y: 0,
            width: 100,
            height: 100,
        },
        foreground: true,
        available: true,
        unavailable_reason: None,
    }
}

#[derive(Default)]
pub(super) struct Driver {
    pub(super) sessions: StdMutex<BTreeMap<String, ComputerSessionSummary>>,
    pub(super) requests: StdMutex<Vec<ComputerDriverRequest>>,
    pub(super) revoked: StdMutex<BTreeSet<String>>,
    pub(super) closed: AtomicUsize,
    pub(super) inputs: AtomicUsize,
    pub(super) paused: AtomicBool,
    pub(super) unknown: AtomicBool,
    pub(super) bad_png: AtomicBool,
    pub(super) cleanup_unknown: AtomicBool,
    pub(super) modal: AtomicBool,
    pub(super) open_modal_on_input: AtomicBool,
    pub(super) foreign_root: AtomicBool,
}

#[async_trait]
impl ComputerDriver for Driver {
    fn capabilities(&self) -> ComputerCapabilities {
        ComputerCapabilities {
            available: true,
            platform: "windows".into(),
            limits: Default::default(),
            unavailable_reason: None,
        }
    }
    async fn execute(
        &self,
        request: ComputerDriverRequest,
        control: &RunControl,
    ) -> Result<ComputerDriverReply, ComputerDriverError> {
        if control.is_cancelled()
            || self
                .revoked
                .lock()
                .unwrap()
                .contains(&request.binding.run_id)
        {
            return Err(ComputerDriverError::Denied);
        }
        request.validate().unwrap();
        self.requests.lock().unwrap().push(request.clone());
        let mut sessions = self.sessions.lock().unwrap();
        match request.command {
            ComputerCommand::Windows { .. } => Ok(ComputerDriverReply::Windows(vec![window()])),
            ComputerCommand::Open { window_id, .. } => {
                let session = ComputerSessionSummary {
                    session_id: ComputerSessionId::parse(format!(
                        "cu_s_{}",
                        Uuid::now_v7().simple()
                    ))
                    .unwrap(),
                    control_generation: 1,
                    window_id,
                    state: ComputerControlState::Active,
                    reason: None,
                };
                sessions.insert(request.binding.run_id, session.clone());
                Ok(ComputerDriverReply::Opened(session))
            }
            ComputerCommand::Status { .. } => {
                let session = sessions
                    .get_mut(&request.binding.run_id)
                    .ok_or(ComputerDriverError::Stale)?;
                if self.paused.load(Ordering::SeqCst) {
                    session.state = ComputerControlState::Paused;
                    session.reason = Some(ComputerStopReason::UserPaused);
                    session.control_generation = 2;
                }
                Ok(ComputerDriverReply::Status(session.clone()))
            }
            ComputerCommand::Snapshot {
                session_id,
                control_generation,
                ..
            } => Ok(ComputerDriverReply::Snapshot(ComputerObservation {
                session_id,
                control_generation,
                observation_id: observation_id(),
                window_id: window_id(),
                width: 100,
                height: 100,
                scale_milli: 1000,
                nodes: vec![],
                windows: vec![window()],
            })),
            ComputerCommand::Screenshot {
                session_id,
                control_generation,
                ..
            } => {
                let png = BASE64.decode(PNG).unwrap();
                let descriptor = ComputerScreenshotDescriptor {
                    session_id,
                    control_generation,
                    observation_id: observation_id(),
                    window_id: if self.modal.load(Ordering::SeqCst) {
                        ComputerWindowId::parse(format!("cu_w_{}", "b".repeat(32))).unwrap()
                    } else {
                        window_id()
                    },
                    width: 1,
                    height: 1,
                    size_bytes: png.len() as u32,
                    sha256: if self.bad_png.load(Ordering::SeqCst) {
                        "0".repeat(64)
                    } else {
                        hex::encode(Sha256::digest(&png))
                    },
                };
                Ok(ComputerDriverReply::Screenshot(ComputerScreenshotCapture {
                    root_window_id: if self.foreign_root.load(Ordering::SeqCst) {
                        ComputerWindowId::parse(format!("cu_w_{}", "c".repeat(32))).unwrap()
                    } else {
                        window_id()
                    },
                    descriptor,
                    png,
                }))
            }
            ComputerCommand::Input { .. } => {
                self.inputs.fetch_add(1, Ordering::SeqCst);
                if self.open_modal_on_input.load(Ordering::SeqCst) {
                    self.modal.store(true, Ordering::SeqCst);
                }
                if self.unknown.load(Ordering::SeqCst) {
                    return Err(ComputerDriverError::OutcomeUnknown);
                }
                Ok(ComputerDriverReply::Applied(
                    sessions.get(&request.binding.run_id).unwrap().clone(),
                ))
            }
            ComputerCommand::Close { .. } => {
                sessions.remove(&request.binding.run_id);
                Ok(ComputerDriverReply::Closed)
            }
        }
    }
    fn revoke_run(&self, binding: &ComputerBinding) {
        self.revoked.lock().unwrap().insert(binding.run_id.clone());
    }
    async fn close_run(&self, binding: &ComputerBinding) -> Result<(), ComputerDriverError> {
        if self.cleanup_unknown.load(Ordering::SeqCst) {
            return Err(ComputerDriverError::OutcomeUnknown);
        }
        self.closed.fetch_add(1, Ordering::SeqCst);
        self.sessions.lock().unwrap().remove(&binding.run_id);
        Ok(())
    }
}

struct NoFallback;
#[async_trait]
impl ToolExecutor for NoFallback {
    async fn execute(&self, call: ToolCall, _: ExecutionContext) -> Result<ToolResult, ToolError> {
        Err(ToolError::Unknown(call.name))
    }
}

pub(super) fn configuration(workspace: &Path) -> RuntimeConfig {
    let mut config = RuntimeConfig::offline_template(workspace.join("state.redb"));
    config.use_ephemeral_storage();
    config.access.profile = AccessProfile::AllowAll;
    config.computer.enabled = true;
    config
}

pub(super) fn runtime(driver: Arc<Driver>) -> (Runtime, tempfile::TempDir) {
    let workspace = private_tempdir();
    let config = configuration(workspace.path());
    let options = RuntimeOpenOptions::for_workspace(workspace.path())
        .unwrap()
        .with_computer_host(RuntimeComputerHost::new(driver));
    (
        Runtime::open_with_options(&config, Arc::new(DenyApproval), None, options).unwrap(),
        workspace,
    )
}

pub(super) fn begin(runtime: &Runtime, id: &str) -> ExecutionContext {
    let context = ExecutionContext {
        run_id: Some(id.into()),
        session_id: Some("conversation".into()),
        offered_tools: runtime
            .tools
            .list_specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect(),
        ..Default::default()
    };
    runtime
        .computer
        .as_ref()
        .unwrap()
        .begin_run(
            &context,
            &Actor {
                actor_type: ActorType::Application,
                id: "app:fixture".into(),
            },
            RunControl::default(),
        )
        .unwrap();
    context
}

pub(super) fn executor(runtime: &Runtime) -> ComputerToolExecutor {
    ComputerToolExecutor {
        gateway: Arc::clone(&runtime.gateway),
        registry: Arc::clone(&runtime.tools),
        computer: Arc::clone(runtime.computer.as_ref().unwrap()),
        journal: runtime.journal(),
        inner: Arc::new(NoFallback),
    }
}

pub(super) async fn call(
    executor: &ComputerToolExecutor,
    context: &ExecutionContext,
    name: &str,
    arguments: Value,
) -> Result<Value, ToolError> {
    let result = executor
        .execute(
            ToolCall {
                call_id: Uuid::now_v7().to_string(),
                name: name.into(),
                arguments,
            },
            context.clone(),
        )
        .await?;
    Ok(serde_json::from_str(&result.output).unwrap())
}

pub(super) async fn open(executor: &ComputerToolExecutor, context: &ExecutionContext) -> Value {
    call(executor, context, "computer.windows", json!({}))
        .await
        .unwrap();
    call(
        executor,
        context,
        "computer.open",
        json!({"window_id":window_id()}),
    )
    .await
    .unwrap()
}

pub(super) fn session(opened: &Value) -> Value {
    json!({"session_id":opened["session"]["session_id"],"control_generation":opened["session"]["control_generation"]})
}

pub(super) async fn observe(
    executor: &ComputerToolExecutor,
    context: &ExecutionContext,
    opened: &Value,
) -> Value {
    let observation = call(executor, context, "computer.snapshot", session(opened))
        .await
        .unwrap();
    let mut arguments = session(opened);
    arguments["observation_id"] = observation["observation"]["observation_id"].clone();
    arguments
}

pub(super) fn with(mut envelope: Value, properties: Value) -> Value {
    envelope
        .as_object_mut()
        .unwrap()
        .extend(properties.as_object().unwrap().clone());
    envelope
}
