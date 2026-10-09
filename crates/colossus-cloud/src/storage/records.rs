//! Typed cloud persistence values. Database encoding belongs to the adapter.
use super::{EntityKey, EntityKind};
use crate::{
    CloudError, CloudHost, CloudMessage, CloudNode, CloudProject, CloudResult, CloudTask,
    CloudThread, CloudWorkspace, Enrollment, LocalCredential, OidcIdentity, PendingCommand,
    ProjectMembership, UserAccount,
    settings::{ControlPlaneSettings, ProjectPolicyExpectation},
};
use serde::Serialize;
use std::collections::BTreeSet;

/// Bounded per-node admission state, committed with its task allocation.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Admission {
    /// Tasks retaining an admission slot until their outcome is settled.
    pub active: BTreeSet<String>,
}

/// Public certificate renewal receipt used to reconcile an exact retry.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CertificateRenewal {
    /// Native-generated renewal identity.
    pub id: String,
    /// Certificate replaced by this renewal.
    pub previous_fingerprint: String,
    /// Hash of the native certificate request.
    pub csr_sha256: String,
    /// Issued public certificate.
    pub certificate_pem: String,
    /// Fingerprint of the issued certificate.
    pub certificate_sha256: String,
    /// UTC Unix second issuance time.
    pub issued_at: u64,
}

/// One-time cloud account bootstrap completion marker.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapMarker {
    /// Whether the bootstrap transaction completed.
    pub completed: bool,
    /// Version of the bootstrap procedure.
    pub version: u64,
}

/// Domain values accepted by the cloud transaction boundary.
///
/// Serialization is available to adapters for audit hashing and nested payloads;
/// application repositories pass and receive the typed variants directly.
#[derive(Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum EntityValue {
    /// Human account and revocation epoch.
    User(UserAccount),
    /// Verified issuer/subject binding.
    OidcIdentity(OidcIdentity),
    /// Host-only salted password credential.
    LocalCredential(LocalCredential),
    /// Named project and display hierarchy.
    Project(CloudProject),
    /// Explicit project membership.
    Membership(ProjectMembership),
    /// Native host metadata.
    Host(CloudHost),
    /// Enrolled runtime metadata.
    Node(CloudNode),
    /// Shared workspace metadata.
    Workspace(CloudWorkspace),
    /// Conversation metadata and ordered submission queue.
    Thread(CloudThread),
    /// Released conversation message.
    ThreadMessage(CloudMessage),
    /// Fixed-node task metadata and SDK snapshots.
    Task(Box<CloudTask>),
    /// Durable command and reconciled receipt.
    Command(Box<PendingCommand>),
    /// Task/run or session/thread identity mapping.
    Reference(String),
    /// Per-node admission slots.
    Admission(Admission),
    /// Single-use enrollment invitation.
    Invitation(Enrollment),
    /// Certificate rotation receipt.
    Renewal(CertificateRenewal),
    /// Deployment display configuration.
    DisplaySettings(ControlPlaneSettings),
    /// Monitoring baseline with no execution authority.
    PolicyExpectation(ProjectPolicyExpectation),
    /// One-time account configuration marker.
    BootstrapMarker(BootstrapMarker),
    /// Opaque encrypted authorization flows and operational maintenance metadata.
    AuthFlow(serde_json::Value),
}

// Credentials and encrypted authorization envelopes must not appear in diagnostics.
impl std::fmt::Debug for EntityValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EntityValue(..)")
    }
}

/// Conversion between a cloud domain type and the object-safe transaction value.
pub trait EntityData: Clone {
    /// Retain this typed domain value for persistence.
    fn entity_value(&self) -> EntityValue;
    /// Recover the expected domain type, rejecting a mismatched entity variant.
    fn from_entity(value: EntityValue) -> CloudResult<Self>;
}

macro_rules! entity_data {
    (@value $value:ident) => { $value };
    (@value $value:ident boxed) => { *$value };
    ($($variant:ident($ty:ty) $($boxed:ident)?),+ $(,)?) => {$(
        impl EntityData for $ty {
            fn entity_value(&self) -> EntityValue { EntityValue::$variant(self.clone().into()) }
            fn from_entity(value: EntityValue) -> CloudResult<Self> {
                match value {
                    EntityValue::$variant(value) => Ok(entity_data!(@value value $($boxed)?)),
                    _ => Err(CloudError::Storage)
                }
            }
        }
        impl From<$ty> for EntityValue {
            fn from(value: $ty) -> Self { Self::$variant(value.into()) }
        }
        impl TryFrom<EntityValue> for $ty {
            type Error = CloudError;
            fn try_from(value: EntityValue) -> CloudResult<Self> { Self::from_entity(value) }
        }
    )+};
}
entity_data! {
    User(UserAccount), OidcIdentity(OidcIdentity), LocalCredential(LocalCredential),
    Project(CloudProject), Membership(ProjectMembership), Host(CloudHost), Node(CloudNode),
    Workspace(CloudWorkspace), Thread(CloudThread), ThreadMessage(CloudMessage),
    Task(CloudTask) boxed, Command(PendingCommand) boxed, Reference(String), Admission(Admission),
    Invitation(Enrollment), Renewal(CertificateRenewal), DisplaySettings(ControlPlaneSettings),
    PolicyExpectation(ProjectPolicyExpectation), BootstrapMarker(BootstrapMarker)
}

