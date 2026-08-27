use super::*;

impl Runtime {
    /// Validate exact local bytes with the shared bounded run-input image contract.
    pub fn validate_run_input_image(
        &self,
        file_name: &str,
        declared_media_type: Option<&str>,
        bytes: &[u8],
    ) -> Result<ValidatedImage, RuntimeError> {
        validate_image_bytes(file_name, declared_media_type, bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Resolve an owner-authorized encrypted artifact into durable image metadata.
    pub fn run_input_image_reference(
        &self,
        owner_id: &str,
        artifact_id: &str,
    ) -> Result<ModelImageReference, RuntimeError> {
        self.run_input_media
            .image_reference(owner_id, artifact_id)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Retrieve exact verified bytes for a trusted local preview of a canonical reference.
    pub async fn preview_run_input_image(
        &self,
        reference: &ModelImageReference,
    ) -> Result<Vec<u8>, RuntimeError> {
        let resolved = colossus_ports::RunInputMediaResolver::resolve_image(
            self.run_input_media.as_ref(),
            reference,
        )
        .await
        .map_err(|error| RuntimeError::Config(error.to_string()))?;
        Ok(resolved.bytes)
    }

    /// Render bounded UTF-8 attachment files into a model prompt after policy-authorized reads.
    pub async fn prompt_with_text_attachments(
        &self,
        prompt: &str,
        attachments: &[PathBuf],
    ) -> Result<String, RuntimeError> {
        let mut resolved = Vec::with_capacity(attachments.len());
        for path in attachments {
            let bytes = self.read_file_bytes(path).await?;
            resolved.push((path.clone(), bytes));
        }
        self.prompt_with_text_attachment_bytes(prompt, &resolved)
    }

    /// Render bounded, already-authorized UTF-8 attachment bytes without reading them again.
    pub fn prompt_with_text_attachment_bytes(
        &self,
        prompt: &str,
        attachments: &[(PathBuf, Vec<u8>)],
    ) -> Result<String, RuntimeError> {
        const MAX_ATTACHMENTS: usize = 16;
        const MAX_INPUT_BYTES: usize = 1_048_576;

        if attachments.len() > MAX_ATTACHMENTS {
            return Err(RuntimeError::Config(format!(
                "at most {MAX_ATTACHMENTS} attachments may be supplied"
            )));
        }
        if prompt.len() > MAX_INPUT_BYTES {
            return Err(RuntimeError::Config(
                "combined prompt and attachments exceed 1 MiB".into(),
            ));
        }
        let mut rendered = String::with_capacity(prompt.len());
        rendered.push_str(prompt);
        for (path, bytes) in attachments {
            append_text_attachment(&mut rendered, path, bytes, MAX_INPUT_BYTES)?;
        }
        Ok(rendered)
    }

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
        let bytes = self.read_file_bytes(path).await?;
        String::from_utf8(bytes)
            .map_err(|error| RuntimeError::Config(format!("file is not valid UTF-8: {error}")))
    }

    /// Read bounded bytes through the universal filesystem effect boundary.
    pub async fn read_file_bytes(&self, path: impl AsRef<Path>) -> Result<Vec<u8>, RuntimeError> {
        let path = workspace_absolute_path(&self.workspace, path.as_ref());
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            "filesystem.read",
            path.display().to_string(),
            json!({"path": path.display().to_string(), "encoding": "binary"}),
        );
        request.capabilities = vec!["filesystem.read".into()];
        Ok(self
            .gateway
            .execute(request, self.filesystem_executor.as_ref())
            .await
            .map(|result| result.bytes)?)
    }

    /// Read one local run-input candidate through a dedicated 16 MiB policy ceiling.
    pub async fn read_run_input_file_bytes(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<Vec<u8>, RuntimeError> {
        let path = workspace_absolute_path(&self.workspace, path.as_ref());
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            RUN_INPUT_FILE_READ_ACTION,
            path.display().to_string(),
            json!({"path": path.display().to_string(), "encoding": "binary"}),
        );
        request.capabilities = vec![RUN_INPUT_FILE_READ_ACTION.into()];
        Ok(self
            .gateway
            .execute(request, self.filesystem_executor.as_ref())
            .await
            .map(|result| result.bytes)?)
    }

    /// Write bounded UTF-8 text through policy, approval, and the filesystem adapter.
    pub async fn write_text_file(
        &self,
        path: impl AsRef<Path>,
        text: &str,
    ) -> Result<Value, RuntimeError> {
        let path = workspace_absolute_path(&self.workspace, path.as_ref());
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

    /// Write bounded bytes through policy, approval, and the filesystem adapter.
    pub async fn write_file_bytes(
        &self,
        path: impl AsRef<Path>,
        bytes: &[u8],
    ) -> Result<Value, RuntimeError> {
        let path = workspace_absolute_path(&self.workspace, path.as_ref());
        let mut request = effect_request(
            Actor {
                actor_type: ActorType::User,
                id: "terminal-user".into(),
            },
            "filesystem.write",
            path.display().to_string(),
            json!({"content_base64": BASE64.encode(bytes)}),
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
            let executable = workspace_absolute_path(&self.workspace, executable.as_ref());
            fs::canonicalize(executable)?
        };
        let cwd = fs::canonicalize(workspace_absolute_path(&self.workspace, cwd.as_ref()))?;
        let spec = ProcessSpec {
            cwd,
            args,
            environment,
            stdin_base64: None,
            stdin_completion: None,
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
        let path = workspace_absolute_path(&self.workspace, path.as_ref());
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

fn append_text_attachment(
    rendered: &mut String,
    path: &Path,
    bytes: &[u8],
    max_input_bytes: usize,
) -> Result<(), RuntimeError> {
    let content = std::str::from_utf8(bytes)
        .map_err(|_| RuntimeError::Config("CLI attachments must contain UTF-8 text".into()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| RuntimeError::Config("attachment name is invalid".into()))?;
    let required = "\n\n[Attached file: "
        .len()
        .saturating_add(name.len())
        .saturating_add("]\n".len())
        .saturating_add(content.len())
        .saturating_add("\n[End attached file]".len());
    if rendered.len().saturating_add(required) > max_input_bytes {
        return Err(RuntimeError::Config(
            "combined prompt and attachments exceed 1 MiB".into(),
        ));
    }
    rendered.push_str("\n\n[Attached file: ");
    rendered.push_str(name);
    rendered.push_str("]\n");
    rendered.push_str(content);
    rendered.push_str("\n[End attached file]");
    Ok(())
}

#[cfg(test)]
mod attachment_tests {
    use super::append_text_attachment;
    use std::path::Path;

    #[test]
    fn rendering_uses_only_the_display_name_and_bounded_utf8_content() {
        let mut rendered = "Inspect this".to_owned();
        append_text_attachment(
            &mut rendered,
            Path::new("/private/work/review.md"),
            b"# Review\nready",
            1_048_576,
        )
        .expect("render attachment");
        assert!(rendered.contains("[Attached file: review.md]"));
        assert!(rendered.contains("# Review\nready"));
        assert!(!rendered.contains("/private/work"));
        assert!(
            append_text_attachment(
                &mut String::new(),
                Path::new("binary.bin"),
                &[0xff, 0xfe],
                1_048_576,
            )
            .is_err()
        );
        assert!(
            append_text_attachment(
                &mut String::new(),
                Path::new("large.txt"),
                &vec![b'x'; 1_048_576],
                1_048_576,
            )
            .is_err()
        );
    }
}
