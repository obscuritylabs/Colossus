use crate::rows::{self, Metadata};
use colossus_cloud::{
    CloudError, CloudResult,
    storage::{
        EntityKey, EntityKind, EntityMutation, EntityOrder, EntityQuery, EntityRecord, EntityValue,
        conflict, decode_page_cursor, encode_page_cursor,
    },
};
use colossus_ports::StoreError;
use diesel::{
    pg::Pg,
    sql_query,
    sql_types::{Array, BigInt, Bool, Nullable, Text},
};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde_json::Value;
use sha2::{Digest, Sha256};

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
    let selection = rows::selection(key.kind, "r", "''::TEXT");
    let joins = rows::joins(key.kind, "r");
    let query = sql_query(format!("SELECT {selection} FROM {table} r {joins} WHERE ($1 = r.project_id OR ($1 = '' AND $4)) AND r.parent_id=$2 AND r.id=$3 AND NOT r.deleted LIMIT 1"))
        .into_boxed::<Pg>()
        .bind::<Text,_>(key.project_id.clone()).bind::<Text,_>(key.parent_id.clone().unwrap_or_default())
        .bind::<Text,_>(key.id.clone()).bind::<Bool,_>(key.kind==EntityKind::Invitation);
    rows::load(conn, key.kind, query, None)
        .await
        .map_err(CloudError::from)?
        .pop()
        .map(|row| row.record)
        .ok_or(CloudError::NotFound)
}

pub(super) async fn projects(
    conn: &mut AsyncPgConnection,
    after: Option<&str>,
    limit: usize,
) -> CloudResult<Vec<EntityRecord>> {
    if !(1..=100).contains(&limit) {
        return Err(CloudError::InvalidArgument);
    }
    let selection = rows::selection(EntityKind::Project, "r", "''::TEXT");
    let joins = rows::joins(EntityKind::Project, "r");
    let query = sql_query(format!("SELECT {selection} FROM projects r {joins} WHERE NOT r.deleted AND ($1::TEXT IS NULL OR r.id>$1) ORDER BY r.id LIMIT $2"))
        .into_boxed::<Pg>().bind::<Nullable<Text>,_>(after.map(str::to_owned)).bind::<BigInt,_>(limit as i64);
    Ok(rows::load(conn, EntityKind::Project, query, None)
        .await
        .map_err(CloudError::from)?
        .into_iter()
        .map(|row| row.record)
        .collect())
}

