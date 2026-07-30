use super::*;

/// Client-side handler for authenticated worker approval and input prompts.
#[async_trait]
pub trait WorkerPromptHandler: Send + Sync {
    /// Present one non-blocking policy notice from the active run.
    async fn notice(&self, _notice: ApprovalReviewNotice) -> Result<(), WorkerError> {
        Ok(())
    }

    /// Return one bounded answer, or `None` to fail closed.
    async fn prompt(&self, prompt: WorkerPrompt) -> Result<Option<String>, WorkerError>;
}

/// Authenticated one-request-per-connection worker client.
#[derive(Clone)]
pub struct WorkerClient {
    endpoint: String,
    authentication_key: WorkerAuthenticationKey,
}

impl WorkerClient {
    /// Resolve a client only when a platform endpoint may currently exist.
    pub fn discover(config: &RuntimeConfig) -> Result<Option<Self>, WorkerError> {
        let endpoint = config.worker_ipc_endpoint()?;
        if !platform::endpoint_is_trusted(&endpoint)? {
            return Ok(None);
        }
        Ok(Some(Self {
            endpoint,
            authentication_key: WorkerAuthenticationKey::new(config.worker_ipc_auth_key()?),
        }))
    }

    /// Resolve the platform endpoint and authentication key from runtime configuration.
    pub fn from_config(config: &RuntimeConfig) -> Result<Self, WorkerError> {
        let endpoint = config.worker_ipc_endpoint()?;
        if !platform::endpoint_is_trusted(&endpoint)? {
            return Err(WorkerError::Unavailable(endpoint));
        }
        Ok(Self {
            endpoint,
            authentication_key: WorkerAuthenticationKey::new(config.worker_ipc_auth_key()?),
        })
    }

    /// Resolve only the trusted endpoint from configuration while using a key
    /// delivered through an inherited native channel.
    pub fn from_config_with_authentication(
        config: &RuntimeConfig,
        authentication_key: WorkerAuthenticationKey,
    ) -> Result<Self, WorkerError> {
        let endpoint = config.worker_ipc_endpoint()?;
        if !platform::endpoint_is_trusted(&endpoint)? {
            return Err(WorkerError::Unavailable(endpoint));
        }
        Ok(Self {
            endpoint,
            authentication_key,
        })
    }

    /// Exact configured Unix socket or Windows named-pipe endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Return readiness metadata when an authenticated worker is listening.
    pub async fn ping(&self) -> Result<Value, WorkerError> {
        self.call(WorkerOperation::Ping).await
    }

