use super::*;

/// Canonical PostgreSQL journal adapter.
pub struct PostgresEventJournal {
    config: PostgresJournalConfig,
    tls_connector: Option<MakeRustlsConnect>,
    keys: Arc<dyn KeyProvider>,
    signer: Arc<dyn CheckpointSigner>,
    last_checkpoint: Mutex<Instant>,
    recovery_mode: AtomicBool,
    recovery_reason: Mutex<Option<String>>,
}

impl PostgresEventJournal {
    /// Connect, create the isolated schema, migrate idempotently, and verify before writes.
    pub fn open(
        config: PostgresJournalConfig,
        keys: Arc<dyn KeyProvider>,
        signer: Arc<dyn CheckpointSigner>,
    ) -> Result<Self, StoreError> {
        config.validate()?;
        let tls_connector = match &config.tls {
            PostgresTlsConfig::Disabled => None,
            tls => Some(Self::build_tls_connector(tls)?),
        };
        let journal = Self {
            config,
            tls_connector,
            keys,
            signer,
            last_checkpoint: Mutex::new(Instant::now()),
            recovery_mode: AtomicBool::new(false),
            recovery_reason: Mutex::new(None),
        };
        journal.migrate()?;
        let startup = journal.verify_inner().and_then(|report| {
            let checkpoint_sequence = report
                .checkpoint
                .as_ref()
                .map_or(0, |checkpoint| checkpoint.global_sequence);
            if report.last_sequence.saturating_sub(checkpoint_sequence) >= CHECKPOINT_INTERVAL {
                journal.checkpoint()?;
            }
            Ok(())
        });
        if let Err(error) = startup {
            journal.recovery_mode.store(true, Ordering::Release);
            *journal.recovery_reason.lock().map_err(adapter_error)? = Some(error.to_string());
        }
        Ok(journal)
    }

    /// Bounded reason startup entered read-only recovery mode.
    pub fn recovery_reason(&self) -> Result<Option<String>, StoreError> {
        Ok(self.recovery_reason.lock().map_err(adapter_error)?.clone())
    }

    /// Stable adapter diagnostic without a connection string or credential value.
    #[must_use]
    pub fn diagnostic(&self) -> Value {
        serde_json::json!({
            "adapter": "postgresql",
            "connection_variable": self.config.connection_variable,
            "schema": self.config.schema,
            "tls": match &self.config.tls {
                PostgresTlsConfig::WebpkiRoots => "webpki_roots",
                PostgresTlsConfig::CustomCa { .. } => "custom_ca",
                PostgresTlsConfig::Disabled => "disabled",
            },
            "statement_timeout_ms": self.config.statement_timeout_ms,
        })
    }

    fn pg_config(&self) -> Result<PgConfig, StoreError> {
        let value = std::env::var(&self.config.connection_variable).map_err(|_| {
            StoreError::KeyUnavailable(format!(
                "PostgreSQL connection variable {} is unset",
                self.config.connection_variable
            ))
        })?;
        let mut config = PgConfig::from_str(&value).map_err(|_| {
            StoreError::Adapter(format!(
                "PostgreSQL connection variable {} is invalid",
                self.config.connection_variable
            ))
        })?;
        config.application_name("colossus-journal");
        match &self.config.tls {
            PostgresTlsConfig::Disabled => {
                config.ssl_mode(SslMode::Disable);
                let is_local = config.get_hosts().iter().all(|host| match host {
                    Host::Tcp(host) => {
                        host.eq_ignore_ascii_case("localhost")
                            || host
                                .parse::<std::net::IpAddr>()
                                .is_ok_and(|address| address.is_loopback())
                    }
                    #[cfg(unix)]
                    Host::Unix(_) => true,
                });
                if !is_local {
                    return Err(StoreError::Adapter(
                        "disabling PostgreSQL TLS is restricted to loopback or Unix-socket connections"
                            .into(),
                    ));
                }
            }
            PostgresTlsConfig::WebpkiRoots | PostgresTlsConfig::CustomCa { .. } => {
                config.ssl_mode(SslMode::Require);
            }
        }
        Ok(config)
    }

