//! Shared encrypted custody and distributed runtime placement. Local disk is cache only.
use super::*;
use chacha20poly1305::{
    KeyInit as _, XChaCha20Poly1305, XNonce,
    aead::{Aead as _, Payload},
};
use colossus_cloud_postgres::{ChatLease, ChatRecordWrite, ChatSqlStore, CloudDatabaseConfig};
use colossus_contracts::{CredentialError, VaultRecord};
use colossus_ports::{CredentialKey, CredentialVault, RuntimeExecutionFence, StoreError};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

pub(crate) struct SharedChat {
    pub sql: Arc<ChatSqlStore>,
    pub database: CloudDatabaseConfig,
    pub keys: colossus_cloud_chat::SharedChatKeyConfig,
    key: Zeroizing<[u8; 32]>,
}

impl SharedChat {
    pub(super) fn fence_reader(
        self: &Arc<Self>,
        inner: Arc<dyn CredentialVault>,
        lease: ChatLease,
    ) -> Arc<dyn CredentialVault> {
        Arc::new(LeaseBoundReader {
            shared: self.clone(),
            inner,
            lease,
        })
    }
    pub(crate) fn open(
        deployment: &ChatConfig,
        database: CloudDatabaseConfig,
    ) -> std::result::Result<Option<Arc<Self>>, &'static str> {
        let Some(shared) = &deployment.shared else {
            return Ok(None);
        };
        let value = Zeroizing::new(
            std::env::var(&deployment.wrapping_key_variable)
                .map_err(|_| "shared chat custody key unavailable")?,
        );
        let mut key = Zeroizing::new([0u8; 32]);
        hex::decode_to_slice(value.trim(), key.as_mut())
            .map_err(|_| "shared chat custody key invalid")?;
        let sql =
            ChatSqlStore::open(database.clone()).map_err(|_| "shared chat SQL unavailable")?;
        Ok(Some(Arc::new(Self {
            sql: Arc::new(sql),
            database,
            keys: shared.keys.clone(),
            key,
        })))
    }

    pub(super) fn vault(
        self: &Arc<Self>,
        scope: &str,
        lease: Option<ChatLease>,
    ) -> Arc<dyn CredentialVault> {
        Arc::new(SqlVault {
            shared: self.clone(),
            namespace: format!("vault-{}", hex::encode(Sha256::digest(scope.as_bytes()))),
            lease,
            gate: None,
        })
    }
    pub(crate) fn editing_vault(
        self: &Arc<Self>,
        scope: &str,
        project: &str,
        owner: &str,
    ) -> Arc<dyn CredentialVault> {
        Arc::new(SqlVault {
            shared: self.clone(),
            namespace: format!("vault-{}", hex::encode(Sha256::digest(scope.as_bytes()))),
            lease: None,
            gate: Some((project.into(), owner.into())),
        })
    }

    pub(super) fn runtime_storage(
        self: &Arc<Self>,
        namespace: &str,
        lease: &ChatLease,
    ) -> std::result::Result<
        (
            colossus_journal_postgres::PostgresJournalConfig,
            colossus_runtime::RuntimeSharedStorage,
        ),
        CloudError,
    > {
        use colossus_journal_postgres::{
            PostgresEventJournal, PostgresJournalConfig, PostgresJournalFence, PostgresTlsConfig,
        };
        let tls = match &self.database.tls {
            colossus_cloud_postgres::CloudDatabaseTls::Disabled => PostgresTlsConfig::Disabled,
            colossus_cloud_postgres::CloudDatabaseTls::WebpkiRoots => {
                PostgresTlsConfig::WebpkiRoots
            }
            colossus_cloud_postgres::CloudDatabaseTls::CustomCa { ca_pem_path } => {
                PostgresTlsConfig::CustomCa {
                    ca_pem_path: ca_pem_path.clone(),
                }
            }
        };
        // Runtime journals remain outside the cloud relational schema, which rejects
        // journal tables during startup. Namespaces share this dedicated runtime schema.
        let journal_schema = format!(
            "chat_runtime_{}",
            &hex::encode(Sha256::digest(self.database.schema.as_bytes()))[..32]
        );
        let mut config = PostgresJournalConfig::new(
            self.database.connection_variable.clone(),
            journal_schema,
            tls,
        )
        .map_err(|_| CloudError::InvalidArgument)?;
        config.runtime_namespace = namespace.into();
        config.statement_timeout_ms = self.database.statement_timeout_ms;
        let (keys, signer) = colossus_cloud_chat::shared_chat_keys(&self.keys, namespace)
            .map_err(|_| CloudError::Storage)?;
        let journal = Arc::new(
            PostgresEventJournal::open_fenced(
                config.clone(),
                keys.clone(),
                signer,
                PostgresJournalFence {
                    lease_schema: self.database.schema.clone(),
                    owner_id: lease.owner_id.clone(),
                    generation: lease.generation,
                },
            )
            .map_err(|_| CloudError::Storage)?,
        );
        let fence = Arc::new(SqlFence {
            sql: self.sql.clone(),
            lease: lease.clone(),
        });
        let durability = colossus_runtime::RuntimeSharedStorage::new(
            namespace.into(),
            journal.clone(),
            journal,
            keys,
            fence,
        )
        .map_err(|_| CloudError::Storage)?;
        Ok((config, durability))
    }

    fn codex_namespace(project: &str) -> String {
        format!("codex-{}", hex::encode(Sha256::digest(project.as_bytes())))
    }
    fn codex_binding(project: &str) -> String {
        format!(
            "cloud-chat-codex:v1:{}",
            hex::encode(Sha256::digest(project.as_bytes()))
        )
    }
    pub(super) fn codex_auth(
        self: &Arc<Self>,
        project: &str,
        lease: Option<ChatLease>,
    ) -> std::result::Result<colossus_runtime::CodexAuthStore, CloudError> {
        colossus_runtime::CodexAuthStore::shared(
            Arc::new(SqlCodexAuth {
                shared: self.clone(),
                namespace: Self::codex_namespace(project),
                lease,
                gate: None,
            }),
            Self::codex_binding(project),
        )
        .map_err(|_| CloudError::Storage)
    }

    pub(super) fn seal(
        &self,
        namespace: &str,
        id: &str,
        revision: u64,
        bytes: &[u8],
    ) -> std::result::Result<Vec<u8>, CloudError> {
        let mut nonce = [0u8; 24];
        getrandom::fill(&mut nonce).map_err(|_| CloudError::Storage)?;
        let aad = serde_json::to_vec(&("colossus-chat-sql-v1", namespace, id, revision))
            .map_err(|_| CloudError::Storage)?;
        let mut ciphertext = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| CloudError::Storage)?
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: bytes,
                    aad: &aad,
                },
            )
            .map_err(|_| CloudError::Storage)?;
        let mut encoded = Vec::with_capacity(ciphertext.len() + 25);
        encoded.push(1);
        encoded.extend_from_slice(&nonce);
        encoded.append(&mut ciphertext);
        Ok(encoded)
    }

    pub(super) fn unseal(
        &self,
        namespace: &str,
        id: &str,
        revision: u64,
        bytes: &[u8],
    ) -> std::result::Result<Zeroizing<Vec<u8>>, CloudError> {
        if bytes.len() < 41 || bytes[0] != 1 {
            return Err(CloudError::Storage);
        }
        let aad = serde_json::to_vec(&("colossus-chat-sql-v1", namespace, id, revision))
            .map_err(|_| CloudError::Storage)?;
        let plaintext = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| CloudError::Storage)?
            .decrypt(
                XNonce::from_slice(&bytes[1..25]),
                Payload {
                    msg: &bytes[25..],
                    aad: &aad,
                },
            )
            .map_err(|_| CloudError::Storage)?;
        Ok(Zeroizing::new(plaintext))
    }

    pub(super) fn record_id(key: &CredentialKey) -> String {
        let mut hash = Sha256::new();
        hash.update(key.purpose().as_bytes());
        hash.update([0]);
        hash.update(key.id().as_bytes());
        hex::encode(hash.finalize())
    }

    pub(super) fn configuration_namespace(project: &str) -> String {
        format!(
            "project-{}",
            hex::encode(Sha256::digest(project.as_bytes()))
        )
    }

    pub(crate) fn configuration(
        &self,
        project: &str,
        seed: &ChatProjectConfig,
    ) -> std::result::Result<ChatProjectConfig, CloudError> {
        let namespace = Self::configuration_namespace(project);
        let mut record = self.sql.read_record(&namespace, "configuration")?;
        if record.revision == 0 {
            let bytes = Zeroizing::new(serde_json::to_vec(seed).map_err(|_| CloudError::Storage)?);
            let ciphertext = self.seal(&namespace, "configuration", 1, &bytes)?;
            match self.sql.write_records(&[ChatRecordWrite {
                namespace: &namespace,
                record_id: "configuration",
                expected_revision: 0,
                ciphertext: Some(&ciphertext),
                ttl_seconds: None,
            }]) {
                Ok(_) | Err(CloudError::Conflict) => {}
                Err(error) => return Err(error),
            };
            record = self.sql.read_record(&namespace, "configuration")?;
        }
        let ciphertext = record.ciphertext.ok_or(CloudError::Storage)?;
        let plaintext = self.unseal(&namespace, "configuration", record.revision, &ciphertext)?;
        let configuration: ChatProjectConfig =
            serde_json::from_slice(&plaintext).map_err(|_| CloudError::Storage)?;
        if configuration.revision != record.revision {
            return Err(CloudError::Storage);
        }
        crate::chat_settings::validate_retained(&configuration, seed)?;
        Ok(configuration)
    }

    pub(crate) fn save_configuration(
        &self,
        project: &str,
        configuration: &ChatProjectConfig,
        extra: Option<(&CredentialKey, &VaultRecord)>,
        gate: Option<(&str, &str)>,
    ) -> std::result::Result<(), CloudError> {
        let namespace = Self::configuration_namespace(project);
        let previous = configuration
            .revision
            .checked_sub(1)
            .ok_or(CloudError::InvalidArgument)?;
        let plaintext =
            Zeroizing::new(serde_json::to_vec(configuration).map_err(|_| CloudError::Storage)?);
        let ciphertext = self.seal(
            &namespace,
            "configuration",
            configuration.revision,
            &plaintext,
        )?;
        let mut writes = vec![ChatRecordWrite {
            namespace: &namespace,
            record_id: "configuration",
            expected_revision: previous,
            ciphertext: Some(&ciphertext),
            ttl_seconds: None,
        }];
        let extra_namespace = format!(
            "vault-{}",
            hex::encode(Sha256::digest(
                format!("project/{project}/connections").as_bytes()
            ))
        );
        let extra_id = extra.map(|(key, _)| Self::record_id(key));
        let extra_record = extra_id
            .as_ref()
            .map(|id| self.sql.read_record(&extra_namespace, id))
            .transpose()?;
        let extra_ciphertext = if let Some((_, record)) = extra {
            Some(self.seal(
                &extra_namespace,
                extra_id.as_deref().ok_or(CloudError::Storage)?,
                extra_record.as_ref().ok_or(CloudError::Storage)?.revision + 1,
                record.expose(),
            )?)
        } else {
            None
        };
        if let Some(bytes) = &extra_ciphertext {
            writes.push(ChatRecordWrite {
                namespace: &extra_namespace,
                record_id: extra_id.as_deref().ok_or(CloudError::Storage)?,
                expected_revision: extra_record.as_ref().ok_or(CloudError::Storage)?.revision,
                ciphertext: Some(bytes),
                ttl_seconds: None,
            });
        }
        self.sql.write_records_guarded(&writes, None, gate)?;
        Ok(())
    }
}

