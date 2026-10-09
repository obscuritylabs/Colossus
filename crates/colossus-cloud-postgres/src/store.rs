use crate::{CloudDatabaseConfig, connection, entities, operational};
use colossus_cloud::{CloudError, CloudResult, storage::*};
use colossus_network::AdditionalRootCertificates;
use colossus_ports::StoreError;
use diesel::{
    sql_query,
    sql_types::{BigInt, Bool, Text},
};
use diesel_async::{
    AsyncConnection, AsyncPgConnection, RunQueryDsl, SimpleAsyncConnection,
    pooled_connection::{AsyncDieselConnectionManager, ManagerConfig, bb8::Pool},
};
use std::{sync::Arc, time::Duration};

/// Relational cloud adapter with bounded asynchronous connections and per-domain CAS.
#[derive(Clone)]
pub struct CloudPostgresStore {
    pub(super) pool: Pool<AsyncPgConnection>,
    pub(super) schema: String,
    pub(super) profiler: Arc<crate::profiling::Profiler>,
    notifications: tokio::sync::broadcast::Sender<String>,
    _pump: Arc<NotificationPump>,
}
struct NotificationPump(tokio::task::JoinHandle<()>);
impl Drop for NotificationPump {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl CloudPostgresStore {
    /// Resolve the native credential reference, establish verified transport, and migrate.
    /// Connection URLs and database diagnostic payloads are never included in errors.
    pub async fn open(
        config: CloudDatabaseConfig,
        roots: &AdditionalRootCertificates,
    ) -> CloudResult<Self> {
        config.validate()?;
        let url = std::env::var(&config.connection_variable).map_err(|_| CloudError::Storage)?;
        connection::parse(&url, &config)?;
        let tls = connection::trust(&config, roots)?;
        let setup_config = config.clone();
        let listener_url = url.clone();
        let listener_tls = tls.clone();
        let mut manager = ManagerConfig::default();
        manager.custom_setup = Box::new(move |url| {
            let config = setup_config.clone();
            let tls = tls.clone();
            Box::pin(async move { connection::establish(url, &config, tls).await })
        });
        let manager =
            AsyncDieselConnectionManager::<AsyncPgConnection>::new_with_config(url, manager);
        let pool = Pool::builder()
            .max_size(config.max_connections)
            .connection_timeout(Duration::from_millis(config.connection_timeout_ms))
            .build(manager)
            .await
            .map_err(|_| CloudError::Storage)?;
        let (notifications, _) = tokio::sync::broadcast::channel(256);
        let pump_notifications = notifications.clone();
        let listener_config = config.clone();
        let pump = tokio::spawn(async move {
            use futures::StreamExt;
            let mut delay = Duration::from_millis(500);
            loop {
                if let Ok(mut conn) =
                    connection::establish(&listener_url, &listener_config, listener_tls.clone())
                        .await
                    && conn
                        .batch_execute("LISTEN colossus_cloud_changed")
                        .await
                        .is_ok()
                {
                    delay = Duration::from_millis(500);
                    let stream = conn.notifications_stream();
                    tokio::pin!(stream);
                    let mut pending = std::collections::BTreeSet::new();
                    let mut publication = tokio::time::interval(Duration::from_millis(100));
                    publication.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        tokio::select! {
                            notification=stream.next()=>{
                                let Some(Ok(notification))=notification else{break;};
                                let payload=&notification.payload;
                                if pending.len()<1024&&!payload.is_empty()&&payload.len()<=128&&payload.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'_'|b'-'|b'.')){pending.insert(payload.to_owned());}
                            },
                            _=publication.tick()=>{for project in std::mem::take(&mut pending){let _=pump_notifications.send(project);}}
                        }
                    }
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        });
        let store = Self {
            pool,
            schema: config.schema.clone(),
            profiler: Arc::new(crate::profiling::Profiler::default()),
            notifications,
            _pump: Arc::new(NotificationPump(pump)),
        };
        let mut conn = store.pool.get().await.map_err(|_| CloudError::Storage)?;
        // Schema setup is operator-owned; only migration execution takes a schema lock.
        conn.batch_execute(&format!(
            "CREATE SCHEMA IF NOT EXISTS \"{}\"; SET search_path TO \"{}\"",
            config.schema, config.schema
        ))
        .await
        .map_err(|_| CloudError::Storage)?;
        conn.transaction(async |conn|{
            sql_query("SELECT pg_advisory_xact_lock(hashtext($1))").bind::<Text,_>(format!("colossus-cloud-migrate:{}",config.schema)).execute(conn).await?;
            #[derive(diesel::QueryableByName)]struct Legacy{#[diesel(sql_type=Bool)]present:bool}
            let legacy=sql_query("SELECT to_regclass('journal_metadata') IS NOT NULL OR to_regclass('journal_events') IS NOT NULL AS present").get_result::<Legacy>(conn).await?;
            if legacy.present{return Err(TransactionError::Store(StoreError::Adapter("cloud storage requires a distinct schema from runtime journals".into())));}
            conn.batch_execute("CREATE TABLE IF NOT EXISTS cloud_schema_migrations(version BIGINT PRIMARY KEY,checksum TEXT NOT NULL,applied_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp())").await?;
            #[derive(diesel::QueryableByName)]struct Applied{#[diesel(sql_type=Text)]checksum:String}
            for (version,sql) in [(1_i64,include_str!("../migrations/00000000000001_cloud_relational/up.sql"))] {
            let checksum=entities::digest(&serde_json::Value::String(sql.into())).map_err(TransactionError::Store)?;
            match sql_query("SELECT checksum FROM cloud_schema_migrations WHERE version=$1").bind::<BigInt,_>(version).get_result::<Applied>(conn).await{
                Ok(applied) if applied.checksum==checksum=>{},
                Ok(_)=>return Err(TransactionError::Store(StoreError::Verification("cloud migration checksum changed".into()))),
                Err(diesel::result::Error::NotFound)=>{conn.batch_execute(sql).await?;sql_query("INSERT INTO cloud_schema_migrations(version,checksum) VALUES($1,$2)").bind::<BigInt,_>(version).bind::<Text,_>(&checksum).execute(conn).await?;},
                Err(error)=>return Err(error.into()),
            }}Ok::<_,TransactionError>(())
        }).await.map_err(|_|CloudError::Storage)?;
        drop(conn);
        Ok(store)
    }
}