    fn connect(&self) -> Result<Client, StoreError> {
        let config = self.pg_config()?;
        let mut client = match &self.config.tls {
            PostgresTlsConfig::Disabled => config.connect(NoTls).map_err(database_error)?,
            PostgresTlsConfig::WebpkiRoots | PostgresTlsConfig::CustomCa { .. } => config
                .connect(self.required_tls_connector()?)
                .map_err(database_error)?,
        };
        let timeout = self.config.statement_timeout_ms.to_string();
        client
            .execute(
                "SELECT set_config('statement_timeout', $1, false), set_config('lock_timeout', $1, false)",
                &[&timeout],
            )
            .map_err(database_error)?;
        client
            .batch_execute(&format!("SET search_path TO \"{}\"", self.config.schema))
            .map_err(database_error)?;
        Ok(client)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        let mut client = self.pg_config().and_then(|config| match &self.config.tls {
            PostgresTlsConfig::Disabled => config.connect(NoTls).map_err(database_error),
            PostgresTlsConfig::WebpkiRoots | PostgresTlsConfig::CustomCa { .. } => config
                .connect(self.required_tls_connector()?)
                .map_err(database_error),
        })?;
        client
            .batch_execute(&format!(
                "CREATE SCHEMA IF NOT EXISTS \"{}\"; SET search_path TO \"{}\"; {TABLES}",
                self.config.schema, self.config.schema
            ))
            .map_err(database_error)
    }

    fn required_tls_connector(&self) -> Result<MakeRustlsConnect, StoreError> {
        self.tls_connector
            .clone()
            .ok_or_else(|| StoreError::Adapter("PostgreSQL TLS connector is not configured".into()))
    }

