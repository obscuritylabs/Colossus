use super::*;
use colossus_contracts::SessionMessagePage;
use colossus_ports::SessionMessagePosition;

// The journal buffers at most this many envelopes, including non-message events.
const READ_BATCH: usize = 8;

impl EventSourcedSessionRepository {
    pub(super) fn read_message_page(
        &self,
        id: &str,
        before_sequence: Option<u64>,
        limit: usize,
        max_bytes: usize,
    ) -> Result<SessionMessagePage, StoreError> {
        validate_session_id(id)?;
        let stream = Self::stream(id);
        let creation = self.journal.read_stream_from(&stream, 0, 1)?;
        match creation.first() {
            None => return Err(StoreError::NotFound(format!("session {id}"))),
            Some(event) if event.event_type != SESSION_EVENT => {
                return Err(StoreError::Verification(
                    "session has no creation event".into(),
                ));
            }
            Some(_) => {}
        }
        let mut page = MessagePage::new(limit, max_bytes);
        if before_sequence.is_some_and(|sequence| sequence <= 1) {
            return Ok(page.finish(false));
        }
        let mut before_version = None;
        let mut before_message = before_sequence;
        if let Some(catalog) = &self.catalog
            && !self.journal.is_recovery_mode()
        {
            while let Some(positions) =
                catalog.message_positions(id, before_message, page.next_read_size())?
            {
                let Some(last) = positions.last() else {
                    if before_version.is_some() && before_message != Some(1) {
                        return Err(invalid_index());
                    }
                    return Ok(page.finish(false));
                };
                if positions.len() > page.next_read_size()
                    || positions
                        .iter()
                        .any(|p| p.sequence == 0 || p.stream_version <= 1)
                    || before_message.is_some_and(|cursor| positions[0].sequence >= cursor)
                    || (before_version.is_some()
                        && positions[0].sequence.checked_add(1) != before_message)
                    || positions.windows(2).any(|pair| {
                        pair[1].sequence.checked_add(1) != Some(pair[0].sequence)
                            || pair[0].stream_version <= pair[1].stream_version
                    })
                {
                    return Err(invalid_index());
                }
                if self.read_indexed_messages(&stream, id, &positions, &mut page)? {
                    return Ok(page.finish(true));
                }
                before_version = Some(last.stream_version);
                before_message = Some(last.sequence);
            }
        }
        loop {
            let events = self.journal.read_stream_backwards(
                &stream,
                before_version,
                page.next_read_size(),
            )?;
            let Some(last) = events.last() else {
                return Ok(page.finish(false));
            };
            if before_version.is_some_and(|cursor| last.stream_version >= cursor) {
                return Err(StoreError::Verification(
                    "session stream cursor did not advance".into(),
                ));
            }
            before_version = Some(last.stream_version);
            for event in events
                .iter()
                .filter(|event| event.event_type == MESSAGE_EVENT)
            {
                let message = message_from_event(self.journal.as_ref(), id, event)?;
                if before_message.is_none_or(|upper| message.sequence < upper)
                    && page.push(message)?
                {
                    return Ok(page.finish(true));
                }
            }
        }
    }

    fn read_indexed_messages(
        &self,
        stream: &str,
        id: &str,
        mut positions: &[SessionMessagePosition],
        page: &mut MessagePage,
    ) -> Result<bool, StoreError> {
        while let Some(first) = positions.first() {
            // Coalesce nearby coordinates into one bounded read, even across title/tool markers.
            let count = positions
                .iter()
                .take_while(|position| {
                    first.stream_version - position.stream_version < READ_BATCH as u64
                })
                .count();
            let (batch, rest) = positions.split_at(count);
            let oldest = batch[count - 1].stream_version;
            let span = (first.stream_version - oldest + 1) as usize;
            let events = self.journal.read_stream_from(stream, oldest - 1, span)?;
            if events.len() != span {
                return Err(invalid_index());
            }
            for position in batch {
                let event = &events[(position.stream_version - oldest) as usize];
                if event.stream_id != stream
                    || event.stream_version != position.stream_version
                    || event.event_type != MESSAGE_EVENT
                {
                    return Err(invalid_index());
                }
                let message = message_from_event(self.journal.as_ref(), id, event)?;
                if message.sequence != position.sequence {
                    return Err(invalid_index());
                }
                if page.push(message)? {
                    return Ok(true);
                }
            }
            positions = rest;
        }
        Ok(false)
    }
}

fn invalid_index() -> StoreError {
    StoreError::Verification("session message index disagrees with the journal".into())
}

struct MessagePage {
    messages: Vec<SessionMessage>,
    limit: usize,
    max_bytes: usize,
    bytes: usize,
}

impl MessagePage {
    fn new(limit: usize, max_bytes: usize) -> Self {
        Self {
            messages: Vec::new(),
            limit: limit.max(1),
            max_bytes,
            bytes: 0,
        }
    }

    fn next_read_size(&self) -> usize {
        // Read one additional event to distinguish a full page from end of history.
        self.limit
            .saturating_sub(self.messages.len())
            .saturating_add(1)
            .min(READ_BATCH)
    }

    fn push(&mut self, message: SessionMessage) -> Result<bool, StoreError> {
        let encoded =
            serde_json::to_vec(&message).map_err(|error| StoreError::Adapter(error.to_string()))?;
        if encoded.len() > self.max_bytes {
            return Err(StoreError::Adapter(format!(
                "session message {} exceeds the bounded page size",
                message.sequence
            )));
        }
        let bytes = self.bytes.saturating_add(encoded.len());
        if self.messages.len() == self.limit || bytes > self.max_bytes {
            return Ok(true);
        }
        self.messages.push(message);
        self.bytes = bytes;
        Ok(false)
    }

    fn finish(mut self, has_more: bool) -> SessionMessagePage {
        self.messages.reverse();
        SessionMessagePage {
            before_sequence: self.messages.first().map(|message| message.sequence),
            has_more,
            messages: self.messages,
        }
    }
}
