use super::*;

const PUBLIC_RUN_SHUTDOWN_GRACE: Duration = Duration::from_secs(30);
const PUBLIC_TRANSPORT_FORCE_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Long-running single-writer runtime owner and authenticated IPC server.
pub struct WorkerServer {
    endpoint: String,
    authentication_key: [u8; 32],
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
        let endpoint = config.worker_ipc_endpoint_at(&options.workspace)?;
        let authentication_key = config.worker_ipc_auth_key_at(&options.workspace)?;
        let interactions = Arc::new(colossus_api_runtime::PublicInteractionRouter::new(
            approvals, None,
        ));
        let approval_interface: Arc<dyn ApprovalProvider> = interactions.clone();
        let prompt_interface: Arc<dyn UserPromptProvider> = interactions.clone();
        Ok(Self {
            endpoint,
            authentication_key,
            runtime: Arc::new(Runtime::open_with_options(
                config,
                approval_interface,
                Some(prompt_interface),
                options,
            )?),
            replay: Arc::new(Mutex::new(ReplayGuard::default())),
            maintenance: Arc::new(tokio::sync::Mutex::new(())),
            public_interactions: interactions,
            public_api: None,
        })
    }

    /// Open a worker whose protocol-v4 attached clients own prompts and cancellation.
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
        let approvals: Arc<dyn ApprovalProvider> = Arc::new(WorkerInteractiveApproval {
            mode: approval_mode,
        });
        let user_prompts: Arc<dyn UserPromptProvider> = Arc::new(WorkerInteractiveUserPrompt);
        let endpoint = config.worker_ipc_endpoint_at(&options.workspace)?;
        let authentication_key = config.worker_ipc_auth_key_at(&options.workspace)?;
        let interactions = Arc::new(colossus_api_runtime::PublicInteractionRouter::new(
            approvals,
            Some(user_prompts),
        ));
        let approval_interface: Arc<dyn ApprovalProvider> = interactions.clone();
        let prompt_interface: Arc<dyn UserPromptProvider> = interactions.clone();
        Ok(Self {
            endpoint,
            authentication_key,
            runtime: Arc::new(Runtime::open_with_options(
                config,
                approval_interface,
                Some(prompt_interface),
                options,
            )?),
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

    /// Serve until Ctrl-C or an authenticated shutdown request, then checkpoint cleanly.
    pub async fn serve(mut self) -> Result<(), WorkerError> {
        let mut listener = platform::Listener::bind(&self.endpoint).await?;
        let runtime = Arc::clone(&self.runtime);
        let replay = Arc::clone(&self.replay);
        let maintenance = Arc::clone(&self.maintenance);
        let drain_notify = Arc::new(tokio::sync::Notify::new());
        let key = self.authentication_key;
        let mut drain_interval = tokio::time::interval(Duration::from_secs(1));
        drain_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        drain_interval.tick().await;
        let draining = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut tasks = tokio::task::JoinSet::new();
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
        while !stop {
            tokio::select! {
                biased;
                signal = tokio::signal::ctrl_c() => {
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
                _ = drain_notify.notified() => {
                    spawn_drain_if_idle(&mut tasks, &runtime, &maintenance, &draining);
                }
                _ = drain_interval.tick() => {
                    spawn_drain_if_idle(&mut tasks, &runtime, &maintenance, &draining);
                }
                completed = tasks.join_next(), if !tasks.is_empty() => {
                    if let Some(result) = completed {
                        result.map_err(|error| WorkerError::Protocol(error.to_string()))?;
                    }
                }
                accepted = listener.accept() => {
                    let stream = accepted?;
                    let runtime = Arc::clone(&runtime);
                    let replay = Arc::clone(&replay);
                    let maintenance = Arc::clone(&maintenance);
                    let drain_notify = Arc::clone(&drain_notify);
                    let shutdown = shutdown_tx.clone();
                    tasks.spawn(async move {
                        if handle_connection(
                            stream,
                            &key,
                            runtime.as_ref(),
                            replay.as_ref(),
                            maintenance.as_ref(),
                            drain_notify.as_ref(),
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

fn spawn_drain_if_idle(
    tasks: &mut tokio::task::JoinSet<()>,
    runtime: &Arc<Runtime>,
    maintenance: &Arc<tokio::sync::Mutex<()>>,
    draining: &Arc<AtomicBool>,
) {
    if draining
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let runtime = Arc::clone(runtime);
    let draining = Arc::clone(draining);
    let maintenance = Arc::clone(maintenance);
    tasks.spawn(async move {
        let _ = drain_once(runtime.as_ref(), maintenance.as_ref()).await;
        draining.store(false, Ordering::Release);
    });
}

pub(super) fn operation_requests_drain(operation: &WorkerOperation) -> bool {
    matches!(
        operation,
        WorkerOperation::AgentQueue { .. }
            | WorkerOperation::AgentRequeue { .. }
            | WorkerOperation::WorkflowStart { queued: true, .. }
    )
}

async fn handle_connection<S>(
    mut stream: S,
    key: &[u8; 32],
    runtime: &Runtime,
    replay: &Mutex<ReplayGuard>,
    maintenance: &tokio::sync::Mutex<()>,
    drain_notify: &tokio::sync::Notify,
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
        WorkerOperation::RunModelControlled {
            role,
            instructions,
            prompt,
            max_turns,
            session_id,
            explicit_skills,
            sticky_skills,
        } => {
            let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(256);
            let (prompt_tx, mut prompt_rx) = tokio::sync::mpsc::channel(16);
            let responses = Arc::new(tokio::sync::Mutex::new(BTreeMap::new()));
            let bridge = InteractiveRunBridge {
                prompts: prompt_tx,
                responses,
            };
            let control = RunControl::default();
            let mut observer = ChannelWorkerObserver { sender: event_tx };
            let run = ACTIVE_INTERACTIVE_RUN.scope(
                bridge.clone(),
                runtime.run_model_with_skills_stream_controlled(
                    &role,
                    &instructions,
                    &prompt,
                    max_turns,
                    Some(&session_id),
                    &explicit_skills,
                    &sticky_skills,
                    &mut observer,
                    &control,
                ),
            );
            tokio::pin!(run);
            let (mut reader, mut writer) = tokio::io::split(stream);
            let (client_tx, mut client_rx) = tokio::sync::mpsc::channel(16);
            let reader_key = *key;
            let reader_request_id = request_id.clone();
            let reader_connection_nonce = connection_nonce.clone();
            let reader_task = tokio::spawn(async move {
                let mut sequence = 0_u64;
                loop {
                    let frame =
                        read_message::<_, WorkerClientFrame>(&mut reader, MAX_REQUEST_BYTES)
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
            });
            let mut sequence = 0_u64;
            loop {
                tokio::select! {
                    result = &mut run => {
                        sequence = sequence.saturating_add(1);
                        let content = match result {
                            Ok(outcome) => WorkerFrameContent::Complete {
                                result: serde_json::to_value(outcome)?,
                            },
                            Err(error) => WorkerFrameContent::Error {
                                message: bounded_error(&error.to_string()),
                            },
                        };
                        write_signed_frame(&mut writer, key, &request_id, sequence, content).await?;
                        reader_task.abort();
                        bridge.cancel_all().await;
                        return Ok(false);
                    }
                    event = event_rx.recv() => {
                        let Some(event) = event else { continue; };
                        sequence = sequence.saturating_add(1);
                        write_signed_frame(
                            &mut writer,
                            key,
                            &request_id,
                            sequence,
                            WorkerFrameContent::Event { event },
                        ).await?;
                    }
                    prompt = prompt_rx.recv() => {
                        let Some(prompt) = prompt else { continue; };
                        sequence = sequence.saturating_add(1);
                        write_signed_frame(
                            &mut writer,
                            key,
                            &request_id,
                            sequence,
                            WorkerFrameContent::Prompt { prompt },
                        ).await?;
                    }
                    client = client_rx.recv() => {
                        match client {
                            Some(Ok(ClientFrameContent::PromptResponse { prompt_id, answer })) => {
                                bridge.respond(&prompt_id, answer).await?;
                            }
                            Some(Ok(ClientFrameContent::Cancel)) => control.cancel(),
                            Some(Err(error)) => {
                                control.cancel();
                                bridge.cancel_all().await;
                                reader_task.abort();
                                return Err(error);
                            }
                            None => {
                                control.cancel();
                                bridge.cancel_all().await;
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
        WorkerOperation::RunModel {
            role,
            instructions,
            prompt,
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
            let result = dispatch(runtime, operation, maintenance).await;
            let succeeded = result.is_ok();
            let content = match result {
                Ok(result) => WorkerFrameContent::Complete { result },
                Err(error) => WorkerFrameContent::Error {
                    message: bounded_error(&error.to_string()),
                },
            };
            write_signed_frame(&mut stream, key, &request_id, 1, content).await?;
            if succeeded && requests_drain {
                drain_notify.notify_one();
            }
            Ok(shutdown && succeeded)
        }
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;

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
