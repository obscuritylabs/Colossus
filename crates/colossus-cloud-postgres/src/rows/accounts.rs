//! Relational accounts, identity bindings, project hierarchy and memberships.
use super::{DomainRow, Metadata, mismatch, set, signed, unsigned};
use colossus_cloud::{
    CloudError, CloudPermission, CloudProject, CloudResult, CloudUser, LocalCredential,
    LoginIdentityMetadata, OidcIdentity, ProjectMembership, ProjectRole, UserAccount,
    storage::EntityValue,
};
use colossus_ports::StoreError;
use diesel::{
    QueryableByName,
    sql_types::{Array, BigInt, Bool, Nullable, Text},
};

row!(ProjectRow {
    project_name: String => Text,
    description: String => Text,
    parent_project_id: Option<String> => Nullable<Text>,
    archived: bool => Bool,
    created_at_text: String => Text,
    updated_at_text: String => Text,
});
impl DomainRow for ProjectRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Project(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            project_name: value.name.clone(),
            description: value.description.clone(),
            parent_project_id: value.parent_project_id.clone(),
            archived: value.archived,
            created_at_text: value.created_at.clone(),
            updated_at_text: value.updated_at.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(CloudProject {
            revision: self.metadata.revision()?,
            id: self.metadata.id,
            name: self.project_name,
            description: self.description,
            parent_project_id: self.parent_project_id,
            archived: self.archived,
            created_at: self.created_at_text,
            updated_at: self.updated_at_text,
        }
        .into())
    }
}

#[derive(diesel::QueryableByName)]
pub(super) struct LoginColumns {
    #[diesel(sql_type = Array<Text>)]
    login_kinds: Vec<String>,
    #[diesel(sql_type = Array<Text>)]
    login_labels: Vec<String>,
    #[diesel(sql_type = Array<Nullable<Text>>)]
    login_usernames: Vec<Option<String>>,
    #[diesel(sql_type = Array<Nullable<Text>>)]
    login_issuers: Vec<Option<String>>,
    #[diesel(sql_type = Array<Nullable<Text>>)]
    login_subjects: Vec<Option<String>>,
}
impl LoginColumns {
    fn from_metadata(values: &[LoginIdentityMetadata]) -> Self {
        Self {
            login_kinds: values.iter().map(|v| v.kind.clone()).collect(),
            login_labels: values.iter().map(|v| v.label.clone()).collect(),
            login_usernames: values.iter().map(|v| v.username.clone()).collect(),
            login_issuers: values.iter().map(|v| v.issuer.clone()).collect(),
            login_subjects: values.iter().map(|v| v.subject.clone()).collect(),
        }
    }
    fn into_metadata(self) -> CloudResult<Vec<LoginIdentityMetadata>> {
        let count = self.login_kinds.len();
        if count > 16
            || [
                self.login_labels.len(),
                self.login_usernames.len(),
                self.login_issuers.len(),
                self.login_subjects.len(),
            ]
            .into_iter()
            .any(|len| len != count)
        {
            return Err(CloudError::Storage);
        }
        Ok(self
            .login_kinds
            .into_iter()
            .zip(self.login_labels)
            .zip(self.login_usernames)
            .zip(self.login_issuers)
            .zip(self.login_subjects)
            .map(
                |((((kind, label), username), issuer), subject)| LoginIdentityMetadata {
                    kind,
                    label,
                    username,
                    issuer,
                    subject,
                },
            )
            .collect())
    }
}

