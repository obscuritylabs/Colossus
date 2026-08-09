use super::*;

impl Runtime {
    /// Authoritative event journal for bounded audit views.
    pub fn journal(&self) -> Arc<dyn EventJournal> {
        Arc::clone(&self.journal)
    }

    /// Read bounded ciphertext-free evidence from the authoritative journal.
    pub fn audit_evidence(
        &self,
        from: u64,
        limit: usize,
    ) -> Result<Vec<AuditEvidence>, RuntimeError> {
        self.journal
            .read_global(from.max(1), limit.clamp(1, 10_000))
            .map(|events| events.iter().map(evidence).collect())
            .map_err(Into::into)
    }

    /// Durable audit-export consumer readiness and retry state.
    pub fn audit_export_status(&self) -> Result<AuditExportStatus, RuntimeError> {
        self.audit_exports.status().map_err(Into::into)
    }

    /// Drain configured external audit evidence work.
    pub async fn drain_audit_exports(&self) -> Result<AuditExportReport, RuntimeError> {
        self.audit_exports
            .drain(256, 16_384)
            .await
            .map_err(Into::into)
    }

    /// Operator-authorized replay of all configured audit evidence.
    pub fn reset_audit_exports(&self) -> Result<AuditExportStatus, RuntimeError> {
        self.audit_exports.reset().map_err(Into::into)
    }

    /// List recent metadata-only run telemetry.
    pub fn telemetry_runs(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RunTelemetrySummary>, RuntimeError> {
        self.telemetry
            .list_runs(session_id, limit)
            .map_err(Into::into)
    }

    /// Inspect a full or uniquely prefixed run without exposing event payloads.
    pub fn telemetry_run(
        &self,
        id_or_prefix: &str,
        limit: usize,
    ) -> Result<RunTelemetryDetail, RuntimeError> {
        self.telemetry
            .get_run(id_or_prefix, limit)
            .map_err(Into::into)
    }

    /// Aggregate metadata-only counters over recent runs.
    pub fn telemetry_metrics(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<TelemetryMetrics, RuntimeError> {
        self.telemetry
            .metrics(session_id, limit)
            .map_err(Into::into)
    }

    /// List selected declarative skills in deterministic precedence order.
    pub fn list_skills(&self) -> Result<Vec<SkillRecord>, RuntimeError> {
        self.skills.list_skills().map_err(Into::into)
    }

    /// Load one selected declarative skill.
    pub fn get_skill(&self, name: &str) -> Result<Option<SkillRecord>, RuntimeError> {
        self.skills.get_skill(name).map_err(Into::into)
    }

    /// Report duplicate skills and the configured winner.
    pub fn skill_duplicates(&self) -> Result<Vec<SkillDuplicate>, RuntimeError> {
        self.skills.duplicate_names().map_err(Into::into)
    }

    /// Preview deterministic skill composition without executing a model turn.
    pub fn compose_skills(
        &self,
        instructions: &str,
        prompt: &str,
        explicit: &[String],
        sticky: &[String],
    ) -> Result<SkillComposition, RuntimeError> {
        self.skill_composer
            .compose(
                instructions,
                prompt,
                explicit,
                sticky,
                self.skills_enabled,
                &self.tools.list_specs(),
            )
            .map_err(Into::into)
    }

    pub(super) async fn execute_skill_operation(
        &self,
        operation: SkillOperation,
    ) -> Result<Value, RuntimeError> {
        let active_skills = match &operation {
            SkillOperation::ListResources { active_skills, .. }
            | SkillOperation::ReadResource { active_skills, .. } => active_skills.clone(),
            _ => Vec::new(),
        };
        let mut request = effect_request(
            terminal_actor(),
            operation.action(),
            operation.resource(),
            serde_json::to_value(&operation)
                .map_err(|error| RuntimeError::Config(error.to_string()))?,
        );
        request.capabilities = vec![operation.action().into()];
        request.context.skill_ids = active_skills;
        let released = self
            .gateway
            .execute(request, self.skill_executor.as_ref())
            .await?;
        serde_json::from_slice(&released.bytes)
            .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Create a new installed data-only skill through approval and a one-use permit.
    pub async fn scaffold_skill(
        &self,
        name: &str,
        description: &str,
        instructions: &str,
        resource_dirs: &[String],
    ) -> Result<SkillScaffoldResult, RuntimeError> {
        serde_json::from_value(
            self.execute_skill_operation(SkillOperation::Scaffold {
                name: name.into(),
                description: description.into(),
                instructions: instructions.into(),
                resource_dirs: resource_dirs.to_vec(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Inspect metadata and hashes for an installed user skill through policy.
    pub async fn inspect_skill(&self, name: &str) -> Result<SkillInspection, RuntimeError> {
        serde_json::from_value(
            self.execute_skill_operation(SkillOperation::Inspect { name: name.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Read one authorable installed user-skill file through policy.
    pub async fn read_skill_file(
        &self,
        name: &str,
        path: &str,
    ) -> Result<SkillFileRead, RuntimeError> {
        serde_json::from_value(
            self.execute_skill_operation(SkillOperation::ReadFile {
                name: name.into(),
                path: path.into(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Write one installed user-skill file through approval and optimistic concurrency.
    pub async fn write_skill_file(
        &self,
        name: &str,
        path: &str,
        content: &str,
        expected_sha256: Option<&str>,
    ) -> Result<SkillWriteResult, RuntimeError> {
        serde_json::from_value(
            self.execute_skill_operation(SkillOperation::WriteFile {
                name: name.into(),
                path: path.into(),
                content: content.into(),
                expected_sha256: expected_sha256.map(Into::into),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Validate an installed user skill through policy.
    pub async fn validate_installed_skill(
        &self,
        name: &str,
    ) -> Result<SkillValidationResult, RuntimeError> {
        serde_json::from_value(
            self.execute_skill_operation(SkillOperation::ValidateInstalled { name: name.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Validate a workspace-local skill directory through policy.
    pub async fn validate_local_skill(
        &self,
        path: &str,
    ) -> Result<SkillValidationResult, RuntimeError> {
        serde_json::from_value(
            self.execute_skill_operation(SkillOperation::ValidateLocal { path: path.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Install a validated workspace-local skill through approval and a one-use permit.
    pub async fn install_local_skill(
        &self,
        path: &str,
    ) -> Result<SkillInstallResult, RuntimeError> {
        serde_json::from_value(
            self.execute_skill_operation(SkillOperation::InstallLocal { path: path.into() })
                .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// List resources for an explicitly active skill through the permission boundary.
    pub async fn skill_resources(
        &self,
        skill_name: &str,
        active_skills: &[String],
    ) -> Result<Vec<SkillResourceEntry>, RuntimeError> {
        serde_json::from_value(
            self.execute_skill_operation(SkillOperation::ListResources {
                skill_name: skill_name.into(),
                active_skills: active_skills.to_vec(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }

    /// Read one bounded text resource for an explicitly active skill through policy.
    pub async fn read_skill_resource(
        &self,
        skill_name: &str,
        path: &str,
        active_skills: &[String],
    ) -> Result<SkillResourceRead, RuntimeError> {
        serde_json::from_value(
            self.execute_skill_operation(SkillOperation::ReadResource {
                skill_name: skill_name.into(),
                path: path.into(),
                active_skills: active_skills.to_vec(),
            })
            .await?,
        )
        .map_err(|error| RuntimeError::Config(error.to_string()))
    }
}
