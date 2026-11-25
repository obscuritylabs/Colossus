use colossus_contracts::{EventEnvelope, ProjectionWorkItem};
use colossus_ports::{MAX_STREAM_LIST_BATCH, MAX_STREAM_READ_BATCH, StoreError};

use crate::{
    driver::{Connection, Param, Row},
    schema::{MAX_SQL_SEQUENCE, decode, invalid, prefix_range, sql_limit, sql_sequence},
};

pub(crate) fn decode_event(row: &Row<'_, '_>, bytes: &[u8]) -> Result<EventEnvelope, StoreError> {
    let event: EventEnvelope = decode(bytes)?;
    if event.schema_version != 1
        || event.global_sequence == 0
        || event.stream_version == 0
        || event.stream_id.contains('\0')
        || row.sequence(0)? != event.global_sequence
        || row.bytes(1)?.as_ref() != event.stream_id.as_bytes()
        || row.sequence(2)? != event.stream_version
        || row.text(3)?.as_ref() != event.event_id
        || row.text(4)?.as_ref() != event.record_hash
    {
        return Err(invalid(
            "event envelope does not match its embedded journal indexes",
        ));
    }
    Ok(event)
}

pub(crate) fn event_at<T>(
    db: &Connection,
    sequence: u64,
    mut use_event: impl FnMut(EventEnvelope, &[u8]) -> Result<T, StoreError>,
) -> Result<Option<T>, StoreError> {
    if sequence > MAX_SQL_SEQUENCE {
        return Ok(None);
    }
    db.one(
        "SELECT sequence, stream_id, stream_version, event_id, record_hash, envelope
         FROM events WHERE sequence = ?1",
        &[sql_sequence(sequence)?],
        |row| {
            let bytes = row.bytes(5)?;
            use_event(decode_event(&row, &bytes)?, &bytes)
        },
    )
}

pub(crate) fn stream_version(db: &Connection, stream_id: &str) -> Result<u64, StoreError> {
    Ok(db
        .one(
            "SELECT version FROM streams WHERE stream_id = ?1",
            &[Param::Bytes(stream_id.as_bytes())],
            |row| row.sequence(0),
        )?
        .unwrap_or(0))
}

pub(crate) fn stream(
    db: &Connection,
    stream_id: &str,
    after: u64,
    limit: usize,
) -> Result<Vec<EventEnvelope>, StoreError> {
    if limit == 0 || after >= MAX_SQL_SEQUENCE {
        return Ok(Vec::new());
    }
    let durable = stream_version(db, stream_id)?;
    let mut events = Vec::new();
    db.visit(
        "SELECT sequence, stream_id, stream_version, event_id, record_hash, envelope
         FROM events WHERE stream_id = ?1 AND stream_version > ?2 ORDER BY stream_version LIMIT ?3",
        &[
            Param::Bytes(stream_id.as_bytes()),
            sql_sequence(after)?,
            sql_limit(limit),
        ],
        |row| {
            let event = decode_event(&row, &row.bytes(5)?)?;
            if event.stream_version != after + events.len() as u64 + 1 {
                return Err(invalid("embedded journal stream version gap"));
            }
            events.push(event);
            Ok(())
        },
    )?;
    if events.len() as u64 != row_count(durable.saturating_sub(after), limit) {
        return Err(invalid(
            "embedded journal stream does not match its durable version",
        ));
    }
    Ok(events)
}

pub(crate) fn stream_backwards(
    db: &Connection,
    stream_id: &str,
    before: Option<u64>,
    limit: usize,
) -> Result<Vec<EventEnvelope>, StoreError> {
    let limit = limit.min(MAX_STREAM_READ_BATCH);
    if limit == 0 || before.is_some_and(|version| version <= 1) {
        return Ok(Vec::new());
    }
    let durable = stream_version(db, stream_id)?;
    let upper = before
        .map_or(MAX_SQL_SEQUENCE, |version| version - 1)
        .min(durable);
    let mut events = Vec::new();
    db.visit(
        "SELECT sequence, stream_id, stream_version, event_id, record_hash, envelope
         FROM events WHERE stream_id = ?1 AND stream_version <= ?2
         ORDER BY stream_version DESC LIMIT ?3",
        &[
            Param::Bytes(stream_id.as_bytes()),
            sql_sequence(upper)?,
            sql_limit(limit),
        ],
        |row| {
            let event = decode_event(&row, &row.bytes(5)?)?;
            if upper.checked_sub(events.len() as u64) != Some(event.stream_version) {
                return Err(invalid("embedded journal backwards stream version gap"));
            }
            events.push(event);
            Ok(())
        },
    )?;
    if events.len() as u64 != row_count(upper, limit) {
        return Err(invalid(
            "embedded journal backwards stream does not match its durable version",
        ));
    }
    Ok(events)
}