row!(UserRow {
    display_name: String => Text,
    email: Option<String> => Nullable<Text>,
    active: bool => Bool,
    is_admin: bool => Bool,
    security_epoch: i64 => BigInt,
    created_at_text: String => Text,
    updated_at_text: String => Text,
} read { logins: LoginColumns });
impl DomainRow for UserRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::User(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            display_name: value.user.display_name.clone(),
            email: value.user.email.clone(),
            active: value.user.active,
            is_admin: value.user.is_admin,
            security_epoch: signed(value.security_epoch)?,
            created_at_text: value.user.created_at.clone(),
            updated_at_text: value.user.updated_at.clone(),
            logins: LoginColumns::from_metadata(&value.user.identities),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(UserAccount {
            security_epoch: unsigned(self.security_epoch)?,
            user: CloudUser {
                revision: self.metadata.revision()?,
                id: self.metadata.id,
                display_name: self.display_name,
                email: self.email,
                active: self.active,
                is_admin: self.is_admin,
                created_at: self.created_at_text,
                updated_at: self.updated_at_text,
                identities: self.logins.into_metadata()?,
            },
        }
        .into())
    }
}

row!(IdentityRow { user_id: String => Text, issuer: String => Text, subject: String => Text });
impl DomainRow for IdentityRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::OidcIdentity(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            user_id: value.user_id.clone(),
            issuer: value.issuer.clone(),
            subject: value.subject.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(OidcIdentity {
            user_id: self.user_id,
            issuer: self.issuer,
            subject: self.subject,
        }
        .into())
    }
}

row!(CredentialRow { user_id: String => Text, username: String => Text, password_hash: String => Text });
impl DomainRow for CredentialRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::LocalCredential(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            user_id: value.user_id.clone(),
            username: value.username.clone(),
            password_hash: value.password_hash.clone(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        Ok(LocalCredential {
            user_id: self.user_id,
            username: self.username,
            password_hash: self.password_hash,
        }
        .into())
    }
}

fn role_name(value: ProjectRole) -> &'static str {
    match value {
        ProjectRole::Viewer => "viewer",
        ProjectRole::Operator => "operator",
        ProjectRole::Approver => "approver",
        ProjectRole::ProjectAdmin => "project_admin",
    }
}
fn role(value: &str) -> CloudResult<ProjectRole> {
    match value {
        "viewer" => Ok(ProjectRole::Viewer),
        "operator" => Ok(ProjectRole::Operator),
        "approver" => Ok(ProjectRole::Approver),
        "project_admin" => Ok(ProjectRole::ProjectAdmin),
        _ => Err(CloudError::Storage),
    }
}
fn permission_name(value: CloudPermission) -> &'static str {
    match value {
        CloudPermission::Read => "read",
        CloudPermission::Execute => "execute",
        CloudPermission::Control => "control",
        CloudPermission::Approve => "approve",
        CloudPermission::Administer => "administer",
    }
}
fn permission(value: String) -> CloudResult<CloudPermission> {
    match value.as_str() {
        "read" => Ok(CloudPermission::Read),
        "execute" => Ok(CloudPermission::Execute),
        "control" => Ok(CloudPermission::Control),
        "approve" => Ok(CloudPermission::Approve),
        "administer" => Ok(CloudPermission::Administer),
        _ => Err(CloudError::Storage),
    }
}

row!(MembershipRow { user_id: String => Text, project_role: String => Text, permissions: Vec<String> => Array<Text> });
impl DomainRow for MembershipRow {
    fn from_value(metadata: Metadata, value: &EntityValue) -> Result<Self, StoreError> {
        let EntityValue::Membership(value) = value else {
            return Err(mismatch());
        };
        Ok(Self {
            metadata,
            user_id: value.user_id.clone(),
            project_role: role_name(value.role).into(),
            permissions: value
                .permissions
                .iter()
                .map(|p| permission_name(*p).into())
                .collect(),
        })
    }
    fn into_value(self) -> CloudResult<EntityValue> {
        let permissions = self
            .permissions
            .into_iter()
            .map(permission)
            .collect::<CloudResult<Vec<_>>>()?;
        Ok(ProjectMembership {
            revision: self.metadata.revision()?,
            project_id: self.metadata.project_id,
            subject: self.user_id.clone(),
            user_id: self.user_id,
            role: role(&self.project_role)?,
            permissions: set(permissions)?,
        }
        .into())
    }
}
