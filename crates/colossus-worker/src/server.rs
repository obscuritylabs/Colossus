use super::*;

/// Long-running single-writer runtime owner and authenticated IPC server.
pub struct WorkerServer {
    endpoint: String,
    authentication_key: [u8; 32],
    runtime: Arc<Runtime>,
    replay: Arc<Mutex<ReplayGuard>>,
    maintenance: Arc<tokio::sync::Mutex<()>>,
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
        Ok(Self {
            endpoint,
            authentication_key,
            runtime: Arc::new(Runtime::open_with_options(
                config, approvals, None, options,
            )?),
            replay: Arc::new(Mutex::new(ReplayGuard::default())),
            maintenance: Arc::new(tokio::sync::Mutex::new(())),
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
        Ok(Self {
            endpoint,
            authentication_key,
            runtime: Arc::new(Runtime::open_with_options(
                config,
                approvals,
                Some(user_prompts),
                options,
            )?),
            replay: Arc::new(Mutex::new(ReplayGuard::default())),
            maintenance: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// Exact bound endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Serve until Ctrl-C or an authenticated shutdown request, then checkpoint cleanly.
    pub async fn serve(self) -> Result<(), WorkerError> {
        let mut listener = platform::Listener::bind(&self.endpoint).await?;
        let runtime = Arc::clone(&self.runtime);
        let replay = Arc::clone(&self.replay);
        let maintenance = Arc::clone(&self.maintenance);
        let key = self.authentication_key;
        let mut drain_interval = tokio::time::interval(Duration::from_secs(1));
        drain_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        drain_interval.tick().await;
        let draining = Arc::new(AtomicBool::new(false));
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
        let mut tasks = tokio::task::JoinSet::new();
        let mut stop = false;
        while !stop {
            tokio::select! {
                accepted = listener.accept() => {
                    let stream = accepted?;
                    let runtime = Arc::clone(&runtime);
                    let replay = Arc::clone(&replay);
                    let maintenance = Arc::clone(&maintenance);
                    let shutdown = shutdown_tx.clone();
                    tasks.spawn(async move {
                        if handle_connection(
                            stream,
                            &key,
                            runtime.as_ref(),
                            replay.as_ref(),
                            maintenance.as_ref(),
                        )
                            .await
                            .is_ok_and(|stopping| stopping)
                        {
                            let _ = shutdown.send(()).await;
                        }
                    });
                }
                signal = tokio::signal::ctrl_c() => {
                    signal?;
                    stop = true;
                }
                requested = shutdown_rx.recv() => {
                    stop = requested.is_some();
                }
                _ = drain_interval.tick() => {
                    if draining.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                        let runtime = Arc::clone(&runtime);
                        let draining = Arc::clone(&draining);
                        let maintenance = Arc::clone(&maintenance);
                        tasks.spawn(async move {
                            let _ = drain_once(runtime.as_ref(), maintenance.as_ref()).await;
                            draining.store(false, Ordering::Release);
                        });
                    }
                }
                completed = tasks.join_next(), if !tasks.is_empty() => {
                    if let Some(result) = completed {
                        result.map_err(|error| WorkerError::Protocol(error.to_string()))?;
                    }
                }
            }
        }
        drop(shutdown_tx);
        while let Some(result) = tasks.join_next().await {
            result.map_err(|error| WorkerError::Protocol(error.to_string()))?;
        }
        runtime.checkpoint()?;
        listener.cleanup();
        Ok(())
    }
}

async fn handle_connection<S>(
    mut stream: S,
    key: &[u8; 32],
    runtime: &Runtime,
    replay: &Mutex<ReplayGuard>,
    maintenance: &tokio::sync::Mutex<()>,
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
            Ok(shutdown && succeeded)
        }
    }
}
