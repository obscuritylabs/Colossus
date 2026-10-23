//! Narrow synchronous custody/placement adapter for the synchronous vault and journal ports.
//! A connection failure is never retried within a mutation: its outcome may be uncertain.

use crate::{CloudDatabaseConfig, connection};
use colossus_cloud::{CloudError, CloudResult};
use colossus_network::AdditionalRootCertificates;
use postgres::{Client, NoTls};
use std::{sync::Mutex, time::Duration};
use tokio_postgres_rustls::MakeRustlsConnect;

const MAX_CIPHERTEXT: usize = 1_049_600;
const DB_NOW: &str = "EXTRACT(EPOCH FROM clock_timestamp())::bigint";
#[cfg(test)]
mod tests;

/// One exact hosted runtime owner. A newer generation permanently fences its predecessor.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChatLease {
    /// Stable project/user runtime partition.
    pub namespace: String,
    /// Project authority bound during provisioning.
    pub project_id: String,
    /// Personal owner bound during provisioning.
    pub user_id: String,
    /// Unique process/runtime attempt, never a browser-selected identity.
    pub owner_id: String,
    /// Monotonic fencing token.
    pub generation: u64,
    /// Database-clock expiry in Unix seconds.
    pub expires_at: u64,
}

/// Opaque encrypted record and the optimistic revision returned by SQL.
pub struct ChatRecord {
    /// Monotonic record revision, including retained tombstones.
    pub revision: u64,
    /// Ciphertext only; decryption belongs to the trusted custody adapter.
    pub ciphertext: Option<Vec<u8>>,
}

/// One bounded encrypted mutation. A batch commits all revisions atomically.
pub struct ChatRecordWrite<'a> {
    /// Trusted custody namespace.
    pub namespace: &'a str,
    /// Purpose-bound opaque record identifier.
    pub record_id: &'a str,
    /// Exact prior revision; zero means the record has never existed.
    pub expected_revision: u64,
    /// None creates a revisioned tombstone.
    pub ciphertext: Option<&'a [u8]>,
    /// Optional TTL, measured by the database clock.
    pub ttl_seconds: Option<u64>,
}

/// One at-most-once management request captured beneath the execution owner's generation.
pub struct ChatManagementRequest {
    /// Opaque request identity.
    pub id: String,
    /// Authenticated encrypted payload, decoded by its trusted host only.
    pub ciphertext: Vec<u8>,
}

/// Shared SQL state beneath the existing, independently migrated cloud schema.
/// The mutex bounds synchronous connections to one per adapter; callers never log its URL.
pub struct ChatSqlStore {
    config: CloudDatabaseConfig,
    tls: Option<MakeRustlsConnect>,
    client: Mutex<Option<Client>>,
    access: Mutex<()>,
}

impl Drop for ChatSqlStore {
    fn drop(&mut self) {
        let client = match self.client.get_mut() {
            Ok(client) => client.take(),
            Err(error) => error.into_inner().take(),
        };
        if let Some(client) = client {
            let _ = std::thread::spawn(move || drop(client)).join();
        }
    }
}

impl ChatSqlStore {
    /// Validate transport and connect only after cloud migrations have completed.
    pub fn open(config: CloudDatabaseConfig) -> CloudResult<Self> {
        config.validate()?;
        let tls = connection::trust(&config, &AdditionalRootCertificates::default())?;
        let value = Self {
            config,
            tls,
            client: Mutex::new(None),
            access: Mutex::new(()),
        };
        value.with_client(|client| {
            client.query_one("SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_schema=current_schema() AND table_name='hosted_chat_leases')", &[])
                .map_err(|_| CloudError::Storage).and_then(|row| if row.get::<_, bool>(0) {Ok(())} else {Err(CloudError::Storage)})
        })?;
        Ok(value)
    }