    pub(super) fn build_tls_connector(
        tls: &PostgresTlsConfig,
    ) -> Result<MakeRustlsConnect, StoreError> {
        let roots = match tls {
            PostgresTlsConfig::WebpkiRoots => RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            },
            PostgresTlsConfig::CustomCa { ca_pem_path } => {
                let bytes = fs::read(ca_pem_path).map_err(|_| {
                    StoreError::Adapter("PostgreSQL CA bundle is unreadable".into())
                })?;
                let mut roots = RootCertStore::empty();
                let mut count = 0_u64;
                for certificate in CertificateDer::pem_slice_iter(&bytes) {
                    roots
                        .add(certificate.map_err(|_| {
                            StoreError::Adapter("PostgreSQL CA bundle is invalid".into())
                        })?)
                        .map_err(|_| {
                            StoreError::Adapter("PostgreSQL CA bundle is invalid".into())
                        })?;
                    count += 1;
                }
                if count == 0 {
                    return Err(StoreError::Adapter(
                        "PostgreSQL CA bundle contains no certificates".into(),
                    ));
                }
                roots
            }
            PostgresTlsConfig::Disabled => {
                return Err(StoreError::Adapter(
                    "cannot build a PostgreSQL TLS connector while TLS is disabled".into(),
                ));
            }
        };
        let config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .map_err(|_| {
                    StoreError::Adapter("PostgreSQL TLS protocol configuration failed".into())
                })?
                .with_root_certificates(roots)
                .with_no_client_auth();
        Ok(MakeRustlsConnect::new(config))
    }

    fn encrypt_payload(
        &self,
        envelope: &EventEnvelope,
        plaintext: &[u8],
    ) -> Result<EncryptedPayload, StoreError> {
        let (key_id, key) = self.keys.active_key()?;
        let mut nonce = [0_u8; 24];
        getrandom::fill(&mut nonce).map_err(adapter_error)?;
        let aad = serde_json::to_vec(&associated_data(envelope)).map_err(adapter_error)?;
        let ciphertext = XChaCha20Poly1305::new((&key).into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(adapter_error)?;
        Ok(EncryptedPayload {
            key_id,
            algorithm: "XChaCha20-Poly1305".into(),
            nonce: hex::encode(nonce),
            ciphertext: hex::encode(ciphertext),
            plaintext_hash: sha256_hex(plaintext),
        })
    }

    fn decrypt_persisted(
        &self,
        event: &EventEnvelope,
        persisted: &PersistedEventEnvelope,
    ) -> Result<Vec<u8>, StoreError> {
        if event.payload.algorithm != "XChaCha20-Poly1305" {
            return Err(StoreError::Verification(format!(
                "unsupported payload algorithm {}",
                event.payload.algorithm
            )));
        }
        let key = self.keys.key_by_id(&event.payload.key_id)?;
        let nonce: [u8; 24] = hex::decode(&event.payload.nonce)
            .map_err(adapter_error)?
            .try_into()
            .map_err(|_| StoreError::Verification("invalid XChaCha20 nonce length".into()))?;
        let ciphertext = hex::decode(&event.payload.ciphertext).map_err(adapter_error)?;
        let aad =
            serde_json::to_vec(&persisted_associated_data(persisted)).map_err(adapter_error)?;
        XChaCha20Poly1305::new((&key).into())
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| {
                StoreError::Verification(format!(
                    "event {} payload authentication failed",
                    event.event_id
                ))
            })
    }

    fn append_locked(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        if self.is_recovery_mode() {
            return Err(StoreError::RecoveryMode);
        }
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let mut client = self.connect()?;
        let mut transaction = client.transaction().map_err(database_error)?;
        let head = transaction
            .query_one(
                "SELECT last_sequence, last_hash FROM journal_metadata WHERE singleton = TRUE FOR UPDATE",
                &[],
            )
            .map_err(database_error)?;
        let mut sequence = to_u64(head.get::<_, i64>(0), "journal head")?;
        let mut previous_hash = head.get::<_, String>(1);
        let mut batch_versions = BTreeMap::<String, u64>::new();
        let mut persisted = Vec::with_capacity(events.len());

        for event in events {
            let durable_version = if let Some(version) = batch_versions.get(&event.stream_id) {
                *version
            } else {
                transaction
                    .query_opt(
                        "SELECT stream_version FROM journal_stream_versions WHERE stream_id = $1",
                        &[&event.stream_id],
                    )
                    .map_err(database_error)?
                    .map_or(Ok(0), |row| to_u64(row.get::<_, i64>(0), "stream version"))?
            };
            if event.expected_stream_version != durable_version {
                return Err(StoreError::Conflict {
                    stream_id: event.stream_id,
                    expected: event.expected_stream_version,
                    actual: durable_version,
                });
            }
            sequence = sequence
                .checked_add(1)
                .ok_or_else(|| StoreError::Adapter("global sequence overflow".into()))?;
            let stream_version = durable_version
                .checked_add(1)
                .ok_or_else(|| StoreError::Adapter("stream version overflow".into()))?;
            let mut envelope = EventEnvelope {
                schema_version: 1,
                event_version: event.event_version,
                event_id: Uuid::now_v7().to_string(),
                global_sequence: sequence,
                stream_id: event.stream_id,
                stream_version,
                classification: event.classification,
                event_type: event.event_type,
                actor: event.actor,
                context: event.context,
                occurred_at: utc_now()?,
                payload: EncryptedPayload {
                    key_id: String::new(),
                    algorithm: String::new(),
                    nonce: String::new(),
                    ciphertext: String::new(),
                    plaintext_hash: String::new(),
                },
                previous_hash: previous_hash.clone(),
                record_hash: String::new(),
            };
            let plaintext = serde_json::to_vec(&event.payload).map_err(adapter_error)?;
            envelope.payload = self.encrypt_payload(&envelope, &plaintext)?;
            envelope.record_hash = record_hash(&envelope)?;
            previous_hash.clone_from(&envelope.record_hash);
            let encoded = serde_json::to_vec(&envelope).map_err(adapter_error)?;
            transaction
                .execute(
                    "INSERT INTO journal_events (global_sequence, event_id, stream_id, stream_version, envelope) VALUES ($1, $2, $3, $4, $5)",
                    &[
                        &to_i64(sequence, "global sequence")?,
                        &envelope.event_id,
                        &envelope.stream_id,
                        &to_i64(stream_version, "stream version")?,
                        &encoded,
                    ],
                )
                .map_err(database_error)?;
            transaction
                .execute(
                    "INSERT INTO journal_stream_versions (stream_id, stream_version) VALUES ($1, $2) ON CONFLICT (stream_id) DO UPDATE SET stream_version = EXCLUDED.stream_version",
                    &[&envelope.stream_id, &to_i64(stream_version, "stream version")?],
                )
                .map_err(database_error)?;
            transaction
                .execute(
                    "INSERT INTO projection_outbox (global_sequence, event_id) VALUES ($1, $2)",
                    &[&to_i64(sequence, "global sequence")?, &envelope.event_id],
                )
                .map_err(database_error)?;
            batch_versions.insert(envelope.stream_id.clone(), stream_version);
            persisted.push(envelope);
        }
        transaction
            .execute(
                "UPDATE journal_metadata SET last_sequence = $1, last_hash = $2 WHERE singleton = TRUE",
                &[&to_i64(sequence, "global sequence")?, &previous_hash],
            )
            .map_err(database_error)?;
        #[cfg(test)]
        crash_at_test_fault("before_commit");
        transaction.commit().map_err(commit_error)?;
        #[cfg(test)]
        crash_at_test_fault("after_commit");
        Ok(persisted)
    }

    fn load_persisted(&self, event: &EventEnvelope) -> Result<PersistedEventEnvelope, StoreError> {
        let mut client = self.connect()?;
        let bytes = client
            .query_opt(
                "SELECT envelope FROM journal_events WHERE global_sequence = $1",
                &[&to_i64(event.global_sequence, "global sequence")?],
            )
            .map_err(database_error)?
            .ok_or_else(|| {
                StoreError::Verification(format!(
                    "event {} is absent from the journal",
                    event.event_id
                ))
            })?
            .get::<_, Vec<u8>>(0);
        let stored: EventEnvelope = serde_json::from_slice(&bytes).map_err(adapter_error)?;
        if stored != *event {
            return Err(StoreError::Verification(format!(
                "event {} does not match its persisted envelope",
                event.event_id
            )));
        }
        serde_json::from_slice(&bytes).map_err(adapter_error)
    }

    fn checkpoint_sequence(&self) -> Result<u64, StoreError> {
        let mut client = self.connect()?;
        client
            .query_one(
                "SELECT latest_checkpoint FROM journal_metadata WHERE singleton = TRUE",
                &[],
            )
            .map_err(database_error)?
            .get::<_, Option<Vec<u8>>>(0)
            .map(|bytes| {
                serde_json::from_slice::<SignedCheckpoint>(&bytes)
                    .map(|checkpoint| checkpoint.global_sequence)
                    .map_err(adapter_error)
            })
            .transpose()
            .map(|sequence| sequence.unwrap_or(0))
    }

    fn verify_checkpoint(
        &self,
        checkpoint: &SignedCheckpoint,
        event_hashes: &BTreeMap<u64, String>,
    ) -> Result<(), StoreError> {
        if checkpoint.algorithm != "Ed25519" || checkpoint.key_id != self.signer.key_id() {
            return Err(StoreError::Verification(
                "checkpoint signer identity or algorithm mismatch".into(),
            ));
        }
        if event_hashes.get(&checkpoint.global_sequence) != Some(&checkpoint.record_hash) {
            return Err(StoreError::Verification(
                "checkpoint does not match journal record".into(),
            ));
        }
        self.signer.verify(
            &checkpoint_message(checkpoint.global_sequence, &checkpoint.record_hash),
            &hex::decode(&checkpoint.signature).map_err(adapter_error)?,
        )
    }

    fn verify_inner(&self) -> Result<VerificationReport, StoreError> {
        let mut client = self.connect()?;
        let mut transaction = client.transaction().map_err(database_error)?;
        let metadata = transaction
            .query_one(
                "SELECT last_sequence, last_hash, latest_checkpoint FROM journal_metadata WHERE singleton = TRUE FOR SHARE",
                &[],
            )
            .map_err(database_error)?;
        let metadata_sequence = to_u64(metadata.get::<_, i64>(0), "journal head")?;
        let metadata_hash = metadata.get::<_, String>(1);
        let checkpoint_bytes = metadata.get::<_, Option<Vec<u8>>>(2);
        let mut expected_sequence = 1_u64;
        let mut previous_hash = ZERO_HASH.to_owned();
        let mut stream_versions = BTreeMap::<String, u64>::new();
        let mut event_hashes = BTreeMap::<u64, String>::new();

        for row in transaction
            .query(
                "SELECT global_sequence, envelope FROM journal_events ORDER BY global_sequence",
                &[],
            )
            .map_err(database_error)?
        {
            let sequence = to_u64(row.get::<_, i64>(0), "global sequence")?;
            if sequence != expected_sequence {
                return Err(StoreError::Verification(format!(
                    "global sequence gap: expected {expected_sequence}, got {sequence}"
                )));
            }
            let bytes = row.get::<_, Vec<u8>>(1);
            let persisted: PersistedEventEnvelope =
                serde_json::from_slice(&bytes).map_err(adapter_error)?;
            let envelope: EventEnvelope = serde_json::from_slice(&bytes).map_err(adapter_error)?;
            if envelope.global_sequence != sequence || envelope.previous_hash != previous_hash {
                return Err(StoreError::Verification(format!(
                    "event {} sequence or previous hash mismatch",
                    envelope.event_id
                )));
            }
            let expected_stream = stream_versions
                .get(&envelope.stream_id)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
            if envelope.stream_version != expected_stream {
                return Err(StoreError::Verification(format!(
                    "stream {} version mismatch",
                    envelope.stream_id
                )));
            }
            if persisted_record_hash(&persisted)? != persisted.record_hash
                || envelope.record_hash != persisted.record_hash
            {
                return Err(StoreError::Verification(format!(
                    "event {} record hash mismatch",
                    envelope.event_id
                )));
            }
            let plaintext = self.decrypt_persisted(&envelope, &persisted)?;
            if sha256_hex(&plaintext) != envelope.payload.plaintext_hash {
                return Err(StoreError::Verification(format!(
                    "event {} plaintext hash mismatch",
                    envelope.event_id
                )));
            }
            serde_json::from_slice::<Value>(&plaintext).map_err(adapter_error)?;
            let queued = transaction
                .query_opt(
                    "SELECT event_id FROM projection_outbox WHERE global_sequence = $1",
                    &[&to_i64(sequence, "global sequence")?],
                )
                .map_err(database_error)?
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "projection outbox record {sequence} is absent"
                    ))
                })?;
            if queued.get::<_, String>(0) != envelope.event_id {
                return Err(StoreError::Verification(format!(
                    "projection outbox record {sequence} targets a different event"
                )));
            }
            previous_hash.clone_from(&envelope.record_hash);
            event_hashes.insert(sequence, envelope.record_hash);
            stream_versions.insert(envelope.stream_id, envelope.stream_version);
            expected_sequence = expected_sequence.saturating_add(1);
        }

        let last_sequence = expected_sequence.saturating_sub(1);
        if metadata_sequence != last_sequence || metadata_hash != previous_hash {
            return Err(StoreError::Verification(
                "journal head metadata does not match event chain".into(),
            ));
        }
        let outbox_count = to_u64(
            transaction
                .query_one("SELECT COUNT(*) FROM projection_outbox", &[])
                .map_err(database_error)?
                .get::<_, i64>(0),
            "projection outbox count",
        )?;
        if outbox_count != last_sequence {
            return Err(StoreError::Verification(
                "projection outbox position does not match journal head".into(),
            ));
        }
        let mut durable_stream_versions = BTreeMap::new();
        for row in transaction
            .query(
                "SELECT stream_id, stream_version FROM journal_stream_versions ORDER BY stream_id",
                &[],
            )
            .map_err(database_error)?
        {
            durable_stream_versions.insert(
                row.get::<_, String>(0),
                to_u64(row.get::<_, i64>(1), "stream version")?,
            );
        }
        if durable_stream_versions != stream_versions {
            return Err(StoreError::Verification(
                "durable stream versions do not match journal replay".into(),
            ));
        }
        for row in transaction
            .query("SELECT projection, position FROM projection_positions", &[])
            .map_err(database_error)?
        {
            let projection = row.get::<_, String>(0);
            validate_projection(&projection)?;
            let position = to_u64(row.get::<_, i64>(1), "projection position")?;
            if position > last_sequence {
                return Err(StoreError::Verification(format!(
                    "projection {projection} position {position} is ahead of journal head {last_sequence}"
                )));
            }
        }
        for row in transaction
            .query(
                "SELECT projection, record_key, value FROM projection_records",
                &[],
            )
            .map_err(database_error)?
        {
            let projection = row.get::<_, String>(0);
            let key = row.get::<_, String>(1);
            validate_projection(&projection)?;
            validate_record_key(&key)?;
            serde_json::from_slice::<Value>(&row.get::<_, Vec<u8>>(2)).map_err(|error| {
                StoreError::Verification(format!(
                    "projection record {projection}/{key} is invalid JSON: {error}"
                ))
            })?;
        }
        let checkpoint = checkpoint_bytes
            .map(|bytes| serde_json::from_slice(&bytes).map_err(adapter_error))
            .transpose()?;
        if let Some(checkpoint) = &checkpoint {
            self.verify_checkpoint(checkpoint, &event_hashes)?;
        }
        if let Some((anchor_sequence, anchor_hash)) = self.keys.load_anchor()?
            && event_hashes.get(&anchor_sequence) != Some(&anchor_hash)
        {
            return Err(StoreError::Verification(
                "secure anchor is missing or differs from journal".into(),
            ));
        }
        transaction.commit().map_err(database_error)?;
        Ok(VerificationReport {
            event_count: last_sequence,
            last_sequence,
            last_hash: previous_hash,
            checkpoint,
        })
    }
}

