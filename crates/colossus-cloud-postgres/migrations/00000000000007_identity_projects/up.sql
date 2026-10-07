-- Dedicated product identities; project hierarchy remains display-only authority.
CREATE TABLE cloud_users(
    project_id TEXT NOT NULL CHECK(project_id='__identity'),
    parent_id TEXT NOT NULL DEFAULT '' CHECK(parent_id=''),
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(project_id,parent_id,id),
    display_name TEXT GENERATED ALWAYS AS(record#>>'{user,display_name}') STORED,
    active BOOLEAN GENERATED ALWAYS AS(COALESCE((record#>>'{user,active}')::BOOLEAN,FALSE)) STORED,
    is_admin BOOLEAN GENERATED ALWAYS AS(COALESCE((record#>>'{user,is_admin}')::BOOLEAN,FALSE)) STORED,
    security_epoch BIGINT GENERATED ALWAYS AS((record->>'security_epoch')::BIGINT) STORED,
    UNIQUE(id)
);
CREATE INDEX cloud_users_page ON cloud_users(project_id,id) WHERE NOT deleted;
CREATE TABLE user_identities(
    project_id TEXT NOT NULL CHECK(project_id='__identity'),
    parent_id TEXT NOT NULL DEFAULT '' CHECK(parent_id=''),
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(project_id,parent_id,id),
    user_id TEXT GENERATED ALWAYS AS(record->>'user_id') STORED,
    issuer TEXT GENERATED ALWAYS AS(record->>'issuer') STORED,
    subject TEXT GENERATED ALWAYS AS(record->>'subject') STORED,
    UNIQUE(issuer,subject),
    FOREIGN KEY(user_id) REFERENCES cloud_users(id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX user_identities_page ON user_identities(project_id,id) WHERE NOT deleted;
CREATE TABLE local_credentials(
    project_id TEXT NOT NULL CHECK(project_id='__identity'),
    parent_id TEXT NOT NULL DEFAULT '' CHECK(parent_id=''),
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(project_id,parent_id,id),
    user_id TEXT GENERATED ALWAYS AS(record->>'user_id') STORED,
    username TEXT GENERATED ALWAYS AS(record->>'username') STORED,
    password_hash TEXT GENERATED ALWAYS AS(record->>'password_hash') STORED,
    UNIQUE(username),
    FOREIGN KEY(user_id) REFERENCES cloud_users(id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX local_credentials_page ON local_credentials(project_id,id) WHERE NOT deleted;
CREATE TABLE control_plane_settings(
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '' CHECK(parent_id=''),
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(project_id,parent_id,id),
    setting_name TEXT GENERATED ALWAYS AS(id) STORED,
    scope_project_id TEXT GENERATED ALWAYS AS(CASE WHEN project_id='__identity' THEN NULL ELSE project_id END) STORED,
    FOREIGN KEY(scope_project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX control_plane_settings_page ON control_plane_settings(project_id,id) WHERE NOT deleted;
CREATE INDEX cloud_users_active_admin ON cloud_users(id) WHERE active AND is_admin AND NOT deleted;
CREATE INDEX cloud_users_display_search ON cloud_users(lower(display_name),id) WHERE NOT deleted;
ALTER TABLE project_memberships ADD COLUMN user_id TEXT GENERATED ALWAYS AS(record->>'user_id') STORED;
ALTER TABLE project_memberships ADD COLUMN project_role TEXT GENERATED ALWAYS AS(record->>'role') STORED;
ALTER TABLE project_memberships ADD FOREIGN KEY(user_id) REFERENCES cloud_users(id) DEFERRABLE INITIALLY DEFERRED;
CREATE UNIQUE INDEX project_user_membership ON project_memberships(project_id,user_id) WHERE NOT deleted AND user_id IS NOT NULL;
ALTER TABLE projects ADD COLUMN parent_project_id TEXT GENERATED ALWAYS AS(record->>'parent_project_id') STORED;
ALTER TABLE projects ADD COLUMN archived BOOLEAN GENERATED ALWAYS AS(COALESCE((record->>'archived')::BOOLEAN,FALSE)) STORED;
ALTER TABLE projects ADD FOREIGN KEY(parent_project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE projects ADD CHECK(parent_project_id IS NULL OR parent_project_id<>project_id);
CREATE INDEX project_hierarchy ON projects(parent_project_id,project_id) WHERE NOT deleted;
ALTER TABLE browser_sessions ADD COLUMN security_epoch BIGINT NOT NULL DEFAULT 0 CHECK(security_epoch>=0);

CREATE INDEX local_credential_account ON local_credentials(user_id) WHERE NOT deleted;
CREATE INDEX provider_identity_account ON user_identities(user_id) WHERE NOT deleted;