struct SqlVault {
    shared: Arc<SharedChat>,
    namespace: String,
    lease: Option<ChatLease>,
    gate: Option<(String, String)>,
}
impl CredentialVault for SqlVault {
    fn read(
        &self,
        key: &CredentialKey,
    ) -> std::result::Result<Option<VaultRecord>, CredentialError> {
        if let Some(lease) = &self.lease {
            self.shared
                .sql
                .verify_lease(lease)
                .map_err(credential_error)?;
        }
        let id = SharedChat::record_id(key);
        let record = self
            .shared
            .sql
            .read_record(&self.namespace, &id)
            .map_err(credential_error)?;
        let Some(bytes) = record.ciphertext else {
            return Ok(None);
        };
        let plaintext = self
            .shared
            .unseal(&self.namespace, &id, record.revision, &bytes)
            .map_err(|_| CredentialError::Corrupt)?;
        let record = VaultRecord::new(plaintext.to_vec())?;
        if let Some(lease) = &self.lease {
            self.shared
                .sql
                .verify_lease(lease)
                .map_err(credential_error)?;
        }
        Ok(Some(record))
    }
    fn write(
        &self,
        key: &CredentialKey,
        record: &VaultRecord,
    ) -> std::result::Result<(), CredentialError> {
        let id = SharedChat::record_id(key);
        let previous = self
            .shared
            .sql
            .read_record(&self.namespace, &id)
            .map_err(credential_error)?;
        let ciphertext = self
            .shared
            .seal(&self.namespace, &id, previous.revision + 1, record.expose())
            .map_err(credential_error)?;
        self.shared
            .sql
            .write_records_guarded(
                &[ChatRecordWrite {
                    namespace: &self.namespace,
                    record_id: &id,
                    expected_revision: previous.revision,
                    ciphertext: Some(&ciphertext),
                    ttl_seconds: None,
                }],
                self.lease.as_ref(),
                self.gate
                    .as_ref()
                    .map(|(project, owner)| (project.as_str(), owner.as_str())),
            )
            .map_err(credential_error)?;
        Ok(())
    }
    fn delete(&self, key: &CredentialKey) -> std::result::Result<(), CredentialError> {
        let id = SharedChat::record_id(key);
        let previous = self
            .shared
            .sql
            .read_record(&self.namespace, &id)
            .map_err(credential_error)?;
        if previous.ciphertext.is_none() {
            return Ok(());
        }
        self.shared
            .sql
            .write_records_guarded(
                &[ChatRecordWrite {
                    namespace: &self.namespace,
                    record_id: &id,
                    expected_revision: previous.revision,
                    ciphertext: None,
                    ttl_seconds: None,
                }],
                self.lease.as_ref(),
                self.gate
                    .as_ref()
                    .map(|(project, owner)| (project.as_str(), owner.as_str())),
            )
            .map_err(credential_error)?;
        Ok(())
    }
}
fn credential_error(error: CloudError) -> CredentialError {
    if error == CloudError::Conflict {
        CredentialError::Busy
    } else {
        CredentialError::Unavailable
    }
}

