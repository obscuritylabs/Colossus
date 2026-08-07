use super::*;

const PUBLIC_RUN_SHUTDOWN_GRACE: Duration = Duration::from_secs(30);
const PUBLIC_TRANSPORT_FORCE_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Long-running single-writer runtime owner and authenticated IPC server.
pub struct WorkerServer {
    endpoint: String,
    listener: Option<platform::Listener>,
    authentication_key: WorkerAuthenticationKey,
    runtime: Arc<Runtime>,
    replay: Arc<Mutex<ReplayGuard>>,
    maintenance: Arc<tokio::sync::Mutex<()>>,
    public_interactions: Arc<colossus_api_runtime::PublicInteractionRouter>,
    public_api: Option<public_api::PreparedPublicApi>,
}

impl WorkerServer {
    /// Open the runtime (and therefore acquire the writer lease) before binding IPC.
    pub fn open(
        config: &RuntimeConfig,
        approvals: Arc<dyn colossus_ports::ApprovalProvider>,
    ) -> Result<Self, WorkerError> {
        Self::open_at_workspace(
            config,
            approvals,
            RuntimeOpenOptions::for_workspace(std::env::current_dir()?)?,
        )
    }

    /// Open the runtime for one explicit workspace.
    pub fn open_at_workspace(
        config: &RuntimeConfig,
        approvals: Arc<dyn colossus_ports::ApprovalProvider>,
        options: RuntimeOpenOptions,
    ) -> Result<Self, WorkerError> {
        let options = RuntimeOpenOptions::for_workspace(&options.workspace)?;
        let endpoint = config.worker_ipc_endpoint_at(&options.workspace)?;
        let authentication_key =
            WorkerAuthenticationKey::new(config.worker_ipc_auth_key_at(&options.workspace)?);
        let interactions = Arc::new(colossus_api_runtime::PublicInteractionRouter::new(
            approvals, None,
        ));
        let approval_interface: Arc<dyn ApprovalProvider> = interactions.clone();
        let prompt_interface: Arc<dyn UserPromptProvider> = interactions.clone();
        let runtime = Arc::new(Runtime::open_with_options(
            config,
            approval_interface,
            Some(prompt_interface),
            options,
        )?);
        Ok(Self {
            endpoint,
            listener: None,
            authentication_key,
            runtime,
            replay: Arc::new(Mutex::new(ReplayGuard::default())),
            maintenance: Arc::new(tokio::sync::Mutex::new(())),
            public_interactions: interactions,
            public_api: None,
        })
    }

    /// Open a worker whose protocol-v7 attached clients own prompts, notices, and cancellation.
    pub fn open_with_mode(
        config: &RuntimeConfig,
        approval_mode: WorkerApprovalMode,
    ) -> Result<Self, WorkerError> {
        Self::open_with_mode_at_workspace(
            config,
            approval_mode,
            RuntimeOpenOptions::for_workspace(std::env::current_dir()?)?,
        )
    }

    /// Open an interactive worker for one explicit workspace.
    pub fn open_with_mode_at_workspace(
        config: &RuntimeConfig,
        approval_mode: WorkerApprovalMode,
        options: RuntimeOpenOptions,
    ) -> Result<Self, WorkerError> {
        Self::open_with_mode_at_workspace_and_provider_credentials(
            config,
            approval_mode,
            options,
            Arc::new(EnvironmentCredentialResolver),
        )
    }

    /// Open an interactive worker with a host-provided late-bound provider credential
    /// resolver. Credential values remain behind the permit-bearing provider adapter.
    pub fn open_with_mode_at_workspace_and_provider_credentials(
        config: &RuntimeConfig,
        approval_mode: WorkerApprovalMode,
        options: RuntimeOpenOptions,
        provider_credentials: Arc<dyn CredentialResolver>,
    ) -> Result<Self, WorkerError> {
        let options = RuntimeOpenOptions::for_workspace(&options.workspace)?;
        let authentication_key =
            WorkerAuthenticationKey::new(config.worker_ipc_auth_key_at(&options.workspace)?);
        Self::open_with_mode_at_workspace_provider_credentials_and_authentication(
            config,
            approval_mode,
            options,
            provider_credentials,
            authentication_key,
        )
    }

