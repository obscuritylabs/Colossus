use super::*;

pub(super) fn default_obligations() -> PolicyObligations {
    PolicyObligations {
        sandbox_backend: "broker".into(),
        sandbox_profile: "offline-default".into(),
        filesystem: Vec::new(),
        protected_filesystem: Vec::new(),
        network_destinations: Vec::new(),
        allowed_environment: Vec::new(),
        allow_sandbox_downgrade: false,
        timeout_ms: 30_000,
        max_output_bytes: 1024 * 1024,
        max_processes: 1,
        max_memory_bytes: 256 * 1024 * 1024,
        max_concurrency: 1,
        required_redactions: Vec::new(),
        require_post_effect: false,
        audit_labels: BTreeMap::new(),
        retention: "standard".into(),
    }
}

/// Offline policy with an explicit action outcome map and deny-by-default behavior.
pub struct BuiltInPolicy {
    revision: String,
    actions: BTreeMap<String, DecisionOutcome>,
    obligations: PolicyObligations,
    action_obligations: BTreeMap<String, PolicyObligations>,
    workspace_development: Option<WorkspaceDevelopmentObligations>,
}

#[derive(Clone)]
struct WorkspaceDevelopmentObligations {
    filesystem: Vec<colossus_contracts::FilesystemGrant>,
    protected_filesystem: Vec<String>,
    allowed_environment: Vec<String>,
}

impl BuiltInPolicy {
    /// Secure offline defaults: only the deterministic echo provider is allowed.
    pub fn offline_default() -> Self {
        Self {
            revision: "builtin/offline-v1".into(),
            actions: BTreeMap::from([("provider.echo".into(), DecisionOutcome::Allow)]),
            obligations: default_obligations(),
            action_obligations: BTreeMap::new(),
            workspace_development: None,
        }
    }

    /// Add or replace an exact action decision.
    pub fn with_action(mut self, action: impl Into<String>, outcome: DecisionOutcome) -> Self {
        self.actions.insert(action.into(), outcome);
        self
    }

    /// Require a post-effect release decision for allowed actions.
    pub fn with_post_effect(mut self, required: bool) -> Self {
        self.obligations.require_post_effect = required;
        self
    }

    /// Add one canonical read-only filesystem root to built-in obligations.
    pub fn with_filesystem_read_root(mut self, root: impl Into<String>) -> Self {
        self = self.with_filesystem_root(root, "read");
        self
    }

    /// Add one canonical filesystem root and known access mode.
    pub fn with_filesystem_root(
        mut self,
        root: impl Into<String>,
        mode: impl Into<String>,
    ) -> Self {
        self.obligations
            .filesystem
            .push(colossus_contracts::FilesystemGrant {
                root: root.into(),
                mode: mode.into(),
            });
        self
    }

    /// Select the sandbox backend/profile and explicit downgrade behavior.
    pub fn with_sandbox(
        mut self,
        backend: impl Into<String>,
        profile: impl Into<String>,
        allow_downgrade: bool,
    ) -> Self {
        self.obligations.sandbox_backend = backend.into();
        self.obligations.sandbox_profile = profile.into();
        self.obligations.allow_sandbox_downgrade = allow_downgrade;
        self
    }

    /// Allow one exact environment variable name inside sandboxed processes.
    pub fn with_environment(mut self, name: impl Into<String>) -> Self {
        self.obligations.allowed_environment.push(name.into());
        self
    }

    /// Add development-only resource obligations for non-workflow users and agents.
    pub fn with_workspace_development(
        mut self,
        filesystem: Vec<colossus_contracts::FilesystemGrant>,
        protected_filesystem: Vec<String>,
        allowed_environment: Vec<String>,
    ) -> Self {
        self.workspace_development = Some(WorkspaceDevelopmentObligations {
            filesystem,
            protected_filesystem,
            allowed_environment,
        });
        self
    }

    /// Allow one canonical HTTP(S) origin for brokered network requests.
    pub fn with_network_destination(mut self, origin: impl Into<String>) -> Self {
        self.obligations.network_destinations.push(origin.into());
        self
    }

    /// Apply bounded process, memory, output, timeout, and concurrency ceilings.
    pub fn with_limits(
        mut self,
        timeout_ms: u64,
        max_output_bytes: u64,
        max_processes: u32,
        max_memory_bytes: u64,
        max_concurrency: u32,
    ) -> Self {
        self.obligations.timeout_ms = timeout_ms;
        self.obligations.max_output_bytes = max_output_bytes;
        self.obligations.max_processes = max_processes;
        self.obligations.max_memory_bytes = max_memory_bytes;
        self.obligations.max_concurrency = max_concurrency;
        self
    }

    /// Restrict one exact action to its own filesystem, environment, and network grants.
    pub fn with_action_restrictions(
        mut self,
        action: impl Into<String>,
        filesystem: Vec<colossus_contracts::FilesystemGrant>,
        allowed_environment: Vec<String>,
        network_destinations: Vec<String>,
    ) -> Self {
        let mut obligations = self.obligations.clone();
        obligations.filesystem = filesystem;
        obligations.allowed_environment = allowed_environment;
        obligations.network_destinations = network_destinations;
        self.action_obligations.insert(action.into(), obligations);
        self
    }

