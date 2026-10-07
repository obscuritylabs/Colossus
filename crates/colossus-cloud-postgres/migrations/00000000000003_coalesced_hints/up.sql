-- Outbox entries are publication hints, never commands or released event authority.
-- Keep the newest pending hint for each scope; every source commit already publishes
-- its original notification. Repeated source output must not grow an unconsumed queue.
WITH duplicates AS (
    SELECT outbox_id,ROW_NUMBER() OVER(PARTITION BY project_id,scope_id ORDER BY outbox_id DESC) AS ordinal
    FROM delivery_outbox WHERE delivered_at IS NULL
)
UPDATE delivery_outbox SET delivered_at=clock_timestamp()
WHERE outbox_id IN(SELECT outbox_id FROM duplicates WHERE ordinal>1);
CREATE UNIQUE INDEX outbox_pending_scope ON delivery_outbox(project_id,scope_id) WHERE delivered_at IS NULL;
