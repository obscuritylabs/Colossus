use super::entities::{db_error, digest, integer};
use colossus_cloud::{
    CloudError, CloudResult,
    storage::{
        AuthSession, ConnectionLease, CursorMutation, ReleasedEvent, validate_lease_ttl,
        validate_session,
    },
};
use colossus_ports::StoreError;
use diesel::{
    sql_query,
    sql_types::{BigInt, Jsonb, Text},
};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde_json::Value;

#[derive(diesel::QueryableByName)]
struct Sequence {
    #[diesel(sql_type=BigInt)]
    sequence: i64,
}
#[derive(diesel::QueryableByName)]
struct EventRow {
    #[diesel(sql_type=BigInt)]
    sequence: i64,
    #[diesel(sql_type=Jsonb)]
    record: Value,
    #[diesel(sql_type=Text)]
    digest: String,
    #[diesel(sql_type=Text)]
    previous_hash: String,
    #[diesel(sql_type=Text)]
    chain_hash: String,
}
#[derive(diesel::QueryableByName)]
struct EventHead {
    #[diesel(sql_type=BigInt)]
    sequence: i64,
    #[diesel(sql_type=Text)]
    last_hash: String,
}
fn event_hash(
    project: &str,
    scope: &str,
    sequence: u64,
    content: &str,
    previous: &str,
) -> Result<String, StoreError> {
    digest(&serde_json::json!([
        "released_events",
        project,
        scope,
        sequence,
        content,
        previous
    ]))
}
pub(super) async fn append_event(
    conn: &mut AsyncPgConnection,
    event: ReleasedEvent,
    profile: &crate::profiling::Profiler,
) -> Result<(), StoreError> {
    let sql_span = profile.span(crate::profiling::Stage::Sql);
    let head_row=sql_query("INSERT INTO released_event_heads(project_id,scope_id) VALUES($1,$2) ON CONFLICT(project_id,scope_id) DO UPDATE SET last_sequence=released_event_heads.last_sequence RETURNING last_sequence AS sequence,last_hash").bind::<Text,_>(&event.project_id).bind::<Text,_>(&event.scope_id).get_result::<EventHead>(conn).await.map_err(db_error)?;
    drop(sql_span);
    let head = head_row.sequence;
    let sequence = integer(event.sequence)?;
    let hash = {
        let _cpu_span = profile.span(crate::profiling::Stage::Cpu);
        digest(&event.value)?
    };
    if sequence <= head {
        let sql_span = profile.span(crate::profiling::Stage::Sql);
        let existing=sql_query("SELECT sequence,record,digest,previous_hash,chain_hash FROM released_events WHERE project_id=$1 AND scope_id=$2 AND sequence=$3").bind::<Text,_>(&event.project_id).bind::<Text,_>(&event.scope_id).bind::<BigInt,_>(sequence).get_result::<EventRow>(conn).await.map_err(db_error)?;
        drop(sql_span);
        let _cpu_span = profile.span(crate::profiling::Stage::Cpu);
        return if existing.digest == hash
            && existing.record == event.value
            && event_hash(
                &event.project_id,
                &event.scope_id,
                event.sequence,
                &hash,
                &existing.previous_hash,
            )? == existing.chain_hash
        {
            Ok(())
        } else {
            Err(StoreError::Conflict {
                stream_id: event.scope_id,
                expected: event.sequence,
                actual: head as u64,
            })
        };
    }
    if sequence != head + 1 {
        return Err(StoreError::Conflict {
            stream_id: event.scope_id,
            expected: (head + 1) as u64,
            actual: event.sequence,
        });
    }
    let cpu_span = profile.span(crate::profiling::Stage::Cpu);
    let chain_hash = event_hash(
        &event.project_id,
        &event.scope_id,
        event.sequence,
        &hash,
        &head_row.last_hash,
    )?;
    drop(cpu_span);
    let _sql_span = profile.span(crate::profiling::Stage::Sql);
    sql_query("WITH released AS(INSERT INTO released_events(project_id,scope_id,sequence,record,digest,previous_hash,chain_hash) VALUES($1,$2,$3,$4,$5,$6,$7) RETURNING project_id,scope_id,sequence),audited AS(INSERT INTO cloud_audit(project_id,entity_kind,parent_id,id,revision,actor,operation,content_digest,previous_hash,chain_hash) SELECT project_id,'released_events','',scope_id,sequence,'runtime','cloud.output.released.v1',$5,$6,$7 FROM released RETURNING project_id,id,revision),hinted AS(INSERT INTO delivery_outbox(project_id,scope_id,event_kind,revision) SELECT project_id,id,'cloud.output.released.v1',revision FROM audited ON CONFLICT(project_id) WHERE delivered_at IS NULL DO NOTHING) UPDATE released_event_heads SET last_sequence=$3,last_hash=$7 WHERE project_id=$1 AND scope_id=$2")
        .bind::<Text,_>(&event.project_id).bind::<Text,_>(&event.scope_id).bind::<BigInt,_>(sequence).bind::<Jsonb,_>(&event.value).bind::<Text,_>(&hash).bind::<Text,_>(&head_row.last_hash).bind::<Text,_>(&chain_hash).execute(conn).await.map_err(db_error)?;
    Ok(())
}

