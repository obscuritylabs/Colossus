use colossus_cloud::{
    CloudError, CloudResult,
    storage::{
        EntityKey, EntityKind, EntityMutation, EntityOrder, EntityQuery, EntityRecord, conflict,
        decode_page_cursor, encode_page_cursor,
    },
};
use colossus_ports::StoreError;
use diesel::{
    QueryableByName, sql_query,
    sql_types::{Array, BigInt, Bool, Jsonb, Nullable, Text},
};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(diesel::QueryableByName)]
pub(super) struct Row {
    #[diesel(sql_type=Text)]
    pub project_id: String,
    #[diesel(sql_type=Text)]
    pub parent_id: String,
    #[diesel(sql_type=Text)]
    pub id: String,
    #[diesel(sql_type=BigInt)]
    pub revision: i64,
    #[diesel(sql_type=Jsonb)]
    pub record: Value,
    #[diesel(sql_type=Text)]
    pub audit_hash: String,
}
#[derive(diesel::QueryableByName)]
struct VerifiedRow {
    #[diesel(embed)]
    entity: Row,
    #[diesel(embed)]
    audit: Audit,
}
#[derive(diesel::QueryableByName)]
struct PageRow {
    #[diesel(embed)]
    verified: VerifiedRow,
    #[diesel(sql_type=Text)]
    page_time: String,
}

pub(super) fn table(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::User => "cloud_users",
        EntityKind::OidcIdentity => "user_identities",
        EntityKind::LocalCredential => "local_credentials",
        EntityKind::Setting => "control_plane_settings",
        EntityKind::Project => "projects",
        EntityKind::Membership => "project_memberships",
        EntityKind::AuthFlow => "oidc_flows",
        EntityKind::Host => "hosts",
        EntityKind::Node => "runtime_agents",
        EntityKind::Workspace => "workspaces",
        EntityKind::Thread => "conversation_threads",
        EntityKind::ThreadMessage => "conversation_messages",
        EntityKind::SessionMapping => "thread_sources",
        EntityKind::Task => "tasks",
        EntityKind::Command => "commands",
        EntityKind::Run => "run_allocations",
        EntityKind::NodeTask => "node_task_placements",
        EntityKind::Admission => "admission_counters",
        EntityKind::Invitation => "enrollment_invitations",
        EntityKind::Renewal => "certificate_renewals",
    }
}
pub(super) fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Adapter("cloud numeric bound exceeded".into()))
}
pub(super) fn db_error(error: diesel::result::Error) -> StoreError {
    match error {
        diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::ClosedConnection,
            _,
        ) => StoreError::OutcomeUnknown(
            "cloud database commit outcome requires reconciliation".into(),
        ),
        _ => StoreError::Adapter("cloud database operation failed".into()),
    }
}
pub(super) fn digest(value: &Value) -> Result<String, StoreError> {
    crate::canonical::bytes(value).map(|bytes| hex::encode(Sha256::digest(bytes)))
}

pub(super) async fn read(
    conn: &mut AsyncPgConnection,
    key: &EntityKey,
) -> CloudResult<EntityRecord> {
    let table = table(key.kind);
    let verified=sql_query(format!("SELECT r.project_id,r.parent_id,r.id,r.revision,r.record,r.audit_hash,a.actor,a.operation,a.content_digest,a.previous_hash,a.chain_hash FROM {table} r LEFT JOIN cloud_audit a ON a.project_id=r.project_id AND a.entity_kind='{table}' AND a.parent_id=r.parent_id AND a.id=r.id AND a.revision=r.revision WHERE ($1 = r.project_id OR ($1 = '' AND $4)) AND r.parent_id=$2 AND r.id=$3 AND NOT r.deleted"))
        .bind::<Text,_>(&key.project_id).bind::<Text,_>(key.parent_id.as_deref().unwrap_or_default()).bind::<Text,_>(&key.id).bind::<Bool,_>(key.kind==EntityKind::Invitation)
        .get_result::<VerifiedRow>(conn).await.map_err(|error|if matches!(error,diesel::result::Error::NotFound){CloudError::NotFound}else{CloudError::Storage})?;
    verify_audit(key.kind, &verified.entity, &verified.audit)?;
    record(key.kind, verified.entity)
}

