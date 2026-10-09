use super::*;

impl RedbEventJournal {
    pub(super) fn stream_events_index_version(&self) -> Result<Option<u64>, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        metadata
            .get(STREAM_EVENTS_INDEX_KEY)
            .map_err(adapter_error)?
            .map(|value| decode_journal_json(value.value()))
            .transpose()
    }

    pub(super) fn ensure_stream_events_index(&self) -> Result<bool, StoreError> {
        let _guard = self.writer.lock().map_err(adapter_error)?;
        if self.stream_events_index_version()? == Some(STREAM_EVENTS_INDEX_VERSION) {
            return Ok(false);
        }
        let write = self.database.begin_write().map_err(adapter_error)?;
        let index_version = {
            let metadata = write.open_table(METADATA).map_err(adapter_error)?;
            metadata
                .get(STREAM_EVENTS_INDEX_KEY)
                .map_err(adapter_error)?
                .map(|value| decode_journal_json(value.value()))
                .transpose()?
        };
        if index_version == Some(STREAM_EVENTS_INDEX_VERSION) {
            return Ok(false);
        }
        if index_version.is_some() {
            return Err(StoreError::Verification(
                "stream event index version is unsupported".into(),
            ));
        }
        {
            let event_table = write.open_table(EVENTS).map_err(adapter_error)?;
            let mut stream_events = write.open_table(STREAM_EVENTS).map_err(adapter_error)?;
            if !stream_events.is_empty().map_err(adapter_error)? {
                return Err(StoreError::Verification(
                    "unversioned stream event index is not empty".into(),
                ));
            }
            for entry in event_table.iter().map_err(adapter_error)? {
                let (sequence, value) = entry.map_err(adapter_error)?;
                let sequence = sequence.value();
                let event: EventEnvelope = decode_journal_json(value.value())?;
                if event.global_sequence != sequence {
                    return Err(StoreError::Verification(format!(
                        "event {} global sequence does not match its key",
                        event.event_id
                    )));
                }
                if stream_events
                    .insert(&(event.stream_id.as_str(), event.stream_version), &sequence)
                    .map_err(adapter_error)?
                    .is_some()
                {
                    return Err(StoreError::Verification(format!(
                        "stream {} has duplicate version {}",
                        event.stream_id, event.stream_version
                    )));
                }
            }
        }
        {
            let mut metadata = write.open_table(METADATA).map_err(adapter_error)?;
            let version =
                serde_json::to_vec(&STREAM_EVENTS_INDEX_VERSION).map_err(adapter_error)?;
            metadata
                .insert(STREAM_EVENTS_INDEX_KEY, version.as_slice())
                .map_err(adapter_error)?;
        }
        write.commit().map_err(adapter_error)?;
        Ok(true)
    }

    pub(super) fn read_indexed_stream(
        &self,
        stream_id: &str,
        after_version: u64,
        limit: Option<usize>,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        if limit == Some(0) || after_version == u64::MAX {
            return Ok(Vec::new());
        }
        let start_version = after_version.saturating_add(1);
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let index_version = metadata
            .get(STREAM_EVENTS_INDEX_KEY)
            .map_err(adapter_error)?
            .map(|value| decode_journal_json(value.value()))
            .transpose()?;
        if index_version != Some(STREAM_EVENTS_INDEX_VERSION) {
            return Err(StoreError::Verification(
                "stream event index is unavailable".into(),
            ));
        }
        let stream_events = read.open_table(STREAM_EVENTS).map_err(adapter_error)?;
        let event_table = read.open_table(EVENTS).map_err(adapter_error)?;
        let stream_versions = read.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
        let mut events = Vec::with_capacity(limit.unwrap_or(0).min(MAX_STREAM_READ_BATCH));
        for entry in stream_events
            .range((stream_id, start_version)..=(stream_id, u64::MAX))
            .map_err(adapter_error)?
        {
            if limit.is_some_and(|limit| events.len() >= limit) {
                break;
            }
            let (key, sequence) = entry.map_err(adapter_error)?;
            let (indexed_stream, indexed_version) = key.value();
            let sequence = sequence.value();
            let persisted = event_table
                .get(sequence)
                .map_err(adapter_error)?
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "stream event index references absent event {sequence}"
                    ))
                })?;
            let event: EventEnvelope = decode_journal_json(persisted.value())?;
            if event.global_sequence != sequence
                || event.stream_id != indexed_stream
                || event.stream_version != indexed_version
            {
                return Err(StoreError::Verification(format!(
                    "stream event index entry {indexed_stream}/{indexed_version} is invalid"
                )));
            }
            events.push(event);
        }
        let mut expected_version = start_version;
        for event in &events {
            if event.stream_version != expected_version {
                return Err(StoreError::Verification(format!(
                    "stream {stream_id} index has a version gap at {expected_version}"
                )));
            }
            expected_version = expected_version.saturating_add(1);
        }
        let durable_version = stream_versions
            .get(stream_id)
            .map_err(adapter_error)?
            .map_or(0, |version| version.value());
        if limit.is_none_or(|limit| events.len() < limit)
            && events
                .last()
                .map_or(after_version.min(durable_version), |event| {
                    event.stream_version
                })
                != durable_version
        {
            return Err(StoreError::Verification(format!(
                "stream {stream_id} index does not reach durable version {durable_version}"
            )));
        }
        Ok(events)
    }

    pub(super) fn read_indexed_stream_backwards(
        &self,
        stream_id: &str,
        before_version: Option<u64>,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, StoreError> {
        let limit = limit.min(MAX_STREAM_READ_BATCH);
        if limit == 0 || before_version.is_some_and(|version| version <= 1) {
            return Ok(Vec::new());
        }
        let last_version = before_version.map_or(u64::MAX, |version| version.saturating_sub(1));
        let read = self.database.begin_read().map_err(adapter_error)?;
        let metadata = read.open_table(METADATA).map_err(adapter_error)?;
        let index_version = metadata
            .get(STREAM_EVENTS_INDEX_KEY)
            .map_err(adapter_error)?
            .map(|value| decode_journal_json(value.value()))
            .transpose()?;
        if index_version != Some(STREAM_EVENTS_INDEX_VERSION) {
            return Err(StoreError::Verification(
                "stream event index is unavailable".into(),
            ));
        }
        let stream_events = read.open_table(STREAM_EVENTS).map_err(adapter_error)?;
        let event_table = read.open_table(EVENTS).map_err(adapter_error)?;
        let stream_versions = read.open_table(STREAM_VERSIONS).map_err(adapter_error)?;
        let mut events = Vec::with_capacity(limit);
        for entry in stream_events
            .range((stream_id, 1)..=(stream_id, last_version))
            .map_err(adapter_error)?
            .rev()
        {
            if events.len() >= limit {
                break;
            }
            let (key, sequence) = entry.map_err(adapter_error)?;
            let (indexed_stream, indexed_version) = key.value();
            let sequence = sequence.value();
            let persisted = event_table
                .get(sequence)
                .map_err(adapter_error)?
                .ok_or_else(|| {
                    StoreError::Verification(format!(
                        "stream event index references absent event {sequence}"
                    ))
                })?;
            let event: EventEnvelope = decode_journal_json(persisted.value())?;
            if event.global_sequence != sequence
                || event.stream_id != indexed_stream
                || event.stream_version != indexed_version
            {
                return Err(StoreError::Verification(format!(
                    "stream event index entry {indexed_stream}/{indexed_version} is invalid"
                )));
            }
            events.push(event);
        }
        let durable_version = stream_versions
            .get(stream_id)
            .map_err(adapter_error)?
            .map_or(0, |version| version.value());
        let expected_first = before_version.map_or(durable_version, |version| {
            version.saturating_sub(1).min(durable_version)
        });
        if events.first().map(|event| event.stream_version)
            != (expected_first > 0).then_some(expected_first)
        {
            return Err(StoreError::Verification(format!(
                "stream {stream_id} reverse index does not begin at version {expected_first}"
            )));
        }
        if events.len() < limit
            && expected_first > 0
            && events.last().map(|event| event.stream_version) != Some(1)
        {
            return Err(StoreError::Verification(format!(
                "stream {stream_id} reverse index does not reach version 1"
            )));
        }
        for pair in events.windows(2) {
            if pair[0].stream_version != pair[1].stream_version.saturating_add(1) {
                return Err(StoreError::Verification(format!(
                    "stream {stream_id} reverse index has a version gap"
                )));
            }
        }
        Ok(events)
    }
}
