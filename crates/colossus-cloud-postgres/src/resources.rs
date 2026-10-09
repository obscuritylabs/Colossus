//! Cross-replica resource requests with exact-lease dispatch and retained audit metadata.
use crate::{
    CloudPostgresStore,
    entities::{digest, integer},
    operational,
};
use colossus_cloud::{
    CloudCaller, CloudError, CloudResult,
    storage::{ConnectionLease, RuntimeResourceRequest, validate_resource_request},
};
use colossus_cloud_protocol::{MAX_RESOURCE_REQUESTS, ResourceOperation, ResourceReply};
use diesel::{
    sql_query,
    sql_types::{BigInt, Bool, Jsonb, Nullable, Text},
};
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde_json::Value;

#[derive(Debug)]
enum TransactionError {
    Database(diesel::result::Error),
    Cloud(CloudError),
}
impl From<diesel::result::Error> for TransactionError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Database(error)
    }
}
impl From<CloudError> for TransactionError {
    fn from(error: CloudError) -> Self {
        Self::Cloud(error)
    }
}
impl TransactionError {
    fn cloud(self) -> CloudError {
        match self {
            Self::Cloud(error) => error,
            Self::Database(_error) => CloudError::Storage,
        }
    }
}
#[derive(diesel::QueryableByName)]
struct Flag {
    #[diesel(sql_type=Bool)]
    present: bool,
}
#[derive(diesel::QueryableByName)]
struct Count {
    #[diesel(sql_type=BigInt)]
    total: i64,
    #[diesel(sql_type=BigInt)]
    node: i64,
}
#[derive(diesel::QueryableByName)]
struct RequestRow {
    #[diesel(sql_type=Text)]
    request_id: String,
    #[diesel(sql_type=Text)]
    actor: String,
    #[diesel(sql_type=Jsonb)]
    operation: Value,
    #[diesel(sql_type=Text)]
    digest: String,
    #[diesel(sql_type=BigInt)]
    created_at: i64,
    #[diesel(sql_type=BigInt)]
    expires_at: i64,
}
#[derive(diesel::QueryableByName)]
struct ReplyRow {
    #[diesel(sql_type=Nullable<Jsonb>)]
    reply: Option<Value>,
    #[diesel(sql_type=Nullable<Text>)]
    reply_digest: Option<String>,
}
fn request_digest(
    lease: &ConnectionLease,
    id: &str,
    actor: &str,
    operation: &Value,
    created: u64,
    expires: u64,
) -> CloudResult<String> {
    digest(&serde_json::json!([
        lease.project_id,
        lease.node_id,
        lease.owner_id,
        lease.generation,
        id,
        actor,
        operation,
        created,
        expires
    ]))
    .map_err(|_| CloudError::Storage)
}
impl CloudPostgresStore {
    pub(super) async fn connect_resources(
        &self,
        lease: &ConnectionLease,
        enabled: bool,
    ) -> CloudResult<()> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        conn.transaction::<_,TransactionError,_>(async |conn| {
            operational::verify(conn,lease,None,true).await?;
            if enabled {
                sql_query("INSERT INTO resource_connections(project_id,node_id,owner_id,generation) VALUES($1,$2,$3,$4) ON CONFLICT(project_id,node_id) DO UPDATE SET owner_id=$3,generation=$4")
                    .bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).bind::<Text,_>(&lease.owner_id).bind::<BigInt,_>(integer(lease.generation).map_err(|_|CloudError::InvalidArgument)?).execute(conn).await?;
            } else {
                sql_query("DELETE FROM resource_connections WHERE project_id=$1 AND node_id=$2")
                    .bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).execute(conn).await?;
            }
            Ok(())
        }).await.map_err(TransactionError::cloud)
    }
    pub(super) async fn submit_resource(
        &self,
        caller: &CloudCaller,
        lease: &ConnectionLease,
        operation: ResourceOperation,
        now: u64,
    ) -> CloudResult<String> {
        validate_resource_request(caller, lease, &operation)?;
        let id = uuid::Uuid::now_v7().simple().to_string();
        let operation = serde_json::to_value(operation).map_err(|_| CloudError::InvalidArgument)?;
        let expires = now.saturating_add(20);
        let content = request_digest(lease, &id, caller.subject(), &operation, now, expires)?;
        let chain = digest(&serde_json::json!([
            "runtime_resource_requests",
            lease.project_id,
            lease.node_id,
            id,
            1,
            caller.subject(),
            "runtime.resource.requested.v1",
            content,
            ""
        ]))
        .map_err(|_| CloudError::Storage)?;
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        conn.transaction::<_,TransactionError,_>(async |conn| {
            // Human management has independent low-rate admission. Never serialize
            // the normal run/event path on this quota lock.
            sql_query("SELECT pg_advisory_xact_lock(hashtext(current_schema()||':runtime-resource-admission'))").execute(conn).await?;
            operational::verify(conn,lease,None,true).await?;
            let capable=sql_query("SELECT EXISTS(SELECT 1 FROM resource_connections WHERE project_id=$1 AND node_id=$2 AND owner_id=$3 AND generation=$4) AS present")
                .bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).bind::<Text,_>(&lease.owner_id).bind::<BigInt,_>(integer(lease.generation).map_err(|_|CloudError::InvalidArgument)?).get_result::<Flag>(conn).await?;
            if !capable.present {return Err(CloudError::Conflict.into());}
            sql_query("WITH expired AS(SELECT request_id FROM runtime_resource_requests WHERE expires_at<$1-60 ORDER BY expires_at,request_id LIMIT 128) DELETE FROM runtime_resource_requests WHERE request_id IN(SELECT request_id FROM expired)").bind::<BigInt,_>(integer(now).map_err(|_|CloudError::InvalidArgument)?).execute(conn).await?;
            let retained=sql_query("SELECT count(*) AS total,count(*) FILTER(WHERE project_id=$1 AND node_id=$2) AS node FROM runtime_resource_requests")
                .bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).get_result::<Count>(conn).await?;
            if retained.total>=4096 || retained.node>=256 {return Err(CloudError::ResourceExhausted.into());}
            let count=sql_query("SELECT count(*) AS total,count(*) FILTER(WHERE project_id=$1 AND node_id=$2) AS node FROM runtime_resource_requests WHERE reply IS NULL AND expires_at>$3")
                .bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).bind::<BigInt,_>(integer(now).map_err(|_|CloudError::InvalidArgument)?).get_result::<Count>(conn).await?;
            if count.total>=128 || count.node>=MAX_RESOURCE_REQUESTS as i64 {return Err(CloudError::ResourceExhausted.into());}
            sql_query("WITH requested AS(INSERT INTO runtime_resource_requests(request_id,project_id,node_id,owner_id,generation,actor,operation,digest,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING request_id) INSERT INTO cloud_audit(project_id,entity_kind,parent_id,id,revision,actor,operation,content_digest,previous_hash,chain_hash) SELECT $2,'runtime_resource_requests',$3,request_id,1,$6,'runtime.resource.requested.v1',$8,'',$11 FROM requested")
                .bind::<Text,_>(&id).bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).bind::<Text,_>(&lease.owner_id).bind::<BigInt,_>(integer(lease.generation).map_err(|_|CloudError::InvalidArgument)?).bind::<Text,_>(caller.subject()).bind::<Jsonb,_>(&operation).bind::<Text,_>(&content).bind::<BigInt,_>(integer(now).map_err(|_|CloudError::InvalidArgument)?).bind::<BigInt,_>(integer(expires).map_err(|_|CloudError::InvalidArgument)?).bind::<Text,_>(&chain).execute(conn).await?;
            Ok(id.clone())
        }).await.map_err(TransactionError::cloud)
    }
    pub(super) async fn take_resources(
        &self,
        lease: &ConnectionLease,
        now: u64,
    ) -> CloudResult<Vec<RuntimeResourceRequest>> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        conn.transaction::<_,TransactionError,_>(async |conn| {
            operational::verify(conn,lease,None,true).await?;
            let rows=sql_query("UPDATE runtime_resource_requests SET dispatched=TRUE WHERE request_id IN(SELECT request_id FROM runtime_resource_requests WHERE project_id=$1 AND node_id=$2 AND owner_id=$3 AND generation=$4 AND expires_at>$5 AND NOT dispatched AND reply IS NULL ORDER BY request_id LIMIT $6 FOR UPDATE) RETURNING request_id,actor,operation,digest,created_at,expires_at")
                .bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).bind::<Text,_>(&lease.owner_id).bind::<BigInt,_>(integer(lease.generation).map_err(|_|CloudError::InvalidArgument)?).bind::<BigInt,_>(integer(now).map_err(|_|CloudError::InvalidArgument)?).bind::<BigInt,_>(MAX_RESOURCE_REQUESTS as i64).load::<RequestRow>(conn).await?;
            rows.into_iter().map(|row| {
                if row.digest!=request_digest(lease,&row.request_id,&row.actor,&row.operation,row.created_at as u64,row.expires_at as u64)? {return Err(CloudError::Storage.into());}
                Ok(RuntimeResourceRequest {request_id:row.request_id,lease:lease.clone(),actor:row.actor,operation:serde_json::from_value(row.operation).map_err(|_|CloudError::Storage)?,created_at:row.created_at as u64,expires_at:row.expires_at as u64,dispatched:true,reply:None})
            }).collect()
        }).await.map_err(TransactionError::cloud)
    }
    pub(super) async fn complete_resource(
        &self,
        lease: &ConnectionLease,
        id: &str,
        reply: ResourceReply,
        now: u64,
    ) -> CloudResult<()> {
        if colossus_cloud_protocol::encode(&reply)
            .map_err(|_| CloudError::InvalidArgument)?
            .len()
            > colossus_cloud_protocol::MAX_PAYLOAD_BYTES
        {
            return Err(CloudError::InvalidArgument);
        }
        let reply = serde_json::to_value(reply).map_err(|_| CloudError::InvalidArgument)?;
        let reply_digest = digest(&reply).map_err(|_| CloudError::Storage)?;
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        conn.transaction::<_,TransactionError,_>(async |conn| {
            operational::verify(conn,lease,None,true).await?;
            let changed=sql_query("UPDATE runtime_resource_requests SET reply=$6,reply_digest=$8 WHERE request_id=$1 AND project_id=$2 AND node_id=$3 AND owner_id=$4 AND generation=$5 AND dispatched AND reply IS NULL AND expires_at>$7")
                .bind::<Text,_>(id).bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).bind::<Text,_>(&lease.owner_id).bind::<BigInt,_>(integer(lease.generation).map_err(|_|CloudError::InvalidArgument)?).bind::<Jsonb,_>(&reply).bind::<BigInt,_>(integer(now).map_err(|_|CloudError::InvalidArgument)?).bind::<Text,_>(&reply_digest).execute(conn).await?;
            // Late/duplicate replies cannot disclose data to a newer connection.
            if changed==0{return Ok(());}
            #[derive(diesel::QueryableByName)] struct Head { #[diesel(sql_type=Text)] chain_hash:String, #[diesel(sql_type=Text)] actor:String }
            let head=sql_query("SELECT chain_hash,actor FROM cloud_audit WHERE project_id=$1 AND entity_kind='runtime_resource_requests' AND parent_id=$2 AND id=$3 AND revision=1")
                .bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).bind::<Text,_>(id).get_result::<Head>(conn).await?;
            let chain=digest(&serde_json::json!(["runtime_resource_requests",lease.project_id,lease.node_id,id,2,head.actor,"runtime.resource.responded.v1",reply_digest,head.chain_hash])).map_err(|_|CloudError::Storage)?;
            sql_query("INSERT INTO cloud_audit(project_id,entity_kind,parent_id,id,revision,actor,operation,content_digest,previous_hash,chain_hash) VALUES($1,'runtime_resource_requests',$2,$3,2,$4,'runtime.resource.responded.v1',$5,$6,$7)")
                .bind::<Text,_>(&lease.project_id).bind::<Text,_>(&lease.node_id).bind::<Text,_>(id).bind::<Text,_>(&head.actor).bind::<Text,_>(&reply_digest).bind::<Text,_>(&head.chain_hash).bind::<Text,_>(&chain).execute(conn).await?;
            Ok(())
        }).await.map_err(TransactionError::cloud)
    }
    pub(super) async fn read_resource(
        &self,
        caller: &CloudCaller,
        id: &str,
    ) -> CloudResult<Option<ResourceReply>> {
        caller.require(colossus_cloud::CloudPermission::Read)?;
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        let row=sql_query("SELECT reply,reply_digest FROM runtime_resource_requests WHERE request_id=$1 AND project_id=$2 AND actor=$3")
            .bind::<Text,_>(id).bind::<Text,_>(caller.project_id()).bind::<Text,_>(caller.subject()).get_result::<ReplyRow>(&mut conn).await.map_err(|error|if matches!(error,diesel::result::Error::NotFound){CloudError::NotFound}else{CloudError::Storage})?;
        row.reply
            .map(|reply| {
                if Some(digest(&reply).map_err(|_| CloudError::Storage)?) != row.reply_digest {
                    return Err(CloudError::Storage);
                }
                serde_json::from_value(reply).map_err(|_| CloudError::Storage)
            })
            .transpose()
    }
}
