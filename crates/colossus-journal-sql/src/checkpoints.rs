use colossus_contracts::{EventEnvelope, SecureAnchor, SecureAnchorStatus, SignedCheckpoint};
use colossus_journal_format::{checkpoint_message, utc_now};
use colossus_ports::{JournalPayloadProtection, StoreError};

use crate::{
    SqlEventJournal,
    driver::{Connection, Param},
    reads,
    schema::{decode, encode, head, invalid, sql_sequence},
    verification::verify_outbox_item,
};

pub(crate) const ANCHOR_FORMAT: u16 = 2;
const VERIFICATION_PROFILE: &str = "embedded-sql-full-v1";

pub(crate) fn attested(anchor: &SecureAnchor) -> bool {
    anchor.format_version == ANCHOR_FORMAT
        && anchor.status == SecureAnchorStatus::Verified
        && anchor.verification_profile.as_deref() == Some(VERIFICATION_PROFILE)
}

impl SqlEventJournal {
    pub(crate) fn load_anchor(&self) -> Result<Option<SecureAnchor>, StoreError> {
        if self.protection == JournalPayloadProtection::Plaintext {
            return Ok(None);
        }
        self.keys.load_anchor()
    }

    pub(crate) fn quarantine_anchor(&self) {
        if let Ok(Some(mut anchor)) = self.load_anchor() {
            anchor.status = SecureAnchorStatus::Quarantined;
            let _ = self.keys.store_anchor(&anchor);
        }
    }

    pub(crate) fn verify_anchor(
        &self,
        db: &Connection,
        head: u64,
        anchor: &SecureAnchor,
    ) -> Result<EventEnvelope, StoreError> {
        if anchor.sequence == 0 || anchor.sequence > head {
            return Err(invalid("secure anchor falls outside the embedded journal"));
        }
        let event = reads::event_at(db, anchor.sequence, |event, bytes| {
            self.verify_envelope(&event, bytes)?;
            Ok(event)
        })?
        .ok_or_else(|| invalid("secure anchor record is absent from the embedded journal"))?;
        if event.record_hash != anchor.hash {
            return Err(invalid(
                "secure anchor does not match its embedded journal record",
            ));
        }
        verify_outbox_item(db, &event)?;
        Ok(event)
    }

    pub(crate) fn checkpoint_at(
        &self,
        db: &Connection,
        sequence: u64,
        hash: &str,
    ) -> Result<Option<SignedCheckpoint>, StoreError> {
        db.one(
            "SELECT checkpoint FROM checkpoints WHERE sequence = ?1",
            &[sql_sequence(sequence)?],
            |row| self.verify_checkpoint(sequence, hash, &row.bytes(0)?),
        )
    }

    pub(crate) fn verify_checkpoints(
        &self,
        db: &Connection,
        after: u64,
    ) -> Result<Option<SignedCheckpoint>, StoreError> {
        let mut latest = None;
        db.visit(
            "SELECT c.sequence, COALESCE(e.record_hash, ''), c.checkpoint
             FROM checkpoints c LEFT JOIN events e ON e.sequence = c.sequence
             WHERE c.sequence > ?1 ORDER BY c.sequence",
            &[sql_sequence(after)?],
            |row| {
                latest = Some(self.verify_checkpoint(
                    row.sequence(0)?,
                    &row.text(1)?,
                    &row.bytes(2)?,
                )?);
                Ok(())
            },
        )?;
        Ok(latest)
    }

    fn verify_checkpoint(
        &self,
        sequence: u64,
        event_hash: &str,
        bytes: &[u8],
    ) -> Result<SignedCheckpoint, StoreError> {
        let checkpoint: SignedCheckpoint = decode(bytes)?;
        if self.protection == JournalPayloadProtection::Plaintext
            || sequence != checkpoint.global_sequence
            || event_hash.is_empty()
            || checkpoint.record_hash != event_hash
            || checkpoint.algorithm != "Ed25519"
            || checkpoint.key_id != self.signer.key_id()
        {
            return Err(invalid(
                "embedded journal checkpoint identity or boundary mismatch",
            ));
        }
        let signature = hex::decode(&checkpoint.signature)
            .map_err(|_| invalid("invalid embedded journal checkpoint signature encoding"))?;
        self.signer
            .verify(&checkpoint_message(sequence, event_hash), &signature)?;
        Ok(checkpoint)
    }

    pub(crate) fn persist_checkpoint(
        &self,
        db: &Connection,
    ) -> Result<Option<SignedCheckpoint>, StoreError> {
        let (sequence, hash) = head(db)?;
        if sequence == 0 {
            return Ok(None);
        }
        self.verify_local_head(db, sequence, &hash)?;
        let checkpoint = match self.checkpoint_at(db, sequence, &hash)? {
            Some(checkpoint) => checkpoint,
            None => {
                let signature = self.signer.sign(&checkpoint_message(sequence, &hash))?;
                let checkpoint = SignedCheckpoint {
                    global_sequence: sequence,
                    record_hash: hash.clone(),
                    key_id: self.signer.key_id().into(),
                    algorithm: "Ed25519".into(),
                    signature: hex::encode(signature),
                    created_at: utc_now()?,
                };
                let bytes = encode(&checkpoint)?;
                db.write(|| {
                    db.execute(
                        "INSERT INTO checkpoints (sequence, checkpoint) VALUES (?1, ?2)",
                        &[sql_sequence(sequence)?, Param::Bytes(&bytes)],
                    )
                })?;
                checkpoint
            }
        };

        // Persist the checkpoint first. An interruption can leave the external anchor
        // behind, so startup verifies the intervening tail before advancing it.
        self.store_checkpoint_anchor(&checkpoint)?;
        Ok(Some(checkpoint))
    }

    pub(crate) fn store_checkpoint_anchor(
        &self,
        checkpoint: &SignedCheckpoint,
    ) -> Result<(), StoreError> {
        self.keys
            .store_anchor(&SecureAnchor {
                format_version: ANCHOR_FORMAT,
                sequence: checkpoint.global_sequence,
                hash: checkpoint.record_hash.clone(),
                verification_profile: Some(VERIFICATION_PROFILE.into()),
                status: SecureAnchorStatus::Verified,
            })
            .map_err(|_| StoreError::OutcomeUnknown(
                "embedded journal checkpoint committed but its secure anchor could not be confirmed".into(),
            ))
    }
}
