use crate::{CloudPostgresStore, entities, store::TransactionError};
use colossus_cloud::{
    CloudError, CloudResult,
    storage::{EntityKey, EntityKind},
};
use colossus_ports::StoreError;
use diesel::{
    sql_query,
    sql_types::{BigInt, Text},
};
use diesel_async::RunQueryDsl;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use zeroize::Zeroizing;

/// Retained per-entity/feed audit identity, without released message or credential content.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudAuditHead {
    /// Closed cloud table or released-feed namespace.
    pub entity_kind: String,
    /// Runtime/thread parent, empty for root resources.
    pub parent_id: String,
    /// Opaque entity/feed identity.
    pub id: String,
    /// Highest retained audit revision or source sequence.
    pub revision: u64,
    /// Exact retained chain head.
    pub chain_hash: String,
}
/// Metadata-only snapshot retained independently of PostgreSQL.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudAuditCheckpoint {
    /// Current checkpoint envelope version.
    pub format_version: u32,
    /// Exact project protected by the export.
    pub project_id: String,
    /// UTC operator export time.
    pub generated_at: String,
    /// Number of checked retained audit records.
    pub verified_audit_records: u64,
    /// Deterministically ordered retained heads.
    pub heads: Vec<CloudAuditHead>,
}
/// An Ed25519 signed checkpoint. The independent key must never be an auth/flow key.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCloudAuditCheckpoint {
    /// Metadata-only checkpoint contents.
    pub checkpoint: CloudAuditCheckpoint,
    /// SHA-256 of canonical serialized checkpoint contents.
    pub checkpoint_sha256: String,
    /// Public verification key, hexadecimal; operators must pin it independently.
    pub public_key: String,
    /// Ed25519 signature over the exact canonical checkpoint bytes.
    pub signature: String,
}

