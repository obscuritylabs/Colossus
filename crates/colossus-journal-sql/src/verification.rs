use std::collections::BTreeMap;

use colossus_contracts::{EventEnvelope, StartupVerificationMode, StartupVerificationReport};
use colossus_journal_format::{GENESIS_HASH, PersistedEnvelope, verify_payload};
use colossus_ports::{JournalPayloadProtection, StoreError, VerificationReport};
use serde_json::Value;

use crate::{
    SqlEventJournal,
    checkpoints::{ANCHOR_FORMAT, attested},
    driver::{Connection, Param},
    journal::JournalState,
    projections::{verify_positions, verify_records},
    reads::{decode_event, event_at},
    schema::{decode, head, invalid, sql_sequence},
};

struct StreamReplay {
    first_version: u64,
    tail_version: u64,
    tail_sequence: u64,
}

impl SqlEventJournal {
    pub(crate) fn decrypt_stored(
        &self,
        db: &Connection,
        expected: &EventEnvelope,
    ) -> Result<Value, StoreError> {
        event_at(db, expected.global_sequence, |envelope, bytes| {
            if envelope != *expected {
                return Err(invalid(
                    "event does not match its stored embedded journal envelope",
                ));
            }
            self.verify_envelope(&envelope, bytes)
        })?
        .ok_or_else(|| invalid("event is absent from the embedded journal"))
    }

    pub(crate) fn verify_envelope(
        &self,
        envelope: &EventEnvelope,
        bytes: &[u8],
    ) -> Result<Value, StoreError> {
        let persisted: PersistedEnvelope<'_> = decode(bytes)?;
        verify_payload(self.protection, self.keys.as_ref(), envelope, &persisted)
    }

    pub(crate) fn verify_full(&self, db: &Connection) -> Result<VerificationReport, StoreError> {
        let (sequence, hash) = checked_extent(db)?;
        self.verify_range(db, 0, GENESIS_HASH, sequence, &hash)?;
        let checkpoint = self.verify_checkpoints(db, 0)?;
        if let Some(anchor) = self.load_anchor()? {
            self.verify_anchor(db, sequence, &anchor)?;
        }
        verify_records(db)?;
        Ok(VerificationReport {
            event_count: sequence,
            last_sequence: sequence,
            last_hash: hash,
            checkpoint,
        })
    }

    pub(crate) fn startup(
        &self,
        state: &mut JournalState,
        mode: StartupVerificationMode,
    ) -> Result<StartupVerificationReport, StoreError> {
        let db = &state.db;
        let (sequence, hash) = checked_extent(db)?;
        let anchor = self.load_anchor()?;
        let trusted_anchor = anchor.as_ref().filter(|anchor| attested(anchor));
        let (path, from, count) = match (mode, self.protection, trusted_anchor) {
            (_, _, _) if sequence == 0 => {
                if anchor.is_some() || self.verify_checkpoints(db, 0)?.is_some() {
                    return Err(invalid("empty embedded journal has prior trust records"));
                }
                ("empty", None, 0)
            }
            (StartupVerificationMode::Incremental, JournalPayloadProtection::Plaintext, _) => {
                self.verify_local_head(db, sequence, &hash)?;
                self.verify_checkpoints(db, 0)?;
                ("local_integrity", Some(sequence), 1)
            }
            (
                StartupVerificationMode::Incremental,
                JournalPayloadProtection::Encrypted,
                Some(anchor),
            ) => {
                let boundary = self.verify_anchor(db, sequence, anchor)?;
                if anchor.sequence == sequence {
                    verify_stream_tail(db, &boundary.stream_id, boundary.stream_version, sequence)?;
                }
                if self
                    .checkpoint_at(db, anchor.sequence, &anchor.hash)?
                    .is_none()
                {
                    return Err(invalid(
                        "secure anchor has no durable embedded journal checkpoint",
                    ));
                }
                self.verify_range(db, anchor.sequence, &anchor.hash, sequence, &hash)?;
                state.checkpoint_sequence = anchor.sequence;
                if let Some(checkpoint) = self.verify_checkpoints(db, anchor.sequence)? {
                    self.store_checkpoint_anchor(&checkpoint)?;
                    state.checkpoint_sequence = checkpoint.global_sequence;
                }
                (
                    "incremental",
                    Some(anchor.sequence),
                    sequence - anchor.sequence + 1,
                )
            }
            _ => {
                self.verify_range(db, 0, GENESIS_HASH, sequence, &hash)?;
                self.verify_checkpoints(db, 0)?;
                if let Some(anchor) = &anchor {
                    self.verify_anchor(db, sequence, anchor)?;
                }
                verify_records(db)?;
                if self.protection == JournalPayloadProtection::Encrypted {
                    self.persist_checkpoint(db)?;
                    state.checkpoint_sequence = sequence;
                }
                let path = if mode == StartupVerificationMode::Full {
                    "full"
                } else {
                    "bootstrap_full"
                };
                (path, Some(1), sequence)
            }
        };

        let anchor_format_version = (state.checkpoint_sequence != 0).then_some(ANCHOR_FORMAT);
        Ok(StartupVerificationReport {
            configured_mode: mode,
            path: path.into(),
            verified_from_sequence: from,
            verified_through_sequence: sequence,
            verified_event_count: count,
            anchor_format_version,
        })
    }

