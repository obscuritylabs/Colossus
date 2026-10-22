DROP INDEX runtime_agents_execution_kind;
ALTER TABLE runtime_agents DROP COLUMN execution_kind;
DROP INDEX hosted_unfinished_tasks;
