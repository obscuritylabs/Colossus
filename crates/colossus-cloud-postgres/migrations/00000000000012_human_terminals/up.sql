ALTER TABLE project_memberships DROP CONSTRAINT project_memberships_project_role_check;
ALTER TABLE project_memberships DROP CONSTRAINT project_memberships_permissions_check;
ALTER TABLE project_memberships ADD CONSTRAINT project_memberships_project_role_check
    CHECK(project_role IN ('viewer','operator','terminal_operator','approver','project_admin'));
ALTER TABLE project_memberships ADD CONSTRAINT project_memberships_permissions_check
    CHECK(permissions <@ ARRAY['read','execute','control','approve','administer','terminal']::TEXT[]);

-- Process incarnation routes contain no shell bytes, browser credentials or application secrets.
CREATE TABLE control_plane_replica_routes (
    replica_id TEXT PRIMARY KEY CHECK(length(replica_id) BETWEEN 8 AND 64),
    address TEXT NOT NULL CHECK(length(address) <= 128),
    expires_at BIGINT NOT NULL
);
CREATE INDEX control_plane_replica_routes_expiry ON control_plane_replica_routes(expires_at);

-- Authenticated Hello metadata is replaced with every fenced connection claim.
ALTER TABLE connection_leases ADD COLUMN capabilities TEXT[] NOT NULL DEFAULT '{}'
    CHECK(cardinality(capabilities) <= 64 AND octet_length(array_to_string(capabilities, '')) <= 8192);