    /// Open an interactive worker with host-provided provider credentials and an
    /// independent worker key delivered through inherited native bootstrap memory.
    pub fn open_with_mode_at_workspace_provider_credentials_and_authentication(
        config: &RuntimeConfig,
        approval_mode: WorkerApprovalMode,
        options: RuntimeOpenOptions,
        provider_credentials: Arc<dyn CredentialResolver>,
        authentication_key: WorkerAuthenticationKey,
    ) -> Result<Self, WorkerError> {
        let options = RuntimeOpenOptions::for_workspace(&options.workspace)?;
        let approvals: Arc<dyn ApprovalProvider> = Arc::new(WorkerInteractiveApproval {
            mode: approval_mode,
        });
        let user_prompts: Arc<dyn UserPromptProvider> = Arc::new(WorkerInteractiveUserPrompt);
        let endpoint = config.worker_ipc_endpoint_at(&options.workspace)?;
        let interactions = Arc::new(colossus_api_runtime::PublicInteractionRouter::new(
            approvals,
            Some(user_prompts),
        ));
        let approval_interface: Arc<dyn ApprovalProvider> = interactions.clone();
        let prompt_interface: Arc<dyn UserPromptProvider> = interactions.clone();
        let runtime = Arc::new(Runtime::open_with_provider_credentials(
            config,
            approval_interface,
            Some(prompt_interface),
            options,
            provider_credentials,
        )?);
        Ok(Self {
            endpoint,
            listener: None,
            authentication_key,
            runtime,
            replay: Arc::new(Mutex::new(ReplayGuard::default())),
            maintenance: Arc::new(tokio::sync::Mutex::new(())),
            public_interactions: interactions,
            public_api: None,
        })
    }

    /// Bind a credential manager to this worker's authoritative journal.
    ///
    /// The key must come from a stable, API-specific platform secret and must not be
    /// derived from or reused as any worker IPC, journal, signing, TLS, or provider
    /// secret. The returned manager issues bearer material exactly once and journals
    /// only a verifier plus the bounded application grant. Trusted composition code
    /// must deliver the bearer directly to an inherited bootstrap channel or OS
    /// credential store; it must never use files, descriptors, argv, environment
    /// variables, logs, error messages, or renderer state.
    ///
    /// Use the same manager when constructing [`PublicApiHostOptions`]. The worker
    /// rejects options bound to any other journal.
    pub fn public_api_credential_manager(
        &self,
        authentication_key: PublicApiAuthenticationKey,
    ) -> PublicApiCredentialManager {
        PublicApiCredentialManager::bind(self.runtime.journal(), authentication_key)
    }

    /// Bind and securely publish the independently keyed public application API.
    pub async fn enable_public_api(
        mut self,
        options: PublicApiHostOptions,
    ) -> Result<Self, WorkerError> {
        if self.public_api.is_some() {
            return Err(WorkerError::PublicApi(
                "public API is already enabled".into(),
            ));
        }
        if !options.is_bound_to(&self.runtime.journal()) {
            return Err(WorkerError::PublicApi(
                "public API credentials are bound to another worker journal".into(),
            ));
        }
        self.public_api = Some(
            public_api::PreparedPublicApi::prepare(
                options,
                Arc::clone(&self.runtime),
                Arc::clone(&self.public_interactions),
            )
            .await?,
        );
        Ok(self)
    }

    /// Exact bound endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Bind private worker IPC before advertising any dependent public transport.
    ///
    /// Managed sidecars call this during bootstrap so a local endpoint failure is
    /// reported on the inherited channel instead of being masked as a later gRPC
    /// connection failure. Ordinary workers may continue to bind inside `serve`.
    pub async fn prepare_worker_ipc(mut self) -> Result<Self, WorkerError> {
        if self.listener.is_none() {
            self.listener = Some(platform::Listener::bind(&self.endpoint).await?);
        }
        Ok(self)
    }

