-- A thread's oldest incomplete source must remain visible in its aggregate status.
CREATE INDEX task_incomplete_thread ON tasks(project_id,thread_id,id)
WHERE NOT deleted AND (
    COALESCE((record->>'output_limited')::BOOLEAN,FALSE)
    OR COALESCE((record->>'history_bounded')::BOOLEAN,FALSE)
    OR (record->>'subject'='runtime' AND NOT COALESCE((record->>'history_complete')::BOOLEAN,FALSE))
    OR COALESCE((record#>>'{snapshot,run,last_sequence}')::BIGINT,0)>COALESCE(last_sequence,0)
);