pub(super) async fn projects(
    conn: &mut AsyncPgConnection,
    after: Option<&str>,
    limit: usize,
) -> CloudResult<Vec<EntityRecord>> {
    if !(1..=100).contains(&limit) {
        return Err(CloudError::InvalidArgument);
    }
    let rows=sql_query("SELECT r.project_id,r.parent_id,r.id,r.revision,r.record,r.audit_hash,a.actor,a.operation,a.content_digest,a.previous_hash,a.chain_hash FROM projects r LEFT JOIN cloud_audit a ON a.project_id=r.project_id AND a.entity_kind='projects' AND a.parent_id=r.parent_id AND a.id=r.id AND a.revision=r.revision WHERE NOT r.deleted AND ($1::TEXT IS NULL OR r.id>$1) ORDER BY r.id LIMIT $2")
        .bind::<Nullable<Text>,_>(after).bind::<BigInt,_>(limit as i64).load::<VerifiedRow>(conn).await.map_err(|_|CloudError::Storage)?;
    rows.into_iter()
        .map(|row| {
            verify_audit(EntityKind::Project, &row.entity, &row.audit)?;
            record(EntityKind::Project, row.entity)
        })
        .collect()
}

pub(super) async fn identities(
    conn: &mut AsyncPgConnection,
    user: &str,
) -> CloudResult<Vec<EntityRecord>> {
    let mut result = Vec::new();
    for kind in [EntityKind::LocalCredential, EntityKind::OidcIdentity] {
        let table = table(kind);
        let rows=sql_query(format!("SELECT r.project_id,r.parent_id,r.id,r.revision,r.record,r.audit_hash,a.actor,a.operation,a.content_digest,a.previous_hash,a.chain_hash FROM {table} r LEFT JOIN cloud_audit a ON a.project_id=r.project_id AND a.entity_kind='{table}' AND a.parent_id=r.parent_id AND a.id=r.id AND a.revision=r.revision WHERE r.user_id=$1 AND NOT r.deleted ORDER BY r.id LIMIT 16"))
            .bind::<Text,_>(user).load::<VerifiedRow>(conn).await.map_err(|_|CloudError::Storage)?;
        for row in rows {
            verify_audit(kind, &row.entity, &row.audit)?;
            result.push(record(kind, row.entity)?);
        }
    }
    Ok(result)
}

pub(super) async fn accounts(
    conn: &mut AsyncPgConnection,
    users: &[String],
) -> CloudResult<Vec<EntityRecord>> {
    if users.len() > 100 {
        return Err(CloudError::InvalidArgument);
    }
    let rows=sql_query("SELECT r.project_id,r.parent_id,r.id,r.revision,r.record,r.audit_hash,a.actor,a.operation,a.content_digest,a.previous_hash,a.chain_hash FROM cloud_users r LEFT JOIN cloud_audit a ON a.project_id=r.project_id AND a.entity_kind='cloud_users' AND a.parent_id=r.parent_id AND a.id=r.id AND a.revision=r.revision WHERE r.id=ANY($1) AND NOT r.deleted ORDER BY r.id")
        .bind::<Array<Text>,_>(users).load::<VerifiedRow>(conn).await.map_err(|_|CloudError::Storage)?;
    rows.into_iter()
        .map(|row| {
            verify_audit(EntityKind::User, &row.entity, &row.audit)?;
            record(EntityKind::User, row.entity)
        })
        .collect()
}