    /// Sanitized bound public API identity available before service begins.
    pub fn public_api_ready_metadata(&self) -> Option<&PublicApiReadyMetadata> {
        self.public_api.as_ref().map(|api| &api.metadata)
    }

    /// Serve until Ctrl-C or an authenticated shutdown request, then checkpoint cleanly.
    pub async fn serve(self) -> Result<(), WorkerError> {
        self.serve_with_signal(tokio::signal::ctrl_c()).await
    }

    /// Serve until an external guardian resolves or an authenticated shutdown request
    /// arrives, preserving the normal public-run drain, transport force-close, and
    /// checkpoint sequence.
    pub async fn serve_until(
        self,
        shutdown: impl std::future::Future<Output = ()> + Send,
    ) -> Result<(), WorkerError> {
        self.serve_with_signal(async move {
            shutdown.await;
            Ok(())
        })
        .await
    }

    async fn serve_with_signal(
        mut self,
        signal: impl std::future::Future<Output = std::io::Result<()>> + Send,
    ) -> Result<(), WorkerError> {
        let mut listener = match self.listener.take() {
            Some(listener) => listener,
            None => platform::Listener::bind(&self.endpoint).await?,
        };
        let runtime = Arc::clone(&self.runtime);
        let replay = Arc::clone(&self.replay);
        let maintenance = Arc::clone(&self.maintenance);
        let key = self.authentication_key.clone();
        let mut drain_interval = tokio::time::interval(Duration::from_secs(1));
        drain_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        drain_interval.tick().await;
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut tasks = tokio::task::JoinSet::new();
        let (drain_request_tx, drain_request_rx) = tokio::sync::mpsc::channel::<()>(1);
        let drain_runtime = Arc::clone(&runtime);
        let drain_maintenance = Arc::clone(&maintenance);
        tasks.spawn(run_background_drains(drain_request_rx, move || {
            let runtime = Arc::clone(&drain_runtime);
            let maintenance = Arc::clone(&drain_maintenance);
            async move {
                let _ = drain_background_once(runtime.as_ref(), maintenance.as_ref()).await;
            }
        }));
        let mut public_api = self.public_api.take();
        let (public_error_tx, mut public_error_rx) = tokio::sync::mpsc::channel::<String>(1);
        let mut public_error_open = false;
        let mut public_shutdown = None;
        let mut public_force_shutdown = None;
        let mut public_completion = None;
        let mut public_abort = None;
        if let Some(api) = public_api.as_mut() {
            let server = api
                .server
                .take()
                .ok_or_else(|| WorkerError::PublicApi("public API server is absent".into()))?;
            let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel();
            let (force_shutdown, force_shutdown_rx) = tokio::sync::oneshot::channel();
            let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
            public_shutdown = Some(shutdown);
            public_force_shutdown = Some(force_shutdown);
            public_completion = Some(completion_rx);
            public_error_open = true;
            let errors = public_error_tx.clone();
            let server_task = tokio::spawn(async move {
                server
                    .serve_with_force_shutdown(
                        async move {
                            let _ = shutdown_rx.await;
                        },
                        async move {
                            let _ = force_shutdown_rx.await;
                        },
                    )
                    .await
            });
            public_abort = Some(server_task.abort_handle());
            tasks.spawn(async move {
                let result = match server_task.await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(error)) => Err(error.to_string()),
                    Err(error) => Err(format!("public API server task failed: {error}")),
                };
                if let Err(error) = &result {
                    let _ = errors.send(error.clone()).await;
                }
                let _ = completion_tx.send(result);
            });
        }
        drop(public_error_tx);
        let mut stop = false;
        let mut public_failure = None;
        tokio::pin!(signal);
        while !stop {
            tokio::select! {
                biased;
                signal = &mut signal => {
                    signal?;
                    stop = true;
                }
                requested = shutdown_rx.recv() => {
                    stop = requested.is_some();
                }
                failure = public_error_rx.recv(), if public_error_open => {
                    match failure {
                        Some(error) => {
                            public_failure = Some(error);
                            stop = true;
                        }
                        None => public_error_open = false,
                    }
                }
                _ = drain_interval.tick() => {
                    request_background_drain(&drain_request_tx);
                }
                completed = tasks.join_next(), if !tasks.is_empty() => {
                    if let Some(result) = completed {
                        result.map_err(|error| WorkerError::Protocol(error.to_string()))?;
                    }
                }
                accepted = listener.accept() => {
                    let stream = accepted?;
                    let key = key.clone();
                    let runtime = Arc::clone(&runtime);
                    let replay = Arc::clone(&replay);
                    let maintenance = Arc::clone(&maintenance);
                    let drain_requests = drain_request_tx.clone();
                    let shutdown = shutdown_tx.clone();
                    tasks.spawn(async move {
                        if handle_connection(
                            stream,
                            key.expose(),
                            runtime,
                            replay.as_ref(),
                            maintenance.as_ref(),
                            &drain_requests,
                        )
                            .await
                            .is_ok_and(|stopping| stopping)
                        {
                            let _ = shutdown.send(()).await;
                        }
                    });
                }
            }
        }
        if let Some(shutdown) = public_shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(api) = public_api.as_ref()
            && !api.runs.shutdown_and_wait(PUBLIC_RUN_SHUTDOWN_GRACE).await
        {
            public_failure.get_or_insert_with(|| {
                "public runs did not reach a durable terminal state before shutdown".into()
            });
        }
        if let Some(force_shutdown) = public_force_shutdown.take() {
            let _ = force_shutdown.send(());
        }
        if let (Some(completion), Some(abort)) = (public_completion.take(), public_abort.take())
            && let Err(error) = await_public_server_shutdown(
                completion,
                abort,
                PUBLIC_TRANSPORT_FORCE_CLOSE_TIMEOUT,
            )
            .await
        {
            public_failure.get_or_insert(error);
        }
        drop(shutdown_tx);
        drop(drain_request_tx);
        while let Some(result) = tasks.join_next().await {
            result.map_err(|error| WorkerError::Protocol(error.to_string()))?;
        }
        runtime.checkpoint()?;
        listener.cleanup();
        drop(public_api);
        match public_failure {
            Some(error) => Err(WorkerError::PublicApi(error)),
            None => Ok(()),
        }
    }
}