impl EntityValue {
    /// Reject a value whose domain identity disagrees with its transaction key.
    pub fn validate_key(&self, key: &EntityKey) -> CloudResult<()> {
        if !self.matches_kind(key.kind)
            || key.project_id.is_empty()
            || key.id.is_empty()
            || key.parent_id.as_ref().is_some_and(String::is_empty)
        {
            return Err(CloudError::InvalidArgument);
        }
        let root = key.parent_id.is_none();
        let identity = match self {
            Self::User(value) => {
                root && key.project_id == crate::IDENTITY_NAMESPACE
                    && value.user.id == key.id
                    && value.user.identities.len() <= 16
            }
            Self::OidcIdentity(_) | Self::LocalCredential(_) => {
                root && key.project_id == crate::IDENTITY_NAMESPACE
            }
            Self::Project(value) => root && value.id == key.id && value.id == key.project_id,
            Self::Membership(value) => {
                root && value.project_id == key.project_id && value.subject == value.user_id
            }
            Self::Host(value) => {
                root && value.host_id == key.id && value.project_id == key.project_id
            }
            Self::Node(value) => {
                root && value.node_id == key.id && value.project_id == key.project_id
            }
            Self::Workspace(value) => {
                root && value.workspace_id == key.id && value.project_id == key.project_id
            }
            Self::Thread(value) => {
                root && value.thread_id == key.id && value.project_id == key.project_id
            }
            Self::ThreadMessage(value) => {
                value.message_id == key.id
                    && value.project_id == key.project_id
                    && key.parent_id.as_deref() == Some(value.thread_id.as_str())
            }
            Self::Task(value) => {
                root && value.task_id == key.id && value.project_id == key.project_id
            }
            Self::Command(value) => {
                value.command_id == key.id
                    && key.parent_id.as_deref() == Some(value.node_id.as_str())
            }
            Self::Reference(value) => {
                key.parent_id.is_some() && (key.kind != EntityKind::NodeTask || *value == key.id)
            }
            Self::Admission(_) | Self::Renewal(_) | Self::AuthFlow(_) => root,
            Self::Invitation(value) => {
                root && value.token_hash == key.id && value.project_id == key.project_id
            }
            Self::DisplaySettings(_) => {
                root && key.project_id == crate::IDENTITY_NAMESPACE && key.id == "display"
            }
            Self::PolicyExpectation(_) => {
                root && key.project_id != crate::IDENTITY_NAMESPACE
                    && key.id == "policy-expectation"
            }
            Self::BootstrapMarker(_) => {
                root && key.project_id == crate::IDENTITY_NAMESPACE
                    && matches!(
                        key.id.as_str(),
                        "identity-bootstrap-v3" | "administrator-bootstrap-v3"
                    )
            }
        };
        if identity {
            Ok(())
        } else {
            Err(CloudError::InvalidArgument)
        }
    }
    /// Keep the domain's visible revision consistent with the committed aggregate.
    pub fn set_revision(&mut self, revision: u64) {
        match self {
            Self::User(value) => value.user.revision = revision,
            Self::Project(value) => value.revision = revision,
            Self::Membership(value) => value.revision = revision,
            Self::Host(value) => value.revision = revision,
            Self::Node(value) => value.revision = revision,
            Self::Workspace(value) => value.revision = revision,
            Self::Thread(value) => value.revision = revision,
            Self::ThreadMessage(value) => value.revision = revision,
            Self::Task(value) => value.revision = revision,
            Self::Command(value) => value.revision = revision,
            Self::DisplaySettings(value) => value.revision = revision,
            Self::PolicyExpectation(value) => value.revision = revision,
            _ => {}
        }
    }
    /// Whether this value belongs to the selected domain table.
    pub fn matches_kind(&self, kind: EntityKind) -> bool {
        matches!(
            (self, kind),
            (Self::User(_), EntityKind::User)
                | (Self::OidcIdentity(_), EntityKind::OidcIdentity)
                | (Self::LocalCredential(_), EntityKind::LocalCredential)
                | (Self::Project(_), EntityKind::Project)
                | (Self::Membership(_), EntityKind::Membership)
                | (Self::Host(_), EntityKind::Host)
                | (Self::Node(_), EntityKind::Node)
                | (Self::Workspace(_), EntityKind::Workspace)
                | (Self::Thread(_), EntityKind::Thread)
                | (Self::ThreadMessage(_), EntityKind::ThreadMessage)
                | (Self::Task(_), EntityKind::Task)
                | (Self::Command(_), EntityKind::Command)
                | (
                    Self::Reference(_),
                    EntityKind::Run | EntityKind::NodeTask | EntityKind::SessionMapping
                )
                | (Self::Admission(_), EntityKind::Admission)
                | (Self::Invitation(_), EntityKind::Invitation)
                | (Self::Renewal(_), EntityKind::Renewal)
                | (
                    Self::DisplaySettings(_)
                        | Self::PolicyExpectation(_)
                        | Self::BootstrapMarker(_),
                    EntityKind::Setting
                )
                | (Self::AuthFlow(_), EntityKind::AuthFlow)
        )
    }

