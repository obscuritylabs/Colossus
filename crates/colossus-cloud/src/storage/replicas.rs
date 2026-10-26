//! Short-lived internal routing metadata, separate from durable application records.
use crate::{CloudError, CloudResult};
use std::net::SocketAddr;

/// Operator-advertised endpoint of one live Control Plane process.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplicaRoute {
    /// Random process incarnation; never reused by a replacement pod.
    pub replica_id: String,
    /// Internal TLS gRPC listener, never supplied by a browser.
    pub address: SocketAddr,
    /// UTC Unix expiry; a route is unusable after this time.
    pub expires_at: u64,
}

/// Bound internal route identities, expiry and addresses before adapter access.
pub fn validate_replica_route(route: &ReplicaRoute, now: u64) -> CloudResult<()> {
    if !(8..=64).contains(&route.replica_id.len())
        || !route
            .replica_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        || route.address.port() == 0
        || route.address.ip().is_unspecified()
        || route.address.ip().is_multicast()
        || !(1..=90).contains(&route.expires_at.saturating_sub(now))
    {
        return Err(CloudError::InvalidArgument);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn routes_expire_and_replacement_processes_do_not_inherit_addresses() {
        use crate::storage::{CloudStore, MemoryCloudStore};
        let store = MemoryCloudStore::default();
        let route = ReplicaRoute {
            replica_id: "0123456789abcdef".into(),
            address: "127.0.0.1:8443".parse().unwrap(),
            expires_at: 160,
        };
        store
            .publish_replica_route(route.clone(), 100)
            .await
            .unwrap();
        assert_eq!(
            store
                .read_replica_route(&route.replica_id, 159)
                .await
                .unwrap(),
            route
        );
        assert!(
            store
                .read_replica_route(&route.replica_id, 160)
                .await
                .is_err()
        );
        assert!(store.read_replica_route("replacement", 110).await.is_err());
        let mut invalid = route;
        invalid.address = "0.0.0.0:8443".parse().unwrap();
        assert!(store.publish_replica_route(invalid, 100).await.is_err());
    }
}