pub(super) async fn events(
    conn: &mut AsyncPgConnection,
    project: &str,
    scope: &str,
    after: u64,
    limit: usize,
) -> CloudResult<Vec<ReleasedEvent>> {
    let rows=sql_query("SELECT sequence,record,digest,previous_hash,chain_hash FROM released_events WHERE project_id=$1 AND scope_id=$2 AND sequence>$3 ORDER BY sequence LIMIT $4").bind::<Text,_>(project).bind::<Text,_>(scope).bind::<BigInt,_>(integer(after).map_err(|_|CloudError::InvalidArgument)?).bind::<BigInt,_>(limit.min(100)as i64).load::<EventRow>(conn).await.map_err(|_|CloudError::Storage)?;
    let mut result = Vec::with_capacity(rows.len());
    let mut previous: Option<String> = None;
    for (expected, row) in (after + 1..).zip(rows) {
        if row.sequence as u64 != expected
            || previous.as_ref().is_some_and(|p| *p != row.previous_hash)
            || digest(&row.record).map_err(|_| CloudError::Storage)? != row.digest
            || event_hash(
                project,
                scope,
                row.sequence as u64,
                &row.digest,
                &row.previous_hash,
            )
            .map_err(|_| CloudError::Storage)?
                != row.chain_hash
        {
            return Err(CloudError::Storage);
        }
        previous = Some(row.chain_hash);
        result.push(ReleasedEvent {
            project_id: project.into(),
            scope_id: scope.into(),
            sequence: row.sequence as u64,
            value: row.record,
        });
    }
    Ok(result)
}
pub(super) async fn cursor(
    conn: &mut AsyncPgConnection,
    project: &str,
    source: &str,
    scope: &str,
) -> CloudResult<u64> {
    match sql_query(
        "SELECT sequence FROM sync_cursors WHERE project_id=$1 AND source_id=$2 AND scope_id=$3",
    )
    .bind::<Text, _>(project)
    .bind::<Text, _>(source)
    .bind::<Text, _>(scope)
    .get_result::<Sequence>(conn)
    .await
    {
        Ok(row) => Ok(row.sequence as u64),
        Err(diesel::result::Error::NotFound) => Ok(0),
        Err(_) => Err(CloudError::Storage),
    }
}
pub(super) async fn advance_cursor(
    conn: &mut AsyncPgConnection,
    cursor: CursorMutation,
    profile: &crate::profiling::Profiler,
) -> Result<(), StoreError> {
    if cursor.sequence < cursor.expected_sequence {
        return Err(StoreError::Adapter("cloud cursor cannot regress".into()));
    }
    let expected = integer(cursor.expected_sequence)?;
    let sequence = integer(cursor.sequence)?;
    if sequence == expected {
        let actual = self::cursor(
            conn,
            &cursor.project_id,
            &cursor.source_id,
            &cursor.scope_id,
        )
        .await
        .map_err(|_| StoreError::Adapter("cloud cursor unavailable".into()))?;
        return if actual == cursor.expected_sequence {
            Ok(())
        } else {
            Err(StoreError::Conflict {
                stream_id: cursor.scope_id,
                expected: cursor.expected_sequence,
                actual,
            })
        };
    }
    #[derive(diesel::QueryableByName)]
    struct CursorHead {
        #[diesel(sql_type=BigInt)]
        sequence: i64,
        #[diesel(sql_type=Text)]
        previous_hash: String,
    }
    let sql_span = profile.span(crate::profiling::Stage::Sql);
    let head=sql_query("WITH held AS(INSERT INTO sync_cursors(project_id,source_id,scope_id,sequence) VALUES($1,$2,$3,0) ON CONFLICT(project_id,source_id,scope_id) DO UPDATE SET sequence=sync_cursors.sequence RETURNING sequence) SELECT held.sequence,COALESCE(a.chain_hash,'') AS previous_hash FROM held LEFT JOIN cloud_audit a ON a.project_id=$1 AND a.entity_kind='sync_cursors' AND a.parent_id=$2 AND a.id=$3 AND a.revision=held.sequence")
        .bind::<Text,_>(&cursor.project_id).bind::<Text,_>(&cursor.source_id).bind::<Text,_>(&cursor.scope_id).get_result::<CursorHead>(conn).await.map_err(db_error)?;
    drop(sql_span);
    let cpu_span = profile.span(crate::profiling::Stage::Cpu);
    if head.sequence != expected {
        return Err(StoreError::Conflict {
            stream_id: cursor.scope_id,
            expected: cursor.expected_sequence,
            actual: head.sequence as u64,
        });
    }
    if head.sequence > 0 && head.previous_hash.is_empty() {
        return Err(StoreError::Verification(
            "cloud cursor audit is absent".into(),
        ));
    }
    let content = digest(&serde_json::json!([
        cursor.project_id,
        cursor.source_id,
        cursor.scope_id,
        cursor.sequence
    ]))?;
    let hash = digest(&serde_json::json!([
        "sync_cursors",
        cursor.project_id,
        cursor.source_id,
        cursor.scope_id,
        cursor.sequence,
        content,
        head.previous_hash
    ]))?;
    drop(cpu_span);
    let _sql_span = profile.span(crate::profiling::Stage::Sql);
    let changed=sql_query("WITH changed AS(UPDATE sync_cursors SET sequence=$4,updated_at=clock_timestamp() WHERE project_id=$1 AND source_id=$2 AND scope_id=$3 AND sequence=$5 RETURNING project_id,source_id,scope_id,sequence),audited AS(INSERT INTO cloud_audit(project_id,entity_kind,parent_id,id,revision,actor,operation,content_digest,previous_hash,chain_hash) SELECT project_id,'sync_cursors',source_id,scope_id,sequence,'runtime','cloud.sync.advanced.v1',$6,$7,$8 FROM changed RETURNING project_id,id,revision),hinted AS(INSERT INTO delivery_outbox(project_id,scope_id,event_kind,revision) SELECT project_id,id,'cloud.sync.advanced.v1',revision FROM audited ON CONFLICT(project_id) WHERE delivered_at IS NULL DO NOTHING) SELECT COUNT(*)::BIGINT AS sequence FROM changed")
        .bind::<Text,_>(&cursor.project_id).bind::<Text,_>(&cursor.source_id).bind::<Text,_>(&cursor.scope_id).bind::<BigInt,_>(sequence).bind::<BigInt,_>(expected).bind::<Text,_>(&content).bind::<Text,_>(&head.previous_hash).bind::<Text,_>(&hash).get_result::<Sequence>(conn).await.map_err(db_error)?;
    if changed.sequence != 1 {
        return Err(StoreError::Adapter("cloud cursor mutation failed".into()));
    }
    Ok(())
}