impl EventJournal for PostgresEventJournal {
    fn append(&self, event: NewEvent) -> Result<EventEnvelope, StoreError> {
        self.append_batch(vec![event])?
            .pop()
            .ok_or_else(|| StoreError::Adapter("append returned no event".into()))
    }

    fn append_batch(&self, events: Vec<NewEvent>) -> Result<Vec<EventEnvelope>, StoreError> {
        let persisted = self.append_locked(events)?;
        let checkpoint_sequence = if persisted.is_empty() {
            0
        } else {
            self.checkpoint_sequence().map_err(|_| {
                StoreError::OutcomeUnknown(
                    "event append committed but checkpoint status is unavailable".into(),
                )
            })?
        };
        let count_due = persisted.last().is_some_and(|event| {
            event.global_sequence.saturating_sub(checkpoint_sequence) >= CHECKPOINT_INTERVAL
        });
        let age_due = self
            .last_checkpoint
            .lock()
            .map_err(adapter_error)?
            .elapsed()
            >= CHECKPOINT_MAX_AGE;
        if count_due || age_due {
            self.checkpoint().map_err(|error| {
                if persisted.is_empty() {
                    error
                } else {
                    StoreError::OutcomeUnknown(
                        "event append committed but checkpoint advancement failed".into(),
                    )
                }
            })?;
        }
        Ok(persisted)
    }