pub(super) async fn identities(
    conn: &mut AsyncPgConnection,
    user: &str,
) -> CloudResult<Vec<EntityRecord>> {
    let mut result = Vec::new();
    for kind in [EntityKind::LocalCredential, EntityKind::OidcIdentity] {
        let table = table(kind);
        let selection = rows::selection(kind, "r", "''::TEXT");
        let joins = rows::joins(kind, "r");
        let query = sql_query(format!("SELECT {selection} FROM {table} r {joins} WHERE r.user_id=$1 AND NOT r.deleted ORDER BY r.id LIMIT 16"))
            .into_boxed::<Pg>().bind::<Text,_>(user.to_owned());
        result.extend(
            rows::load(conn, kind, query, None)
                .await
                .map_err(CloudError::from)?
                .into_iter()
                .map(|row| row.record),
        );
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
    let selection = rows::selection(EntityKind::User, "r", "''::TEXT");
    let joins = rows::joins(EntityKind::User, "r");
    let query = sql_query(format!("SELECT {selection} FROM cloud_users r {joins} WHERE r.id=ANY($1) AND NOT r.deleted ORDER BY r.id"))
        .into_boxed::<Pg>().bind::<Array<Text>,_>(users.to_vec());
    Ok(rows::load(conn, EntityKind::User, query, None)
        .await
        .map_err(CloudError::from)?
        .into_iter()
        .map(|row| row.record)
        .collect())
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
        _ => "NULL::TEXT",
    };
    let status = if query.kind == EntityKind::User {
        "CASE WHEN active THEN 'active' ELSE 'disabled' END"
    } else if query.kind == EntityKind::Membership {
        "CASE WHEN cardinality(permissions)>0 THEN 'active' ELSE 'removed' END"
    } else if query.kind == EntityKind::Task {
        "status"
    } else if query.kind == EntityKind::Command {
        "CASE WHEN reply IS NULL THEN 'pending' ELSE 'reconciled' END"
    } else {
        "''::TEXT"
    };
    let archived = if matches!(query.kind, EntityKind::Thread | EntityKind::Project) {
        "archived"
    } else {
        "FALSE"
    };
    let status_filter =
        if query.kind == EntityKind::User && query.status.as_deref() == Some("administrator") {
            "($6::TEXT='administrator' AND active AND is_admin)".to_owned()
        } else if query.kind == EntityKind::Command {
            match query.status.as_deref() {
                Some("pending") => "($6::TEXT='pending' AND reply IS NULL)".to_owned(),
                Some("reconciled") => "($6::TEXT='reconciled' AND reply IS NOT NULL)".to_owned(),
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
    let search = match query.kind {
        EntityKind::User => "display_name",
        EntityKind::Project => "project_name",
        EntityKind::Host => "host_name",
        EntityKind::Node | EntityKind::Invitation => "label",
        EntityKind::Workspace => "workspace_name",
        EntityKind::Thread => "title",
        _ => "''::TEXT",
    };
    let page = format!(
        "SELECT source.*,{page_time} AS page_time,ROW_NUMBER() OVER (ORDER BY {order}) AS page_ordinal FROM {table} source WHERE project_id=$1 AND ($2::TEXT IS NULL OR {parent}=$2) AND ($3::TEXT IS NULL OR {comparison}) AND NOT deleted AND ($4::TEXT IS NULL OR {node}=$4) AND ($5::TEXT IS NULL OR strpos(lower({search}),lower($5))>0) AND {status_filter} AND ($7 IS NULL OR {archived}=$7) ORDER BY {order} LIMIT $8"
    );
    let selection = rows::selection(query.kind, "r", "r.page_time");
    let joins = rows::joins(query.kind, "r");
    let sql = format!("SELECT {selection} FROM ({page}) r {joins} ORDER BY r.page_ordinal");
    let statement = sql_query(sql)
        .into_boxed::<Pg>()
        .bind::<Text, _>(query.project_id.clone())
        .bind::<Nullable<Text>, _>(query.parent_id.clone())
        .bind::<Nullable<Text>, _>(after_id.map(str::to_owned))
        .bind::<Nullable<Text>, _>(query.node_id.clone())
        .bind::<Nullable<Text>, _>(query.query.clone())
        .bind::<Nullable<Text>, _>(query.status.clone())
        .bind::<Nullable<Bool>, _>(query.archived)
        .bind::<BigInt, _>(query.limit.min(100) as i64)
        .bind::<Nullable<Text>, _>(after_time.map(str::to_owned))
        .bind::<Nullable<Text>, _>(after_parent.map(str::to_owned));
    let rows = rows::load(conn, query.kind, statement, None)
        .await
        .map_err(CloudError::from)?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let mut record = row.record;
        record.page_cursor = Some(encode_page_cursor(
            query,
            &row.page_time,
            &record.key.id,
            record.key.parent_id.as_deref().unwrap_or_default(),
        )?);
        result.push(record);
    }
    Ok(result)
}

pub(super) async fn thread_incomplete(
    conn: &mut AsyncPgConnection,
    project: &str,
    thread: &str,
) -> CloudResult<bool> {
    let selection = rows::selection(EntityKind::Task, "r", "''::TEXT");
    let joins = rows::joins(EntityKind::Task, "r");
    let statement=sql_query(format!("SELECT {selection} FROM tasks r {joins} WHERE r.project_id=$1 AND r.thread_id=$2 AND NOT r.deleted AND (r.output_limited OR r.history_bounded OR (r.subject='runtime' AND NOT r.history_complete) OR r.snapshot_last_sequence>r.last_sequence) ORDER BY r.id LIMIT 1"))
        .into_boxed::<Pg>().bind::<Text,_>(project.to_owned()).bind::<Text,_>(thread.to_owned());
    Ok(!rows::load(conn, EntityKind::Task, statement, None)
        .await
        .map_err(CloudError::from)?
        .is_empty())
}

pub(super) async fn memberships(
    conn: &mut AsyncPgConnection,
    user: &str,
) -> CloudResult<Vec<EntityRecord>> {
    let selection = rows::selection(EntityKind::Membership, "r", "''::TEXT");
    let joins = rows::joins(EntityKind::Membership, "r");
    let statement=sql_query(format!("SELECT {selection} FROM project_memberships r {joins} WHERE r.user_id=$1 AND NOT r.deleted ORDER BY r.project_id LIMIT 1024"))
        .into_boxed::<Pg>().bind::<Text,_>(user.to_owned());
    Ok(rows::load(conn, EntityKind::Membership, statement, None)
        .await
        .map_err(CloudError::from)?
        .into_iter()
        .map(|row| row.record)
        .collect())
}

#[derive(diesel::QueryableByName)]
pub(crate) struct Audit {
    #[diesel(sql_type=Text, column_name=audit_actor)]
    actor: String,
    #[diesel(sql_type=Text, column_name=audit_operation)]
    operation: String,
    #[diesel(sql_type=Text, column_name=audit_content_digest)]
    content_digest: String,
    #[diesel(sql_type=Text, column_name=audit_previous_hash)]
    previous_hash: String,
    #[diesel(sql_type=Text, column_name=audit_chain_hash)]
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

pub(crate) fn verify_audit(
    metadata: &Metadata,
    key: &EntityKey,
    value: &EntityValue,
    audit: &Audit,
) -> CloudResult<()> {
    // Tombstones are never released even if their mutable deletion flag was altered.
    if audit.operation == "cloud.entity.consumed.v1" {
        return Err(CloudError::Storage);
    }
    // Serialization here is canonical audit encoding, not a database row format.
    let audit_value = serde_json::to_value(value).map_err(|_| CloudError::Storage)?;
    let content = digest(&audit_value).map_err(|_| CloudError::Storage)?;
    let computed = chain(
        key,
        metadata.revision()?,
        &audit.actor,
        &audit.operation,
        &content,
        &audit.previous_hash,
    )
    .map_err(|_| CloudError::Storage)?;
    if content != audit.content_digest
        || computed != audit.chain_hash
        || computed != metadata.audit_hash
    {
        return Err(CloudError::Storage);
    }
    Ok(())
}

pub(super) async fn mutate(
    conn: &mut AsyncPgConnection,
    mutation: EntityMutation,
) -> Result<(), StoreError> {
    mutate_profiled(conn, mutation, &crate::profiling::Profiler::default()).await
}
pub(super) async fn mutate_profiled(
    conn: &mut AsyncPgConnection,
    mut mutation: EntityMutation,
    profile: &crate::profiling::Profiler,
) -> Result<(), StoreError> {
    if mutation.value.validate_key(&mutation.key).is_err() {
        return Err(StoreError::Adapter("cloud entity kind mismatch".into()));
    }
    mutation.value.set_revision(
        mutation
            .expected_revision
            .checked_add(1)
            .ok_or_else(|| StoreError::Adapter("cloud revision bound exceeded".into()))?,
    );
    // Canonical audit encoding only; native row bindings are built separately.
    let audit_value = serde_json::to_value(&mutation.value)
        .map_err(|_| StoreError::Adapter("cloud entity encoding failed".into()))?;
    let key = &mutation.key;
    let table = table(key.kind);
    let parent = key.parent_id.as_deref().unwrap_or_default();
    let selection = rows::selection(key.kind, "r", "''::TEXT");
    let joins = rows::joins(key.kind, "r");
    let expected = integer(mutation.expected_revision)?;
    let sql_span = profile.span(crate::profiling::Stage::Sql);
    let statement=sql_query(format!("SELECT {selection} FROM {table} r {joins} WHERE r.project_id=$1 AND r.parent_id=$2 AND r.id=$3 FOR UPDATE OF r"))
        .into_boxed::<Pg>().bind::<Text,_>(key.project_id.clone()).bind::<Text,_>(parent.to_owned()).bind::<Text,_>(key.id.clone());
    let prior = rows::load(
        conn,
        key.kind,
        statement,
        Some((key, mutation.expected_revision)),
    )
    .await?;
    drop(sql_span);
    let cpu_span = profile.span(crate::profiling::Stage::Cpu);
    let (actual, previous) = match prior.into_iter().next() {
        Some(row) => (integer(row.record.revision)?, row.audit_hash),
        None => (0, String::new()),
    };
    if actual != expected {
        return Err(conflict(key, mutation.expected_revision, actual as u64));
    }
    let revision = actual
        .checked_add(1)
        .ok_or_else(|| StoreError::Adapter("cloud revision bound exceeded".into()))?;
    let content = digest(&audit_value)?;
    let hash = chain(
        key,
        revision as u64,
        &mutation.actor,
        &mutation.operation,
        &content,
        &previous,
    )?;
    drop(cpu_span);
    let _sql_span = profile.span(crate::profiling::Stage::Sql);
    rows::write(conn, &mutation, &content, &hash, &previous).await?;
    if let colossus_cloud::storage::EntityValue::User(account) = &mutation.value {
        if account.user.identities.len() > 16 {
            return Err(StoreError::Adapter(
                "cloud login metadata bound exceeded".into(),
            ));
        }
        sql_query("DELETE FROM user_login_metadata WHERE user_id=$1")
            .bind::<Text, _>(&key.id)
            .execute(conn)
            .await
            .map_err(db_error)?;
        for (ordinal, metadata) in account.user.identities.iter().enumerate() {
            sql_query("INSERT INTO user_login_metadata(user_id,ordinal,kind,label,username,issuer,subject) VALUES($1,$2::INTEGER,$3,$4,$5,$6,$7)")
                .bind::<Text,_>(&key.id).bind::<BigInt,_>(ordinal as i64).bind::<Text,_>(&metadata.kind).bind::<Text,_>(&metadata.label)
                .bind::<Nullable<Text>,_>(&metadata.username).bind::<Nullable<Text>,_>(&metadata.issuer).bind::<Nullable<Text>,_>(&metadata.subject)
                .execute(conn).await.map_err(db_error)?;
        }
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
