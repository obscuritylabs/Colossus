use crate::{CloudPostgresStore, entities, store::TransactionError};
use colossus_cloud::{CloudError, CloudResult, storage::*};
use colossus_ports::StoreError;
use diesel::{
    sql_query,
    sql_types::{Array, BigInt, Bool, Text},
};
use diesel_async::{AsyncConnection, RunQueryDsl, SimpleAsyncConnection};

impl CloudPostgresStore {
    pub(super) async fn maintain_operational(
        &self,
        now: u64,
        policy: &CloudMaintenancePolicy,
    ) -> CloudResult<CloudMaintenanceReport> {
        if !(1..=1024).contains(&policy.batch_limit)
            || !(60..=2592000).contains(&policy.delivered_outbox_retention_seconds)
        {
            return Err(CloudError::InvalidArgument);
        }
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        conn.transaction(async|conn|{
            #[derive(diesel::QueryableByName)]struct Outbox{#[diesel(sql_type=BigInt)]outbox_id:i64,#[diesel(sql_type=Text)]project_id:String}
            let pending=sql_query("SELECT outbox_id,project_id FROM delivery_outbox WHERE delivered_at IS NULL ORDER BY outbox_id LIMIT $1 FOR UPDATE SKIP LOCKED").bind::<BigInt,_>(policy.batch_limit as i64).load::<Outbox>(conn).await?;
            let projects:std::collections::BTreeSet<_>=pending.iter().map(|entry|entry.project_id.as_str()).collect();
            for project in projects{sql_query("SELECT pg_notify('colossus_cloud_changed',$1)").bind::<Text,_>(project).execute(conn).await?;}
            let ids:Vec<_>=pending.iter().map(|entry|entry.outbox_id).collect();
            let published=sql_query("UPDATE delivery_outbox SET delivered_at=clock_timestamp() WHERE outbox_id=ANY($1)").bind::<Array<BigInt>,_>(&ids).execute(conn).await?;
            let before=entities::integer(now.saturating_sub(policy.delivered_outbox_retention_seconds))?;
            let removed=sql_query("WITH expired AS(SELECT outbox_id FROM delivery_outbox WHERE delivered_at<to_timestamp($1) ORDER BY delivered_at,outbox_id LIMIT $2 FOR UPDATE SKIP LOCKED) DELETE FROM delivery_outbox WHERE outbox_id IN(SELECT outbox_id FROM expired)").bind::<BigInt,_>(before).bind::<BigInt,_>(policy.batch_limit as i64).execute(conn).await?;
            let now=entities::integer(now)?;
            let sessions=sql_query("WITH expired AS(SELECT session_hash FROM browser_sessions WHERE expires_at<=$1 ORDER BY expires_at LIMIT $2 FOR UPDATE SKIP LOCKED) DELETE FROM browser_sessions WHERE session_hash IN(SELECT session_hash FROM expired)").bind::<BigInt,_>(now).bind::<BigInt,_>(policy.batch_limit as i64).execute(conn).await?;
            #[derive(diesel::QueryableByName)]struct Flow{#[diesel(sql_type=Text)]project_id:String,#[diesel(sql_type=Text)]parent_id:String,#[diesel(sql_type=Text)]id:String,#[diesel(sql_type=BigInt)]revision:i64,#[diesel(sql_type=Bool)]deleted:bool}
            let flows=sql_query("SELECT project_id,parent_id,id,revision,deleted FROM oidc_flows WHERE expires_at<=$1 ORDER BY expires_at,id LIMIT $2 FOR UPDATE SKIP LOCKED").bind::<BigInt,_>(now).bind::<BigInt,_>(policy.batch_limit as i64).load::<Flow>(conn).await?;
            for flow in &flows{let key=EntityKey{kind:EntityKind::AuthFlow,project_id:flow.project_id.clone(),parent_id:(!flow.parent_id.is_empty()).then(||flow.parent_id.clone()),id:flow.id.clone()};if !flow.deleted{entities::delete(conn,&key,flow.revision as u64).await?;}
                sql_query("DELETE FROM oidc_flows WHERE project_id=$1 AND parent_id=$2 AND id=$3").bind::<Text,_>(&flow.project_id).bind::<Text,_>(&flow.parent_id).bind::<Text,_>(&flow.id).execute(conn).await?;
            }
            let resources=sql_query("WITH expired AS(SELECT request_id FROM runtime_resource_requests WHERE expires_at<$1-60 ORDER BY expires_at,request_id LIMIT $2 FOR UPDATE SKIP LOCKED) DELETE FROM runtime_resource_requests WHERE request_id IN(SELECT request_id FROM expired)").bind::<BigInt,_>(now).bind::<BigInt,_>(policy.batch_limit as i64).execute(conn).await?;
            let report=CloudMaintenanceReport{expired_resource_requests:resources,published_outbox:published,removed_outbox:removed,expired_sessions:sessions,expired_auth_flows:flows.len()};
            if published+removed+sessions+flows.len()>0 {
                let key=EntityKey{kind:EntityKind::AuthFlow,project_id:"__maintenance".into(),parent_id:None,id:"operational".into()};
                let revision=match entities::read(conn,&key).await{Ok(record)=>record.revision,Err(CloudError::NotFound)=>0,Err(_)=>return Err(TransactionError::Store(StoreError::Adapter("maintenance audit unavailable".into())))};
                entities::mutate(conn,EntityMutation{key,expected_revision:revision,value:EntityValue::AuthFlow(serde_json::json!({"observed_at":now,"published_outbox":published,"removed_outbox":removed,"expired_sessions":sessions,"expired_auth_flows":flows.len()})),actor:"cloud-maintenance".into(),operation:"cloud.operational-maintenance.v1".into()}).await?;
            }
            Ok::<_,TransactionError>(report)
        }).await.map_err(|error|CloudError::from(error.into_store()))
    }
    /// Remove only this adapter's explicitly generated local measurement namespace.
    /// Ordinary cloud schema names are rejected, and a caller cannot nominate another schema.
    pub async fn remove_fixture_schema(&self, schema: &str) -> CloudResult<()> {
        let suffix = schema
            .strip_prefix("cloud_load_")
            .or_else(|| schema.strip_prefix("cloud_acceptance_"));
        if schema != self.schema
            || !suffix.is_some_and(|suffix| {
                suffix.len() == 32 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return Err(CloudError::PermissionDenied);
        }
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        conn.batch_execute(&format!("DROP SCHEMA \"{schema}\" CASCADE"))
            .await
            .map_err(|_| CloudError::Storage)?;
        Ok(())
    }
}