    fn read_stream(&self, stream_id: &str) -> Result<Vec<EventEnvelope>, StoreError> {
        let mut client = self.connect()?;
        client
            .query(
                "SELECT envelope FROM journal_events WHERE stream_id = $1 ORDER BY stream_version",
                &[&stream_id],
            )
            .map_err(database_error)?
            .into_iter()
            .map(|row| serde_json::from_slice(&row.get::<_, Vec<u8>>(0)).map_err(adapter_error))
            .collect()
    }

    fn read_stream_from(
        &self,
        stream_id: &str,
        after_version: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let limit = limit.min(MAX_STREAM_READ_BATCH);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let after_version = i64::try_from(after_version).unwrap_or(i64::MAX);
        let mut client = self.connect()?;
        client
            .query(
                "SELECT envelope FROM journal_events WHERE stream_id = $1 AND stream_version > $2 ORDER BY stream_version LIMIT $3",
                &[
                    &stream_id,
                    &after_version,
                    &bounded_limit(limit)?,
                ],
            )
            .map_err(database_error)?
            .into_iter()
            .map(|row| serde_json::from_slice(&row.get::<_, Vec<u8>>(0)).map_err(adapter_error))
            .collect()
    }

    fn read_stream_backwards(
        &self,
        stream_id: &str,
        before_version: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let limit = limit.min(MAX_STREAM_READ_BATCH);
        if limit == 0 || before_version.is_some_and(|version| version <= 1) {
            return Ok(Vec::new());
        }
        let mut client = self.connect()?;
        let rows = match before_version.and_then(|version| i64::try_from(version).ok()) {
            Some(before_version) => client.query(
                "SELECT envelope FROM journal_events WHERE stream_id = $1 AND stream_version < $2 ORDER BY stream_version DESC LIMIT $3",
                &[&stream_id, &before_version, &bounded_limit(limit)?],
            ),
            None => client.query(
                "SELECT envelope FROM journal_events WHERE stream_id = $1 ORDER BY stream_version DESC LIMIT $2",
                &[&stream_id, &bounded_limit(limit)?],
            ),
        }
        .map_err(database_error)?;
        rows.into_iter()
            .map(|row| serde_json::from_slice(&row.get::<_, Vec<u8>>(0)).map_err(adapter_error))
            .collect()
    }