pub(super) async fn list(
    conn: &mut AsyncPgConnection,
    query: &EntityQuery,
) -> CloudResult<Vec<EntityRecord>> {
    let table = table(query.kind);
    let parent = if query.kind == EntityKind::Task {
        "thread_id"
    } else {
        "parent_id"
    };
    let node = match query.kind {
        EntityKind::Node => "id",
        EntityKind::Workspace
        | EntityKind::Thread
        | EntityKind::Task
        | EntityKind::Command
        | EntityKind::Run
        | EntityKind::NodeTask
        | EntityKind::Admission
        | EntityKind::Invitation
        | EntityKind::Renewal
        | EntityKind::SessionMapping => "node_id",
        _ => "record->>'node_id'",
    };
    let status = if query.kind == EntityKind::User {
        "CASE WHEN active THEN 'active' ELSE 'disabled' END"
    } else if query.kind == EntityKind::Membership {
        "CASE WHEN user_id IS NOT NULL AND jsonb_array_length(COALESCE(permissions,'[]'::JSONB))>0 THEN 'active' ELSE 'removed' END"
    } else if query.kind == EntityKind::Task {
        "status"
    } else if query.kind == EntityKind::Command {
        "CASE WHEN reply IS NULL OR reply='null'::JSONB THEN 'pending' ELSE 'reconciled' END"
    } else {
        "COALESCE(record->>'status','')"
    };
    let archived = if query.kind == EntityKind::Thread {
        "archived"
    } else {
        "COALESCE((record->>'archived')::BOOLEAN,FALSE)"
    };
    let status_filter = if query.kind == EntityKind::User
        && query.status.as_deref() == Some("administrator")
    {
        "($6::TEXT='administrator' AND active AND is_admin)".to_owned()
    } else if query.kind == EntityKind::Command {
        match query.status.as_deref() {
            Some("pending") => {
                "($6::TEXT='pending' AND (reply IS NULL OR reply='null'::JSONB))".to_owned()
            }
            Some("reconciled") => {
                "($6::TEXT='reconciled' AND reply IS NOT NULL AND reply<>'null'::JSONB)".to_owned()
            }
            _ => "$6::TEXT IS NULL".to_owned(),
        }
    } else {
        format!("($6 IS NULL OR {status}=$6)")
    };
    let position = decode_page_cursor(query)?;
    let after_id = if matches!(query.order, EntityOrder::IdAsc | EntityOrder::IdDesc) {
        query.after.as_deref()
    } else {
        position.as_ref().map(|position| position.id.as_str())
    };
    let after_time = position
        .as_ref()
        .map(|position| position.timestamp.as_str());
    let after_parent = position
        .as_ref()
        .map(|position| position.parent_id.as_str());
    let (comparison, order, time_column) = match query.order {
        EntityOrder::IdAsc => (
            "id > $3 AND $9::TEXT IS NULL AND $10::TEXT IS NULL",
            "id ASC,parent_id ASC",
            "domain_created_at",
        ),
        EntityOrder::IdDesc => (
            "id < $3 AND $9::TEXT IS NULL AND $10::TEXT IS NULL",
            "id DESC,parent_id DESC",
            "domain_created_at",
        ),
        EntityOrder::UpdatedDesc => (
            "(domain_updated_at,id,parent_id)<($9::TIMESTAMPTZ,$3,$10)",
            "domain_updated_at DESC,id DESC,parent_id DESC",
            "domain_updated_at",
        ),
        EntityOrder::CreatedAsc => (
            "(domain_created_at,id,parent_id)>($9::TIMESTAMPTZ,$3,$10)",
            "domain_created_at ASC,id ASC,parent_id ASC",
            "domain_created_at",
        ),
        EntityOrder::CreatedDesc => (
            "(domain_created_at,id,parent_id)<($9::TIMESTAMPTZ,$3,$10)",
            "domain_created_at DESC,id DESC,parent_id DESC",
            "domain_created_at",
        ),
    };
    let page_time =
        format!("to_char({time_column} AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')");
    let page = format!(
        "SELECT project_id,parent_id,id,revision,record,audit_hash,{page_time} AS page_time,ROW_NUMBER() OVER (ORDER BY {order}) AS page_ordinal FROM {table} WHERE project_id=$1 AND ($2 IS NULL OR {parent}=$2) AND ($3 IS NULL OR {comparison}) AND NOT deleted AND ($4 IS NULL OR {node}=$4) AND ($5 IS NULL OR strpos(lower(COALESCE(record#>>'{{user,display_name}}',record->>'title',record->>'label',record->>'name',record#>>'{{request,prompt}}','')),lower($5))>0) AND {status_filter} AND ($7 IS NULL OR {archived}=$7) ORDER BY {order} LIMIT $8"
    );
    let sql = format!(
        "SELECT r.*,a.actor,a.operation,a.content_digest,a.previous_hash,a.chain_hash FROM ({page}) r LEFT JOIN cloud_audit a ON a.project_id=r.project_id AND a.entity_kind='{table}' AND a.parent_id=r.parent_id AND a.id=r.id AND a.revision=r.revision ORDER BY r.page_ordinal"
    );
    let rows = sql_query(sql)
        .bind::<Text, _>(&query.project_id)
        .bind::<Nullable<Text>, _>(&query.parent_id)
        .bind::<Nullable<Text>, _>(after_id)
        .bind::<Nullable<Text>, _>(&query.node_id)
        .bind::<Nullable<Text>, _>(&query.query)
        .bind::<Nullable<Text>, _>(&query.status)
        .bind::<Nullable<Bool>, _>(query.archived)
        .bind::<BigInt, _>(query.limit.min(100) as i64)
        .bind::<Nullable<Text>, _>(after_time)
        .bind::<Nullable<Text>, _>(after_parent)
        .load::<PageRow>(conn)
        .await
        .map_err(|_| CloudError::Storage)?;
    let mut result = Vec::with_capacity(rows.len());
    for page in rows {
        let verified = page.verified;
        verify_audit(query.kind, &verified.entity, &verified.audit)?;
        let cursor = encode_page_cursor(
            query,
            &page.page_time,
            &verified.entity.id,
            &verified.entity.parent_id,
        )?;
        let mut value = record(query.kind, verified.entity)?;
        value.page_cursor = Some(cursor);
        result.push(value);
    }
    Ok(result)
}
fn record(kind: EntityKind, row: Row) -> CloudResult<EntityRecord> {
    Ok(EntityRecord {
        key: EntityKey {
            kind,
            project_id: row.project_id,
            parent_id: (!row.parent_id.is_empty()).then_some(row.parent_id),
            id: row.id,
        },
        revision: u64::try_from(row.revision).map_err(|_| CloudError::Storage)?,
        value: row.record,
        page_cursor: None,
    })
}
pub(super) async fn thread_incomplete(
    conn: &mut AsyncPgConnection,
    project: &str,
    thread: &str,
) -> CloudResult<bool> {
    let candidate=sql_query("SELECT r.project_id,r.parent_id,r.id,r.revision,r.record,r.audit_hash,a.actor,a.operation,a.content_digest,a.previous_hash,a.chain_hash FROM (SELECT project_id,parent_id,id,revision,record,audit_hash FROM tasks WHERE project_id=$1 AND thread_id=$2 AND NOT deleted AND (COALESCE((record->>'output_limited')::BOOLEAN,FALSE) OR COALESCE((record->>'history_bounded')::BOOLEAN,FALSE) OR (record->>'subject'='runtime' AND NOT COALESCE((record->>'history_complete')::BOOLEAN,FALSE)) OR COALESCE((record#>>'{snapshot,run,last_sequence}')::BIGINT,0)>COALESCE(last_sequence,0)) ORDER BY id LIMIT 1) r LEFT JOIN cloud_audit a ON a.project_id=r.project_id AND a.entity_kind='tasks' AND a.parent_id=r.parent_id AND a.id=r.id AND a.revision=r.revision")
        .bind::<Text,_>(project).bind::<Text,_>(thread).get_result::<VerifiedRow>(conn).await;
    match candidate {
        Ok(candidate) => {
            verify_audit(EntityKind::Task, &candidate.entity, &candidate.audit)?;
            Ok(true)
        }
        Err(diesel::result::Error::NotFound) => Ok(false),
        Err(_) => Err(CloudError::Storage),
    }
}
pub(super) async fn memberships(
    conn: &mut AsyncPgConnection,
    subject: &str,
) -> CloudResult<Vec<EntityRecord>> {
    let rows=sql_query("SELECT r.project_id,r.parent_id,r.id,r.revision,r.record,r.audit_hash,a.actor,a.operation,a.content_digest,a.previous_hash,a.chain_hash FROM project_memberships r LEFT JOIN cloud_audit a ON a.project_id=r.project_id AND a.entity_kind='project_memberships' AND a.parent_id=r.parent_id AND a.id=r.id AND a.revision=r.revision WHERE r.subject=$1 AND NOT r.deleted ORDER BY r.project_id LIMIT 1024").bind::<Text,_>(subject).load::<VerifiedRow>(conn).await.map_err(|_|CloudError::Storage)?;
    let mut result = Vec::with_capacity(rows.len());
    for verified in rows {
        verify_audit(EntityKind::Membership, &verified.entity, &verified.audit)?;
        result.push(record(EntityKind::Membership, verified.entity)?);
    }
    Ok(result)
}

