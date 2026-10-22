-- Personal owners are immutable application metadata. Existing conversations remain shared.
-- NULL is omitted from canonical domain encoding, preserving existing audit digests.
ALTER TABLE conversation_threads ADD COLUMN owner_user_id TEXT
    CHECK(owner_user_id IS NULL OR (length(owner_user_id) BETWEEN 1 AND 256));
CREATE INDEX conversation_threads_owner_page
    ON conversation_threads(project_id,owner_user_id,domain_updated_at DESC,id DESC)
    WHERE NOT deleted;
CREATE INDEX tasks_thread_visibility ON tasks(project_id,thread_id,id) WHERE NOT deleted;
ALTER TABLE runtime_agents ADD COLUMN owner_user_id TEXT
    CHECK(owner_user_id IS NULL OR (length(owner_user_id) BETWEEN 1 AND 256));
CREATE INDEX runtime_agents_owner_page ON runtime_agents(project_id,owner_user_id,id) WHERE NOT deleted;
ALTER TABLE enrollment_invitations ADD COLUMN owner_user_id TEXT
    CHECK(owner_user_id IS NULL OR (length(owner_user_id) BETWEEN 1 AND 256));