    fn connect(&self) -> CloudResult<Client> {
        let value = zeroize::Zeroizing::new(
            std::env::var(&self.config.connection_variable).map_err(|_| CloudError::Storage)?,
        );
        // Reuse the cloud adapter's verified-host and local-plaintext rules.
        connection::parse(&value, &self.config)?;
        let mut config = value
            .parse::<postgres::Config>()
            .map_err(|_| CloudError::Storage)?;
        config.application_name("colossus-hosted-chat");
        config.connect_timeout(Duration::from_millis(self.config.connection_timeout_ms));
        config.ssl_mode(if self.tls.is_some() {
            postgres::config::SslMode::Require
        } else {
            postgres::config::SslMode::Disable
        });
        let mut client = match &self.tls {
            Some(tls) => config.connect(tls.clone()),
            None => config.connect(NoTls),
        }
        .map_err(|_| CloudError::Storage)?;
        client
            .batch_execute(&format!(
                "SET search_path TO \"{}\"; SET statement_timeout={}; SET lock_timeout={}",
                self.config.schema,
                self.config.statement_timeout_ms,
                self.config.statement_timeout_ms
            ))
            .map_err(|_| CloudError::Storage)?;
        Ok(client)
    }

    fn with_client<T: Send>(
        &self,
        operation: impl FnOnce(&mut Client) -> CloudResult<T> + Send,
    ) -> CloudResult<T> {
        // postgres::Client owns a private blocking reactor. Keep it off any caller's
        // Tokio reactor, including synchronous CredentialVault calls made by async tools.
        // One admission mutex bounds worker threads to one per adapter.
        let _admission = self.access.lock().map_err(|_| CloudError::Storage)?;
        std::thread::scope(|scope| {
            scope
                .spawn(|| self.with_client_inner(operation))
                .join()
                .map_err(|_| CloudError::Storage)?
        })
    }

    fn with_client_inner<T>(
        &self,
        operation: impl FnOnce(&mut Client) -> CloudResult<T>,
    ) -> CloudResult<T> {
        let mut guard = self.client.lock().map_err(|_| CloudError::Storage)?;
        if guard.is_none() {
            *guard = Some(self.connect()?);
        }
        let result = operation(guard.as_mut().ok_or(CloudError::Storage)?);
        if matches!(result, Err(CloudError::Storage)) {
            *guard = None;
        }
        result
    }

    /// Claim a free/expired partition before opening its journal or executing recovery.
    pub fn claim_lease(
        &self,
        namespace: &str,
        project: &str,
        user: &str,
        owner: &str,
        ttl: u64,
    ) -> CloudResult<ChatLease> {
        validate_id(namespace)?;
        validate_id(project)?;
        validate_id(user)?;
        validate_id(owner)?;
        let ttl = validate_ttl(ttl)?;
        self.with_client(|client| {
            let mut tx=client.transaction().map_err(|_|CloudError::Storage)?;
            tx.execute("INSERT INTO hosted_chat_project_gates(project_id) VALUES($1) ON CONFLICT DO NOTHING", &[&project]).map_err(|_|CloudError::Storage)?;
            let gate=tx.query_one(&format!("SELECT expires_at>{DB_NOW} FROM hosted_chat_project_gates WHERE project_id=$1 FOR SHARE"), &[&project]).map_err(|_|CloudError::Storage)?;
            if gate.get::<_,bool>(0){return Err(CloudError::Conflict);}
            let row = tx.query_opt(&format!("INSERT INTO hosted_chat_leases(namespace,project_id,user_id,owner_id,generation,expires_at) VALUES($1,$2,$3,$4,1,{DB_NOW}+$5) ON CONFLICT(namespace) DO UPDATE SET owner_id=$4,generation=hosted_chat_leases.generation+1,expires_at={DB_NOW}+$5 WHERE hosted_chat_leases.project_id=$2 AND hosted_chat_leases.user_id=$3 AND (hosted_chat_leases.expires_at<={DB_NOW} OR hosted_chat_leases.owner_id=$4) RETURNING namespace,project_id,user_id,owner_id,generation,expires_at"), &[&namespace,&project,&user,&owner,&ttl]).map_err(|_| CloudError::Storage)?.ok_or(CloudError::Conflict)?;
            let lease=lease_row(&row)?;
            tx.commit().map_err(|_|CloudError::Storage)?;
            Ok(lease)
        })
    }