#[derive(diesel::QueryableByName)]
struct Audit {
    #[diesel(sql_type=Text)]
    actor: String,
    #[diesel(sql_type=Text)]
    operation: String,
    #[diesel(sql_type=Text)]
    content_digest: String,
    #[diesel(sql_type=Text)]
    previous_hash: String,
    #[diesel(sql_type=Text)]
    chain_hash: String,
}
pub(super) fn chain(
    key: &EntityKey,
    revision: u64,
    actor: &str,
    operation: &str,
    content: &str,
    previous: &str,
) -> Result<String, StoreError> {
    digest(&serde_json::json!([
        key, revision, actor, operation, content, previous
    ]))
}

fn verify_audit(kind: EntityKind, row: &Row, audit: &Audit) -> CloudResult<()> {
    // Tombstones are never released even if their mutable deletion flag was altered.
    if audit.operation == "cloud.entity.consumed.v1" {
        return Err(CloudError::Storage);
    }
    let key = EntityKey {
        kind,
        project_id: row.project_id.clone(),
        parent_id: (!row.parent_id.is_empty()).then(|| row.parent_id.clone()),
        id: row.id.clone(),
    };
    let content = digest(&row.record).map_err(|_| CloudError::Storage)?;
    let computed = chain(
        &key,
        row.revision as u64,
        &audit.actor,
        &audit.operation,
        &content,
        &audit.previous_hash,
    )
    .map_err(|_| CloudError::Storage)?;
    if content != audit.content_digest || computed != audit.chain_hash || computed != row.audit_hash
    {
        return Err(CloudError::Storage);
    }
    Ok(())
}

