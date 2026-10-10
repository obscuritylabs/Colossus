use super::*;
use serde::{Deserialize, Deserializer, Serialize};

/// Browser placement; neither mode permits an automatic fallback to a personal browser.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMode {
    /// Native guest inside Colossus Desktop.
    Embedded,
    /// Unattended browser without a display server.
    Headless,
}

/// Authenticated application scope, derived by the trusted runtime adapter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserScope {
    /// One conversation. This identity does not extend network authority across runs.
    Conversation {
        /// Server-derived conversation identity.
        id: String,
    },
    /// One workflow execution.
    Workflow {
        /// Server-derived workflow execution identity.
        id: String,
    },
}

/// Trusted binding. Model arguments and renderer requests must never nominate these fields.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSessionBinding {
    /// Supervised runtime instance identity.
    pub runtime_id: String,
    /// Canonical workspace identity, rather than its filesystem path.
    pub workspace_id: String,
    /// Authenticated application owner.
    pub application_id: String,
    /// Owning conversation or workflow execution.
    pub scope: BrowserScope,
}

/// In-process actor evidence for a single run. This is not a model tool argument.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserActor {
    /// Trusted application/session binding.
    pub binding: BrowserSessionBinding,
    /// Runtime-derived current run identity; delegated runs need distinct authority.
    pub run_id: String,
}

/// Initial configuration; no profile path, engine executable, private key, or CDP endpoint.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserOpenOptions {
    /// Explicit placement.
    pub mode: BrowserMode,
    /// Exact immutable destinations; native adapters must enforce all browser traffic.
    pub allowed_origins: Vec<BrowserOrigin>,
    /// Optional initial navigation in that envelope.
    #[serde(default)]
    pub initial_url: Option<BrowserUrl>,
    /// Temporary by default; an opaque profile requires separate native owner validation.
    #[serde(default)]
    pub profile: BrowserProfileSelection,
}

/// Closed persistence choice; native filesystem paths and personal profiles are forbidden.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserProfileSelection {
    /// Fresh ephemeral browser state, removed after verified native cleanup.
    #[default]
    Temporary,
    /// Existing explicit profile belonging to this authenticated workspace/application.
    Workspace {
        /// Opaque native-created identifier; never an engine path or directory name.
        id: BrowserProfileId,
    },
}

impl<'de> Deserialize<'de> for BrowserProfileSelection {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Serde's internally tagged unit variant silently accepts extra fields.
        // An empty struct variant enforces the closed temporary-profile shape
        // while preserving the public unit variant used by callers.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum StrictProfileSelection {
            Temporary {},
            Workspace { id: BrowserProfileId },
        }

        match StrictProfileSelection::deserialize(deserializer)? {
            StrictProfileSelection::Temporary {} => Ok(Self::Temporary),
            StrictProfileSelection::Workspace { id } => Ok(Self::Workspace { id }),
        }
    }
}

/// Credential-free profile management metadata; no cache path or stored site data.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserProfileSummary {
    /// Opaque profile, independently checked against native ownership on every use.
    pub id: BrowserProfileId,
    /// Bounded user label, treated as untrusted text.
    pub name: String,
    /// A positively reaped abnormal shutdown requires explicit reset before reuse.
    pub reset_required: bool,
}

/// Engine lifecycle, independent from human/agent control.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLifecycle {
    /// Allocation has not completed.
    Allocating,
    /// The native driver confirmed readiness.
    Ready,
    /// New dispatch is blocked while cleanup runs.
    Closing,
    /// Driver confirmed cleanup.
    Closed,
    /// Host or cleanup evidence is uncertain; dispatch remains blocked.
    Interrupted,
}

/// Writer ownership, independent from engine lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserControlState {
    /// Human browsing; no agent writer exists.
    Human,
    /// A single run holds current control.
    Agent,
    /// Control was explicitly paused or revoked.
    Paused,
    /// Browser engine is unavailable or interrupted.
    Unavailable,
}

/// Bounded operational limits, additionally narrowed by runtime policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserLimits {
    /// Active sessions across this coordinator.
    pub max_sessions: u16,
    /// Aggregate tabs across this coordinator.
    pub max_tabs: u16,
    /// Concurrent in-flight actions across independent sessions.
    pub max_concurrent_actions: u16,
    /// Maximum semantic snapshot nodes.
    pub max_snapshot_nodes: u16,
    /// Maximum serialized observation bytes, never over the model's 64 KiB ceiling.
    pub max_observation_bytes: u32,
    /// Per-action native response deadline.
    pub action_timeout_ms: u32,
    /// Navigation/allocation native response deadline.
    pub navigation_timeout_ms: u32,
    /// Maximum writer-lease duration; authority is never renewed by waiting.
    pub max_lease_ms: u32,
}

