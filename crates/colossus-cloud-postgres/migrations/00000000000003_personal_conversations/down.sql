-- Destructive operator-only rollback: personal ownership must not be silently discarded.
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM conversation_threads WHERE owner_user_id IS NOT NULL)
        OR EXISTS (SELECT 1 FROM runtime_agents WHERE owner_user_id IS NOT NULL)
        OR EXISTS (SELECT 1 FROM enrollment_invitations WHERE owner_user_id IS NOT NULL) THEN
        RAISE EXCEPTION 'personal conversations must be retained with their owner metadata';
    END IF;
END $$;
DROP INDEX tasks_thread_visibility;
DROP INDEX conversation_threads_owner_page;
ALTER TABLE conversation_threads DROP COLUMN owner_user_id;
DROP INDEX runtime_agents_owner_page;
ALTER TABLE runtime_agents DROP COLUMN owner_user_id;
ALTER TABLE enrollment_invitations DROP COLUMN owner_user_id;