pub(super) enum TransactionError {
    Store(StoreError),
    Database(diesel::result::Error),
}
impl From<diesel::result::Error> for TransactionError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Database(error)
    }
}
impl From<StoreError> for TransactionError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}
impl TransactionError {
    pub(super) fn into_store(self) -> StoreError {
        match self {
            Self::Store(error) => error,
            Self::Database(error) => entities::db_error(error),
        }
    }
}

#[async_trait::async_trait]
impl CloudStore for CloudPostgresStore {
    async fn user_accounts(&self, user_ids: &[String]) -> CloudResult<Vec<EntityRecord>> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        entities::accounts(&mut conn, user_ids).await
    }
    async fn user_identities(&self, user_id: &str) -> CloudResult<Vec<EntityRecord>> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        entities::identities(&mut conn, user_id).await
    }
    async fn statistics(
        &self,
        project: &str,
        node: Option<&str>,
        now: u64,
        days: u16,
    ) -> CloudResult<colossus_cloud::observability::OperationalStatistics> {
        self.operational_statistics(project, node, now, days).await
    }
    async fn maintain(
        &self,
        now: u64,
        policy: &CloudMaintenancePolicy,
    ) -> CloudResult<CloudMaintenanceReport> {
        self.maintain_operational(now, policy).await
    }
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.notifications.subscribe()
    }
    async fn readiness(&self) -> CloudResult<()> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        sql_query("SELECT 1")
            .execute(&mut conn)
            .await
            .map_err(|_| CloudError::Storage)?;
        Ok(())
    }
    async fn list_projects(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> CloudResult<Vec<EntityRecord>> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        entities::projects(&mut conn, after, limit).await
    }
    async fn read(&self, key: &EntityKey) -> CloudResult<EntityRecord> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        entities::read(&mut conn, key).await
    }
    async fn delete_entity(&self, key: &EntityKey, revision: u64) -> CloudResult<()> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        conn.transaction(async |conn| {
            entities::delete(conn, key, revision).await?;
            Ok::<_, TransactionError>(())
        })
        .await
        .map_err(|e| CloudError::from(e.into_store()))
    }
    async fn list(&self, query: &EntityQuery) -> CloudResult<Vec<EntityRecord>> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        entities::list(&mut conn, query).await
    }
    async fn memberships(&self, subject: &str) -> CloudResult<Vec<EntityRecord>> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        entities::memberships(&mut conn, subject).await
    }
    async fn thread_incomplete(&self, project: &str, thread: &str) -> CloudResult<bool> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        entities::thread_incomplete(&mut conn, project, thread).await
    }
    async fn commit(&self, mut transaction: CloudTransaction) -> Result<(), StoreError> {
        let _transaction_span = self.profiler.span(crate::profiling::Stage::Transaction);
        let acquisition_span = self.profiler.span(crate::profiling::Stage::Pool);
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|_| StoreError::Adapter("cloud connection pool unavailable".into()))?;
        drop(acquisition_span);
        // Stable entity acquisition order prevents lock inversion across batched updates.
        transaction.entities.sort_by(|a, b| a.key.cmp(&b.key));
        transaction.events.sort_by(|a, b| {
            (&a.project_id, &a.scope_id, a.sequence).cmp(&(&b.project_id, &b.scope_id, b.sequence))
        });
        transaction.cursors.sort_by(|a, b| {
            (&a.project_id, &a.source_id, &a.scope_id).cmp(&(
                &b.project_id,
                &b.source_id,
                &b.scope_id,
            ))
        });
        let projects: std::collections::BTreeSet<_> = transaction
            .entities
            .iter()
            .map(|m| m.key.project_id.clone())
            .chain(transaction.events.iter().map(|e| e.project_id.clone()))
            .chain(transaction.cursors.iter().map(|c| c.project_id.clone()))
            .collect();
        if transaction
            .lease
            .as_ref()
            .is_some_and(|lease| projects.iter().any(|project| *project != lease.project_id))
        {
            return Err(StoreError::Adapter(
                "cloud connection cannot mutate another project".into(),
            ));
        }
        conn.transaction(async |conn| {
            let administration = crate::identity_guard::begin(conn, &transaction.entities).await?;
            if let Some(lease) = transaction.lease {
                operational::verify(conn, &lease, None, true)
                    .await
                    .map_err(|error| {
                        TransactionError::Store(if error == CloudError::Conflict {
                            StoreError::WriterLeaseHeld
                        } else {
                            StoreError::Adapter(
                                "cloud connection lease verification unavailable".into(),
                            )
                        })
                    })?;
            }
            for mutation in transaction.entities {
                entities::mutate_profiled(conn, mutation, &self.profiler).await?;
            }
            for event in transaction.events {
                operational::append_event(conn, event, &self.profiler).await?;
            }
            for cursor in transaction.cursors {
                operational::advance_cursor(conn, cursor, &self.profiler).await?;
            }
            crate::identity_guard::finish(conn, administration).await?;
            // Notifications are wakeups; durable outbox and rows remain the source of truth.
            for project in projects {
                sql_query("SELECT pg_notify('colossus_cloud_changed',$1)")
                    .bind::<Text, _>(project)
                    .execute(conn)
                    .await?;
            }
            Ok::<_, TransactionError>(())
        })
        .await
        .map_err(TransactionError::into_store)
    }
    async fn events(
        &self,
        project: &str,
        scope: &str,
        after: u64,
        limit: usize,
    ) -> CloudResult<Vec<ReleasedEvent>> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        operational::events(&mut conn, project, scope, after, limit).await
    }
    async fn cursor(&self, project: &str, source: &str, scope: &str) -> CloudResult<u64> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        operational::cursor(&mut conn, project, source, scope).await
    }
    async fn put_session(&self, session: AuthSession) -> CloudResult<()> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        operational::put_session(&mut conn, session).await
    }
    async fn read_session(&self, hash: &str, now: u64) -> CloudResult<AuthSession> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        operational::read_session(&mut conn, hash, now).await
    }
    async fn delete_session(&self, hash: &str) -> CloudResult<()> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        operational::delete_session(&mut conn, hash).await
    }
    async fn claim_lease(
        &self,
        project: &str,
        node: &str,
        owner: &str,
        now: u64,
        ttl: u64,
    ) -> CloudResult<ConnectionLease> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        operational::claim(&mut conn, project, node, owner, now, ttl).await
    }
    async fn read_lease(
        &self,
        project: &str,
        node: &str,
        now: u64,
    ) -> CloudResult<ConnectionLease> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        operational::read_lease(&mut conn, project, node, now).await
    }
    async fn renew_lease(
        &self,
        lease: &ConnectionLease,
        now: u64,
        ttl: u64,
    ) -> CloudResult<ConnectionLease> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        operational::renew(&mut conn, lease, now, ttl).await
    }
    async fn release_lease(&self, lease: &ConnectionLease) -> CloudResult<()> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        operational::release(&mut conn, lease).await
    }
    async fn verify_lease(&self, lease: &ConnectionLease, now: u64) -> CloudResult<()> {
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        operational::verify(&mut conn, lease, Some(now), false).await
    }
}