pub(super) struct SqlFence {
    pub sql: Arc<ChatSqlStore>,
    pub lease: ChatLease,
}
impl RuntimeExecutionFence for SqlFence {
    fn verify(&self) -> std::result::Result<(), StoreError> {
        self.sql
            .verify_lease(&self.lease)
            .map_err(|_| StoreError::WriterLeaseHeld)
    }
}

pub(super) struct LeaseClaim {
    shared: Option<Arc<SharedChat>>,
    lease: Option<ChatLease>,
    retained: bool,
    renewal: Option<tokio::task::JoinHandle<()>>,
}
impl LeaseClaim {
    pub(super) fn new(shared: Option<Arc<SharedChat>>, lease: Option<ChatLease>) -> Self {
        let renewal = if let (Some(shared), Some(lease)) = (&shared, &lease) {
            let shared = shared.clone();
            let lease = lease.clone();
            Some(tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    if shared.sql.renew_lease(&lease, 30).is_err() {
                        break;
                    }
                }
            }))
        } else {
            None
        };
        Self {
            shared,
            lease,
            retained: false,
            renewal,
        }
    }
    pub(super) fn retain(&mut self) {
        self.retained = true;
    }
}
impl Drop for LeaseClaim {
    fn drop(&mut self) {
        if let Some(task) = self.renewal.take() {
            task.abort();
        }
        if !self.retained
            && let (Some(shared), Some(lease)) = (&self.shared, &self.lease)
        {
            let _ = shared.sql.release_lease(lease);
        }
    }
}