    /// Observe live placement without creating a runtime on the serving HTTP replica.
    pub fn read_lease(&self, namespace: &str) -> CloudResult<ChatLease> {
        validate_id(namespace)?;
        self.with_client(|client| {
            let row=client.query_opt(&format!("SELECT namespace,project_id,user_id,owner_id,generation,expires_at FROM hosted_chat_leases WHERE namespace=$1 AND expires_at>{DB_NOW}"), &[&namespace]).map_err(|_| CloudError::Storage)?.ok_or(CloudError::NotFound)?;
            lease_row(&row)
        })
    }

    /// Extend only a still-live exact generation; an expired worker cannot resurrect itself.
    pub fn renew_lease(&self, lease: &ChatLease, ttl: u64) -> CloudResult<ChatLease> {
        let ttl = validate_ttl(ttl)?;
        let generation = integer(lease.generation)?;
        self.with_client(|client| {
            let row=client.query_opt(&format!("UPDATE hosted_chat_leases SET expires_at={DB_NOW}+$4 WHERE namespace=$1 AND owner_id=$2 AND generation=$3 AND expires_at>{DB_NOW} RETURNING namespace,project_id,user_id,owner_id,generation,expires_at"), &[&lease.namespace,&lease.owner_id,&generation,&ttl]).map_err(|_| CloudError::Storage)?.ok_or(CloudError::Conflict)?;
            lease_row(&row)
        })
    }

    /// Refuse stale, expired or superseded owners before sensitive runtime work.
    pub fn verify_lease(&self, lease: &ChatLease) -> CloudResult<()> {
        let generation = integer(lease.generation)?;
        self.with_client(|client| {
            if client.query_opt(&format!("SELECT generation FROM hosted_chat_leases WHERE namespace=$1 AND owner_id=$2 AND generation=$3 AND expires_at>{DB_NOW}"), &[&lease.namespace,&lease.owner_id,&generation]).map_err(|_|CloudError::Storage)?.is_none() {return Err(CloudError::Conflict);}
            Ok(())
        })
    }

    /// Release only the captured owner; delayed cleanup cannot release its successor.
    pub fn release_lease(&self, lease: &ChatLease) -> CloudResult<()> {
        let generation = integer(lease.generation)?;
        self.with_client(|client| client.execute("UPDATE hosted_chat_leases SET expires_at=0 WHERE namespace=$1 AND owner_id=$2 AND generation=$3", &[&lease.namespace,&lease.owner_id,&generation]).map(|_|()).map_err(|_|CloudError::Storage))
    }

    /// Read ciphertext and revision. Expired records retain their revision as a tombstone.
    pub fn read_record(&self, namespace: &str, record_id: &str) -> CloudResult<ChatRecord> {
        validate_id(namespace)?;
        validate_id(record_id)?;
        self.with_client(|client| {
            let Some(row)=client.query_opt(&format!("SELECT revision,CASE WHEN expires_at IS NULL OR expires_at>{DB_NOW} THEN ciphertext ELSE NULL END FROM hosted_chat_records WHERE namespace=$1 AND record_id=$2"), &[&namespace,&record_id]).map_err(|_|CloudError::Storage)? else {return Ok(ChatRecord{revision:0,ciphertext:None});};
            Ok(ChatRecord{revision:positive(row.get(0))?,ciphertext:row.get(1)})
        })
    }

