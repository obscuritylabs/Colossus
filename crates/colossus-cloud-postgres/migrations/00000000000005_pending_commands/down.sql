DROP INDEX command_pending;
CREATE INDEX command_pending ON commands(project_id,node_id,id) WHERE NOT deleted AND reply IS NULL;
