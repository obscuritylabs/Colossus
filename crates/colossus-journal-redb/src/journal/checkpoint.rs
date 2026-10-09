use super::*;

impl RedbEventJournal {
    pub(super) fn checkpoint_sequence(&self) -> Result<u64, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        metadata
            .get("latest_checkpoint")
            .map_err(adapter_error)?
            .map(|value| {
                decode_journal_json::<SignedCheckpoint>(value.value())
                    .map(|checkpoint| checkpoint.global_sequence)
            })
            .transpose()
            .map(|sequence| sequence.unwrap_or(0))
    }

    pub(super) fn verify_checkpoint(
        &self,
        checkpoint: &SignedCheckpoint,
        event_hash: Option<&str>,
    ) -> Result<(), StoreError> {
        if event_hash != Some(checkpoint.record_hash.as_str()) {
            return Err(StoreError::Verification(
                "checkpoint does not match journal record".into(),
            ));
        }
        self.verify_checkpoint_signature(checkpoint)
    }

    pub(super) fn verify_checkpoint_signature(
        &self,
        checkpoint: &SignedCheckpoint,
    ) -> Result<(), StoreError> {
        if checkpoint.algorithm != "Ed25519" || checkpoint.key_id != self.signer.key_id() {
            return Err(StoreError::Verification(
                "checkpoint signer identity or algorithm mismatch".into(),
            ));
        }
        let signature = hex::decode(&checkpoint.signature).map_err(|_| {
            StoreError::Verification("invalid checkpoint signature encoding".into())
        })?;
        self.signer.verify(
            &checkpoint_message(checkpoint.global_sequence, &checkpoint.record_hash),
            &signature,
        )
    }

    pub(super) fn checkpoint_inner(&self) -> Result<Option<SignedCheckpoint>, StoreError> {
        if self.payload_protection == JournalPayloadProtection::Plaintext {
            return Ok(None);
        }
        let _guard = self.writer.lock().map_err(adapter_error)?;
        if self.is_recovery_mode() {
            return Err(StoreError::RecoveryMode);
        }
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let sequence = metadata
            .get("last_sequence")
            .map_err(adapter_error)?
            .map_or(Ok(0_u64), |value| decode_journal_json(value.value()))?;
        if sequence == 0 {
            return Ok(None);
        }
        let hash: String = metadata
            .get("last_hash")
            .map_err(adapter_error)?
            .map(|value| decode_journal_json(value.value()))
            .transpose()?
            .ok_or_else(|| StoreError::Verification("journal head hash is absent".into()))?;
        drop(metadata);
        drop(read);
        let signature = self.signer.sign(&checkpoint_message(sequence, &hash))?;
        let checkpoint = SignedCheckpoint {
            global_sequence: sequence,
            record_hash: hash.clone(),
            key_id: self.signer.key_id().to_owned(),
            algorithm: "Ed25519".into(),
            signature: hex::encode(signature),
            created_at: utc_now()?,
        };
        self.keys.store_anchor(&SecureAnchor {
            format_version: SECURE_ANCHOR_FORMAT_VERSION,
            sequence,
            hash: hash.clone(),
            verification_profile: Some(INCREMENTAL_VERIFICATION_PROFILE.into()),
            status: SecureAnchorStatus::Verified,
        })?;
        #[cfg(test)]
        if std::env::var("COLOSSUS_REDB_TEST_CRASH_SEQUENCE")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .is_none_or(|fault_sequence| fault_sequence == sequence)
        {
            crash_at_test_fault("after_anchor_before_checkpoint_commit");
        }
        let bytes = serde_json::to_vec(&checkpoint).map_err(adapter_error)?;
        let write = self.database.begin_write().map_err(adapter_error)?;
        {
            let mut metadata = write.open_table(METADATA).map_err(adapter_error)?;
            metadata
                .insert("latest_checkpoint", bytes.as_slice())
                .map_err(adapter_error)?;
        }
        write.commit().map_err(adapter_error)?;
        *self.last_checkpoint.lock().map_err(adapter_error)? = Instant::now();
        Ok(Some(checkpoint))
    }
}
