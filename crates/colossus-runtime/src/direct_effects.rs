use super::*;

impl Runtime {
    /// Credential-free, network-free smoke provider routed through policy and journal.
    pub async fn echo(&self, message: &str) -> Result<ReleasedEffectResult, RuntimeError> {
        let request = effect_request(
            system_actor("offline-echo"),
            "provider.echo",
            "provider:echo",
            json!({"message": message}),
        );
        self.gateway
            .execute(request, &EchoExecutor)
            .await
            .map_err(Into::into)
    }

    /// Read bounded UTF-8 text through the universal filesystem effect boundary.
    pub async fn read_text_file(&self, path: impl AsRef<Path>) -> Result<String, RuntimeError> {
        let path = absolute_path(path.as_ref())?;
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            "filesystem.read",
            path.display().to_string(),
            json!({"path": path.display().to_string(), "encoding": "utf-8"}),
        );
        request.capabilities = vec!["filesystem.read".into()];
        let result = self
            .gateway
            .execute(request, self.filesystem_executor.as_ref())
            .await?;
        String::from_utf8(result.bytes)
            .map_err(|error| RuntimeError::Config(format!("file is not valid UTF-8: {error}")))
    }

    /// Write bounded UTF-8 text through policy, approval, and the filesystem adapter.
    pub async fn write_text_file(
        &self,
        path: impl AsRef<Path>,
        text: &str,
    ) -> Result<Value, RuntimeError> {
        let path = absolute_path(path.as_ref())?;
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            "filesystem.write",
            path.display().to_string(),
            json!({"text": text}),
        );
        request.capabilities = vec!["filesystem.write".into()];
        let result = self
            .gateway
            .execute(request, self.filesystem_executor.as_ref())
            .await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Execute an exact program without a shell through the authenticated sandbox helper.
    pub async fn run_process(
        &self,
        executable: impl AsRef<Path>,
        cwd: impl AsRef<Path>,
        args: Vec<String>,
        environment: std::collections::BTreeMap<String, String>,
    ) -> Result<Value, RuntimeError> {
        let executable = if self.sandbox_backend == "oci" {
            let executable = executable.as_ref();
            let value = executable
                .to_str()
                .ok_or_else(|| RuntimeError::Config("OCI executable path must be UTF-8".into()))?;
            if !normalized_oci_path(value) {
                return Err(RuntimeError::Config(
                    "OCI executable must be an exact normalized absolute image path".into(),
                ));
            }
            executable.to_owned()
        } else {
            fs::canonicalize(executable)?
        };
        let cwd = fs::canonicalize(cwd)?;
        let spec = ProcessSpec {
            cwd,
            args,
            environment,
            stdin_base64: None,
            timeout_ms: None,
            max_output_bytes: None,
        };
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            "process.spawn",
            executable.display().to_string(),
            serde_json::to_value(spec).map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec!["process.spawn".into()];
        let result = self
            .gateway
            .execute(request, self.process_executor.as_ref())
            .await?;
        serde_json::from_slice(&result.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Fetch one exact policy-allowed URL into quarantine and post-effect authorization.
    pub async fn http_get(&self, url: &str) -> Result<ReleasedEffectResult, RuntimeError> {
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            "network.http",
            url,
            json!({"method": "GET", "headers": {"accept": "*/*"}}),
        );
        request.capabilities = vec!["network.http".into()];
        self.gateway
            .execute(request, self.http_executor.as_ref())
            .await
            .map_err(Into::into)
    }

    /// Read and validate a workflow path through policy and post-effect release.
    pub async fn validate_workflow_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ValidatedWorkflow, RuntimeError> {
        let yaml = self.read_text_file(path).await?;
        validate_definition(&yaml).map_err(Into::into)
    }

    /// Read, validate, and register a workflow path without bypassing the gateway.
    pub async fn register_workflow_path(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<ValidatedWorkflow, RuntimeError> {
        let path = absolute_path(path.as_ref())?;
        let yaml = self.read_text_file(&path).await?;
        self.workflows
            .register_definition(&yaml, &format!("repo:{}", path.display()))
            .map_err(Into::into)
    }

    /// Sign the current chain head for clean shutdown.
    pub fn checkpoint(&self) -> Result<(), RuntimeError> {
        if self.journal.is_recovery_mode() {
            return Ok(());
        }
        self.drain_projections()?;
        self.journal.checkpoint()?;
        Ok(())
    }

    /// Append metadata-only evidence for an accepted or rejected local worker request.
    pub fn record_worker_ipc_audit(
        &self,
        accepted: bool,
        request_id: Option<&str>,
        operation: Option<&str>,
        reason: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let audit_id = Uuid::now_v7().to_string();
        let correlation_id = request_id.unwrap_or(&audit_id).to_owned();
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: format!("worker-ipc:{audit_id}"),
            expected_stream_version: 0,
            classification: EventClassification::System,
            event_type: if accepted {
                "worker.ipc.accepted.v1"
            } else {
                "worker.ipc.rejected.v1"
            }
            .into(),
            actor: system_actor("local-worker-ipc"),
            context: ExecutionContext {
                correlation_id,
                ..ExecutionContext::default()
            },
            payload: json!({
                "request_id": request_id,
                "operation": operation,
                "reason": reason.map(|value| value.chars().take(1024).collect::<String>()),
                "content_recorded": false,
            }),
        })?;
        Ok(())
    }
}