    /// Borrow an opaque authorization envelope; ordinary domain entities are typed.
    pub fn auth_flow(&self) -> CloudResult<&serde_json::Value> {
        match self {
            Self::AuthFlow(value) => Ok(value),
            _ => Err(CloudError::Storage),
        }
    }

    /// Owning account of a login binding or membership.
    pub fn user_id(&self) -> Option<&str> {
        match self {
            Self::OidcIdentity(value) => Some(&value.user_id),
            Self::LocalCredential(value) => Some(&value.user_id),
            Self::Membership(value) => Some(&value.user_id),
            _ => None,
        }
    }

    /// Runtime identity used by fleet and history queries.
    pub fn node_id(&self) -> Option<&str> {
        match self {
            Self::Node(value) => Some(&value.node_id),
            Self::Workspace(value) => Some(&value.node_id),
            Self::Thread(value) => Some(&value.node_id),
            Self::Task(value) => Some(&value.node_id),
            Self::Command(value) => Some(&value.node_id),
            Self::Invitation(value) => Some(&value.node_id),
            _ => None,
        }
    }

    /// Exact conversation owning a task.
    pub fn thread_id(&self) -> Option<&str> {
        match self {
            Self::Task(value) => value.thread_id.as_deref(),
            Self::ThreadMessage(value) => Some(&value.thread_id),
            _ => None,
        }
    }

    /// Plain display text used for literal collection search.
    pub fn search_text(&self) -> &str {
        match self {
            Self::User(value) => &value.user.display_name,
            Self::Project(value) => &value.name,
            Self::Host(value) => &value.label,
            Self::Node(value) => &value.label,
            Self::Workspace(value) => &value.label,
            Self::Thread(value) => &value.title,
            Self::Invitation(value) => &value.label,
            _ => "",
        }
    }

    /// Domain creation time used for stable collection ordering.
    pub fn created_at(&self) -> Option<&str> {
        match self {
            Self::User(value) => Some(&value.user.created_at),
            Self::Project(value) => Some(&value.created_at),
            Self::Thread(value) => Some(&value.created_at),
            Self::ThreadMessage(value) => Some(&value.created_at),
            Self::Task(value) => Some(&value.created_at),
            _ => None,
        }
    }

    /// Domain update time used for stable collection ordering.
    pub fn updated_at(&self) -> Option<&str> {
        match self {
            Self::User(value) => Some(&value.user.updated_at),
            Self::Project(value) => Some(&value.updated_at),
            Self::Thread(value) => Some(&value.updated_at),
            Self::Task(value) => Some(&value.updated_at),
            _ => None,
        }
    }

    /// Persisted archive preference.
    pub fn archived(&self) -> bool {
        match self {
            Self::Project(value) => value.archived,
            Self::Thread(value) => value.archived,
            _ => false,
        }
    }

    /// Persisted administrator authority.
    pub fn active_administrator(&self) -> bool {
        matches!(self, Self::User(value) if value.user.active && value.user.is_admin)
    }

    /// Whether a task retains incomplete source history.
    pub fn incomplete(&self) -> bool {
        matches!(self, Self::Task(value) if value.output_limited || value.history_bounded
            || (value.subject == "runtime" && !value.history_complete)
            || value.snapshot.as_ref().is_some_and(|snapshot| snapshot.run.last_sequence > value.last_sequence))
    }

    /// User-visible lifecycle state used by collection filters.
    pub fn status(&self) -> &str {
        match self {
            Self::User(value) => {
                if value.user.active {
                    "active"
                } else {
                    "disabled"
                }
            }
            Self::Membership(value) => {
                if value.permissions.is_empty() {
                    "removed"
                } else {
                    "active"
                }
            }
            Self::Command(value) => {
                if value.reply.is_none() {
                    "pending"
                } else {
                    "reconciled"
                }
            }
            Self::Task(value) => {
                if let Some(error) = &value.dispatch_error {
                    return if error.code == colossus_sdk::ApiErrorCode::OutcomeUnknown {
                        "outcome_unknown"
                    } else {
                        "failed"
                    };
                }
                use colossus_sdk::RunStatus;
                match value.snapshot.as_ref().map(|snapshot| snapshot.run.status) {
                    None | Some(RunStatus::Queued) => "queued",
                    Some(RunStatus::Running) => "running",
                    Some(RunStatus::Waiting) => "waiting",
                    Some(RunStatus::Cancelling) => "cancelling",
                    Some(RunStatus::Completed) => "completed",
                    Some(RunStatus::Failed) => "failed",
                    Some(RunStatus::Cancelled) => "cancelled",
                    Some(RunStatus::Interrupted) => "interrupted",
                    Some(RunStatus::OutcomeUnknown) => "outcome_unknown",
                }
            }
            _ => "",
        }
    }
}