    /// Set one action-specific timeout ceiling while retaining its other obligations.
    pub fn with_action_timeout(mut self, action: impl Into<String>, timeout_ms: u64) -> Self {
        let action = action.into();
        let obligations = self
            .action_obligations
            .entry(action)
            .or_insert_with(|| self.obligations.clone());
        obligations.timeout_ms = timeout_ms;
        self
    }
}

#[async_trait]
impl PolicyDecisionPoint for BuiltInPolicy {
    async fn decide(&self, request: &EffectRequest) -> Result<PolicyDecision, PolicyError> {
        let mut outcome = self
            .actions
            .get(&request.action)
            .copied()
            .unwrap_or_else(|| {
                if request.action.starts_with("openapi.")
                    || request.action.starts_with("github.")
                    || request.action.starts_with("searxng.")
                    || request.action.starts_with("opensearch.")
                    || request.action == "web.search"
                    || request.action == "mcp.call"
                {
                    DecisionOutcome::RequireApproval
                } else {
                    DecisionOutcome::Deny
                }
            });
        if outcome == DecisionOutcome::RequireApproval
            && (request.approval.is_some() || request.phase == EffectPhase::PostEffect)
        {
            outcome = DecisionOutcome::Allow;
        }
        let mut obligations = self
            .action_obligations
            .get(&request.action)
            .cloned()
            .unwrap_or_else(|| self.obligations.clone());
        // `shell.run` always receives these runtime-generated values. They are
        // isolated/sanitized by the trusted tool executor and cannot be supplied
        // by model arguments, so legacy explicit-shell configurations do not need
        // to grant them as ambient caller-controlled environment.
        if request.action == "shell.run" {
            obligations.allowed_environment.extend(
                ["HOME", "PATH", "TEMP", "TMP", "TMPDIR"]
                    .into_iter()
                    .map(str::to_owned),
            );
        }
        if let Some(development) = self.workspace_development.as_ref()
            && inherits_workspace_development(request)
        {
            obligations
                .filesystem
                .extend(development.filesystem.iter().cloned());
            obligations
                .protected_filesystem
                .extend(development.protected_filesystem.iter().cloned());
            obligations
                .allowed_environment
                .extend(development.allowed_environment.iter().cloned());
            obligations.filesystem.sort_by(|left, right| {
                left.root
                    .cmp(&right.root)
                    .then_with(|| left.mode.cmp(&right.mode))
            });
            obligations
                .filesystem
                .dedup_by(|left, right| left.root == right.root && left.mode == right.mode);
            obligations.protected_filesystem.sort();
            obligations.protected_filesystem.dedup();
        }
        obligations.allowed_environment.sort();
        obligations.allowed_environment.dedup();
        if request.action.starts_with("filesystem.")
            || is_process_action(&request.action)
            || matches!(
                request.action.as_str(),
                "provider.openai.responses"
                    | "provider.openai.codex"
                    | "provider.openai.chat"
                    | "provider.models"
            )
            || request.action.starts_with("task.")
            || request.action.starts_with("decision.")
            || request.action.starts_with("plan.")
            || request.action.starts_with("goal.")
            || request.action.starts_with("subagent.")
            || request.action.starts_with("memory.")
            || request.action.starts_with("skill.")
            || request.action.starts_with("research.")
            || request.action.starts_with("integration.")
            || request.action.starts_with("openapi.")
            || request.action.starts_with("github.")
            || request.action.starts_with("searxng.")
            || request.action.starts_with("opensearch.")
            || request.action.starts_with("mcp.")
            || request.action.starts_with("pack.")
            || request.action.starts_with("bundle.")
            || request.action.starts_with("collection.")
            || request.action.starts_with("registry.")
            || matches!(
                request.action.as_str(),
                "network.http" | "web.search" | "audit.export.worm.write"
            )
        {
            obligations.require_post_effect = true;
        }
        Ok(PolicyDecision {
            decision_id: Uuid::now_v7().to_string(),
            policy_revision: self.revision.clone(),
            outcome,
            reason: match outcome {
                DecisionOutcome::Allow => "allowed by explicit built-in rule",
                DecisionOutcome::Deny => "denied by built-in default",
                DecisionOutcome::RequireApproval => "explicit operator approval required",
            }
            .into(),
            obligations,
        })
    }

    async fn doctor(&self) -> Result<Value, PolicyError> {
        Ok(json!({
            "ready": true,
            "kind": "built_in",
            "revision": self.revision,
            "default": "deny"
        }))
    }
}

fn inherits_workspace_development(request: &EffectRequest) -> bool {
    matches!(
        request.actor.actor_type,
        ActorType::User | ActorType::Model | ActorType::Subagent
    ) && request.context.workflow_id.is_none()
        && request.context.workflow_hash.is_none()
}
