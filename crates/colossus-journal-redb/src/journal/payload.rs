use super::*;

impl RedbEventJournal {
    pub(super) fn encrypt_payload(
        &self,
        envelope: &EventEnvelope,
        plaintext: &[u8],
    ) -> Result<EncryptedPayload, StoreError> {
        if self.payload_protection == JournalPayloadProtection::Plaintext {
            return Ok(EncryptedPayload {
                key_id: "none".into(),
                algorithm: PLAINTEXT_PAYLOAD_ALGORITHM.into(),
                nonce: String::new(),
                ciphertext: hex::encode(plaintext),
                plaintext_hash: sha256_hex(plaintext),
            });
        }
        let (key_id, key) = self.keys.active_key()?;
        let mut nonce = [0_u8; 24];
        getrandom::fill(&mut nonce).map_err(adapter_error)?;
        let aad = serde_json::to_vec(&associated_data(envelope)).map_err(adapter_error)?;
        let cipher = XChaCha20Poly1305::new((&key).into());
        let ciphertext = cipher
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
            algorithm: ENCRYPTED_PAYLOAD_ALGORITHM.into(),
            nonce: hex::encode(nonce),
            ciphertext: hex::encode(ciphertext),
            plaintext_hash: sha256_hex(plaintext),
        })
    }

    pub(super) fn decrypt_persisted(
        &self,
        event: &EventEnvelope,
        persisted: &PersistedEventEnvelope,
    ) -> Result<Vec<u8>, StoreError> {
        if self.payload_protection == JournalPayloadProtection::Plaintext {
            if event.payload.algorithm != PLAINTEXT_PAYLOAD_ALGORITHM
                || event.payload.key_id != "none"
                || !event.payload.nonce.is_empty()
            {
                return Err(StoreError::Verification(format!(
                    "event {} payload does not match plaintext journal protection",
                    event.event_id
                )));
            }
            return hex::decode(&event.payload.ciphertext).map_err(|_| {
                StoreError::Verification(format!(
                    "event {} plaintext payload encoding is invalid",
                    event.event_id
                ))
            });
        }
        if event.payload.algorithm != ENCRYPTED_PAYLOAD_ALGORITHM {
            return Err(StoreError::Verification(format!(
                "unsupported payload algorithm {}",
                event.payload.algorithm
            )));
        }
        let key = self.keys.key_by_id(&event.payload.key_id)?;
        let nonce = hex::decode(&event.payload.nonce)
            .map_err(|_| StoreError::Verification("invalid XChaCha20 nonce encoding".into()))?;
        let nonce: [u8; 24] = nonce
            .try_into()
            .map_err(|_| StoreError::Verification("invalid XChaCha20 nonce length".into()))?;
        let ciphertext = hex::decode(&event.payload.ciphertext)
            .map_err(|_| StoreError::Verification("invalid encrypted payload encoding".into()))?;
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

    pub(super) fn load_persisted(
        &self,
        event: &EventEnvelope,
    ) -> Result<PersistedEventEnvelope, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(EVENTS).map_err(adapter_error)?;
        let bytes = table
            .get(event.global_sequence)
            .map_err(adapter_error)?
            .ok_or_else(|| {
                StoreError::Verification(format!(
                    "event {} is absent from the journal",
                    event.event_id
                ))
            })?;
        let stored: EventEnvelope = decode_journal_json(bytes.value())?;
        if stored != *event {
            return Err(StoreError::Verification(format!(
                "event {} does not match its persisted envelope",
                event.event_id
            )));
        }
        decode_journal_json(bytes.value())
    }

    pub(super) fn verify_persisted_event(
        &self,
        envelope: &EventEnvelope,
        persisted: &PersistedEventEnvelope,
    ) -> Result<Value, StoreError> {
        let computed_hash = persisted_record_hash(persisted)?;
        if computed_hash != persisted.record_hash || envelope.record_hash != persisted.record_hash {
            return Err(StoreError::Verification(format!(
                "event {} record hash mismatch",
                envelope.event_id
            )));
        }
        let plaintext = self.decrypt_persisted(envelope, persisted)?;
        if sha256_hex(&plaintext) != envelope.payload.plaintext_hash {
            return Err(StoreError::Verification(format!(
                "event {} plaintext hash mismatch",
                envelope.event_id
            )));
        }
        decode_journal_json(&plaintext)
    }
}
