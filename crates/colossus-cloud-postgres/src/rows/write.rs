//! Shared CAS/audit/outbox statement with native bindings from each typed row.
use super::{
    DomainRow, Metadata, accounts::*, conversations::*, inventory::*, placements::*,
    settings::SettingsRow,
};
use colossus_cloud::storage::{EntityKind, EntityMutation, conflict};
use colossus_ports::StoreError;
use diesel::{
    pg::Pg,
    sql_query,
    sql_types::{BigInt, Nullable, Text},
};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

#[derive(diesel::QueryableByName)]
struct Changed {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

async fn write_row<R: DomainRow>(
    conn: &mut AsyncPgConnection,
    mutation: &EntityMutation,
    content: &str,
    hash: &str,
    previous: &str,
) -> Result<(), StoreError> {
    let key = &mutation.key;
    let expected = crate::entities::integer(mutation.expected_revision)?;
    let revision = expected
        .checked_add(1)
        .ok_or_else(|| StoreError::Adapter("cloud revision bound exceeded".into()))?;
    let metadata = Metadata {
        project_id: key.project_id.clone(),
        parent_id: key.parent_id.clone().unwrap_or_default(),
        id: key.id.clone(),
        revision,
        audit_hash: hash.into(),
    };
    let row = R::from_value(metadata, &mutation.value)?;
    let table = crate::entities::table(key.kind);
    let names = R::COLUMNS.join(",");
    let inputs = (13..13 + R::COLUMNS.len())
        .map(|parameter| format!("${parameter}"))
        .collect::<Vec<_>>()
        .join(",");
    let assignments = R::COLUMNS
        .iter()
        .enumerate()
        .map(|(index, column)| format!("{column}=${}", index + 13))
        .collect::<Vec<_>>()
        .join(",");
    let change = if expected == 0 {
        format!(
            "INSERT INTO {table}(project_id,parent_id,id,revision,{names},audit_hash,domain_created_at,domain_updated_at) SELECT $1,$2,$3,$4,{inputs},$5,COALESCE(NULLIF($11,'')::TIMESTAMPTZ,clock_timestamp()),COALESCE(NULLIF($12,'')::TIMESTAMPTZ,clock_timestamp()) WHERE $10=0 ON CONFLICT DO NOTHING RETURNING project_id,parent_id,id,revision"
        )
    } else {
        format!(
            "UPDATE {table} SET revision=$4,{assignments},audit_hash=$5,updated_at=clock_timestamp(),domain_created_at=COALESCE(NULLIF($11,'')::TIMESTAMPTZ,domain_created_at),domain_updated_at=COALESCE(NULLIF($12,'')::TIMESTAMPTZ,clock_timestamp()) WHERE project_id=$1 AND parent_id=$2 AND id=$3 AND revision=$10 AND NOT deleted RETURNING project_id,parent_id,id,revision"
        )
    };
    let sql = format!(
        "WITH changed AS({change}),audited AS(INSERT INTO cloud_audit(project_id,entity_kind,parent_id,id,revision,actor,operation,content_digest,previous_hash,chain_hash) SELECT project_id,'{table}',parent_id,id,revision,$6,$7,$8,$9,$5 FROM changed RETURNING project_id,id,revision),hinted AS(INSERT INTO delivery_outbox(project_id,scope_id,event_kind,revision) SELECT project_id,id,$7,revision FROM audited ON CONFLICT(project_id) WHERE delivered_at IS NULL DO NOTHING) SELECT COUNT(*)::BIGINT AS count FROM audited"
    );
    let query = sql_query(sql)
        .into_boxed::<Pg>()
        .bind::<Text, _>(key.project_id.clone())
        .bind::<Text, _>(key.parent_id.clone().unwrap_or_default())
        .bind::<Text, _>(key.id.clone())
        .bind::<BigInt, _>(revision)
        .bind::<Text, _>(hash.to_owned())
        .bind::<Text, _>(mutation.actor.clone())
        .bind::<Text, _>(mutation.operation.clone())
        .bind::<Text, _>(content.to_owned())
        .bind::<Text, _>(previous.to_owned())
        .bind::<BigInt, _>(expected)
        .bind::<Nullable<Text>, _>(mutation.value.created_at().map(str::to_owned))
        .bind::<Nullable<Text>, _>(mutation.value.updated_at().map(str::to_owned));
    let changed = row
        .bind(query)
        .get_result::<Changed>(conn)
        .await
        .map_err(crate::entities::db_error)?;
    if changed.count != 1 {
        return Err(conflict(key, mutation.expected_revision, revision as u64));
    }
    Ok(())
}

pub(crate) async fn write(
    conn: &mut AsyncPgConnection,
    mutation: &EntityMutation,
    content: &str,
    hash: &str,
    previous: &str,
) -> Result<(), StoreError> {
    macro_rules! write {
        ($row:ty) => {
            write_row::<$row>(conn, mutation, content, hash, previous).await
        };
    }
    match mutation.key.kind {
        EntityKind::Project => write!(ProjectRow),
        EntityKind::User => write!(UserRow),
        EntityKind::OidcIdentity => write!(IdentityRow),
        EntityKind::LocalCredential => write!(CredentialRow),
        EntityKind::Membership => write!(MembershipRow),
        EntityKind::Host => write!(HostRow),
        EntityKind::Node => write!(NodeRow),
        EntityKind::Workspace => write!(WorkspaceRow),
        EntityKind::Thread => write!(ThreadRow),
        EntityKind::ThreadMessage => write!(MessageRow),
        EntityKind::Task => write!(TaskRow),
        EntityKind::Command => write!(CommandRow),
        EntityKind::Run | EntityKind::NodeTask => write!(TaskReferenceRow),
        EntityKind::SessionMapping => write!(SessionRow),
        EntityKind::Admission => write!(AdmissionRow),
        EntityKind::Invitation => write!(InvitationRow),
        EntityKind::Renewal => write!(RenewalRow),
        EntityKind::Setting => write!(SettingsRow),
        EntityKind::AuthFlow => write!(AuthFlowRow),
    }
}