async fn await_public_server_shutdown(
    completion: tokio::sync::oneshot::Receiver<Result<(), String>>,
    abort: tokio::task::AbortHandle,
    timeout: Duration,
) -> Result<(), String> {
    match tokio::time::timeout(timeout, completion).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("public API server completion monitor disappeared".into()),
        Err(_) => {
            abort.abort();
            Err("public API transport did not force-close within its shutdown timeout".into())
        }
    }
}

fn request_background_drain(requests: &tokio::sync::mpsc::Sender<()>) {
    // One pending token is sufficient because every pass drains each durable queue
    // until empty. Keeping that token in the channel while a pass is active makes
    // the wake level-triggered: work queued after the current pass inspected its
    // queue always receives an immediate follow-up pass.
    let _ = requests.try_send(());
}

async fn run_background_drains<F, Fut>(mut requests: tokio::sync::mpsc::Receiver<()>, mut drain: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    while requests.recv().await.is_some() {
        drain().await;
    }
}

pub(super) fn operation_requests_drain(operation: &WorkerOperation) -> bool {
    matches!(
        operation,
        WorkerOperation::AgentQueue { .. }
            | WorkerOperation::AgentRequeue { .. }
            | WorkerOperation::WorkflowStart { queued: true, .. }
    )
}

async fn dispatch_interactive(
    runtime: &Runtime,
    request: InteractiveWorkerRequest,
    observer: &mut dyn RunEventObserver,
    control: &RunControl,
) -> Result<Value, WorkerError> {
    match request {
        InteractiveWorkerRequest::SandboxBoundaryAcknowledge { session_id, mode } => {
            let bridge = ACTIVE_INTERACTIVE_RUN.try_with(Clone::clone).map_err(|_| {
                WorkerError::Protocol("no interactive worker client attached".into())
            })?;
            let acknowledgement_choice = match mode {
                SandboxBoundaryMode::External => "Acknowledge the external boundary",
                SandboxBoundaryMode::DangerFullAccess => "Enable danger full access",
            };
            let title = match mode {
                SandboxBoundaryMode::External => "External sandbox boundary",
                SandboxBoundaryMode::DangerFullAccess => "Danger full access",
            };
            let answer = bridge
                .request(WorkerPrompt {
                    prompt_id: Uuid::now_v7().to_string(),
                    kind: WorkerPromptKind::SandboxBoundaryAcknowledgement,
                    title: title.into(),
                    question: format!(
                        "Acknowledge the configured {} direct-execution boundary for this attached client session?",
                        mode.as_backend()
                    ),
                    choices: vec![
                        acknowledgement_choice.into(),
                        "Keep process execution blocked".into(),
                    ],
                    allow_free_form: false,
                    details: json!({"mode": mode}),
                })
                .await
                .map_err(WorkerError::Protocol)?;
            if answer.as_deref() != Some(acknowledgement_choice) {
                return Ok(json!({"acknowledged": false}));
            }
            let mut acknowledgement = [0_u8; 32];
            getrandom::fill(&mut acknowledgement)
                .map_err(|error| WorkerError::Protocol(error.to_string()))?;
            let acknowledgement =
                SandboxBoundaryAcknowledgement::new(hex::encode(acknowledgement))?;
            runtime.acknowledge_sandbox_boundary_for_interactive_client(
                &session_id,
                mode,
                acknowledgement.expose(),
            )?;
            Ok(json!({
                "acknowledged": true,
                "sandbox_boundary_acknowledgement": acknowledgement,
            }))
        }
        InteractiveWorkerRequest::Run {
            mode,
            role,
            instructions,
            prompt,
            max_turns,
            session_id,
            explicit_skills,
            sticky_skills,
            include_provider_response_diagnostics,
        } => Ok(serde_json::to_value(
            runtime
                .run_with_mode_with_skills_stream_controlled(
                    mode,
                    &role,
                    &instructions,
                    &prompt,
                    max_turns,
                    Some(&session_id),
                    &explicit_skills,
                    &sticky_skills,
                    include_provider_response_diagnostics,
                    observer,
                    control,
                )
                .await?,
        )?),
        InteractiveWorkerRequest::PlanApprove {
            session_id,
            plan_id,
            revision,
        } => Ok(serde_json::to_value(
            runtime
                .approve_plan_at_revision(&session_id, &plan_id, revision)
                .await?,
        )?),
        InteractiveWorkerRequest::PlanDiscard {
            session_id,
            plan_id,
            revision,
        } => Ok(serde_json::to_value(
            runtime
                .discard_plan_at_revision(&session_id, &plan_id, revision)
                .await?,
        )?),
        InteractiveWorkerRequest::PlanExecute {
            role,
            session_id,
            plan_id,
            revision,
            strategy,
            max_turns,
        } => Ok(serde_json::to_value(
            runtime
                .execute_plan_stream_controlled(
                    &role,
                    &session_id,
                    &plan_id,
                    revision,
                    strategy,
                    max_turns,
                    observer,
                    control,
                )
                .await?,
        )?),
        InteractiveWorkerRequest::GoalResume {
            role,
            session_id,
            goal_id,
        } => Ok(serde_json::to_value(
            runtime
                .resume_goal_stream_controlled(&role, &session_id, &goal_id, observer, control)
                .await?,
        )?),
    }
}

