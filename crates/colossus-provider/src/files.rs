use super::*;

const MAX_UPLOAD_RESPONSE_BYTES: usize = 16 * 1024;

impl ProviderExecutor {
    /// Resolve all private bytes before upload and reuse each ID within the request.
    pub(super) async fn upload_files(
        &self,
        request: &ModelRequest,
        media: &mut ProviderResolvedMedia,
        permit: &ExecutionPermit,
        deadline: tokio::time::Instant,
    ) -> Result<(), ProviderError> {
        let references = request
            .messages
            .iter()
            .flat_map(|message| message.content.files())
            .collect::<Vec<_>>();
        if references.is_empty() {
            return Ok(());
        }
        if !matches!(
            self.profile.kind,
            ProviderKind::OpenAiResponses | ProviderKind::OpenAiCompatible
        ) {
            return Err(ProviderError::Configuration(
                "PDF inputs require a Responses or Chat Completions provider".into(),
            ));
        }
        let resolver = self.media.as_ref().ok_or_else(|| {
            ProviderError::Configuration("run-input PDF resolver is unavailable".into())
        })?;
        let mut resolved = BTreeMap::new();
        let mut total = 0_u64;
        if references.len() > 4 {
            return Err(ProviderError::Configuration(
                "provider-visible PDF count exceeds 4".into(),
            ));
        }
        for reference in references {
            total = total
                .checked_add(reference.size_bytes)
                .ok_or_else(|| ProviderError::Configuration("PDF size overflowed".into()))?;
            if reference.size_bytes == 0
                || reference.size_bytes > 16 * 1_048_576
                || total > 32 * 1_048_576
            {
                return Err(ProviderError::Configuration(
                    "provider-visible PDFs exceed their byte bound".into(),
                ));
            }
            let file = tokio::time::timeout_at(deadline, resolver.resolve_file(reference))
                .await
                .map_err(|_| {
                    ProviderError::Transport("PDF resolution exceeded its deadline".into())
                })?
                .map_err(|error| ProviderError::Configuration(error.to_string()))?;
            if &file.reference != reference
                || u64::try_from(file.bytes.len()).unwrap_or(u64::MAX) != reference.size_bytes
            {
                return Err(ProviderError::Configuration(
                    "resolved PDF metadata does not match the request".into(),
                ));
            }
            if let Some(existing) = resolved.insert(reference.artifact_id.clone(), file)
                && &existing.reference != reference
            {
                return Err(ProviderError::Configuration(
                    "PDF artifact has conflicting metadata".into(),
                ));
            }
        }
        for file in resolved.into_values() {
            let reference = file.reference;
            let result =
                tokio::time::timeout_at(deadline, self.upload_file(&reference, file.bytes, permit))
                    .await
                    .map_err(|_| {
                        ProviderError::Transport("PDF upload exceeded its deadline".into())
                    })
                    .and_then(std::convert::identity);
            match result {
                Ok(id) => {
                    media
                        .files
                        .insert(reference.artifact_id.clone(), (reference, id));
                }
                Err(error) => {
                    self.delete_uploaded_files(media, permit).await;
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    async fn upload_file(
        &self,
        reference: &ModelFileReference,
        bytes: Vec<u8>,
        permit: &ExecutionPermit,
    ) -> Result<String, ProviderError> {
        let base =
            self.profile.base_url.as_ref().ok_or_else(|| {
                ProviderError::Configuration("provider has no file endpoint".into())
            })?;
        let url = Url::parse(&format!("{base}/files"))?;
        let client = self.client_for_url(&url, permit, false).await?;
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(reference.file_name.clone())
            .mime_str("application/pdf")?;
        let form = reqwest::multipart::Form::new()
            .text("purpose", "user_data")
            .part("file", part);
        let builder = self.file_request(client.post(url))?;
        // An upload has external state: never retry an ambiguous transport failure.
        let response = builder.multipart(form).send().await?;
        if !response.status().is_success() {
            return Err(ProviderError::Status {
                status: response.status().as_u16(),
                retry_after_ms: None,
            });
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > MAX_UPLOAD_RESPONSE_BYTES {
                return Err(ProviderError::Transport(
                    "file upload response exceeds its bound".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
            ProviderError::Transport("file upload response is not valid JSON".into())
        })?;
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= 256
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            })
            .ok_or_else(|| {
                ProviderError::Transport("file upload response has no valid file ID".into())
            })?;
        Ok(id.to_owned())
    }

    fn file_request(
        &self,
        mut builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        if let Some(reference) = &self.profile.credential_reference {
            let secret = zeroize::Zeroizing::new(self.credentials.resolve(reference)?);
            if secret.is_empty() {
                return Err(ProviderError::Credential(
                    "resolved provider credential is empty".into(),
                ));
            }
            builder = builder.bearer_auth(secret.as_str());
        }
        for (name, value) in colossus_observability::current_trace_headers() {
            builder = builder.header(name, value);
        }
        Ok(builder)
    }

    /// Best-effort bounded cleanup on both successful and failed generations.
    pub(super) async fn delete_uploaded_files(
        &self,
        media: &ProviderResolvedMedia,
        permit: &ExecutionPermit,
    ) {
        let Some(base) = &self.profile.base_url else {
            return;
        };
        let cleanup = async {
            for (_, id) in media.files.values() {
                let Ok(url) = Url::parse(&format!("{base}/files/{id}")) else {
                    continue;
                };
                let Ok(client) = self.client_for_url(&url, permit, false).await else {
                    continue;
                };
                let Ok(builder) = self.file_request(client.delete(url)) else {
                    continue;
                };
                match builder.send().await {
                    Ok(response)
                        if response.status().is_success()
                            || response.status() == reqwest::StatusCode::NOT_FOUND => {}
                    _ => tracing_cleanup_failure(),
                }
            }
        };
        if tokio::time::timeout(
            Duration::from_millis(PROVIDER_STREAM_CLEANUP_RESERVE_MS),
            cleanup,
        )
        .await
        .is_err()
        {
            tracing_cleanup_failure();
        }
    }
}

fn tracing_cleanup_failure() {
    // Do not include remote file IDs, private file metadata, response bodies, or credentials.
    // Cleanup is best-effort and cannot replace the original generation outcome.
    tracing::warn!("provider PDF cleanup did not complete; a remote upload may remain");
}