    fn read_global(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let mut client = self.connect()?;
        client
            .query(
                "SELECT envelope FROM journal_events WHERE global_sequence >= $1 ORDER BY global_sequence LIMIT $2",
                &[
                    &to_i64(from_sequence, "global sequence")?,
                    &bounded_limit(limit)?,
                ],
            )
            .map_err(database_error)?
            .into_iter()
            .map(|row| serde_json::from_slice(&row.get::<_, Vec<u8>>(0)).map_err(adapter_error))
            .collect()
    }

    fn read_projection_work(
        &self,
        from_sequence: u64,
        limit: usize,
    ) -> Result<Vec<ProjectionWorkItem>, StoreError> {
        let mut client = self.connect()?;
        client
            .query(
                "SELECT global_sequence, event_id FROM projection_outbox WHERE global_sequence >= $1 ORDER BY global_sequence LIMIT $2",
                &[
                    &to_i64(from_sequence, "global sequence")?,
                    &bounded_limit(limit)?,
                ],
            )
            .map_err(database_error)?
            .into_iter()
            .map(|row| {
                Ok(ProjectionWorkItem {
                    global_sequence: to_u64(row.get::<_, i64>(0), "global sequence")?,
                    event_id: row.get(1),
                })
            })
            .collect()
    }

    fn head(&self) -> Result<(u64, String), StoreError> {
        let mut client = self.connect()?;
        let row = client
            .query_one(
                "SELECT last_sequence, last_hash FROM journal_metadata WHERE singleton = TRUE",
                &[],
            )
            .map_err(database_error)?;
        Ok((to_u64(row.get::<_, i64>(0), "journal head")?, row.get(1)))
    }