async fn handle_interactive_connection<S>(
    stream: S,
    key: &[u8; 32],
    runtime: &Runtime,
    request_id: &str,
    connection_nonce: &str,
    request: InteractiveWorkerRequest,
    sandbox_boundary_acknowledgement: Option<SandboxBoundaryAcknowledgement>,
) -> Result<bool, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (outbound_tx, outbound_rx) = tokio::sync::mpsc::channel(256);
    let bridge = InteractiveRunBridge::new(outbound_tx.clone());
    let control = RunControl::default();
    let mut observer = ChannelWorkerObserver {
        sender: outbound_tx,
    };
    let run = ACTIVE_INTERACTIVE_RUN.scope(
        bridge.clone(),
        colossus_policy::with_sandbox_boundary_acknowledgement(
            sandbox_boundary_acknowledgement
                .as_ref()
                .map(|acknowledgement| acknowledgement.expose().to_owned()),
            Box::pin(async {
                dispatch_interactive(runtime, request, &mut observer, &control).await
            }),
        ),
    );
    drive_interactive_connection(
        stream,
        InteractiveConnectionContext {
            key,
            request_id,
            connection_nonce,
        },
        outbound_rx,
        bridge,
        &control,
        run,
    )
    .await
}

pub(super) struct InteractiveConnectionContext<'a> {
    pub(super) key: &'a [u8; 32],
    pub(super) request_id: &'a str,
    pub(super) connection_nonce: &'a str,
}