#[derive(diesel::QueryableByName)]
struct Changed {
    #[diesel(sql_type=BigInt)]
    count: i64,
}

pub(super) async fn mutate(
    conn: &mut AsyncPgConnection,
    mutation: EntityMutation,
) -> Result<(), StoreError> {
    mutate_profiled(conn, mutation, &crate::profiling::Profiler::default()).await
}
pub(super) async fn mutate_profiled(
    conn: &mut AsyncPgConnection,
    mutation: EntityMutation,
    profile: &crate::profiling::Profiler,
) -> Result<(), StoreError> {
    let key = &mutation.key;
    let table = table(key.kind);
    let parent = key.parent_id.as_deref().unwrap_or_default();
    let expected = integer(mutation.expected_revision)?;
    let sql_span = profile.span(crate::profiling::Stage::Sql);
    let prior=sql_query(format!("SELECT r.project_id,r.parent_id,r.id,r.revision,r.record,r.audit_hash,a.actor,a.operation,a.content_digest,a.previous_hash,a.chain_hash FROM {table} r LEFT JOIN cloud_audit a ON a.project_id=r.project_id AND a.entity_kind='{table}' AND a.parent_id=r.parent_id AND a.id=r.id AND a.revision=r.revision WHERE r.project_id=$1 AND r.parent_id=$2 AND r.id=$3 FOR UPDATE OF r"))
        .bind::<Text,_>(&key.project_id).bind::<Text,_>(parent).bind::<Text,_>(&key.id).get_result::<VerifiedRow>(conn).await;
    drop(sql_span);
    let cpu_span = profile.span(crate::profiling::Stage::Cpu);
    let (actual, previous) = match prior {
        Ok(verified) => {
            if verified.entity.revision != expected {
                return Err(conflict(
                    key,
                    mutation.expected_revision,
                    verified.entity.revision as u64,
                ));
            }
            verify_audit(key.kind, &verified.entity, &verified.audit)
                .map_err(|_| StoreError::Verification("cloud record audit mismatch".into()))?;
            (verified.entity.revision, verified.entity.audit_hash)
        }
        Err(diesel::result::Error::NotFound) => (0, String::new()),
        Err(error) => return Err(db_error(error)),
    };
    if actual != expected {
        return Err(conflict(key, mutation.expected_revision, actual as u64));
    }
    let revision = actual + 1;
    let created = mutation
        .value
        .get("created_at")
        .or_else(|| mutation.value.pointer("/snapshot/run/created_at"))
        .and_then(Value::as_str);
    let updated = mutation.value.get("updated_at").and_then(Value::as_str);
    let content = digest(&mutation.value)?;
    let hash = chain(
        key,
        revision as u64,
        &mutation.actor,
        &mutation.operation,
        &content,
        &previous,
    )?;
    let change = if actual == 0 {
        format!(
            "INSERT INTO {table}(project_id,parent_id,id,revision,record,audit_hash,domain_created_at,domain_updated_at) SELECT $1,$2,$3,$4,$5,$6,COALESCE(NULLIF($12,'')::TIMESTAMPTZ,clock_timestamp()),COALESCE(NULLIF($13,'')::TIMESTAMPTZ,clock_timestamp()) WHERE $11=0 ON CONFLICT DO NOTHING RETURNING project_id,parent_id,id,revision"
        )
    } else {
        format!(
            "UPDATE {table} SET revision=$4,record=$5,audit_hash=$6,updated_at=clock_timestamp(),domain_created_at=COALESCE(NULLIF($12,'')::TIMESTAMPTZ,domain_created_at),domain_updated_at=COALESCE(NULLIF($13,'')::TIMESTAMPTZ,clock_timestamp()) WHERE project_id=$1 AND parent_id=$2 AND id=$3 AND revision=$11 AND NOT deleted RETURNING project_id,parent_id,id,revision"
        )
    };
    let sql = format!(
        "WITH changed AS({change}),audited AS(INSERT INTO cloud_audit(project_id,entity_kind,parent_id,id,revision,actor,operation,content_digest,previous_hash,chain_hash) SELECT project_id,'{table}',parent_id,id,revision,$7,$8,$9,$10,$6 FROM changed RETURNING project_id,id,revision),hinted AS(INSERT INTO delivery_outbox(project_id,scope_id,event_kind,revision) SELECT project_id,id,$8,revision FROM audited ON CONFLICT(project_id) WHERE delivered_at IS NULL DO NOTHING) SELECT COUNT(*)::BIGINT AS count FROM audited"
    );
    drop(cpu_span);
    let _sql_span = profile.span(crate::profiling::Stage::Sql);
    let changed = sql_query(sql)
        .bind::<Text, _>(&key.project_id)
        .bind::<Text, _>(parent)
        .bind::<Text, _>(&key.id)
        .bind::<BigInt, _>(revision)
        .bind::<Jsonb, _>(&mutation.value)
        .bind::<Text, _>(&hash)
        .bind::<Text, _>(&mutation.actor)
        .bind::<Text, _>(&mutation.operation)
        .bind::<Text, _>(&content)
        .bind::<Text, _>(&previous)
        .bind::<BigInt, _>(expected)
        .bind::<Nullable<Text>, _>(created)
        .bind::<Nullable<Text>, _>(updated)
        .get_result::<Changed>(conn)
        .await
        .map_err(db_error)?;
    if changed.count != 1 {
        return Err(conflict(key, mutation.expected_revision, revision as u64));
    }
    Ok(())
}

pub(super) async fn delete(
    conn: &mut AsyncPgConnection,
    key: &EntityKey,
    expected: u64,
) -> Result<(), StoreError> {
    let current = read(conn, key)
        .await
        .map_err(|_| StoreError::NotFound("cloud resource".into()))?;
    if current.revision != expected {
        return Err(conflict(key, expected, current.revision));
    }
    mutate(
        conn,
        EntityMutation {
            key: key.clone(),
            expected_revision: expected,
            value: current.value,
            actor: "cloud".into(),
            operation: "cloud.entity.consumed.v1".into(),
        },
    )
    .await?;
    sql_query(format!(
        "UPDATE {} SET deleted=TRUE WHERE project_id=$1 AND parent_id=$2 AND id=$3",
        table(key.kind)
    ))
    .bind::<Text, _>(&key.project_id)
    .bind::<Text, _>(key.parent_id.as_deref().unwrap_or_default())
    .bind::<Text, _>(&key.id)
    .execute(conn)
    .await
    .map_err(db_error)?;
    Ok(())
}
