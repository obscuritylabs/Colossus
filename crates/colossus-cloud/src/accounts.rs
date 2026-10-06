use crate::storage::{EntityKey, EntityKind};
use crate::{CloudError, CloudPermission, CloudResult, validate_identifier};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Fixed global authority namespace, separate from project resource authority.
pub const IDENTITY_NAMESPACE: &str = "__identity";

/// Human account metadata. Credentials are retained separately and never released here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloudUser {
    /// Immutable domain identity within its namespace.
    pub id: String,
    /// Bounded human-readable account name.
    pub display_name: String,
    /// Optional display-only contact; never an account-linking proof.
    pub email: Option<String>,
    /// Whether new logins and retained sessions may authenticate.
    pub active: bool,
    /// Global product management and all-project read visibility; execution remains explicit.
    pub is_admin: bool,
    /// Current compare-and-swap revision.
    pub revision: u64,
    /// UTC RFC3339 creation time.
    pub created_at: String,
    /// UTC RFC3339 last-edit time.
    pub updated_at: String,
    #[serde(default)]
    /// Credential-free login binding metadata.
    pub identities: Vec<LoginIdentityMetadata>,
}

/// Safe login binding metadata for account administration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginIdentityMetadata {
    /// Login kind, local or oidc.
    pub kind: String,
    /// Human-readable configured login provider name.
    pub label: String,
    /// Canonical local username, when applicable.
    pub username: Option<String>,
    /// Exact trusted OIDC issuer, when applicable.
    pub issuer: Option<String>,
    /// Verified issuer-bound subject, when applicable.
    pub subject: Option<String>,
}

/// Retained account plus a revocation generation; changing credentials retires old sessions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserAccount {
    /// Credential-free account metadata.
    pub user: CloudUser,
    /// Monotonic generation invalidating previously authenticated sessions.
    pub security_epoch: u64,
}

/// An explicit project role. Hierarchy display never implies inherited membership.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRole {
    /// Read released project data.
    Viewer,
    /// Read, submit tasks and control ordinary task interactions.
    Operator,
    /// Read/control and answer exact native effect approvals.
    Approver,
    /// Configure and operate the project; approval requires a separate explicit grant.
    ProjectAdmin,
}
impl ProjectRole {
    /// Exact permission bundle for a newly assigned built-in role.
    pub fn permissions(self) -> BTreeSet<CloudPermission> {
        use CloudPermission::*;
        match self {
            Self::Viewer => [Read].into(),
            Self::Operator => [Read, Execute, Control].into(),
            Self::Approver => [Read, Control, Approve].into(),
            Self::ProjectAdmin => [Read, Execute, Control, Administer].into(),
        }
    }
}

/// Named project namespace with independently authorized children.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloudProject {
    /// Immutable domain identity within its namespace.
    pub id: String,
    /// Bounded project display name.
    pub name: String,
    #[serde(default)]
    /// Bounded plain-text project description.
    pub description: String,
    /// Display parent; access is independent and never inherited.
    pub parent_project_id: Option<String>,
    /// Retain history while removing cloud execution/control/approval access.
    pub archived: bool,
    /// Current compare-and-swap revision.
    pub revision: u64,
    /// UTC RFC3339 creation time.
    pub created_at: String,
    /// UTC RFC3339 last-edit time.
    pub updated_at: String,
}

/// Persisted project membership with its explicit permission ceiling.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectMembership {
    /// Exact project authority namespace.
    pub project_id: String,
    /// Stable persisted account identity.
    pub user_id: String,
    /// Compatibility correlation field: always equal to the stable user identity.
    pub subject: String,
    /// Server-derived explicit project role.
    pub role: ProjectRole,
    /// Persisted cloud ceiling; intersects the native runtime grant.
    pub permissions: BTreeSet<CloudPermission>,
    /// Current compare-and-swap revision.
    pub revision: u64,
}

/// Verified provider binding. Emails are never used to link accounts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OidcIdentity {
    /// Stable persisted account identity.
    pub user_id: String,
    /// Exact trusted OIDC issuer, when applicable.
    pub issuer: String,
    /// Verified issuer-bound subject, when applicable.
    pub subject: String,
}

/// Native-host-only salted PHC credential. Never expose it in browser responses.
#[derive(Clone, Serialize, Deserialize)]
pub struct LocalCredential {
    /// Stable persisted account identity.
    pub user_id: String,
    /// Canonical local username, when applicable.
    pub username: String,
    /// Salted Argon2id PHC value, retained exclusively by host storage.
    pub password_hash: String,
}

/// Construct a key in the fixed identity authority namespace.
pub fn identity_key(kind: EntityKind, id: &str) -> EntityKey {
    EntityKey {
        kind,
        project_id: IDENTITY_NAMESPACE.into(),
        parent_id: None,
        id: id.into(),
    }
}
/// Construct the canonical project key.
pub fn project_key(id: &str) -> EntityKey {
    EntityKey {
        kind: EntityKind::Project,
        project_id: id.into(),
        parent_id: None,
        id: id.into(),
    }
}
/// Construct one stable account membership key within a project.
pub fn membership_key(project: &str, user: &str) -> EntityKey {
    EntityKey {
        kind: EntityKind::Membership,
        project_id: project.into(),
        parent_id: None,
        id: hash_identity(&[user]),
    }
}
/// Compute a SHA-256 domain identity from length-prefixed components.
pub fn hash_identity(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    hex::encode(hash.finalize())
}
/// Validate and ASCII-fold a bounded canonical local username.
pub fn normalize_username(value: &str) -> CloudResult<String> {
    if !(3..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'@'))
    {
        return Err(CloudError::InvalidArgument);
    }
    Ok(value.to_ascii_lowercase())
}
/// Validate public account metadata before persistence.
pub fn validate_user(user: &CloudUser) -> CloudResult<()> {
    validate_identifier(&user.id)?;
    if user.display_name.trim().is_empty()
        || user.display_name.len() > 256
        || user.display_name.chars().any(char::is_control)
        || user
            .email
            .as_ref()
            .is_some_and(|s| s.len() > 320 || s.chars().any(char::is_control))
    {
        return Err(CloudError::InvalidArgument);
    }
    Ok(())
}
/// Validate project metadata and reserved hierarchy identifiers.
pub fn validate_project(project: &CloudProject) -> CloudResult<()> {
    validate_identifier(&project.id)?;
    if project.id.starts_with("__")
        || project.name.trim().is_empty()
        || project.name.len() > 256
        || project.name.chars().any(char::is_control)
        || project.description.len() > 4096
        || project
            .description
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(CloudError::InvalidArgument);
    }
    if let Some(parent) = &project.parent_project_id {
        validate_identifier(parent)?;
        if parent == &project.id {
            return Err(CloudError::InvalidArgument);
        }
    }
    Ok(())
}
