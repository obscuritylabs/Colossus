ALTER TABLE runtime_agents ADD COLUMN execution_kind TEXT NOT NULL DEFAULT 'connected'
    CHECK (execution_kind IN ('connected', 'hosted'));
CREATE INDEX runtime_agents_execution_kind ON runtime_agents(project_id, execution_kind, id)
    WHERE NOT deleted;
CREATE INDEX hosted_unfinished_tasks ON tasks(project_id, node_id, id)
    WHERE NOT deleted AND dispatch_error IS NULL AND
        (snapshot IS NULL OR snapshot#>'{run,terminal}'='null'::JSONB
         OR snapshot#>'{run,terminal}' IS NULL
         OR (NOT output_limited AND last_sequence < snapshot_last_sequence));
