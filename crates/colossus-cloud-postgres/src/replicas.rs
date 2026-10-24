//! Expiring replica directory. Terminal payloads never enter relational storage.
use crate::CloudPostgresStore;
use colossus_cloud::{
    CloudError, CloudResult,
    storage::{ReplicaRoute, validate_replica_route},
};
use diesel::{
    sql_query,
    sql_types::{BigInt, Text},
};
use diesel_async::RunQueryDsl;

#[derive(diesel::QueryableByName)]
struct RouteRow {
    #[diesel(sql_type=Text)]
    replica_id: String,
    #[diesel(sql_type=Text)]
    address: String,
    #[diesel(sql_type=BigInt)]
    expires_at: i64,
}

impl CloudPostgresStore {
    pub(super) async fn publish_route(&self, route: ReplicaRoute, now: u64) -> CloudResult<()> {
        validate_replica_route(&route, now)?;
        let ttl = i64::try_from(route.expires_at.saturating_sub(now))
            .map_err(|_| CloudError::InvalidArgument)?;
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        sql_query("INSERT INTO control_plane_replica_routes(replica_id,address,expires_at) VALUES($1,$2,floor(extract(epoch FROM clock_timestamp()))::BIGINT+$3) ON CONFLICT(replica_id) DO UPDATE SET address=EXCLUDED.address,expires_at=EXCLUDED.expires_at")
            .bind::<Text,_>(&route.replica_id).bind::<Text,_>(route.address.to_string()).bind::<BigInt,_>(ttl)
            .execute(&mut conn).await.map_err(|_| CloudError::Storage)?;
        sql_query("DELETE FROM control_plane_replica_routes WHERE replica_id IN (SELECT replica_id FROM control_plane_replica_routes WHERE expires_at<=floor(extract(epoch FROM clock_timestamp()))::BIGINT LIMIT 32)")
            .execute(&mut conn).await.map_err(|_| CloudError::Storage)?;
        Ok(())
    }

    pub(super) async fn read_route(&self, replica: &str) -> CloudResult<ReplicaRoute> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        let value = sql_query("SELECT replica_id,address,expires_at FROM control_plane_replica_routes WHERE replica_id=$1 AND expires_at>floor(extract(epoch FROM clock_timestamp()))::BIGINT")
            .bind::<Text,_>(replica).get_result::<RouteRow>(&mut conn).await
            .map_err(|error| if error == diesel::result::Error::NotFound {CloudError::NotFound} else {CloudError::Storage})?;
        Ok(ReplicaRoute {
            replica_id: value.replica_id,
            address: value.address.parse().map_err(|_| CloudError::Storage)?,
            expires_at: u64::try_from(value.expires_at).map_err(|_| CloudError::Storage)?,
        })
    }
}