pub(super) async fn drive_interactive_connection<S, F>(
    stream: S,
    context: InteractiveConnectionContext<'_>,
    mut outbound_rx: tokio::sync::mpsc::Receiver<WorkerFrameContent>,
    bridge: InteractiveRunBridge,
    control: &RunControl,
    run: F,
) -> Result<bool, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    F: std::future::Future<Output = Result<Value, WorkerError>>,
{
    tokio::pin!(run);
    let InteractiveConnectionContext {
        key,
        request_id,
        connection_nonce,
    } = context;
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (client_tx, mut client_rx) = tokio::sync::mpsc::channel(16);
    let reader_key = *key;
    let reader_request_id = request_id.to_owned();
    let reader_connection_nonce = connection_nonce.to_owned();
    let reader_task = AbortTaskOnDrop::new(tokio::spawn(async move {
        let mut sequence = 0_u64;
        loop {
            let frame = read_message::<_, WorkerClientFrame>(&mut reader, MAX_REQUEST_BYTES)
                .await
                .and_then(|frame| {
                    validate_client_frame(
                        &reader_key,
                        &reader_request_id,
                        &reader_connection_nonce,
                        &mut sequence,
                        &frame,
                    )
                });
            let finished = frame.is_err();
            if client_tx.send(frame).await.is_err() || finished {
                break;
            }
        }
    }));

    let mut sequence = 0_u64;
    loop {
        tokio::select! {
            result = &mut run => {
                // A runtime future may become ready immediately after enqueueing its
                // last released output. Flush the one ordered queue before the
                // terminal frame so clients never observe completion out of order.
                while let Ok(content) = outbound_rx.try_recv() {
                    sequence = sequence.saturating_add(1);
                    if let Err(error) = write_signed_frame(
                        &mut writer,
                        key,
                        request_id,
                        sequence,
                        content,
                    ).await {
                        bridge.cancel_run(control).await;
                        reader_task.abort();
                        return Err(error);
                    }
                }
                sequence = sequence.saturating_add(1);
                let content = match result {
                    Ok(result) => WorkerFrameContent::Complete { result },
                    Err(error) => WorkerFrameContent::Error {
                        message: interactive_error(&error),
                    },
                };
                let write_result =
                    write_signed_frame(&mut writer, key, request_id, sequence, content).await;
                reader_task.abort();
                bridge.cancel_all().await;
                write_result?;
                return Ok(false);
            }
            outbound = outbound_rx.recv() => {
                let Some(content) = outbound else { continue; };
                sequence = sequence.saturating_add(1);
                if let Err(error) = write_signed_frame(
                    &mut writer,
                    key,
                    request_id,
                    sequence,
                    content,
                ).await {
                    bridge.cancel_run(control).await;
                    reader_task.abort();
                    return Err(error);
                }
            }
            client = client_rx.recv() => {
                match client {
                    Some(Ok(ClientFrameContent::PromptResponse { prompt_id, answer })) => {
                        if let Err(error) = bridge.respond(&prompt_id, answer).await {
                            bridge.cancel_run(control).await;
                            reader_task.abort();
                            return Err(error);
                        }
                    }
                    Some(Ok(ClientFrameContent::Cancel)) => {
                        // Cancellation must release both the application loop and an
                        // effect or tool currently waiting for attached-user input.
                        bridge.cancel_run(control).await;
                    }
                    Some(Err(error)) => {
                        bridge.cancel_run(control).await;
                        reader_task.abort();
                        return Err(error);
                    }
                    None => {
                        bridge.cancel_run(control).await;
                        reader_task.abort();
                        return Err(WorkerError::Protocol(
                            "interactive worker client disconnected".into(),
                        ));
                    }
                }
            }
        }
    }
}