struct SqlCodexAuth {
    shared: Arc<SharedChat>,
    namespace: String,
    lease: Option<ChatLease>,
    gate: Option<(String, String)>,
}
impl colossus_codex_auth::CodexAuthStorage for SqlCodexAuth {
    fn read(
        &self,
    ) -> std::result::Result<
        colossus_codex_auth::CodexAuthRecord,
        colossus_codex_auth::CodexAuthError,
    > {
        if let Some(lease) = &self.lease {
            self.shared.sql.verify_lease(lease).map_err(codex_error)?;
        }
        let record = self
            .shared
            .sql
            .read_record(&self.namespace, "account")
            .map_err(codex_error)?;
        let bytes = record.ciphertext.ok_or_else(|| {
            colossus_codex_auth::CodexAuthError::Unavailable(
                "shared Codex account is unavailable".into(),
            )
        })?;
        let bytes = self
            .shared
            .unseal(&self.namespace, "account", record.revision, &bytes)
            .map_err(codex_error)?;
        if let Some(lease) = &self.lease {
            self.shared.sql.verify_lease(lease).map_err(codex_error)?;
        }
        Ok(colossus_codex_auth::CodexAuthRecord {
            revision: record.revision,
            bytes,
        })
    }
    fn compare_and_swap(
        &self,
        expected_revision: u64,
        bytes: &[u8],
    ) -> std::result::Result<u64, colossus_codex_auth::CodexAuthError> {
        let next = expected_revision
            .checked_add(1)
            .ok_or_else(|| codex_error(CloudError::InvalidArgument))?;
        let ciphertext = self
            .shared
            .seal(&self.namespace, "account", next, bytes)
            .map_err(codex_error)?;
        let revisions = self
            .shared
            .sql
            .write_records_guarded(
                &[ChatRecordWrite {
                    namespace: &self.namespace,
                    record_id: "account",
                    expected_revision,
                    ciphertext: Some(&ciphertext),
                    ttl_seconds: None,
                }],
                self.lease.as_ref(),
                self.gate
                    .as_ref()
                    .map(|(project, owner)| (project.as_str(), owner.as_str())),
            )
            .map_err(codex_error)?;
        revisions
            .first()
            .copied()
            .ok_or_else(|| codex_error(CloudError::Storage))
    }
}
fn codex_error(_: CloudError) -> colossus_codex_auth::CodexAuthError {
    colossus_codex_auth::CodexAuthError::Storage(
        "shared Codex custody unavailable or changed".into(),
    )
}