    pub(crate) fn verify_local_head(
        &self,
        db: &Connection,
        sequence: u64,
        hash: &str,
    ) -> Result<(), StoreError> {
        let event = event_at(db, sequence, |event, bytes| {
            self.verify_envelope(&event, bytes)?;
            Ok(event)
        })?
        .ok_or_else(|| invalid("embedded journal head record is absent"))?;
        if event.record_hash != hash {
            return Err(invalid(
                "embedded journal head hash does not match its record",
            ));
        }
        verify_outbox_item(db, &event)?;
        verify_stream_tail(db, &event.stream_id, event.stream_version, sequence)
    }

    fn verify_range(
        &self,
        db: &Connection,
        after: u64,
        prior_hash: &str,
        head: u64,
        head_hash: &str,
    ) -> Result<(), StoreError> {
        let mut sequence = after;
        let mut previous_hash = prior_hash.to_owned();
        let mut streams = BTreeMap::<String, StreamReplay>::new();
        db.visit(
            "SELECT e.sequence, e.stream_id, e.stream_version, e.event_id, e.record_hash,
                    e.envelope, COALESCE(o.event_id, '')
             FROM events e LEFT JOIN projection_outbox o ON o.sequence = e.sequence
             WHERE e.sequence > ?1 ORDER BY e.sequence",
            &[sql_sequence(after)?],
            |row| {
                let bytes = row.bytes(5)?;
                let event = decode_event(&row, &bytes)?;
                if event.global_sequence != sequence + 1 || event.previous_hash != previous_hash {
                    return Err(invalid("embedded journal record chain is discontinuous"));
                }
                if event.event_id.is_empty() || row.text(6)?.as_ref() != event.event_id {
                    return Err(invalid(
                        "projection outbox item does not match its embedded journal event",
                    ));
                }
                self.verify_envelope(&event, &bytes)?;
                let stream = streams
                    .entry(event.stream_id)
                    .or_insert_with(|| StreamReplay {
                        first_version: event.stream_version,
                        tail_version: event.stream_version - 1,
                        tail_sequence: 0,
                    });
                if event.stream_version != stream.tail_version + 1 {
                    return Err(invalid("embedded journal stream replay has a version gap"));
                }
                stream.tail_version = event.stream_version;
                stream.tail_sequence = event.global_sequence;
                sequence = event.global_sequence;
                previous_hash = event.record_hash;
                Ok(())
            },
        )?;
        if sequence != head || previous_hash != head_hash {
            return Err(invalid("embedded journal replay does not match the head"));
        }
        if after == 0 {
            verify_full_stream_index(db, &mut streams)
        } else {
            for (id, stream) in streams {
                verify_stream_tail(db, &id, stream.tail_version, stream.tail_sequence)?;
                if stream.first_version > 1 {
                    let prior = db.one(
                        "SELECT sequence FROM events WHERE stream_id = ?1 AND stream_version = ?2",
                        &[
                            Param::Bytes(id.as_bytes()),
                            sql_sequence(stream.first_version - 1)?,
                        ],
                        |row| row.sequence(0),
                    )?;
                    if prior.is_none_or(|sequence| sequence > after) {
                        return Err(invalid(
                            "embedded journal stream tail does not meet its prior version",
                        ));
                    }
                }
            }
            Ok(())
        }
    }
}

