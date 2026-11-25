use super::*;

impl RedbEventJournal {
    pub(super) fn decrypt_stored_event(&self, event: &EventEnvelope) -> Result<Value, StoreError> {
        let read = self.database.begin_read().map_err(adapter_error)?;
        let table = read.open_table(EVENTS).map_err(adapter_error)?;
        let bytes = table
            .get(event.global_sequence)
            .map_err(adapter_error)?
            .ok_or_else(|| StoreError::Verification("event is absent from the journal".into()))?;
        let stored: EventEnvelope = decode_journal_json(bytes.value())?;
        if stored != *event {
            return Err(StoreError::Verification(
                "event does not match its persisted envelope".into(),
            ));
        }
        let persisted = decode_journal_json(bytes.value())?;
        self.verify_persisted_event(event, &persisted)
    }

    pub(super) fn verify_persisted_event(
        &self,
        envelope: &EventEnvelope,
        persisted: &PersistedEventEnvelope<'_>,
    ) -> Result<Value, StoreError> {
        verify_payload(
            self.payload_protection,
            self.keys.as_ref(),
            envelope,
            persisted,
        )
    }
}