    fn decrypt_payload(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        let persisted = self.load_persisted(event)?;
        serde_json::from_slice(&self.decrypt_persisted(event, &persisted)?).map_err(adapter_error)
    }

    fn verify(&self) -> Result<VerificationReport, StoreError> {
        self.verify_inner()
    }

    fn is_recovery_mode(&self) -> bool {
        self.recovery_mode.load(Ordering::Acquire)
    }

    fn checkpoint(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        if self.is_recovery_mode() {
            return Err(StoreError::RecoveryMode);
        }
        let mut client = self.connect()?;
        let mut transaction = client.transaction().map_err(database_error)?;
        let row = transaction
            .query_one(
                "SELECT last_sequence, last_hash FROM journal_metadata WHERE singleton = TRUE FOR UPDATE",
                &[],
            )
            .map_err(database_error)?;
        let sequence = to_u64(row.get::<_, i64>(0), "journal head")?;
        if sequence == 0 {
            transaction.commit().map_err(database_error)?;
            return Ok(None);
        }
        let hash = row.get::<_, String>(1);
        let checkpoint = SignedCheckpoint {
            global_sequence: sequence,
            record_hash: hash.clone(),
            key_id: self.signer.key_id().to_owned(),
            algorithm: "Ed25519".into(),
            signature: hex::encode(self.signer.sign(&checkpoint_message(sequence, &hash))?),
            created_at: utc_now()?,
        };
        self.keys.store_anchor(sequence, &hash)?;
        #[cfg(test)]
        crash_at_test_fault("after_anchor_before_checkpoint_commit");
        transaction
            .execute(
                "UPDATE journal_metadata SET latest_checkpoint = $1 WHERE singleton = TRUE",
                &[&serde_json::to_vec(&checkpoint).map_err(adapter_error)?],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(commit_error)?;
        *self.last_checkpoint.lock().map_err(adapter_error)? = Instant::now();
        Ok(Some(checkpoint))
    }
}

impl ProjectionStore for PostgresEventJournal {
    fn position(&self, projection: &str) -> Result<u64, StoreError> {
        validate_projection(projection)?;
        let mut client = self.connect()?;
        client
            .query_opt(
                "SELECT position FROM projection_positions WHERE projection = $1",
                &[&projection],
            )
            .map_err(database_error)?
            .map_or(Ok(0), |row| {
                to_u64(row.get::<_, i64>(0), "projection position")
            })
    }

