-- Lineage is cloud metadata. Canonical transcript context remains runtime-owned.
-- NULL relations are omitted from canonical encoding, retaining existing audit hashes.
ALTER TABLE conversation_threads
    ADD COLUMN branch_kind TEXT CHECK(branch_kind IN ('fork','aside')),
    ADD COLUMN branch_parent_thread_id TEXT,
    ADD COLUMN branch_source_task_id TEXT,
    ADD COLUMN branch_source_run_id TEXT,
    ADD CONSTRAINT conversation_branch_complete CHECK (
        (branch_kind IS NULL AND branch_parent_thread_id IS NULL
            AND branch_source_task_id IS NULL AND branch_source_run_id IS NULL)
        OR (branch_kind IS NOT NULL AND owner_user_id IS NOT NULL
            AND branch_parent_thread_id IS NOT NULL
            AND branch_source_task_id IS NOT NULL AND branch_source_run_id IS NOT NULL
            AND length(branch_parent_thread_id) > 0
            AND length(branch_source_task_id) > 0
            AND length(branch_source_run_id) > 0
            AND branch_parent_thread_id <> id)
    );
CREATE INDEX conversation_threads_branches_page
    ON conversation_threads(project_id,branch_parent_thread_id,owner_user_id,branch_kind,archived,domain_updated_at DESC,id DESC)
    WHERE NOT deleted AND branch_parent_thread_id IS NOT NULL;