pub(crate) fn list_streams(
    db: &Connection,
    prefix: &str,
    after: Option<&str>,
    limit: usize,
) -> Result<Vec<String>, StoreError> {
    let (start, end) = prefix_range(prefix, after)?;
    let mut streams = Vec::new();
    db.visit(
        "SELECT stream_id FROM streams WHERE stream_id >= ?1 AND stream_id < ?2
         ORDER BY stream_id LIMIT ?3",
        &[
            Param::Bytes(&start),
            Param::Bytes(&end),
            sql_limit(limit.min(MAX_STREAM_LIST_BATCH)),
        ],
        |row| {
            let id = String::from_utf8(row.bytes(0)?.into_owned())
                .map_err(|_| invalid("stream index contains a non-UTF-8 ID"))?;
            if id.contains('\0') || !id.starts_with(prefix) {
                return Err(invalid("stream index contains an invalid ID"));
            }
            streams.push(id);
            Ok(())
        },
    )?;
    Ok(streams)
}

pub(crate) fn global(
    db: &Connection,
    from: u64,
    limit: usize,
) -> Result<Vec<EventEnvelope>, StoreError> {
    if limit == 0 || from > MAX_SQL_SEQUENCE {
        return Ok(Vec::new());
    }
    let from = from.max(1);
    let (head, _) = super::schema::head(db)?;
    let mut events = Vec::new();
    db.visit(
        "SELECT sequence, stream_id, stream_version, event_id, record_hash, envelope
         FROM events WHERE sequence >= ?1 ORDER BY sequence LIMIT ?2",
        &[sql_sequence(from)?, sql_limit(limit)],
        |row| {
            let event = decode_event(&row, &row.bytes(5)?)?;
            if event.global_sequence != from + events.len() as u64 {
                return Err(invalid("embedded journal global sequence gap"));
            }
            events.push(event);
            Ok(())
        },
    )?;
    if events.len() as u64 != row_count((head + 1).saturating_sub(from), limit) {
        return Err(invalid(
            "embedded journal global read does not match its head",
        ));
    }
    Ok(events)
}

pub(crate) fn outbox(
    db: &Connection,
    from: u64,
    limit: usize,
) -> Result<Vec<ProjectionWorkItem>, StoreError> {
    if limit == 0 || from > MAX_SQL_SEQUENCE {
        return Ok(Vec::new());
    }
    let from = from.max(1);
    let (head, _) = super::schema::head(db)?;
    let mut items = Vec::new();
    db.visit(
        "SELECT o.sequence, o.event_id, COALESCE(e.event_id, '')
         FROM projection_outbox o LEFT JOIN events e ON e.sequence = o.sequence
         WHERE o.sequence >= ?1 ORDER BY o.sequence LIMIT ?2",
        &[sql_sequence(from)?, sql_limit(limit)],
        |row| {
            let sequence = row.sequence(0)?;
            let event_id = row.text(1)?;
            if sequence != from + items.len() as u64
                || event_id.is_empty()
                || event_id != row.text(2)?
            {
                return Err(invalid(
                    "projection outbox does not match the embedded journal",
                ));
            }
            items.push(ProjectionWorkItem {
                global_sequence: sequence,
                event_id: event_id.into_owned(),
            });
            Ok(())
        },
    )?;
    if items.len() as u64 != row_count((head + 1).saturating_sub(from), limit) {
        return Err(invalid(
            "projection outbox does not reach the embedded journal head",
        ));
    }
    Ok(items)
}

fn row_count(available: u64, limit: usize) -> u64 {
    available.min(u64::try_from(limit).unwrap_or(u64::MAX))
}