#[derive(diesel::QueryableByName)]
struct SessionRow {
    #[diesel(sql_type=BigInt)]
    security_epoch: i64,
    #[diesel(sql_type=Text)]
    session_hash: String,
    #[diesel(sql_type=Text)]
    subject: String,
    #[diesel(sql_type=Text)]
    csrf_hash: String,
    #[diesel(sql_type=BigInt)]
    created_at: i64,
    #[diesel(sql_type=BigInt)]
    expires_at: i64,
}
pub(super) async fn put_session(
    conn: &mut AsyncPgConnection,
    session: AuthSession,
) -> CloudResult<()> {
    validate_session(&session)?;
    sql_query("INSERT INTO browser_sessions(session_hash,subject,csrf_hash,created_at,expires_at,security_epoch) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(session_hash) DO UPDATE SET subject=$2,csrf_hash=$3,created_at=$4,expires_at=$5,security_epoch=$6")
        .bind::<Text,_>(&session.session_hash).bind::<Text,_>(&session.subject).bind::<Text,_>(&session.csrf_hash).bind::<BigInt,_>(integer(session.created_at).map_err(|_|CloudError::InvalidArgument)?).bind::<BigInt,_>(integer(session.expires_at).map_err(|_|CloudError::InvalidArgument)?).bind::<BigInt,_>(integer(session.security_epoch).map_err(|_|CloudError::InvalidArgument)?).execute(conn).await.map_err(|_|CloudError::Storage)?;
    Ok(())
}
pub(super) async fn read_session(
    conn: &mut AsyncPgConnection,
    hash: &str,
    now: u64,
) -> CloudResult<AuthSession> {
    let row=sql_query("SELECT session_hash,subject,csrf_hash,created_at,expires_at,security_epoch FROM browser_sessions WHERE session_hash=$1 AND expires_at>$2").bind::<Text,_>(hash).bind::<BigInt,_>(integer(now).map_err(|_|CloudError::InvalidArgument)?).get_result::<SessionRow>(conn).await.map_err(|e|if matches!(e,diesel::result::Error::NotFound){CloudError::NotFound}else{CloudError::Storage})?;
    Ok(AuthSession {
        session_hash: row.session_hash,
        security_epoch: row.security_epoch as u64,
        subject: row.subject,
        csrf_hash: row.csrf_hash,
        created_at: row.created_at as u64,
        expires_at: row.expires_at as u64,
    })
}
pub(super) async fn delete_session(conn: &mut AsyncPgConnection, hash: &str) -> CloudResult<()> {
    sql_query("DELETE FROM browser_sessions WHERE session_hash=$1")
        .bind::<Text, _>(hash)
        .execute(conn)
        .await
        .map_err(|_| CloudError::Storage)?;
    Ok(())
}