    /// Commit bounded encrypted records using exact optimistic revisions.
    pub fn write_records(&self, writes: &[ChatRecordWrite<'_>]) -> CloudResult<Vec<u64>> {
        self.write_records_fenced(writes, None)
    }

    /// Atomically fence credential commits beneath the same runtime ownership generation.
    pub fn write_records_fenced(
        &self,
        writes: &[ChatRecordWrite<'_>],
        lease: Option<&ChatLease>,
    ) -> CloudResult<Vec<u64>> {
        self.write_records_guarded(writes, lease, None)
    }

    /// Commit settings/credentials only while the exact administrative admission gate is live.
    pub fn write_records_guarded(
        &self,
        writes: &[ChatRecordWrite<'_>],
        lease: Option<&ChatLease>,
        gate: Option<(&str, &str)>,
    ) -> CloudResult<Vec<u64>> {
        self.write_records_all(writes, lease, gate, None)
    }

    /// Commit instance custody and metadata audit under the current instance edit gate.
    pub fn write_instance_records(
        &self,
        writes: &[ChatRecordWrite<'_>],
        owner: &str,
        actor: &str,
        operation: &str,
    ) -> CloudResult<Vec<u64>> {
        validate_id(owner)?;
        validate_id(actor)?;
        validate_id(operation)?;
        self.write_records_all(writes, None, None, Some((owner, actor, operation)))
    }

    fn write_records_all(
        &self,
        writes: &[ChatRecordWrite<'_>],
        lease: Option<&ChatLease>,
        gate: Option<(&str, &str)>,
        instance: Option<(&str, &str, &str)>,
    ) -> CloudResult<Vec<u64>> {
        if writes.is_empty() || writes.len() > 32 {
            return Err(CloudError::InvalidArgument);
        }
        let mut unique = std::collections::BTreeSet::new();
        for write in writes {
            validate_id(write.namespace)?;
            validate_id(write.record_id)?;
            if write
                .ciphertext
                .is_some_and(|bytes| bytes.len() > MAX_CIPHERTEXT)
                || !unique.insert((write.namespace, write.record_id))
            {
                return Err(CloudError::InvalidArgument);
            }
            if let Some(ttl) = write.ttl_seconds {
                validate_ttl(ttl)?;
            }
        }
        self.with_client(|client| {
            let mut transaction=client.transaction().map_err(|_|CloudError::Storage)?;
            if let Some((owner,_,_))=instance {
                transaction.query_opt(&format!("SELECT owner_id FROM hosted_chat_instance_gate WHERE singleton AND owner_id=$1 AND expires_at>{DB_NOW} FOR SHARE"), &[&owner]).map_err(|_|CloudError::Storage)?.ok_or(CloudError::Conflict)?;
            }
            if let Some((project,owner))=gate {
                transaction.query_opt(&format!("SELECT project_id FROM hosted_chat_project_gates WHERE project_id=$1 AND owner_id=$2 AND expires_at>{DB_NOW} FOR SHARE"), &[&project,&owner]).map_err(|_|CloudError::Storage)?.ok_or(CloudError::Conflict)?;
            }
            if let Some(lease)=lease {
                let generation=integer(lease.generation)?;
                transaction.query_opt(&format!("SELECT generation FROM hosted_chat_leases WHERE namespace=$1 AND owner_id=$2 AND generation=$3 AND expires_at>{DB_NOW} FOR SHARE"), &[&lease.namespace,&lease.owner_id,&generation]).map_err(|_|CloudError::Storage)?.ok_or(CloudError::Conflict)?;
            }
            let mut revisions=Vec::with_capacity(writes.len());
            for write in writes {
                let expected=integer(write.expected_revision)?;
                let ttl=write.ttl_seconds.map(integer).transpose()?;
                let row=if expected==0 {
                    transaction.query_opt(&format!("INSERT INTO hosted_chat_records(namespace,record_id,revision,ciphertext,expires_at) VALUES($1,$2,1,$3,CASE WHEN $4::bigint IS NULL THEN NULL ELSE {DB_NOW}+$4 END) ON CONFLICT DO NOTHING RETURNING revision"), &[&write.namespace,&write.record_id,&write.ciphertext,&ttl])
                } else {
                    transaction.query_opt(&format!("UPDATE hosted_chat_records SET revision=revision+1,ciphertext=$4,expires_at=CASE WHEN $5::bigint IS NULL THEN NULL ELSE {DB_NOW}+$5 END WHERE namespace=$1 AND record_id=$2 AND revision=$3 RETURNING revision"), &[&write.namespace,&write.record_id,&expected,&write.ciphertext,&ttl])
                }.map_err(|_|CloudError::Storage)?.ok_or(CloudError::Conflict)?;
                revisions.push(positive(row.get(0))?);
            }
            if let Some((_,actor,operation))=instance {
                use sha2::{Digest as _, Sha256};
                let mut digest=Sha256::new();
                for write in writes {digest.update(write.namespace.as_bytes());digest.update(write.record_id.as_bytes());if let Some(bytes)=write.ciphertext {digest.update(bytes);}}
                transaction.execute("INSERT INTO hosted_instance_audit(actor_id,operation,ciphertext_sha256) VALUES($1,$2,$3)", &[&actor,&operation,&hex::encode(digest.finalize())]).map_err(|_|CloudError::Storage)?;
            }
            transaction.commit().map_err(|_|CloudError::Storage)?;
            Ok(revisions)
        })
    }

    /// Serialize instance administration across replicas without inventing a project scope.
    pub fn claim_instance_gate(&self, owner: &str, ttl: u64) -> CloudResult<()> {
        validate_id(owner)?;
        let ttl = validate_ttl(ttl)?;
        self.with_client(|client| {
            client.query_opt(&format!("INSERT INTO hosted_chat_instance_gate(singleton,owner_id,expires_at) VALUES(TRUE,$1,{DB_NOW}+$2) ON CONFLICT(singleton) DO UPDATE SET owner_id=$1,expires_at={DB_NOW}+$2 WHERE hosted_chat_instance_gate.expires_at<={DB_NOW} OR hosted_chat_instance_gate.owner_id=$1 RETURNING owner_id"), &[&owner,&ttl]).map_err(|_|CloudError::Storage)?.ok_or(CloudError::Conflict)?;
            Ok(())
        })
    }

    /// Release only this exact instance editor; a late owner cannot release its successor.
    pub fn release_instance_gate(&self, owner: &str) -> CloudResult<()> {
        validate_id(owner)?;
        self.with_client(|client| client.execute("UPDATE hosted_chat_instance_gate SET expires_at=0 WHERE singleton AND owner_id=$1", &[&owner]).map(|_|()).map_err(|_|CloudError::Storage))
    }

    /// Consume one exact expiring ciphertext atomically. Failed identity checks must precede this.
    pub fn take_record(
        &self,
        namespace: &str,
        record_id: &str,
        revision: u64,
    ) -> CloudResult<Vec<u8>> {
        validate_id(namespace)?;
        validate_id(record_id)?;
        let revision = integer(revision)?;
        self.with_client(|client| {
            let mut tx=client.transaction().map_err(|_|CloudError::Storage)?;
            let row=tx.query_opt(&format!("SELECT ciphertext FROM hosted_chat_records WHERE namespace=$1 AND record_id=$2 AND revision=$3 AND ciphertext IS NOT NULL AND (expires_at IS NULL OR expires_at>{DB_NOW}) FOR UPDATE"), &[&namespace,&record_id,&revision]).map_err(|_|CloudError::Storage)?.ok_or(CloudError::Conflict)?;
            let bytes=row.get(0);
            tx.execute("UPDATE hosted_chat_records SET ciphertext=NULL,revision=revision+1 WHERE namespace=$1 AND record_id=$2", &[&namespace,&record_id]).map_err(|_|CloudError::Storage)?;
            tx.commit().map_err(|_|CloudError::Storage)?;
            Ok(bytes)
        })
    }

    /// Hold project configuration admission while checking idle owners and publishing a revision.
    pub fn claim_project_gate(&self, project: &str, owner: &str, ttl: u64) -> CloudResult<()> {
        validate_id(project)?;
        validate_id(owner)?;
        let ttl = validate_ttl(ttl)?;
        self.with_client(|client| {
            client.query_opt(&format!("INSERT INTO hosted_chat_project_gates(project_id,owner_id,expires_at) VALUES($1,$2,{DB_NOW}+$3) ON CONFLICT(project_id) DO UPDATE SET owner_id=$2,expires_at={DB_NOW}+$3 WHERE hosted_chat_project_gates.expires_at<={DB_NOW} OR hosted_chat_project_gates.owner_id=$2 RETURNING project_id"), &[&project,&owner,&ttl]).map_err(|_|CloudError::Storage)?.ok_or(CloudError::Conflict)?;
            Ok(())
        })
    }

    /// Exact-owner release; another administrator's gate is never cleared.
    pub fn release_project_gate(&self, project: &str, owner: &str) -> CloudResult<()> {
        self.with_client(|client|client.execute("UPDATE hosted_chat_project_gates SET expires_at=0 WHERE project_id=$1 AND owner_id=$2", &[&project,&owner]).map(|_|()).map_err(|_|CloudError::Storage))
    }

    /// Live owners within a project; administrative stop is routed to each current owner.
    pub fn project_leases(&self, project: &str) -> CloudResult<Vec<ChatLease>> {
        self.with_client(|client|client.query(&format!("SELECT namespace,project_id,user_id,owner_id,generation,expires_at FROM hosted_chat_leases WHERE project_id=$1 AND expires_at>{DB_NOW} ORDER BY namespace LIMIT 1025"), &[&project]).map_err(|_|CloudError::Storage)?.iter().map(lease_row).collect())
    }

    /// Enqueue exactly once beneath a captured live execution generation.
    pub fn submit_management(
        &self,
        lease: &ChatLease,
        id: &str,
        ciphertext: &[u8],
    ) -> CloudResult<()> {
        validate_id(id)?;
        if ciphertext.len() > MAX_CIPHERTEXT {
            return Err(CloudError::InvalidArgument);
        }
        let generation = integer(lease.generation)?;
        self.with_client(|client| {
            let mut tx=client.transaction().map_err(|_|CloudError::Storage)?;
            tx.query_one("SELECT pg_advisory_xact_lock(hashtextextended($1,0))",&[&format!("hosted-chat-management:{}",lease.namespace)]).map_err(|_|CloudError::Storage)?;
            let pending=tx.query_one(&format!("SELECT COUNT(*) FROM hosted_chat_requests WHERE namespace=$1 AND expires_at>{DB_NOW} AND response IS NULL"),&[&lease.namespace]).map_err(|_|CloudError::Storage)?;
            if pending.get::<_,i64>(0)>=128{return Err(CloudError::ResourceExhausted);}
            tx.execute(&format!("WITH expired AS(SELECT id FROM hosted_chat_requests WHERE expires_at<{DB_NOW}-60 ORDER BY expires_at,id FOR UPDATE SKIP LOCKED LIMIT 32) DELETE FROM hosted_chat_requests WHERE id IN(SELECT id FROM expired)"),&[]).map_err(|_|CloudError::Storage)?;
            let row=tx.query_opt(&format!("INSERT INTO hosted_chat_requests(id,namespace,owner_id,generation,ciphertext,expires_at) SELECT $4,namespace,owner_id,generation,$5,{DB_NOW}+25 FROM hosted_chat_leases WHERE namespace=$1 AND owner_id=$2 AND generation=$3 AND expires_at>{DB_NOW} RETURNING id"), &[&lease.namespace,&lease.owner_id,&generation,&id,&ciphertext]).map_err(|_|CloudError::Storage)?;
            row.ok_or(CloudError::Conflict)?;tx.commit().map_err(|_|CloudError::Storage)?;Ok(())
        })
    }

    /// Claim bounded requests only once. Interrupted token exchanges are not replayed after takeover.
    pub fn take_management(&self, lease: &ChatLease) -> CloudResult<Vec<ChatManagementRequest>> {
        let generation = integer(lease.generation)?;
        self.with_client(|client| {
            let rows=client.query(&format!("UPDATE hosted_chat_requests r SET dispatched=TRUE FROM (SELECT r.id FROM hosted_chat_requests r JOIN hosted_chat_leases l ON l.namespace=r.namespace AND l.owner_id=r.owner_id AND l.generation=r.generation WHERE r.namespace=$1 AND r.owner_id=$2 AND r.generation=$3 AND l.expires_at>{DB_NOW} AND r.expires_at>{DB_NOW} AND NOT r.dispatched ORDER BY r.id FOR UPDATE OF r SKIP LOCKED LIMIT 8) selected WHERE r.id=selected.id RETURNING r.id,r.ciphertext"), &[&lease.namespace,&lease.owner_id,&generation]).map_err(|_|CloudError::Storage)?;
            Ok(rows.iter().map(|row|ChatManagementRequest{id:row.get(0),ciphertext:row.get(1)}).collect())
        })
    }

    /// Commit one encrypted result. Optional handoff reserves an idle runtime for its administrator.
    pub fn complete_management(
        &self,
        lease: &ChatLease,
        id: &str,
        response: &[u8],
        handoff: Option<&str>,
    ) -> CloudResult<Option<ChatLease>> {
        if response.len() > MAX_CIPHERTEXT {
            return Err(CloudError::InvalidArgument);
        }
        let generation = integer(lease.generation)?;
        self.with_client(|client| {
            let mut tx=client.transaction().map_err(|_|CloudError::Storage)?;
            tx.query_opt(&format!("SELECT generation FROM hosted_chat_leases WHERE namespace=$1 AND owner_id=$2 AND generation=$3 AND expires_at>{DB_NOW} FOR UPDATE"), &[&lease.namespace,&lease.owner_id,&generation]).map_err(|_|CloudError::Storage)?.ok_or(CloudError::Conflict)?;
            let changed=tx.execute(&format!("UPDATE hosted_chat_requests SET response=$5 WHERE namespace=$1 AND owner_id=$2 AND generation=$3 AND id=$4 AND dispatched AND response IS NULL AND expires_at>{DB_NOW}"), &[&lease.namespace,&lease.owner_id,&generation,&id,&response]).map_err(|_|CloudError::Storage)?;
            if changed!=1{return Err(CloudError::Conflict);}
            let result=if let Some(owner)=handoff {
                validate_id(owner)?;
                let row=tx.query_one(&format!("UPDATE hosted_chat_leases SET owner_id=$4,generation=generation+1,expires_at={DB_NOW}+30 WHERE namespace=$1 AND owner_id=$2 AND generation=$3 RETURNING namespace,project_id,user_id,owner_id,generation,expires_at"), &[&lease.namespace,&lease.owner_id,&generation,&owner]).map_err(|_|CloudError::Storage)?;
                Some(lease_row(&row)?)
            }else{None};
            tx.commit().map_err(|_|CloudError::Storage)?;Ok(result)
        })
    }

    /// Consume one encrypted result; a superseded execution request never retries automatically.
    pub fn management_response(&self, id: &str) -> CloudResult<Option<Vec<u8>>> {
        validate_id(id)?;
        self.with_client(|client|{
            let mut tx=client.transaction().map_err(|_|CloudError::Storage)?;
            if let Some(row)=tx.query_opt(&format!("DELETE FROM hosted_chat_requests WHERE id=$1 AND expires_at>{DB_NOW} AND response IS NOT NULL RETURNING response"),&[&id]).map_err(|_|CloudError::Storage)?{let response=row.get(0);tx.commit().map_err(|_|CloudError::Storage)?;return Ok(Some(response));}
            tx.query_opt(&format!("SELECT id FROM hosted_chat_requests WHERE id=$1 AND expires_at>{DB_NOW}"),&[&id]).map_err(|_|CloudError::Storage)?.ok_or(CloudError::Conflict)?;tx.commit().map_err(|_|CloudError::Storage)?;Ok(None)
        })
    }

    /// Admit a five-minute browser login with global and per-person bounds across replicas.
    pub fn create_oauth_flow(&self, id: &str, ciphertext: &[u8], scope: &str) -> CloudResult<()> {
        validate_id(id)?;
        validate_id(scope)?;
        if ciphertext.len() > MAX_CIPHERTEXT {
            return Err(CloudError::InvalidArgument);
        }
        self.with_client(|client|{
            let mut tx=client.transaction().map_err(|_|CloudError::Storage)?;
            tx.query_one("SELECT pg_advisory_xact_lock(hashtextextended('hosted-chat-browser-oauth',0))",&[]).map_err(|_|CloudError::Storage)?;
            tx.execute(&format!("WITH expired AS(SELECT record_id FROM hosted_chat_records WHERE namespace='oauth-browser' AND expires_at<{DB_NOW}-60 ORDER BY expires_at,record_id FOR UPDATE SKIP LOCKED LIMIT 16) DELETE FROM hosted_chat_records WHERE namespace='oauth-browser' AND record_id IN(SELECT record_id FROM expired)"),&[]).map_err(|_|CloudError::Storage)?;
            let counts=tx.query_one(&format!("SELECT COUNT(*)::bigint,COUNT(*) FILTER(WHERE admission_scope=$1)::bigint FROM hosted_chat_records WHERE namespace='oauth-browser' AND ciphertext IS NOT NULL AND expires_at>{DB_NOW}"), &[&scope]).map_err(|_|CloudError::Storage)?;
            if counts.get::<_,i64>(0)>=128 || counts.get::<_,i64>(1)>=4{return Err(CloudError::ResourceExhausted);}
            let changed=tx.execute(&format!("INSERT INTO hosted_chat_records(namespace,record_id,revision,ciphertext,expires_at,admission_scope) VALUES('oauth-browser',$1,1,$2,{DB_NOW}+300,$3) ON CONFLICT DO NOTHING"), &[&id,&ciphertext,&scope]).map_err(|_|CloudError::Storage)?;
            if changed!=1{return Err(CloudError::Conflict);}
            tx.commit().map_err(|_|CloudError::Storage)?;Ok(())
        })
    }
}

/// A configuration edit excludes new hosted task admission in the same SQL transaction.
pub(super) async fn guard_admission(
    conn: &mut diesel_async::AsyncPgConnection,
    mutations: &[colossus_cloud::storage::EntityMutation],
) -> std::result::Result<(), colossus_ports::StoreError> {
    use diesel::{
        sql_query,
        sql_types::{Bool, Text},
    };
    use diesel_async::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct Gate {
        #[diesel(sql_type=Bool)]
        blocked: bool,
    }
    let projects: std::collections::BTreeSet<_> = mutations
        .iter()
        .filter_map(|mutation| {
            if mutation.expected_revision != 0 {
                return None;
            }
            let colossus_cloud::storage::EntityValue::Task(task) = &mutation.value else {
                return None;
            };
            (task.node_id.starts_with("chat-") && mutation.value.status() == "queued")
                .then_some(mutation.key.project_id.as_str())
        })
        .collect();
    for project in projects {
        let gate=sql_query("SELECT expires_at>EXTRACT(EPOCH FROM clock_timestamp()) AS blocked FROM hosted_chat_project_gates WHERE project_id=$1 FOR SHARE").bind::<Text,_>(project).get_result::<Gate>(conn).await;
        match gate {
            Ok(gate) if gate.blocked => {
                return Err(colossus_ports::StoreError::Conflict {
                    stream_id: format!("hosted-chat-admission:{project}"),
                    expected: 0,
                    actual: 1,
                });
            }
            Ok(_) | Err(diesel::result::Error::NotFound) => {}
            Err(_) => {
                return Err(colossus_ports::StoreError::Adapter(
                    "hosted chat admission unavailable".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_id(value: &str) -> CloudResult<()> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(CloudError::InvalidArgument)
    } else {
        Ok(())
    }
}
fn validate_ttl(value: u64) -> CloudResult<i64> {
    if !(1..=3600).contains(&value) {
        return Err(CloudError::InvalidArgument);
    }
    integer(value)
}
fn integer(value: u64) -> CloudResult<i64> {
    i64::try_from(value).map_err(|_| CloudError::InvalidArgument)
}
fn positive(value: i64) -> CloudResult<u64> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(CloudError::Storage)
}
fn lease_row(row: &postgres::Row) -> CloudResult<ChatLease> {
    Ok(ChatLease {
        namespace: row.get(0),
        project_id: row.get(1),
        user_id: row.get(2),
        owner_id: row.get(3),
        generation: positive(row.get(4))?,
        expires_at: positive(row.get(5))?,
    })
}