fn interactive_error(error: &WorkerError) -> String {
    match error {
        WorkerError::Runtime(error) => match error.provider_response_diagnostic() {
            Some(diagnostic) => bounded_diagnostic_error(&format!(
                "{error}\n\n{}",
                format_provider_response_diagnostic(diagnostic)
            )),
            None => bounded_error(&error.to_string()),
        },
        _ => bounded_error(&error.to_string()),
    }
}

async fn handle_connection<S>(
    mut stream: S,
    key: &[u8; 32],
    runtime: Arc<Runtime>,
    replay: &Mutex<ReplayGuard>,
    maintenance: &tokio::sync::Mutex<()>,
    drain_requests: &tokio::sync::mpsc::Sender<()>,
) -> Result<bool, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let connection_nonce =
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, server_handshake(&mut stream, key))
            .await
            .map_err(|_| WorkerError::Protocol("worker client handshake timed out".into()))
            .and_then(std::convert::identity)
        {
            Ok(nonce) => nonce,
            Err(error) => {
                runtime.record_worker_ipc_audit(false, None, None, Some(&error.to_string()))?;
                return Err(error);
            }
        };
    let request: WorkerRequest = match tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        read_message(&mut stream, MAX_REQUEST_BYTES),
    )
    .await
    .map_err(|_| WorkerError::Protocol("worker request framing timed out".into()))
    .and_then(std::convert::identity)
    {
        Ok(request) => request,
        Err(error) => {
            runtime.record_worker_ipc_audit(false, None, None, Some(&error.to_string()))?;
            return Err(error);
        }
    };
    if let Err(error) = validate_request(key, &request, replay, &connection_nonce) {
        runtime.record_worker_ipc_audit(
            false,
            Some(&request.request_id),
            Some(operation_name(&request.operation)),
            Some(&error.to_string()),
        )?;
        return Err(error);
    }
    runtime.record_worker_ipc_audit(
        true,
        Some(&request.request_id),
        Some(operation_name(&request.operation)),
        None,
    )?;
    let request_id = request.request_id.clone();
    let requests_drain = operation_requests_drain(&request.operation);
    match request.operation {
        WorkerOperation::RunInteractive {
            request,
            sandbox_boundary_acknowledgement,
        } => {
            handle_interactive_connection(
                stream,
                key,
                runtime.as_ref(),
                &request_id,
                &connection_nonce,
                request,
                sandbox_boundary_acknowledgement,
            )
            .await
        }
        WorkerOperation::RunModel {
            role,
            instructions,
            prompt,
            attachments,
            max_turns,
            session_id,
            explicit_skills,
            sticky_skills,
        } => {
            let mut observer = IpcRunObserver {
                stream: &mut stream,
                key,
                request_id: &request_id,
                sequence: 0,
            };
            let attachment_paths = attachments
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let prompt = match runtime
                .prompt_with_text_attachments(&prompt, &attachment_paths)
                .await
            {
                Ok(prompt) => prompt,
                Err(error) => {
                    observer.error(error.to_string()).await?;
                    return Ok(false);
                }
            };
            let result = runtime
                .run_model_with_skills_stream(
                    &role,
                    &instructions,
                    &prompt,
                    max_turns,
                    session_id.as_deref(),
                    &explicit_skills,
                    &sticky_skills,
                    &mut observer,
                )
                .await;
            match result {
                Ok(result) => observer.complete(serde_json::to_value(result)?).await?,
                Err(error) => observer.error(error.to_string()).await?,
            }
            Ok(false)
        }
        WorkerOperation::RunPlan {
            role,
            instructions,
            prompt,
            attachments,
            max_turns,
            session_id,
            explicit_skills,
            sticky_skills,
        } => {
            let mut observer = IpcRunObserver {
                stream: &mut stream,
                key,
                request_id: &request_id,
                sequence: 0,
            };
            let attachment_paths = attachments
                .into_iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            let prompt = match runtime
                .prompt_with_text_attachments(&prompt, &attachment_paths)
                .await
            {
                Ok(prompt) => prompt,
                Err(error) => {
                    observer.error(error.to_string()).await?;
                    return Ok(false);
                }
            };
            let result = runtime
                .run_plan_with_skills_stream(
                    &role,
                    &instructions,
                    &prompt,
                    max_turns,
                    session_id.as_deref(),
                    &explicit_skills,
                    &sticky_skills,
                    &mut observer,
                )
                .await;
            match result {
                Ok(result) => observer.complete(serde_json::to_value(result)?).await?,
                Err(error) => observer.error(error.to_string()).await?,
            }
            Ok(false)
        }
        operation => {
            let shutdown = matches!(operation, WorkerOperation::Shutdown);
            let result = dispatch(&runtime, operation, maintenance).await;
            let succeeded = result.is_ok();
            if succeeded && requests_drain {
                // Durable work is committed at this point. Request its drain before
                // response I/O so a disconnect cannot suppress scheduling.
                request_background_drain(drain_requests);
            }
            let content = match result {
                Ok(result) => WorkerFrameContent::Complete { result },
                Err(error) => WorkerFrameContent::Error {
                    message: bounded_error(&error.to_string()),
                },
            };
            write_signed_frame(&mut stream, key, &request_id, 1, content).await?;
            Ok(shutdown && succeeded)
        }
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn drain_request_during_active_pass_runs_an_immediate_follow_up() {
        let (request_tx, request_rx) = tokio::sync::mpsc::channel(1);
        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let release_first = Arc::new(tokio::sync::Notify::new());
        let passes = Arc::new(AtomicUsize::new(0));
        let worker = tokio::spawn(run_background_drains(request_rx, {
            let passes = Arc::clone(&passes);
            let release_first = Arc::clone(&release_first);
            move || {
                let pass = passes.fetch_add(1, Ordering::AcqRel) + 1;
                let started = started_tx.clone();
                let release_first = Arc::clone(&release_first);
                async move {
                    started.send(pass).expect("drain observer");
                    if pass == 1 {
                        release_first.notified().await;
                    }
                }
            }
        }));

        request_background_drain(&request_tx);
        assert_eq!(started_rx.recv().await, Some(1));

        // The first pass has already consumed its token and is still active. This
        // request must remain pending instead of being discarded as "already draining."
        request_background_drain(&request_tx);
        release_first.notify_one();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), started_rx.recv())
                .await
                .expect("follow-up drain must not wait for the periodic timer"),
            Some(2)
        );

        drop(request_tx);
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .expect("drain worker shutdown")
            .expect("drain worker task");
        assert_eq!(passes.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn public_server_shutdown_aborts_after_the_force_close_timeout() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let server_task = tokio::spawn(async move {
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        let abort = server_task.abort_handle();
        let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
        let monitor = tokio::spawn(async move {
            let _ = server_task.await;
            let _ = completion_tx.send(Ok(()));
        });
        started_rx.await.expect("server task started");

        let error = await_public_server_shutdown(completion_rx, abort, Duration::from_millis(10))
            .await
            .expect_err("pending public server must time out");
        assert!(error.contains("did not force-close"));
        tokio::time::timeout(Duration::from_secs(1), monitor)
            .await
            .expect("aborted public server monitor must finish")
            .expect("monitor task");
    }

    #[tokio::test]
    async fn public_server_shutdown_preserves_transport_failure() {
        let server_task = tokio::spawn(std::future::pending::<()>());
        let abort = server_task.abort_handle();
        let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
        completion_tx
            .send(Err("transport failed".into()))
            .expect("completion receiver");

        let error =
            await_public_server_shutdown(completion_rx, abort.clone(), Duration::from_secs(1))
                .await
                .expect_err("transport failure");
        assert_eq!(error, "transport failed");
        abort.abort();
    }
}