struct LeaseBoundReader {
    shared: Arc<SharedChat>,
    inner: Arc<dyn CredentialVault>,
    lease: ChatLease,
}
impl CredentialVault for LeaseBoundReader {
    fn read(
        &self,
        key: &CredentialKey,
    ) -> std::result::Result<Option<VaultRecord>, CredentialError> {
        self.shared
            .sql
            .verify_lease(&self.lease)
            .map_err(credential_error)?;
        let record = self.inner.read(key)?;
        self.shared
            .sql
            .verify_lease(&self.lease)
            .map_err(credential_error)?;
        Ok(record)
    }
    fn write(
        &self,
        _: &CredentialKey,
        _: &VaultRecord,
    ) -> std::result::Result<(), CredentialError> {
        Err(CredentialError::InvalidInput)
    }
    fn delete(&self, _: &CredentialKey) -> std::result::Result<(), CredentialError> {
        Err(CredentialError::InvalidInput)
    }
}

/// Explicit trusted-operator transfer from one selected native Codex account into
/// project-bound encrypted SQL custody. Source bytes are never returned or logged;
/// CAS prevents an import from overwriting another refreshed/reauthenticated account.
pub fn import_codex_auth(
    config: &crate::config::Config,
    project: &str,
    source: &std::path::Path,
    expected_revision: u64,
) -> std::result::Result<u64, &'static str> {
    config.validate()?;
    let deployment = config.chat.as_ref().ok_or("hosted chat unavailable")?;
    let project_config = deployment
        .projects
        .get(project)
        .ok_or("chat project unavailable")?;
    if !project_config.codex_auth_shared || project_config.codex_auth_path.is_some() {
        return Err("explicit shared Codex custody is required");
    }
    let shared = SharedChat::open(deployment, config.database.clone())?
        .ok_or("shared chat custody unavailable")?;
    let bytes = colossus_codex_auth::CodexAuthStore::at_path(source)
        .export_shared_record(&SharedChat::codex_binding(project))
        .map_err(|_| "selected Codex account could not be imported")?;
    let owner = format!("codex-import-{}", uuid::Uuid::now_v7().simple());
    shared
        .sql
        .claim_project_gate(project, &owner, 30)
        .map_err(|_| "chat configuration is being changed")?;
    let _gate = ImportGate {
        shared: shared.clone(),
        project: project.into(),
        owner: owner.clone(),
    };
    if !shared
        .sql
        .project_leases(project)
        .map_err(|_| "chat placement unavailable")?
        .is_empty()
    {
        return Err("stop hosted runtimes before importing a shared account");
    }
    let storage = SqlCodexAuth {
        shared,
        namespace: SharedChat::codex_namespace(project),
        lease: None,
        gate: Some((project.into(), owner)),
    };
    colossus_codex_auth::CodexAuthStorage::compare_and_swap(&storage, expected_revision, &bytes)
        .map_err(|_| "shared Codex account revision changed or custody unavailable")
}

struct ImportGate {
    shared: Arc<SharedChat>,
    project: String,
    owner: String,
}
impl Drop for ImportGate {
    fn drop(&mut self) {
        let _ = self
            .shared
            .sql
            .release_project_gate(&self.project, &self.owner);
    }
}