    fn get(&self, projection: &str, key: &str) -> Result<Option<Value>, StoreError> {
        validate_projection(projection)?;
        validate_record_key(key)?;
        let mut client = self.connect()?;
        client
            .query_opt(
                "SELECT value FROM projection_records WHERE projection = $1 AND record_key = $2",
                &[&projection, &key],
            )
            .map_err(database_error)?
            .map(|row| serde_json::from_slice(&row.get::<_, Vec<u8>>(0)).map_err(adapter_error))
            .transpose()
    }

    fn list(
        &self,
        projection: &str,
        key_prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StoreError> {
        validate_projection(projection)?;
        if key_prefix.contains('\0') {
            return Err(StoreError::Adapter(
                "projection key prefix may not contain NUL".into(),
            ));
        }
        let mut client = self.connect()?;
        client
            .query(
                "SELECT record_key, value FROM projection_records WHERE projection = $1 AND left(record_key, char_length($2)) = $2 ORDER BY record_key LIMIT $3",
                &[&projection, &key_prefix, &bounded_limit(limit)?],
            )
            .map_err(database_error)?
            .into_iter()
            .map(|row| {
                Ok((
                    row.get(0),
                    serde_json::from_slice(&row.get::<_, Vec<u8>>(1)).map_err(adapter_error)?,
                ))
            })
            .collect()
    }

    fn apply(&self, batch: ProjectionBatch) -> Result<(), StoreError> {
        validate_projection(&batch.projection)?;
        if batch.through_sequence <= batch.expected_position {
            return Err(StoreError::Adapter(
                "projection position must advance".into(),
            ));
        }
        for mutation in &batch.mutations {
            match mutation {
                ProjectionMutation::Upsert { key, .. } | ProjectionMutation::Delete { key } => {
                    validate_record_key(key)?;
                }
            }
        }
        let mut client = self.connect()?;
        let mut transaction = client.transaction().map_err(database_error)?;
        transaction
            .execute(
                "INSERT INTO projection_positions (projection, position) VALUES ($1, 0) ON CONFLICT (projection) DO NOTHING",
                &[&batch.projection],
            )
            .map_err(database_error)?;
        let actual = to_u64(
            transaction
                .query_one(
                    "SELECT position FROM projection_positions WHERE projection = $1 FOR UPDATE",
                    &[&batch.projection],
                )
                .map_err(database_error)?
                .get::<_, i64>(0),
            "projection position",
        )?;
        if actual != batch.expected_position {
            return Err(StoreError::Conflict {
                stream_id: format!("projection:{}", batch.projection),
                expected: batch.expected_position,
                actual,
            });
        }
        for mutation in batch.mutations {
            match mutation {
                ProjectionMutation::Upsert { key, value } => {
                    transaction
                        .execute(
                            "INSERT INTO projection_records (projection, record_key, value) VALUES ($1, $2, $3) ON CONFLICT (projection, record_key) DO UPDATE SET value = EXCLUDED.value",
                            &[
                                &batch.projection,
                                &key,
                                &serde_json::to_vec(&value).map_err(adapter_error)?,
                            ],
                        )
                        .map_err(database_error)?;
                }
                ProjectionMutation::Delete { key } => {
                    transaction
                        .execute(
                            "DELETE FROM projection_records WHERE projection = $1 AND record_key = $2",
                            &[&batch.projection, &key],
                        )
                        .map_err(database_error)?;
                }
            }
        }
        transaction
            .execute(
                "UPDATE projection_positions SET position = $2 WHERE projection = $1",
                &[
                    &batch.projection,
                    &to_i64(batch.through_sequence, "projection position")?,
                ],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(commit_error)
    }

    fn reset(&self, projection: &str) -> Result<(), StoreError> {
        validate_projection(projection)?;
        let mut client = self.connect()?;
        let mut transaction = client.transaction().map_err(database_error)?;
        transaction
            .execute(
                "DELETE FROM projection_records WHERE projection = $1",
                &[&projection],
            )
            .map_err(database_error)?;
        transaction
            .execute(
                "DELETE FROM projection_positions WHERE projection = $1",
                &[&projection],
            )
            .map_err(database_error)?;
        transaction.commit().map_err(commit_error)
    }
}