    /// Execute a non-streaming worker operation.
    pub async fn call(&self, operation: WorkerOperation) -> Result<Value, WorkerError> {
        let mut stream = self.connect().await?;
        let connection_nonce = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            client_handshake(&mut stream, self.authentication_key.expose()),
        )
        .await
        .map_err(|_| handshake_timeout_error(&self.endpoint))??;
        let request = signed_request(
            self.authentication_key.expose(),
            operation,
            &connection_nonce,
        )?;
        write_message(&mut stream, &request, MAX_REQUEST_BYTES).await?;
        let mut sequence = 0_u64;
        let frame: WorkerFrame = read_message(&mut stream, MAX_FRAME_BYTES).await?;
        let content = validate_frame(
            self.authentication_key.expose(),
            &request.request_id,
            &mut sequence,
            &frame,
        )?;
        match content {
            WorkerFrameContent::Event { .. } => Err(WorkerError::Protocol(
                "non-streaming call received a run event".into(),
            )),
            WorkerFrameContent::Notice { .. } => Err(WorkerError::Protocol(
                "non-interactive call received a notice".into(),
            )),
            WorkerFrameContent::Prompt { .. } => Err(WorkerError::Protocol(
                "non-interactive call received a prompt and failed closed".into(),
            )),
            WorkerFrameContent::Complete { result } => Ok(result),
            WorkerFrameContent::Error { message } => Err(WorkerError::Remote(message)),
        }
    }

    /// Execute one model run while forwarding authenticated released run events.
    pub async fn run_model(
        &self,
        operation: WorkerOperation,
        observer: &mut dyn colossus_ports::RunEventObserver,
    ) -> Result<AgentRunResult, WorkerError> {
        if !matches!(
            operation,
            WorkerOperation::RunModel { .. } | WorkerOperation::RunPlan { .. }
        ) {
            return Err(WorkerError::Protocol(
                "run_model requires a run_model or run_plan operation".into(),
            ));
        }
        let mut stream = self.connect().await?;
        let connection_nonce = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            client_handshake(&mut stream, self.authentication_key.expose()),
        )
        .await
        .map_err(|_| handshake_timeout_error(&self.endpoint))??;
        let request = signed_request(
            self.authentication_key.expose(),
            operation,
            &connection_nonce,
        )?;
        write_message(&mut stream, &request, MAX_REQUEST_BYTES).await?;
        let mut sequence = 0_u64;
        loop {
            let frame: WorkerFrame = read_message(&mut stream, MAX_FRAME_BYTES).await?;
            let content = validate_frame(
                self.authentication_key.expose(),
                &request.request_id,
                &mut sequence,
                &frame,
            )?;
            match content {
                WorkerFrameContent::Event { event } => observer
                    .observe(event)
                    .await
                    .map_err(|error| WorkerError::Remote(error.to_string()))?,
                WorkerFrameContent::Notice { .. } => {
                    return Err(WorkerError::Protocol(
                        "uncontrolled model call received a notice".into(),
                    ));
                }
                WorkerFrameContent::Complete { result } => {
                    return serde_json::from_value(result).map_err(|error| {
                        WorkerError::Protocol(format!("invalid run result: {error}"))
                    });
                }
                WorkerFrameContent::Prompt { .. } => {
                    return Err(WorkerError::Protocol(
                        "uncontrolled model call received a prompt and failed closed".into(),
                    ));
                }
                WorkerFrameContent::Error { message } => return Err(WorkerError::Remote(message)),
            }
        }
    }

    /// Execute one protocol-v6 interactive operation with authenticated prompts,
    /// notices, released events, and cooperative cancellation.
    pub async fn call_interactive<T>(
        &self,
        operation: WorkerOperation,
        observer: &mut dyn RunEventObserver,
        prompts: &dyn WorkerPromptHandler,
        control: &RunControl,
    ) -> Result<T, WorkerError>
    where
        T: serde::de::DeserializeOwned,
    {
        if !matches!(operation, WorkerOperation::RunInteractive { .. }) {
            return Err(WorkerError::Protocol(
                "call_interactive requires run_interactive".into(),
            ));
        }
        if control.is_cancelled() {
            return Err(WorkerError::Cancelled);
        }
        let mut stream = self.connect().await?;
        let connection_nonce = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            client_handshake(&mut stream, self.authentication_key.expose()),
        )
        .await
        .map_err(|_| handshake_timeout_error(&self.endpoint))??;
        if control.is_cancelled() {
            return Err(WorkerError::Cancelled);
        }
        let request = signed_request(
            self.authentication_key.expose(),
            operation,
            &connection_nonce,
        )?;
        write_message(&mut stream, &request, MAX_REQUEST_BYTES).await?;
        drive_interactive_client(
            stream,
            self.authentication_key.expose(),
            &request.request_id,
            &connection_nonce,
            observer,
            prompts,
            control,
        )
        .await
    }

    /// Execute an interactive model or Plan Mode run.
    ///
    /// This compatibility convenience keeps callers that expect an agent outcome
    /// concise while all protocol-v6 operations share [`Self::call_interactive`].
    pub async fn run_model_controlled(
        &self,
        operation: WorkerOperation,
        observer: &mut dyn RunEventObserver,
        prompts: &dyn WorkerPromptHandler,
        control: &RunControl,
    ) -> Result<AgentRunOutcome, WorkerError> {
        self.call_interactive(operation, observer, prompts, control)
            .await
    }

    async fn connect(&self) -> Result<platform::ClientStream, WorkerError> {
        match tokio::time::timeout(CONNECT_TIMEOUT, platform::connect(&self.endpoint)).await {
            Err(_) => Err(WorkerError::Busy(self.endpoint.clone())),
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(error)) if platform::connection_is_busy(&error) => {
                Err(WorkerError::Busy(self.endpoint.clone()))
            }
            Ok(Err(error)) if platform::connection_is_absent(&error) => {
                Err(WorkerError::Unavailable(self.endpoint.clone()))
            }
            Ok(Err(error)) => Err(WorkerError::Io(error)),
        }
    }
}

