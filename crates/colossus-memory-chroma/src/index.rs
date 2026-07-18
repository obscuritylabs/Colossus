use super::*;

/// Disposable Chroma projection whose network effects are individually authorized.
pub struct ChromaMemoryIndex {
    gateway: Arc<EffectGateway>,
    executor: Arc<ChromaExecutor>,
    embedding: Arc<dyn EmbeddingProvider>,
    profile: ChromaProfile,
    position_path: PathBuf,
    state: Mutex<ProjectionState>,
}

impl ChromaMemoryIndex {
    /// Open local projection metadata and bind the remote adapter to the gateway.
    pub fn open(
        gateway: Arc<EffectGateway>,
        executor: Arc<ChromaExecutor>,
        embedding: Arc<dyn EmbeddingProvider>,
        profile: ChromaProfile,
        position_path: impl Into<PathBuf>,
    ) -> Result<Self, StoreError> {
        let position_path = position_path.into();
        let state = read_position(&position_path)?;
        Ok(Self {
            gateway,
            executor,
            embedding,
            profile,
            position_path,
            state: Mutex::new(state),
        })
    }

    fn ensure_known_outcome(&self) -> Result<(), StoreError> {
        if self.state.lock().map_err(adapter)?.outcome_unknown {
            Err(StoreError::OutcomeUnknown(
                "Chroma mutation outcome is unknown; an operator-authorized rebuild is required"
                    .into(),
            ))
        } else {
            Ok(())
        }
    }

    fn mark_outcome_unknown(&self) -> Result<(), StoreError> {
        let mut state = self.state.lock().map_err(adapter)?;
        state.outcome_unknown = true;
        persist_position(&self.position_path, *state)
    }

    fn clear_outcome_unknown(&self) -> Result<(), StoreError> {
        let mut state = self.state.lock().map_err(adapter)?;
        state.outcome_unknown = false;
        persist_position(&self.position_path, *state)
    }

    async fn execute(&self, operation: ChromaOperation) -> Result<Value, StoreError> {
        let action = operation.action();
        let idempotency_id = match &operation {
            ChromaOperation::Upsert { event_id, .. } | ChromaOperation::Remove { event_id, .. } => {
                Some(event_id.clone())
            }
            _ => None,
        };
        let mut request = effect_request(
            system_index_actor(),
            action,
            self.profile.resource(),
            serde_json::to_value(operation).map_err(adapter)?,
        );
        request.capabilities = vec![action.into()];
        request.idempotency_id = idempotency_id;
        request.credential_references = credential_references(self.profile.credential_reference());
        let result = match self.gateway.execute(request, self.executor.as_ref()).await {
            Ok(result) => result,
            Err(GatewayError::OutcomeUnknown(message)) => {
                self.mark_outcome_unknown()?;
                return Err(StoreError::OutcomeUnknown(format!(
                    "Chroma mutation outcome is unknown and automatic retry is blocked: {message}"
                )));
            }
            Err(error) => return Err(adapter(error)),
        };
        serde_json::from_slice(&result.bytes).map_err(adapter)
    }
}

#[async_trait]
impl MemoryIndex for ChromaMemoryIndex {
    fn position(&self) -> Result<u64, StoreError> {
        self.state
            .lock()
            .map(|state| state.position)
            .map_err(adapter)
    }

    async fn set_position(&self, position: u64) -> Result<(), StoreError> {
        let mut state = self.state.lock().map_err(adapter)?;
        state.position = position;
        persist_position(&self.position_path, *state)?;
        Ok(())
    }

    async fn upsert(
        &self,
        event_id: &str,
        memory_id: &str,
        text: &str,
        metadata: &Value,
        embedding: Option<&[f32]>,
    ) -> Result<(), StoreError> {
        self.ensure_known_outcome()?;
        let embedding = match embedding {
            Some(vector) => vector.to_vec(),
            None => self.embedding.embed(text).await?,
        };
        validate_projection_record(event_id, memory_id, text, metadata, &embedding)?;
        self.execute(ChromaOperation::Upsert {
            event_id: event_id.into(),
            memory_id: memory_id.into(),
            text: text.into(),
            metadata: metadata.clone(),
            embedding,
        })
        .await?;
        Ok(())
    }

    async fn remove(&self, event_id: &str, memory_id: &str) -> Result<(), StoreError> {
        self.ensure_known_outcome()?;
        self.execute(ChromaOperation::Remove {
            event_id: event_id.into(),
            memory_id: memory_id.into(),
        })
        .await?;
        Ok(())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<(String, f32)>, StoreError> {
        self.ensure_known_outcome()?;
        let embedding = self.embedding.embed(query).await?;
        let value = self
            .execute(ChromaOperation::Search { embedding, limit })
            .await?;
        serde_json::from_value(
            value
                .get("candidates")
                .cloned()
                .ok_or_else(|| adapter("Chroma candidate output is absent"))?,
        )
        .map_err(adapter)
    }

    async fn status(&self) -> Result<Value, StoreError> {
        if self.state.lock().map_err(adapter)?.outcome_unknown {
            return Ok(json!({
                "ready": false,
                "kind": "chroma",
                "outcome_unknown": true,
                "reason": "operator-authorized rebuild required before retry",
            }));
        }
        match self.execute(ChromaOperation::Status).await {
            Ok(status) => Ok(status),
            Err(error) => Ok(json!({
                "ready": false,
                "kind": "chroma",
                "outcome_unknown": false,
                "reason": error.to_string(),
            })),
        }
    }

    async fn rebuild(&self, records: &[(String, String, Value)]) -> Result<(), StoreError> {
        if records.len() > MAX_REBUILD_RECORDS {
            return Err(adapter("Chroma rebuild exceeds 1000 canonical records"));
        }
        self.execute(ChromaOperation::Reset).await?;
        self.clear_outcome_unknown()?;
        for (id, text, metadata) in records {
            self.upsert(&format!("rebuild:{id}"), id, text, metadata, None)
                .await?;
        }
        Ok(())
    }
}

pub(super) fn read_position(path: &Path) -> Result<ProjectionState, StoreError> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<PositionFile>(&bytes)
            .map_err(adapter)
            .and_then(|value| {
                if value.schema_version == 1 {
                    Ok(ProjectionState {
                        position: value.position,
                        outcome_unknown: value.outcome_unknown,
                    })
                } else {
                    Err(adapter("unsupported Chroma position schema"))
                }
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ProjectionState::default())
        }
        Err(error) => Err(adapter(error)),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PositionFile {
    schema_version: u16,
    position: u64,
    #[serde(default)]
    outcome_unknown: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ProjectionState {
    pub(super) position: u64,
    pub(super) outcome_unknown: bool,
}

pub(super) fn persist_position(path: &Path, state: ProjectionState) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| adapter("Chroma position path has no parent"))?;
    fs::create_dir_all(parent).map_err(adapter)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec(&PositionFile {
        schema_version: 1,
        position: state.position,
        outcome_unknown: state.outcome_unknown,
    })
    .map_err(adapter)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(adapter)?;
    if let Err(error) = (|| -> Result<(), std::io::Error> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })() {
        let _ = fs::remove_file(&temporary);
        return Err(adapter(error));
    }
    Ok(())
}