#[derive(diesel::QueryableByName)]
struct LeaseRow {
    #[diesel(sql_type=Text)]
    project_id: String,
    #[diesel(sql_type=Text)]
    node_id: String,
    #[diesel(sql_type=Text)]
    owner_id: String,
    #[diesel(sql_type=BigInt)]
    generation: i64,
    #[diesel(sql_type=BigInt)]
    expires_at: i64,
}
impl From<LeaseRow> for ConnectionLease {
    fn from(row: LeaseRow) -> Self {
        Self {
            project_id: row.project_id,
            node_id: row.node_id,
            owner_id: row.owner_id,
            generation: row.generation as u64,
            expires_at: row.expires_at as u64,
        }
    }
}
pub(super) async fn claim(
    conn: &mut AsyncPgConnection,
    project: &str,
    node: &str,
    owner: &str,
    now: u64,
    ttl: u64,
) -> CloudResult<ConnectionLease> {
    validate_lease_ttl(ttl)?;
    let row=sql_query("INSERT INTO connection_leases(project_id,node_id,owner_id,generation,expires_at) SELECT project_id,id,$3,1,$4 FROM (SELECT project_id,id FROM runtime_agents WHERE project_id=$1 AND parent_id='' AND id=$2 AND NOT revoked AND NOT deleted FOR SHARE) enrolled WHERE TRUE ON CONFLICT(project_id,node_id) DO UPDATE SET owner_id=$3,generation=connection_leases.generation+1,expires_at=$4 WHERE connection_leases.expires_at <= $5 OR connection_leases.owner_id=$3 RETURNING project_id,node_id,owner_id,generation,expires_at")
        .bind::<Text,_>(project).bind::<Text,_>(node).bind::<Text,_>(owner).bind::<BigInt,_>(integer(now.saturating_add(ttl)).map_err(|_|CloudError::InvalidArgument)?).bind::<BigInt,_>(integer(now).map_err(|_|CloudError::InvalidArgument)?).get_result::<LeaseRow>(conn).await;
    let row = match row {
        Ok(row) => row,
        Err(diesel::result::Error::NotFound) => {
            #[derive(diesel::QueryableByName)]
            struct Enrolled {
                #[diesel(sql_type=diesel::sql_types::Bool)]
                live: bool,
            }
            let enrolled=sql_query("SELECT EXISTS(SELECT 1 FROM runtime_agents WHERE project_id=$1 AND parent_id='' AND id=$2 AND NOT revoked AND NOT deleted) AS live").bind::<Text,_>(project).bind::<Text,_>(node).get_result::<Enrolled>(conn).await.map_err(|_|CloudError::Storage)?;
            return Err(if enrolled.live {
                CloudError::Conflict
            } else {
                CloudError::PermissionDenied
            });
        }
        Err(_) => return Err(CloudError::Storage),
    };
    Ok(row.into())
}
pub(super) async fn read_lease(
    conn: &mut AsyncPgConnection,
    project: &str,
    node: &str,
    now: u64,
) -> CloudResult<ConnectionLease> {
    let row=sql_query("SELECT l.project_id,l.node_id,l.owner_id,l.generation,l.expires_at FROM connection_leases l JOIN runtime_agents n ON n.project_id=l.project_id AND n.parent_id='' AND n.id=l.node_id WHERE l.project_id=$1 AND l.node_id=$2 AND l.expires_at>$3 AND NOT n.revoked AND NOT n.deleted").bind::<Text,_>(project).bind::<Text,_>(node).bind::<BigInt,_>(integer(now).map_err(|_|CloudError::InvalidArgument)?).get_result::<LeaseRow>(conn).await.map_err(|e|if matches!(e,diesel::result::Error::NotFound){CloudError::NotFound}else{CloudError::Storage})?;
    Ok(row.into())
}
pub(super) async fn renew(
    conn: &mut AsyncPgConnection,
    lease: &ConnectionLease,
    now: u64,
    ttl: u64,
) -> CloudResult<ConnectionLease> {
    validate_lease_ttl(ttl)?;
    let row=sql_query("UPDATE connection_leases l SET expires_at=$5 FROM runtime_agents n WHERE n.project_id=l.project_id AND n.parent_id='' AND n.id=l.node_id AND NOT n.revoked AND NOT n.deleted AND l.project_id=$1 AND l.node_id=$2 AND l.owner_id=$3 AND l.generation=$4 AND l.expires_at>$6 RETURNING l.project_id,l.node_id,l.owner_id,l.generation,l.expires_at")
        .bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).bind::<Text,_>(&lease.owner_id).bind::<BigInt,_>(integer(lease.generation).map_err(|_|CloudError::InvalidArgument)?).bind::<BigInt,_>(integer(now.saturating_add(ttl)).map_err(|_|CloudError::InvalidArgument)?).bind::<BigInt,_>(integer(now).map_err(|_|CloudError::InvalidArgument)?).get_result::<LeaseRow>(conn).await.map_err(lease_error)?;
    Ok(row.into())
}
pub(super) async fn release(
    conn: &mut AsyncPgConnection,
    lease: &ConnectionLease,
) -> CloudResult<()> {
    sql_query("UPDATE connection_leases SET expires_at=0 WHERE project_id=$1 AND node_id=$2 AND owner_id=$3 AND generation=$4").bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).bind::<Text,_>(&lease.owner_id).bind::<BigInt,_>(integer(lease.generation).map_err(|_|CloudError::InvalidArgument)?).execute(conn).await.map_err(|_|CloudError::Storage)?;
    Ok(())
}
pub(super) async fn verify(
    conn: &mut AsyncPgConnection,
    lease: &ConnectionLease,
    now: Option<u64>,
    locked: bool,
) -> CloudResult<()> {
    let query = if locked {
        "SELECT l.generation AS sequence FROM connection_leases l JOIN runtime_agents n ON n.project_id=l.project_id AND n.parent_id='' AND n.id=l.node_id WHERE l.project_id=$1 AND l.node_id=$2 AND l.owner_id=$3 AND l.generation=$4 AND l.expires_at>EXTRACT(EPOCH FROM clock_timestamp()) AND NOT n.revoked AND NOT n.deleted FOR UPDATE OF l FOR SHARE OF n"
    } else {
        "SELECT l.generation AS sequence FROM connection_leases l JOIN runtime_agents n ON n.project_id=l.project_id AND n.parent_id='' AND n.id=l.node_id WHERE l.project_id=$1 AND l.node_id=$2 AND l.owner_id=$3 AND l.generation=$4 AND l.expires_at>$5 AND NOT n.revoked AND NOT n.deleted"
    };
    if locked {
        sql_query(query)
            .bind::<Text, _>(&lease.project_id)
            .bind::<Text, _>(&lease.node_id)
            .bind::<Text, _>(&lease.owner_id)
            .bind::<BigInt, _>(integer(lease.generation).map_err(|_| CloudError::InvalidArgument)?)
            .get_result::<Sequence>(conn)
            .await
            .map_err(lease_error)?;
    } else {
        sql_query(query)
            .bind::<Text, _>(&lease.project_id)
            .bind::<Text, _>(&lease.node_id)
            .bind::<Text, _>(&lease.owner_id)
            .bind::<BigInt, _>(integer(lease.generation).map_err(|_| CloudError::InvalidArgument)?)
            .bind::<BigInt, _>(
                integer(now.unwrap_or_default()).map_err(|_| CloudError::InvalidArgument)?,
            )
            .get_result::<Sequence>(conn)
            .await
            .map_err(lease_error)?;
    }
    Ok(())
}
fn lease_error(error: diesel::result::Error) -> CloudError {
    if matches!(error, diesel::result::Error::NotFound) {
        CloudError::Conflict
    } else {
        CloudError::Storage
    }
}
