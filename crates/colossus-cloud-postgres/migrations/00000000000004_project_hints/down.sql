DROP INDEX outbox_pending_project;
CREATE UNIQUE INDEX outbox_pending_scope ON delivery_outbox(project_id,scope_id) WHERE delivered_at IS NULL;