impl Default for BrowserLimits {
    fn default() -> Self {
        Self {
            max_sessions: 4,
            max_tabs: 8,
            max_concurrent_actions: 4,
            max_snapshot_nodes: 1024,
            max_observation_bytes: 64 * 1024,
            action_timeout_ms: 30_000,
            navigation_timeout_ms: 60_000,
            max_lease_ms: 30 * 60 * 1000,
        }
    }
}

/// Driver capability discovery is evidence, never an authority grant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserCapabilities {
    /// A verified, usable engine is installed.
    pub available: bool,
    /// Bounded engine version, absent when unavailable.
    pub engine_version: Option<String>,
    /// Placements proved by installed-artifact acceptance.
    pub modes: Vec<BrowserMode>,
    /// Exact supported operations; unsupported operations fail before dispatch.
    pub actions: Vec<BrowserActionKind>,
    /// Effective coordinator/driver limits.
    pub limits: BrowserLimits,
    /// Private CA provisioning has passed native acceptance.
    pub private_ca_trust: bool,
    /// Scoped client-identity provisioning has passed native acceptance.
    pub client_identities: bool,
    /// Restrictive egress containment has passed native acceptance.
    pub restrictive_egress: bool,
}

impl BrowserCapabilities {
    /// Fail-closed default for distributions without a verified browser component.
    pub fn unavailable() -> Self {
        Self {
            available: false,
            engine_version: None,
            modes: Vec::new(),
            actions: Vec::new(),
            limits: BrowserLimits::default(),
            private_ca_trust: false,
            client_identities: false,
            restrictive_egress: false,
        }
    }
}

/// Opaque writer lease checked against server state on every action.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserControlLease {
    /// Opaque lease identity.
    pub lease_id: BrowserControlLeaseId,
    /// Exact owning browser session.
    pub session_id: BrowserSessionId,
    /// Trusted owning run; possession does not authorize another run.
    pub run_id: String,
    /// Advances on every ownership change.
    pub control_generation: u64,
    /// Unix millisecond display metadata; enforcement uses a monotonic deadline.
    pub expires_at_ms: u64,
}

/// Credential-free tab metadata; full sensitive navigation URLs are intentionally absent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserTabSummary {
    /// Owned opaque tab.
    pub tab_id: BrowserTabId,
    /// Current document identity.
    pub document_id: BrowserDocumentId,
    /// Canonical committed origin; native adapter must derive it from engine evidence.
    pub origin: Option<BrowserOrigin>,
    /// Bounded page-controlled title, always untrusted text.
    pub title: String,
}

/// Safe caller-owned session status. No cookies, profile paths, or engine identifiers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSessionSummary {
    /// Opaque session.
    pub session_id: BrowserSessionId,
    /// Confirmed placement.
    pub mode: BrowserMode,
    /// Engine lifecycle.
    pub lifecycle: BrowserLifecycle,
    /// Writer ownership.
    pub control: BrowserControlState,
    /// Current ownership generation, including paused states.
    pub control_generation: u64,
    /// Bounded owned tabs.
    pub tabs: Vec<BrowserTabSummary>,
    /// Selected tab, absent after the last tab closes.
    pub selected_tab_id: Option<BrowserTabId>,
}

/// One sanitized semantic node. Values of password/credential fields must be withheld.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSnapshotNode {
    /// Snapshot-bound opaque reference.
    pub element: BrowserElementRef,
    /// Bounded accessibility role.
    pub role: String,
    /// Bounded, untrusted accessible name.
    pub name: String,
    /// Bounded ordinary value; absent for protected fields.
    pub value: Option<String>,
}

/// Bounded semantic observation. It is not trusted instruction or permission to act.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSnapshot {
    /// Exact issuing snapshot.
    pub snapshot_id: BrowserSnapshotId,
    /// Current document observed.
    pub document_id: BrowserDocumentId,
    /// Bounded sanitized nodes.
    pub nodes: Vec<BrowserSnapshotNode>,
    /// Additional page content was omitted by a node or byte bound.
    pub truncated: bool,
}

/// Native observation staged for runtime post-effect release, never released by the coordinator.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserObservation {
    /// Exact owned browser session.
    pub session_id: BrowserSessionId,
    /// Updated tab metadata with current document identity.
    pub tab: BrowserTabSummary,
    /// Optional semantic inspection result.
    pub snapshot: Option<BrowserSnapshot>,
    /// Output omitted content due to a configured bound.
    pub truncated: bool,
}
