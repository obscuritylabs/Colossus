ALTER TABLE browser_sessions DROP COLUMN security_epoch;
ALTER TABLE projects DROP COLUMN parent_project_id CASCADE;
ALTER TABLE projects DROP COLUMN archived;
ALTER TABLE project_memberships DROP COLUMN user_id CASCADE;
ALTER TABLE project_memberships DROP COLUMN project_role;
DROP TABLE control_plane_settings,local_credentials,user_identities,cloud_users;