fn checked_extent(db: &Connection) -> Result<(u64, String), StoreError> {
    let (sequence, hash) = head(db)?;
    let event_last = db.one(
        "SELECT sequence FROM events ORDER BY sequence DESC LIMIT 1",
        &[],
        |row| row.sequence(0),
    )?;
    let outbox_last = db.one(
        "SELECT sequence FROM projection_outbox ORDER BY sequence DESC LIMIT 1",
        &[],
        |row| row.sequence(0),
    )?;
    if event_last.unwrap_or(0) != sequence || outbox_last != event_last {
        return Err(invalid(
            "embedded journal head, events, and outbox have different extents",
        ));
    }
    if sequence == 0 {
        let stream = db.one("SELECT version FROM streams LIMIT 1", &[], |row| {
            row.sequence(0)
        })?;
        if hash != GENESIS_HASH || stream.is_some() {
            return Err(invalid(
                "empty embedded journal contains nonempty head or indexes",
            ));
        }
    } else {
        let first = db.one(
            "SELECT sequence FROM events ORDER BY sequence LIMIT 1",
            &[],
            |row| row.sequence(0),
        )?;
        let outbox_first = db.one(
            "SELECT sequence FROM projection_outbox ORDER BY sequence LIMIT 1",
            &[],
            |row| row.sequence(0),
        )?;
        if first != Some(1) || outbox_first != first {
            return Err(invalid(
                "embedded journal or outbox is missing its first record",
            ));
        }
    }
    verify_positions(db, sequence)?;
    Ok((sequence, hash))
}

pub(crate) fn verify_outbox_item(db: &Connection, event: &EventEnvelope) -> Result<(), StoreError> {
    let matches = db.one(
        "SELECT event_id FROM projection_outbox WHERE sequence = ?1",
        &[sql_sequence(event.global_sequence)?],
        |row| Ok(row.text(0)?.as_ref() == event.event_id),
    )?;
    if matches != Some(true) {
        return Err(invalid(
            "embedded journal event is absent from the projection outbox",
        ));
    }
    Ok(())
}

fn verify_full_stream_index(
    db: &Connection,
    streams: &mut BTreeMap<String, StreamReplay>,
) -> Result<(), StoreError> {
    db.visit(
        "SELECT stream_id, version, tail_sequence FROM streams",
        &[],
        |row| {
            let key = row.bytes(0)?;
            let name =
                std::str::from_utf8(&key).map_err(|_| invalid("invalid embedded stream ID"))?;
            let stream = streams
                .remove(name)
                .ok_or_else(|| invalid("embedded stream index has no corresponding events"))?;
            if stream.first_version != 1
                || stream.tail_version != row.sequence(1)?
                || stream.tail_sequence != row.sequence(2)?
            {
                return Err(invalid(
                    "embedded stream index differs from canonical replay",
                ));
            }
            Ok(())
        },
    )?;
    if !streams.is_empty() {
        return Err(invalid(
            "embedded stream index is missing canonical streams",
        ));
    }
    Ok(())
}

fn verify_stream_tail(
    db: &Connection,
    id: &str,
    version: u64,
    sequence: u64,
) -> Result<(), StoreError> {
    let tail = db.one(
        "SELECT version, tail_sequence FROM streams WHERE stream_id = ?1",
        &[Param::Bytes(id.as_bytes())],
        |row| Ok((row.sequence(0)?, row.sequence(1)?)),
    )?;
    if tail != Some((version, sequence)) {
        return Err(invalid(
            "embedded stream tail does not match canonical replay",
        ));
    }
    Ok(())
}
