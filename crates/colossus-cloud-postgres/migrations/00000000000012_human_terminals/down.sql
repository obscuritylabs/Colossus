-- Revoke or migrate terminal_operator memberships before applying this rollback.
ALTER TABLE project_memberships DROP CONSTRAINT project_memberships_project_role_check;
ALTER TABLE project_memberships DROP CONSTRAINT project_memberships_permissions_check;
ALTER TABLE project_memberships ADD CONSTRAINT project_memberships_project_role_check
    CHECK(project_role IN ('viewer','operator','approver','project_admin'));
ALTER TABLE project_memberships ADD CONSTRAINT project_memberships_permissions_check
    CHECK(permissions <@ ARRAY['read','execute','control','approve','administer']::TEXT[]);
DROP TABLE control_plane_replica_routes;
ALTER TABLE connection_leases DROP COLUMN capabilities;
