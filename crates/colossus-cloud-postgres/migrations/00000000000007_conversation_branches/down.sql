DROP INDEX conversation_threads_branches_page;
ALTER TABLE conversation_threads DROP CONSTRAINT conversation_branch_complete,
    DROP COLUMN branch_kind, DROP COLUMN branch_parent_thread_id,
    DROP COLUMN branch_source_task_id, DROP COLUMN branch_source_run_id;