#[derive(diesel::QueryableByName)]
struct AuditRow {
    #[diesel(sql_type=Text)]
    entity_kind: String,
    #[diesel(sql_type=Text)]
    parent_id: String,
    #[diesel(sql_type=Text)]
    id: String,
    #[diesel(sql_type=BigInt)]
    revision: i64,
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
fn kind(table: &str) -> Option<EntityKind> {
    match table {
        "cloud_users" => Some(EntityKind::User),
        "user_identities" => Some(EntityKind::OidcIdentity),
        "local_credentials" => Some(EntityKind::LocalCredential),
        "control_plane_settings" => Some(EntityKind::Setting),
        "projects" => Some(EntityKind::Project),
        "project_memberships" => Some(EntityKind::Membership),
        "oidc_flows" => Some(EntityKind::AuthFlow),
        "hosts" => Some(EntityKind::Host),
        "runtime_agents" => Some(EntityKind::Node),
        "workspaces" => Some(EntityKind::Workspace),
        "conversation_threads" => Some(EntityKind::Thread),
        "conversation_messages" => Some(EntityKind::ThreadMessage),
        "thread_sources" => Some(EntityKind::SessionMapping),
        "tasks" => Some(EntityKind::Task),
        "commands" => Some(EntityKind::Command),
        "run_allocations" => Some(EntityKind::Run),
        "node_task_placements" => Some(EntityKind::NodeTask),
        "admission_counters" => Some(EntityKind::Admission),
        "enrollment_invitations" => Some(EntityKind::Invitation),
        "certificate_renewals" => Some(EntityKind::Renewal),
        _ => None,
    }
}
fn verify_row(project: &str, row: &AuditRow) -> Result<(), StoreError> {
    let computed = match row.entity_kind.as_str() {
        "released_events" => entities::digest(&serde_json::json!([
            "released_events",
            project,
            row.id,
            row.revision,
            row.content_digest,
            row.previous_hash
        ]))?,
        "sync_cursors" => entities::digest(&serde_json::json!([
            "sync_cursors",
            project,
            row.parent_id,
            row.id,
            row.revision,
            row.content_digest,
            row.previous_hash
        ]))?,
        table => {
            let key = EntityKey {
                kind: kind(table).ok_or_else(|| {
                    StoreError::Verification("unknown cloud audit namespace".into())
                })?,
                project_id: project.into(),
                parent_id: (!row.parent_id.is_empty()).then(|| row.parent_id.clone()),
                id: row.id.clone(),
            };
            entities::chain(
                &key,
                row.revision as u64,
                &row.actor,
                &row.operation,
                &row.content_digest,
                &row.previous_hash,
            )?
        }
    };
    if computed != row.chain_hash {
        return Err(StoreError::Verification(
            "cloud audit checkpoint chain mismatch".into(),
        ));
    }
    Ok(())
}
impl CloudPostgresStore {
    /// Verify retained project audit chains in a consistent snapshot and sign only heads.
    /// The explicit signing reference must resolve a separately provisioned 32-byte seed.
    pub async fn export_audit_checkpoint(
        &self,
        project: &str,
        signing_key_variable: &str,
    ) -> CloudResult<SignedCloudAuditCheckpoint> {
        colossus_cloud::validate_identifier(project)?;
        if signing_key_variable.is_empty()
            || signing_key_variable.len() > 128
            || !signing_key_variable
                .bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
            || ["AUTH", "SESSION", "OIDC", "ENCRYPT"]
                .iter()
                .any(|reserved| signing_key_variable.to_ascii_uppercase().contains(reserved))
        {
            return Err(CloudError::InvalidArgument);
        }
        let secret =
            Zeroizing::new(std::env::var(signing_key_variable).map_err(|_| CloudError::Storage)?);
        let decoded =
            Zeroizing::new(hex::decode(secret.as_str()).map_err(|_| CloudError::InvalidArgument)?);
        let seed =
            <&[u8; 32]>::try_from(decoded.as_slice()).map_err(|_| CloudError::InvalidArgument)?;
        let signing = SigningKey::from_bytes(seed);
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        let (heads,count)=conn.build_transaction().repeatable_read().read_only().run(async|conn|{
            let mut heads:BTreeMap<(String,String,String),CloudAuditHead>=BTreeMap::new();let mut count=0_u64;
            let(mut after_kind,mut after_parent,mut after_id,mut after_revision)=(String::new(),String::new(),String::new(),0_i64);
            loop{
                let rows=sql_query("SELECT entity_kind,parent_id,id,revision,actor,operation,content_digest,previous_hash,chain_hash FROM cloud_audit WHERE project_id=$1 AND (entity_kind,parent_id,id,revision)>($2,$3,$4,$5) ORDER BY entity_kind,parent_id,id,revision LIMIT 1000")
                    .bind::<Text,_>(project).bind::<Text,_>(&after_kind).bind::<Text,_>(&after_parent).bind::<Text,_>(&after_id).bind::<BigInt,_>(after_revision).load::<AuditRow>(conn).await?;
                if rows.is_empty(){break;}
                for row in rows {
                    count+=1;if count>2_000_000||heads.len()>100_000{return Err(TransactionError::Store(StoreError::Adapter("checkpoint export bound exceeded".into())));}
                    verify_row(project,&row)?;
                    let identity=(row.entity_kind.clone(),row.parent_id.clone(),row.id.clone());
                    let prior=heads.get(&identity);
                    if prior.map_or(!row.previous_hash.is_empty(),|head|head.chain_hash!=row.previous_hash||row.revision as u64<=head.revision){return Err(TransactionError::Store(StoreError::Verification("cloud audit chain gap".into())));}
                    if row.entity_kind!="sync_cursors"&&row.revision as u64!=prior.map_or(1,|head|head.revision+1){return Err(TransactionError::Store(StoreError::Verification("cloud audit revision gap".into())));}
                    after_kind=row.entity_kind.clone();after_parent=row.parent_id.clone();after_id=row.id.clone();after_revision=row.revision;
                    heads.insert(identity,CloudAuditHead{entity_kind:row.entity_kind,parent_id:row.parent_id,id:row.id,revision:row.revision as u64,chain_hash:row.chain_hash});
                }
            }
            Ok::<_,TransactionError>((heads.into_values().collect::<Vec<_>>(),count))
        }).await.map_err(|error|CloudError::from(error.into_store()))?;
        if heads.is_empty() {
            return Err(CloudError::NotFound);
        }
        let checkpoint = CloudAuditCheckpoint {
            format_version: 1,
            project_id: project.into(),
            generated_at: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .map_err(|_| CloudError::Storage)?,
            verified_audit_records: count,
            heads,
        };
        let bytes = crate::canonical::bytes(
            &serde_json::to_value(&checkpoint).map_err(|_| CloudError::Storage)?,
        )
        .map_err(|_| CloudError::Storage)?;
        let hash =
            entities::digest(&serde_json::to_value(&checkpoint).map_err(|_| CloudError::Storage)?)
                .map_err(|_| CloudError::Storage)?;
        Ok(SignedCloudAuditCheckpoint {
            checkpoint,
            checkpoint_sha256: hash,
            public_key: hex::encode(signing.verifying_key().as_bytes()),
            signature: hex::encode(signing.sign(&bytes).to_bytes()),
        })
    }
    /// Compare a separately retained signed anchor with the current database, detecting
    /// missing/changed anchored audit entries and resource/feed rollback below their heads.
    pub async fn verify_audit_checkpoint(
        &self,
        anchor: &SignedCloudAuditCheckpoint,
        pinned_public_key: &str,
    ) -> CloudResult<()> {
        verify_checkpoint_signature(anchor, pinned_public_key)?;
        if anchor.checkpoint.heads.len() > 100_000 {
            return Err(CloudError::ResourceExhausted);
        }
        let mut conn = self.pool.get().await.map_err(|_| CloudError::Storage)?;
        #[derive(diesel::QueryableByName)]
        struct Hash {
            #[diesel(sql_type=Text)]
            chain_hash: String,
            #[diesel(sql_type=Text)]
            operation: String,
        }
        #[derive(diesel::QueryableByName)]
        struct Revision {
            #[diesel(sql_type=BigInt)]
            revision: i64,
        }
        for head in &anchor.checkpoint.heads {
            let audit=sql_query("SELECT chain_hash,operation FROM cloud_audit WHERE project_id=$1 AND entity_kind=$2 AND parent_id=$3 AND id=$4 AND revision=$5").bind::<Text,_>(&anchor.checkpoint.project_id).bind::<Text,_>(&head.entity_kind).bind::<Text,_>(&head.parent_id).bind::<Text,_>(&head.id).bind::<BigInt,_>(entities::integer(head.revision).map_err(|_|CloudError::InvalidArgument)?).get_result::<Hash>(&mut conn).await.map_err(|_|CloudError::Storage)?;
            if audit.chain_hash != head.chain_hash {
                return Err(CloudError::Storage);
            }
            let revision=match head.entity_kind.as_str(){
                "released_events"=>sql_query("SELECT last_sequence AS revision FROM released_event_heads WHERE project_id=$1 AND scope_id=$2").bind::<Text,_>(&anchor.checkpoint.project_id).bind::<Text,_>(&head.id).get_result::<Revision>(&mut conn).await,
                "sync_cursors"=>sql_query("SELECT sequence AS revision FROM sync_cursors WHERE project_id=$1 AND source_id=$2 AND scope_id=$3").bind::<Text,_>(&anchor.checkpoint.project_id).bind::<Text,_>(&head.parent_id).bind::<Text,_>(&head.id).get_result::<Revision>(&mut conn).await,
                table=>{let kind=kind(table).ok_or(CloudError::InvalidArgument)?;sql_query(format!("SELECT revision FROM {} WHERE project_id=$1 AND parent_id=$2 AND id=$3",entities::table(kind))).bind::<Text,_>(&anchor.checkpoint.project_id).bind::<Text,_>(&head.parent_id).bind::<Text,_>(&head.id).get_result::<Revision>(&mut conn).await},
            };
            match revision {
                Ok(row) if row.revision as u64 >= head.revision => {}
                Err(diesel::result::Error::NotFound)
                    if head.entity_kind == "oidc_flows"
                        && audit.operation == "cloud.entity.consumed.v1" => {}
                _ => return Err(CloudError::Storage),
            }
        }
        Ok(())
    }
}
/// Verify an exported anchor against an independently pinned public key before use.
pub fn verify_checkpoint_signature(
    anchor: &SignedCloudAuditCheckpoint,
    pinned_public_key: &str,
) -> CloudResult<()> {
    if anchor.checkpoint.format_version != 1 || anchor.public_key != pinned_public_key {
        return Err(CloudError::InvalidArgument);
    }
    let key = hex::decode(pinned_public_key).map_err(|_| CloudError::InvalidArgument)?;
    let key = <&[u8; 32]>::try_from(key.as_slice()).map_err(|_| CloudError::InvalidArgument)?;
    let verifying = VerifyingKey::from_bytes(key).map_err(|_| CloudError::InvalidArgument)?;
    let signature = hex::decode(&anchor.signature).map_err(|_| CloudError::InvalidArgument)?;
    let signature = Signature::from_slice(&signature).map_err(|_| CloudError::InvalidArgument)?;
    let value =
        serde_json::to_value(&anchor.checkpoint).map_err(|_| CloudError::InvalidArgument)?;
    if entities::digest(&value).map_err(|_| CloudError::InvalidArgument)?
        != anchor.checkpoint_sha256
    {
        return Err(CloudError::InvalidArgument);
    }
    verifying
        .verify_strict(
            &crate::canonical::bytes(&value).map_err(|_| CloudError::InvalidArgument)?,
            &signature,
        )
        .map_err(|_| CloudError::InvalidArgument)
}
