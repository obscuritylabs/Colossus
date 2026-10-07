-- Cloud relational schema v1. No runtime journal tables or global write-head lock.


CREATE TABLE projects (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    project_name TEXT GENERATED ALWAYS AS (record->>'name') STORED, UNIQUE (project_id)
);
CREATE INDEX projects_project_page ON projects(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE project_memberships (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    subject TEXT GENERATED ALWAYS AS (record->>'subject') STORED, permissions JSONB GENERATED ALWAYS AS (record->'permissions') STORED,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX project_memberships_project_page ON project_memberships(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE oidc_flows (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    expires_at BIGINT GENERATED ALWAYS AS ((record->>'expires_at')::BIGINT) STORED
);
CREATE INDEX oidc_flows_project_page ON oidc_flows(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE hosts (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    host_name TEXT GENERATED ALWAYS AS (record->>'label') STORED, platform TEXT GENERATED ALWAYS AS (record->>'platform') STORED,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX hosts_project_page ON hosts(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE runtime_agents (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    host_id TEXT GENERATED ALWAYS AS (record->>'host_id') STORED, instance_id TEXT GENERATED ALWAYS AS (record->>'instance_id') STORED, certificate_sha256 TEXT GENERATED ALWAYS AS (record->>'certificate_sha256') STORED, revoked BOOLEAN GENERATED ALWAYS AS (COALESCE((record->>'revoked')::BOOLEAN,FALSE)) STORED, UNIQUE(project_id,id),
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX runtime_agents_project_page ON runtime_agents(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE workspaces (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    node_id TEXT GENERATED ALWAYS AS (record->>'node_id') STORED, sharing_mode TEXT GENERATED ALWAYS AS (record->>'sharing') STORED, workspace_name TEXT GENERATED ALWAYS AS (record->>'label') STORED, shared BOOLEAN GENERATED ALWAYS AS (COALESCE((record->>'shared')::BOOLEAN,FALSE)) STORED, UNIQUE(project_id,id),
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX workspaces_project_page ON workspaces(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE conversation_threads (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    node_id TEXT GENERATED ALWAYS AS (record->>'node_id') STORED, workspace_id TEXT GENERATED ALWAYS AS (record->>'workspace_id') STORED, title TEXT GENERATED ALWAYS AS (record->>'title') STORED, archived BOOLEAN GENERATED ALWAYS AS (COALESCE((record->>'archived')::BOOLEAN,FALSE)) STORED, local_session_id TEXT GENERATED ALWAYS AS (record->>'local_session_id') STORED, UNIQUE(project_id,id),
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX conversation_threads_project_page ON conversation_threads(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE conversation_messages (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    thread_id TEXT GENERATED ALWAYS AS (parent_id) STORED, role TEXT GENERATED ALWAYS AS (record->>'role') STORED, source_sequence BIGINT GENERATED ALWAYS AS ((record->>'sequence')::BIGINT) STORED,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX conversation_messages_project_page ON conversation_messages(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE tasks (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    node_id TEXT GENERATED ALWAYS AS (record->>'node_id') STORED, thread_id TEXT GENERATED ALWAYS AS (record->>'thread_id') STORED, run_id TEXT GENERATED ALWAYS AS (record->>'run_id') STORED, subject TEXT GENERATED ALWAYS AS (record->>'subject') STORED, status TEXT GENERATED ALWAYS AS (CASE WHEN record#>>'{dispatch_error,code}'='outcome_unknown' THEN 'outcome_unknown' WHEN record->'dispatch_error' IS NOT NULL AND record->'dispatch_error'<>'null'::JSONB THEN 'failed' ELSE COALESCE(record->>'status',record#>>'{snapshot,run,status}','queued') END) STORED, last_sequence BIGINT GENERATED ALWAYS AS ((record->>'last_sequence')::BIGINT) STORED, UNIQUE(project_id,id),
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX tasks_project_page ON tasks(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE commands (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    node_id TEXT GENERATED ALWAYS AS (parent_id) STORED, task_id TEXT GENERATED ALWAYS AS (record->>'task_id') STORED, operation JSONB GENERATED ALWAYS AS (record->'command') STORED, reply JSONB GENERATED ALWAYS AS (record->'reply') STORED,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX commands_project_page ON commands(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE run_allocations (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    node_id TEXT GENERATED ALWAYS AS (parent_id) STORED, local_run_id TEXT GENERATED ALWAYS AS (id) STORED, task_id TEXT GENERATED ALWAYS AS (CASE WHEN jsonb_typeof(record)='string' THEN record#>>'{}' ELSE record->>'task_id' END) STORED,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX run_allocations_project_page ON run_allocations(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE node_task_placements (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    node_id TEXT GENERATED ALWAYS AS (parent_id) STORED, task_id TEXT GENERATED ALWAYS AS (id) STORED,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX node_task_placements_project_page ON node_task_placements(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE admission_counters (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    node_id TEXT GENERATED ALWAYS AS (id) STORED, active_count INTEGER GENERATED ALWAYS AS (jsonb_array_length(COALESCE(record->'active','[]'::JSONB))) STORED,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX admission_counters_project_page ON admission_counters(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE enrollment_invitations (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    token_hash TEXT GENERATED ALWAYS AS (id) STORED, node_id TEXT GENERATED ALWAYS AS (record->>'node_id') STORED, expires_at BIGINT GENERATED ALWAYS AS ((record->>'expires_at')::BIGINT) STORED, UNIQUE(id),
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX enrollment_invitations_project_page ON enrollment_invitations(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE certificate_renewals (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK (revision > 0),
    record JSONB NOT NULL,
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (project_id, parent_id, id),
    node_id TEXT GENERATED ALWAYS AS (id) STORED, certificate_sha256 TEXT GENERATED ALWAYS AS (record->>'certificate_sha256') STORED, issued_at BIGINT GENERATED ALWAYS AS ((record->>'issued_at')::BIGINT) STORED,
    FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX certificate_renewals_project_page ON certificate_renewals(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE INDEX agents_host ON runtime_agents(project_id,host_id,id) WHERE NOT deleted;
CREATE INDEX workspace_node ON workspaces(project_id,node_id,id) WHERE NOT deleted;
CREATE INDEX thread_workspace ON conversation_threads(project_id,node_id,workspace_id,id) WHERE NOT deleted;
CREATE INDEX thread_archive_page ON conversation_threads(project_id,archived,updated_at DESC,id DESC) WHERE NOT deleted;
CREATE INDEX task_node_status ON tasks(project_id,node_id,status,id) WHERE NOT deleted;
CREATE INDEX task_thread ON tasks(project_id,thread_id,id) WHERE NOT deleted;
CREATE INDEX command_pending ON commands(project_id,node_id,id) WHERE NOT deleted AND reply IS NULL;
ALTER TABLE commands ADD FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE run_allocations ADD FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE node_task_placements ADD FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE conversation_messages ADD FOREIGN KEY(project_id,thread_id) REFERENCES conversation_threads(project_id,id) DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE released_events (
    project_id TEXT NOT NULL, scope_id TEXT NOT NULL, sequence BIGINT NOT NULL CHECK(sequence>0),
    record JSONB NOT NULL, digest TEXT NOT NULL, previous_hash TEXT NOT NULL,chain_hash TEXT NOT NULL, received_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(project_id,scope_id,sequence)
);
CREATE TABLE released_event_heads (
    project_id TEXT NOT NULL,scope_id TEXT NOT NULL,last_sequence BIGINT NOT NULL DEFAULT 0,last_hash TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(project_id,scope_id)
);
CREATE TABLE sync_cursors (
    project_id TEXT NOT NULL,source_id TEXT NOT NULL,scope_id TEXT NOT NULL,sequence BIGINT NOT NULL CHECK(sequence>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(project_id,source_id,scope_id)
);
CREATE TABLE cloud_audit (
    project_id TEXT NOT NULL,entity_kind TEXT NOT NULL,parent_id TEXT NOT NULL,id TEXT NOT NULL,revision BIGINT NOT NULL,
    actor TEXT NOT NULL,operation TEXT NOT NULL,content_digest TEXT NOT NULL,previous_hash TEXT NOT NULL,chain_hash TEXT NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(project_id,entity_kind,parent_id,id,revision)
);
CREATE TABLE delivery_outbox (
    outbox_id BIGSERIAL PRIMARY KEY,project_id TEXT NOT NULL,scope_id TEXT NOT NULL,event_kind TEXT NOT NULL,
    revision BIGINT NOT NULL,created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),delivered_at TIMESTAMPTZ
);
CREATE INDEX outbox_pending ON delivery_outbox(outbox_id) WHERE delivered_at IS NULL;
CREATE TABLE browser_sessions (
    session_hash TEXT PRIMARY KEY CHECK(length(session_hash)=64),subject TEXT NOT NULL,csrf_hash TEXT NOT NULL,
    created_at BIGINT NOT NULL,expires_at BIGINT NOT NULL CHECK(expires_at>created_at)
);
CREATE INDEX browser_session_expiry ON browser_sessions(expires_at);
CREATE TABLE connection_leases (
    project_id TEXT NOT NULL,node_id TEXT NOT NULL,owner_id TEXT NOT NULL,generation BIGINT NOT NULL CHECK(generation>0),
    expires_at BIGINT NOT NULL,PRIMARY KEY(project_id,node_id)
);


CREATE TABLE thread_sources (
 project_id TEXT NOT NULL,parent_id TEXT NOT NULL DEFAULT '',id TEXT NOT NULL,revision BIGINT NOT NULL CHECK(revision>0),
 record JSONB NOT NULL,audit_hash TEXT NOT NULL,deleted BOOLEAN NOT NULL DEFAULT FALSE,
 created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
 node_id TEXT GENERATED ALWAYS AS (parent_id) STORED,
 local_session_id TEXT GENERATED ALWAYS AS (id) STORED,
 thread_id TEXT GENERATED ALWAYS AS (CASE WHEN jsonb_typeof(record)='string' THEN record#>>'{}' ELSE record->>'thread_id' END) STORED,
 PRIMARY KEY(project_id,parent_id,id),
 FOREIGN KEY(project_id,thread_id) REFERENCES conversation_threads(project_id,id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX thread_sources_thread ON thread_sources(project_id,thread_id) WHERE NOT deleted;

CREATE INDEX membership_subject ON project_memberships(subject,project_id) WHERE NOT deleted;

CREATE INDEX task_created_page ON tasks(project_id,domain_created_at DESC,id DESC) WHERE NOT deleted;
CREATE INDEX message_created_page ON conversation_messages(project_id,parent_id,domain_created_at DESC,id DESC) WHERE NOT deleted;
ALTER TABLE hosts ADD UNIQUE(project_id,id);
ALTER TABLE runtime_agents ADD FOREIGN KEY(project_id,host_id) REFERENCES hosts(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE workspaces ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE conversation_threads ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE conversation_threads ADD FOREIGN KEY(project_id,workspace_id) REFERENCES workspaces(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE tasks ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE certificate_renewals ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
