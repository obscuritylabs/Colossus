-- Publication carries only a project hint; aggregate changing entities into one
-- pending project wakeup so retained publication metadata follows projects, not tokens.
WITH duplicates AS (
    SELECT outbox_id,ROW_NUMBER() OVER(PARTITION BY project_id ORDER BY outbox_id DESC) AS ordinal
    FROM delivery_outbox WHERE delivered_at IS NULL
)
UPDATE delivery_outbox SET delivered_at=clock_timestamp()
WHERE outbox_id IN(SELECT outbox_id FROM duplicates WHERE ordinal>1);
DROP INDEX outbox_pending_scope;
CREATE UNIQUE INDEX outbox_pending_project ON delivery_outbox(project_id) WHERE delivered_at IS NULL;
