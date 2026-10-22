//! Shared-schema partition migration and transaction-bound execution fences.

use super::*;

const PARTITION_MIGRATION: &str = r#"
DO $$ BEGIN
IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_schema=current_schema() AND table_name='journal_metadata' AND column_name='runtime_namespace') THEN
ALTER TABLE journal_metadata ADD COLUMN runtime_namespace TEXT NOT NULL DEFAULT '';
ALTER TABLE journal_events ADD COLUMN runtime_namespace TEXT NOT NULL DEFAULT '';
ALTER TABLE journal_stream_versions ADD COLUMN runtime_namespace TEXT NOT NULL DEFAULT '';
ALTER TABLE projection_outbox ADD COLUMN runtime_namespace TEXT NOT NULL DEFAULT '';
ALTER TABLE projection_positions ADD COLUMN runtime_namespace TEXT NOT NULL DEFAULT '';
ALTER TABLE projection_records ADD COLUMN runtime_namespace TEXT NOT NULL DEFAULT '';
ALTER TABLE projection_outbox DROP CONSTRAINT projection_outbox_global_sequence_fkey;
ALTER TABLE journal_metadata DROP CONSTRAINT journal_metadata_pkey;
ALTER TABLE journal_metadata ADD PRIMARY KEY(runtime_namespace,singleton);
ALTER TABLE journal_events DROP CONSTRAINT journal_events_pkey;
ALTER TABLE journal_events DROP CONSTRAINT journal_events_event_id_key;
ALTER TABLE journal_events DROP CONSTRAINT journal_events_stream_id_stream_version_key;
ALTER TABLE journal_events ADD PRIMARY KEY(runtime_namespace,global_sequence);
ALTER TABLE journal_events ADD UNIQUE(runtime_namespace,event_id);
ALTER TABLE journal_events ADD UNIQUE(runtime_namespace,stream_id,stream_version);
ALTER TABLE journal_stream_versions DROP CONSTRAINT journal_stream_versions_pkey;
ALTER TABLE journal_stream_versions ADD PRIMARY KEY(runtime_namespace,stream_id);
ALTER TABLE projection_outbox DROP CONSTRAINT projection_outbox_pkey;
ALTER TABLE projection_outbox DROP CONSTRAINT projection_outbox_event_id_key;
ALTER TABLE projection_outbox ADD PRIMARY KEY(runtime_namespace,global_sequence);
ALTER TABLE projection_outbox ADD UNIQUE(runtime_namespace,event_id);
ALTER TABLE projection_outbox ADD FOREIGN KEY(runtime_namespace,global_sequence) REFERENCES journal_events(runtime_namespace,global_sequence);
ALTER TABLE projection_positions DROP CONSTRAINT projection_positions_pkey;
ALTER TABLE projection_positions ADD PRIMARY KEY(runtime_namespace,projection);
ALTER TABLE projection_records DROP CONSTRAINT projection_records_pkey;
ALTER TABLE projection_records ADD PRIMARY KEY(runtime_namespace,projection,record_key);
END IF;
END $$;
ALTER TABLE journal_metadata ALTER COLUMN runtime_namespace SET DEFAULT COALESCE(current_setting('colossus.runtime_namespace',true),'');
ALTER TABLE journal_events ALTER COLUMN runtime_namespace SET DEFAULT COALESCE(current_setting('colossus.runtime_namespace',true),'');
ALTER TABLE journal_stream_versions ALTER COLUMN runtime_namespace SET DEFAULT COALESCE(current_setting('colossus.runtime_namespace',true),'');
ALTER TABLE projection_outbox ALTER COLUMN runtime_namespace SET DEFAULT COALESCE(current_setting('colossus.runtime_namespace',true),'');
ALTER TABLE projection_positions ALTER COLUMN runtime_namespace SET DEFAULT COALESCE(current_setting('colossus.runtime_namespace',true),'');
ALTER TABLE projection_records ALTER COLUMN runtime_namespace SET DEFAULT COALESCE(current_setting('colossus.runtime_namespace',true),'');
CREATE INDEX IF NOT EXISTS journal_partition_stream_idx ON journal_events(runtime_namespace,stream_id,stream_version);
CREATE INDEX IF NOT EXISTS journal_partition_prefix_idx ON journal_stream_versions(runtime_namespace,stream_id COLLATE "C");
"#;

impl PostgresJournalCore {
    pub(super) fn migrate_partitions(
        &self,
        transaction: &mut postgres::Transaction<'_>,
    ) -> Result<(), StoreError> {
        transaction
            .batch_execute(PARTITION_MIGRATION)
            .map_err(database_error)
    }

    pub(super) fn verify_transaction_fence(
        &self,
        transaction: &mut postgres::Transaction<'_>,
    ) -> Result<(), StoreError> {
        let Some(fence) = &self.fence else {
            return Ok(());
        };
        let query = format!(
            "SELECT generation FROM \"{}\".hosted_chat_leases WHERE namespace=$1 AND owner_id=$2 AND generation=$3 AND expires_at>EXTRACT(EPOCH FROM clock_timestamp()) FOR UPDATE",
            fence.lease_schema
        );
        let generation = to_i64(fence.generation, "execution generation")?;
        if transaction
            .query_opt(
                &query,
                &[&self.config.runtime_namespace, &fence.owner_id, &generation],
            )
            .map_err(database_error)?
            .is_none()
        {
            return Err(StoreError::WriterLeaseHeld);
        }
        Ok(())
    }

    pub(super) fn verify_open_fence(&self) -> Result<(), StoreError> {
        let mut client = self.connect()?;
        let mut transaction = client.transaction().map_err(database_error)?;
        self.verify_transaction_fence(&mut transaction)?;
        transaction.commit().map_err(commit_error)
    }
}
