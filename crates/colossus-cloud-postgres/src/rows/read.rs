//! Decode native Diesel rows and verify the reconstructed domain value.
use super::{
    DomainRow, Query, accounts::*, conversations::*, inventory::*, placements::*,
    settings::SettingsRow,
};
use colossus_cloud::storage::{EntityKey, EntityKind, EntityRecord, conflict};
use colossus_ports::StoreError;
use diesel::{QueryableByName, pg::Pg, sql_types::Text};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

#[derive(diesel::QueryableByName)]
struct VerifiedRow<R> {
    #[diesel(embed)]
    entity: R,
    #[diesel(embed)]
    audit: crate::entities::Audit,
    #[diesel(sql_type = Text)]
    page_time: String,
}

pub(crate) struct Loaded {
    pub record: EntityRecord,
    pub page_time: String,
    pub audit_hash: String,
}

pub(crate) fn selection(kind: EntityKind, alias: &str, page_time: &str) -> String {
    let login = if kind == EntityKind::User {
        "logins.*,"
    } else {
        ""
    };
    format!(
        "{alias}.*,{login}a.actor AS audit_actor,a.operation AS audit_operation,a.content_digest AS audit_content_digest,a.previous_hash AS audit_previous_hash,a.chain_hash AS audit_chain_hash,{page_time} AS page_time"
    )
}

pub(crate) fn joins(kind: EntityKind, alias: &str) -> String {
    let table = crate::entities::table(kind);
    let login = if kind == EntityKind::User {
        format!(
            r#"LEFT JOIN LATERAL (
            SELECT COALESCE(array_agg(kind ORDER BY ordinal),ARRAY[]::TEXT[]) AS login_kinds,
                COALESCE(array_agg(label ORDER BY ordinal),ARRAY[]::TEXT[]) AS login_labels,
                COALESCE(array_agg(username ORDER BY ordinal),ARRAY[]::TEXT[]) AS login_usernames,
                COALESCE(array_agg(issuer ORDER BY ordinal),ARRAY[]::TEXT[]) AS login_issuers,
                COALESCE(array_agg(subject ORDER BY ordinal),ARRAY[]::TEXT[]) AS login_subjects
            FROM user_login_metadata WHERE user_id={alias}.id
        ) logins ON TRUE "#
        )
    } else {
        String::new()
    };
    format!(
        "{login}LEFT JOIN cloud_audit a ON a.project_id={alias}.project_id AND a.entity_kind='{table}' AND a.parent_id={alias}.parent_id AND a.id={alias}.id AND a.revision={alias}.revision"
    )
}

async fn load_rows<R: DomainRow>(
    conn: &mut AsyncPgConnection,
    kind: EntityKind,
    query: Query,
    expected: Option<(&EntityKey, u64)>,
) -> Result<Vec<Loaded>, StoreError>
where
    VerifiedRow<R>: diesel::QueryableByName<Pg> + Send,
{
    let rows = query
        .load::<VerifiedRow<R>>(conn)
        .await
        .map_err(crate::entities::db_error)?;
    rows.into_iter()
        .map(|row| {
            let metadata = row.entity.metadata().clone();
            let revision = metadata
                .revision()
                .map_err(|_| StoreError::Verification("cloud revision invalid".into()))?;
            if let Some((key, expected)) = expected
                && revision != expected
            {
                return Err(conflict(key, expected, revision));
            }
            let key = metadata.key(kind);
            let value = row
                .entity
                .into_value()
                .map_err(|_| StoreError::Verification("cloud entity columns invalid".into()))?;
            value
                .validate_key(&key)
                .map_err(|_| StoreError::Verification("cloud entity identity invalid".into()))?;
            crate::entities::verify_audit(&metadata, &key, &value, &row.audit)
                .map_err(|_| StoreError::Verification("cloud record audit mismatch".into()))?;
            Ok(Loaded {
                record: EntityRecord {
                    key,
                    revision,
                    value,
                    page_cursor: None,
                },
                audit_hash: metadata.audit_hash,
                page_time: row.page_time,
            })
        })
        .collect()
}

pub(crate) async fn load(
    conn: &mut AsyncPgConnection,
    kind: EntityKind,
    query: Query,
    expected: Option<(&EntityKey, u64)>,
) -> Result<Vec<Loaded>, StoreError> {
    macro_rules! load {
        ($row:ty) => {
            load_rows::<$row>(conn, kind, query, expected).await
        };
    }
    match kind {
        EntityKind::Project => load!(ProjectRow),
        EntityKind::User => load!(UserRow),
        EntityKind::OidcIdentity => load!(IdentityRow),
        EntityKind::LocalCredential => load!(CredentialRow),
        EntityKind::Membership => load!(MembershipRow),
        EntityKind::Host => load!(HostRow),
        EntityKind::Node => load!(NodeRow),
        EntityKind::Workspace => load!(WorkspaceRow),
        EntityKind::Thread => load!(ThreadRow),
        EntityKind::ThreadMessage => load!(MessageRow),
        EntityKind::Task => load!(TaskRow),
        EntityKind::Command => load!(CommandRow),
        EntityKind::Run | EntityKind::NodeTask => load!(TaskReferenceRow),
        EntityKind::SessionMapping => load!(SessionRow),
        EntityKind::Admission => load!(AdmissionRow),
        EntityKind::Invitation => load!(InvitationRow),
        EntityKind::Renewal => load!(RenewalRow),
        EntityKind::Setting => load!(SettingsRow),
        EntityKind::AuthFlow => load!(AuthFlowRow),
    }
}