pub(super) async fn drive_interactive_client<S, T>(
    stream: S,
    key: &[u8; 32],
    request_id: &str,
    connection_nonce: &str,
    observer: &mut dyn RunEventObserver,
    prompts: &dyn WorkerPromptHandler,
    control: &RunControl,
) -> Result<T, WorkerError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    T: serde::de::DeserializeOwned,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel(32);
    let reader_task = AbortTaskOnDrop::new(tokio::spawn(async move {
        loop {
            let frame = read_message::<_, WorkerFrame>(&mut reader, MAX_FRAME_BYTES).await;
            let finished = frame.is_err();
            if frame_tx.send(frame).await.is_err() || finished {
                break;
            }
        }
    }));
    let mut server_sequence = 0_u64;
    let mut client_sequence = 0_u64;
    let mut cancellation_sent = false;
    let mut cancellation_poll = tokio::time::interval(Duration::from_millis(50));
    cancellation_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let frame = tokio::select! {
            biased;
            _ = cancellation_poll.tick(), if !cancellation_sent => {
                if control.is_cancelled() {
                    client_sequence = client_sequence.saturating_add(1);
                    write_signed_client_frame(
                        &mut writer,
                        key,
                        request_id,
                        connection_nonce,
                        client_sequence,
                        ClientFrameContent::Cancel,
                    )
                    .await?;
                    cancellation_sent = true;
                }
                continue;
            }
            frame = frame_rx.recv() => frame.ok_or_else(|| {
                WorkerError::Protocol("worker response stream closed".into())
            })??,
        };
        let content = validate_frame(key, request_id, &mut server_sequence, &frame)?;
        match content {
            WorkerFrameContent::Event { event } => observer
                .observe(event)
                .await
                .map_err(|error| WorkerError::Remote(error.to_string()))?,
            WorkerFrameContent::Notice { notice } => prompts.notice(notice).await?,
            WorkerFrameContent::Prompt { prompt } => {
                if cancellation_sent {
                    // The server will release this waiter when it processes the
                    // already-authenticated cancellation frame.
                    continue;
                }
                let prompt_id = prompt.prompt_id.clone();
                let answer = prompts.prompt(prompt);
                tokio::pin!(answer);
                let answer = loop {
                    tokio::select! {
                        biased;
                        _ = cancellation_poll.tick() => {
                            if !control.is_cancelled() {
                                continue;
                            }
                            client_sequence = client_sequence.saturating_add(1);
                            write_signed_client_frame(
                                &mut writer,
                                key,
                                request_id,
                                connection_nonce,
                                client_sequence,
                                ClientFrameContent::Cancel,
                            )
                            .await?;
                            cancellation_sent = true;
                            break None;
                        }
                        frame = frame_rx.recv() => {
                            let frame = frame.ok_or_else(|| {
                                WorkerError::Protocol(
                                    "worker response stream closed while prompting".into(),
                                )
                            })??;
                            let content =
                                validate_frame(key, request_id, &mut server_sequence, &frame)?;
                            match content {
                                WorkerFrameContent::Event { event } => observer
                                    .observe(event)
                                    .await
                                    .map_err(|error| WorkerError::Remote(error.to_string()))?,
                                WorkerFrameContent::Notice { notice } => {
                                    prompts.notice(notice).await?;
                                }
                                WorkerFrameContent::Prompt { .. } => {
                                    return Err(WorkerError::Protocol(
                                        "worker sent overlapping interactive prompts".into(),
                                    ));
                                }
                                WorkerFrameContent::Complete { result } => {
                                    reader_task.abort();
                                    return serde_json::from_value(result).map_err(|error| {
                                        WorkerError::Protocol(format!(
                                            "invalid interactive operation result: {error}"
                                        ))
                                    });
                                }
                                WorkerFrameContent::Error { message } => {
                                    reader_task.abort();
                                    return Err(WorkerError::Remote(message));
                                }
                            }
                        }
                        answer = &mut answer => break Some(answer?),
                    }
                };
                let Some(answer) = answer else {
                    continue;
                };
                if control.is_cancelled() {
                    client_sequence = client_sequence.saturating_add(1);
                    write_signed_client_frame(
                        &mut writer,
                        key,
                        request_id,
                        connection_nonce,
                        client_sequence,
                        ClientFrameContent::Cancel,
                    )
                    .await?;
                    cancellation_sent = true;
                    continue;
                }
                client_sequence = client_sequence.saturating_add(1);
                write_signed_client_frame(
                    &mut writer,
                    key,
                    request_id,
                    connection_nonce,
                    client_sequence,
                    ClientFrameContent::PromptResponse { prompt_id, answer },
                )
                .await?;
            }
            WorkerFrameContent::Complete { result } => {
                reader_task.abort();
                return serde_json::from_value(result).map_err(|error| {
                    WorkerError::Protocol(format!("invalid interactive operation result: {error}"))
                });
            }
            WorkerFrameContent::Error { message } => {
                reader_task.abort();
                return Err(WorkerError::Remote(message));
            }
        }
    }
}

pub(super) fn handshake_timeout_error(endpoint: &str) -> WorkerError {
    // Establishing the transport proves that an endpoint accepted this
    // connection. A delayed authenticated hello is therefore a live/busy or
    // unhealthy endpoint, never evidence that embedded execution is safe.
    WorkerError::Busy(endpoint.into())
}
