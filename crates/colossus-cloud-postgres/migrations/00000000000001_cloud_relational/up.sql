-- Cloud schema v1. Core domain fields are authoritative relational columns.
-- Only SDK payloads, released events and opaque authorization envelopes use JSONB.

CREATE TABLE projects (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    project_name TEXT NOT NULL,
    description TEXT NOT NULL,
    parent_project_id TEXT,
    archived BOOLEAN NOT NULL,
    created_at_text TEXT NOT NULL,
    updated_at_text TEXT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    CHECK(project_id=id),
    UNIQUE(project_id),
    CHECK(parent_project_id IS NULL OR parent_project_id<>project_id)
);
CREATE INDEX projects_project_page ON projects(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE cloud_users (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    display_name TEXT NOT NULL,
    email TEXT,
    active BOOLEAN NOT NULL,
    is_admin BOOLEAN NOT NULL,
    security_epoch BIGINT NOT NULL,
    created_at_text TEXT NOT NULL,
    updated_at_text TEXT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    CHECK(project_id='__identity'),
    UNIQUE(id),
    CHECK(security_epoch>=0)
);
CREATE INDEX cloud_users_project_page ON cloud_users(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE user_identities (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    user_id TEXT NOT NULL,
    issuer TEXT NOT NULL,
    subject TEXT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    CHECK(project_id='__identity'),
    UNIQUE(issuer,subject)
);
CREATE INDEX user_identities_project_page ON user_identities(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE local_credentials (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    user_id TEXT NOT NULL,
    username TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    CHECK(project_id='__identity'),
    UNIQUE(username),
    CHECK(username=lower(username))
);
CREATE INDEX local_credentials_project_page ON local_credentials(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE project_memberships (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    user_id TEXT NOT NULL,
    project_role TEXT NOT NULL,
    permissions TEXT[] NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    CHECK(project_role IN ('viewer','operator','approver','project_admin')),
    CHECK(permissions <@ ARRAY['read','execute','control','approve','administer']::TEXT[])
);
CREATE INDEX project_memberships_project_page ON project_memberships(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE hosts (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    host_name TEXT NOT NULL,
    platform TEXT NOT NULL,
    deployment_kind TEXT NOT NULL,
    last_seen_at BIGINT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    CHECK(last_seen_at>=0)
);
CREATE INDEX hosts_project_page ON hosts(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE runtime_agents (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    instance_id TEXT NOT NULL,
    label TEXT NOT NULL,
    certificate_sha256 TEXT NOT NULL,
    roles TEXT[] NOT NULL,
    revoked BOOLEAN NOT NULL,
    host_id TEXT,
    workspace_id TEXT,
    workspace_label TEXT,
    runtime_ready BOOLEAN NOT NULL,
    policy JSONB,
    policy_observed_at BIGINT,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id='')
);
CREATE INDEX runtime_agents_project_page ON runtime_agents(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE workspaces (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    host_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    workspace_name TEXT NOT NULL,
    sharing_mode TEXT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id='')
);
CREATE INDEX workspaces_project_page ON workspaces(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE conversation_threads (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    node_id TEXT NOT NULL,
    host_id TEXT,
    workspace_id TEXT,
    title TEXT NOT NULL,
    created_at_text TEXT NOT NULL,
    updated_at_text TEXT NOT NULL,
    archived BOOLEAN NOT NULL,
    local_session_id TEXT,
    sync_status TEXT NOT NULL,
    source TEXT NOT NULL,
    can_continue BOOLEAN NOT NULL,
    active_task_id TEXT,
    queued_task_ids TEXT[] NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id='')
);
CREATE INDEX conversation_threads_project_page ON conversation_threads(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE conversation_messages (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    role TEXT NOT NULL,
    message_text TEXT NOT NULL,
    created_at_text TEXT NOT NULL,
    task_id TEXT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    CHECK(parent_id<>'')
);
CREATE INDEX conversation_messages_project_page ON conversation_messages(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE tasks (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    node_id TEXT NOT NULL,
    subject TEXT NOT NULL,
    created_at_text TEXT NOT NULL,
    updated_at_text TEXT NOT NULL,
    request JSONB NOT NULL,
    thread_id TEXT,
    source_read_only BOOLEAN NOT NULL,
    history_complete BOOLEAN NOT NULL,
    history_bounded BOOLEAN NOT NULL,
    run_id TEXT,
    snapshot JSONB,
    dispatch_error JSONB,
    last_sequence BIGINT NOT NULL,
    released_bytes BIGINT NOT NULL,
    output_limited BOOLEAN NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    CHECK(last_sequence>=0 AND released_bytes>=0),
    status TEXT GENERATED ALWAYS AS (CASE WHEN dispatch_error->>'code'='outcome_unknown' THEN 'outcome_unknown' WHEN dispatch_error IS NOT NULL THEN 'failed' ELSE COALESCE(snapshot#>>'{run,status}','queued') END) STORED,
    snapshot_last_sequence BIGINT GENERATED ALWAYS AS (COALESCE((snapshot#>>'{run,last_sequence}')::BIGINT,0)) STORED
);
CREATE INDEX tasks_project_page ON tasks(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE commands (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    task_id TEXT NOT NULL,
    operation JSONB NOT NULL,
    reply JSONB,
    PRIMARY KEY(project_id,parent_id,id),
    CHECK(parent_id<>''),
    node_id TEXT GENERATED ALWAYS AS(parent_id) STORED
);
CREATE INDEX commands_project_page ON commands(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE run_allocations (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    task_id TEXT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    CHECK(parent_id<>''),
    node_id TEXT GENERATED ALWAYS AS(parent_id) STORED,
    local_run_id TEXT GENERATED ALWAYS AS(id) STORED
);
CREATE INDEX run_allocations_project_page ON run_allocations(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE node_task_placements (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    task_id TEXT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    CHECK(parent_id<>''),
    node_id TEXT GENERATED ALWAYS AS(parent_id) STORED,
    CHECK(task_id=id)
);
CREATE INDEX node_task_placements_project_page ON node_task_placements(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE thread_sources (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    thread_id TEXT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    CHECK(parent_id<>''),
    node_id TEXT GENERATED ALWAYS AS(parent_id) STORED,
    local_session_id TEXT GENERATED ALWAYS AS(id) STORED
);
CREATE INDEX thread_sources_project_page ON thread_sources(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE admission_counters (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    active_tasks TEXT[] NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    node_id TEXT GENERATED ALWAYS AS(id) STORED,
    active_count INTEGER GENERATED ALWAYS AS(cardinality(active_tasks)) STORED
);
CREATE INDEX admission_counters_project_page ON admission_counters(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE enrollment_invitations (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    node_id TEXT NOT NULL,
    label TEXT NOT NULL,
    roles TEXT[] NOT NULL,
    expires_at BIGINT NOT NULL,
    redeemed_certificate TEXT,
    redeemed_csr TEXT,
    certificate_pem TEXT,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    UNIQUE(id),
    CHECK(expires_at>=0)
);
CREATE INDEX enrollment_invitations_project_page ON enrollment_invitations(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE certificate_renewals (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    renewal_id TEXT NOT NULL,
    previous_fingerprint TEXT NOT NULL,
    csr_sha256 TEXT NOT NULL,
    certificate_pem TEXT NOT NULL,
    certificate_sha256 TEXT NOT NULL,
    issued_at BIGINT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    node_id TEXT GENERATED ALWAYS AS(id) STORED,
    CHECK(issued_at>=0)
);
CREATE INDEX certificate_renewals_project_page ON certificate_renewals(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE control_plane_settings (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    classification_enabled BOOLEAN,
    classification_text TEXT,
    classification_tone TEXT,
    classification_position TEXT,
    required_sandbox_profile TEXT,
    allowed_approval_modes TEXT[],
    allowed_tools TEXT[],
    completed BOOLEAN,
    bootstrap_version BIGINT,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    setting_type TEXT GENERATED ALWAYS AS(CASE id WHEN 'display' THEN 'display' WHEN 'policy-expectation' THEN 'policy' ELSE 'bootstrap' END) STORED,
    scope_project_id TEXT GENERATED ALWAYS AS(CASE WHEN project_id='__identity' THEN NULL ELSE project_id END) STORED,
    CHECK(id IN ('display','policy-expectation','identity-bootstrap-v3','administrator-bootstrap-v3')),
    CHECK(allowed_approval_modes <@ ARRAY['ask','deny','risk_auto','danger_auto','unknown']::TEXT[]),
    CHECK(
        (setting_type='display' AND project_id='__identity'
            AND classification_enabled IS NOT NULL AND classification_text IS NOT NULL
            AND classification_tone IS NOT NULL AND classification_position IS NOT NULL
            AND classification_tone IN ('neutral','info','warning','danger')
            AND classification_position IN ('top','top_and_bottom'))
        OR (setting_type='policy' AND project_id<>'__identity' AND allowed_approval_modes IS NOT NULL)
        OR (setting_type='bootstrap' AND project_id='__identity' AND completed IS NOT NULL
            AND bootstrap_version IS NOT NULL AND bootstrap_version>0)
    )
);
CREATE INDEX control_plane_settings_project_page ON control_plane_settings(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

CREATE TABLE oidc_flows (
    project_id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT '',
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    record JSONB NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK(parent_id=''),
    expires_at BIGINT GENERATED ALWAYS AS((record->>'expires_at')::BIGINT) STORED
);
CREATE INDEX oidc_flows_project_page ON oidc_flows(project_id,domain_updated_at DESC,id DESC) WHERE NOT deleted;

ALTER TABLE projects ADD FOREIGN KEY(parent_project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE user_identities ADD FOREIGN KEY(user_id) REFERENCES cloud_users(id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE local_credentials ADD FOREIGN KEY(user_id) REFERENCES cloud_users(id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE project_memberships ADD FOREIGN KEY(user_id) REFERENCES cloud_users(id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE control_plane_settings ADD FOREIGN KEY(scope_project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE project_memberships ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE hosts ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE runtime_agents ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE workspaces ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE conversation_threads ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE conversation_messages ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE tasks ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE commands ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE run_allocations ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE node_task_placements ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE thread_sources ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE admission_counters ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE enrollment_invitations ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE certificate_renewals ADD FOREIGN KEY(project_id) REFERENCES projects(project_id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE runtime_agents ADD FOREIGN KEY(project_id,host_id) REFERENCES hosts(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE workspaces ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE workspaces ADD FOREIGN KEY(project_id,host_id) REFERENCES hosts(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE conversation_threads ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE conversation_threads ADD FOREIGN KEY(project_id,workspace_id) REFERENCES workspaces(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE tasks ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE tasks ADD FOREIGN KEY(project_id,thread_id) REFERENCES conversation_threads(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE commands ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE commands ADD FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE run_allocations ADD FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE node_task_placements ADD FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE conversation_messages ADD FOREIGN KEY(project_id,parent_id) REFERENCES conversation_threads(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE conversation_messages ADD FOREIGN KEY(project_id,task_id) REFERENCES tasks(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE thread_sources ADD FOREIGN KEY(project_id,thread_id) REFERENCES conversation_threads(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE run_allocations ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE node_task_placements ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE thread_sources ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE admission_counters ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE certificate_renewals ADD FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id) DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE user_login_metadata (
    user_id TEXT NOT NULL REFERENCES cloud_users(id) DEFERRABLE INITIALLY DEFERRED,
    ordinal INTEGER NOT NULL CHECK(ordinal>=0 AND ordinal<16),
    kind TEXT NOT NULL CHECK(kind IN ('local','oidc')),
    label TEXT NOT NULL,
    username TEXT,
    issuer TEXT,
    subject TEXT,
    PRIMARY KEY(user_id,ordinal)
);
CREATE TABLE released_events (
    project_id TEXT NOT NULL, scope_id TEXT NOT NULL, sequence BIGINT NOT NULL CHECK(sequence>0),
    record JSONB NOT NULL, digest TEXT NOT NULL, previous_hash TEXT NOT NULL,chain_hash TEXT NOT NULL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(project_id,scope_id,sequence)
);
CREATE TABLE released_event_heads (
    project_id TEXT NOT NULL,scope_id TEXT NOT NULL,last_sequence BIGINT NOT NULL DEFAULT 0,last_hash TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(project_id,scope_id)
);
CREATE TABLE sync_cursors (
    project_id TEXT NOT NULL,source_id TEXT NOT NULL,scope_id TEXT NOT NULL,sequence BIGINT NOT NULL CHECK(sequence>=0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
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
CREATE UNIQUE INDEX outbox_pending_project ON delivery_outbox(project_id) WHERE delivered_at IS NULL;
CREATE TABLE browser_sessions (
    session_hash TEXT PRIMARY KEY CHECK(length(session_hash)=64),subject TEXT NOT NULL,csrf_hash TEXT NOT NULL,
    created_at BIGINT NOT NULL,expires_at BIGINT NOT NULL CHECK(expires_at>created_at),
    security_epoch BIGINT NOT NULL DEFAULT 0 CHECK(security_epoch>=0)
);
CREATE TABLE connection_leases (
    project_id TEXT NOT NULL,node_id TEXT NOT NULL,owner_id TEXT NOT NULL,generation BIGINT NOT NULL CHECK(generation>0),
    expires_at BIGINT NOT NULL,PRIMARY KEY(project_id,node_id)
);

CREATE INDEX agents_host ON runtime_agents(project_id,host_id,id) WHERE NOT deleted;
CREATE INDEX workspace_node ON workspaces(project_id,node_id,id) WHERE NOT deleted;
CREATE INDEX thread_workspace ON conversation_threads(project_id,node_id,workspace_id,id) WHERE NOT deleted;
CREATE INDEX thread_archive_page ON conversation_threads(project_id,archived,domain_updated_at DESC,id DESC) WHERE NOT deleted;
CREATE INDEX task_node_status ON tasks(project_id,node_id,status,id) WHERE NOT deleted;
CREATE INDEX task_thread ON tasks(project_id,thread_id,id) WHERE NOT deleted;
CREATE INDEX command_pending ON commands(project_id,node_id,id) WHERE NOT deleted AND reply IS NULL;
CREATE INDEX task_created_page ON tasks(project_id,domain_created_at DESC,id DESC) WHERE NOT deleted;
CREATE INDEX message_created_page ON conversation_messages(project_id,parent_id,domain_created_at DESC,id DESC) WHERE NOT deleted;
CREATE INDEX task_incomplete_thread ON tasks(project_id,thread_id,id) WHERE NOT deleted AND (output_limited OR history_bounded OR (subject='runtime' AND NOT history_complete) OR snapshot_last_sequence>last_sequence);
CREATE INDEX thread_sources_thread ON thread_sources(project_id,thread_id) WHERE NOT deleted;
CREATE INDEX membership_user ON project_memberships(user_id,project_id) WHERE NOT deleted;
CREATE UNIQUE INDEX project_user_membership ON project_memberships(project_id,user_id) WHERE NOT deleted;
CREATE INDEX cloud_users_active_admin ON cloud_users(id) WHERE active AND is_admin AND NOT deleted;
CREATE INDEX cloud_users_display_search ON cloud_users(lower(display_name),id) WHERE NOT deleted;
CREATE INDEX project_hierarchy ON projects(parent_project_id,project_id) WHERE NOT deleted;
CREATE INDEX local_credential_account ON local_credentials(user_id) WHERE NOT deleted;
CREATE INDEX provider_identity_account ON user_identities(user_id) WHERE NOT deleted;
CREATE INDEX browser_session_expiry ON browser_sessions(expires_at);
CREATE INDEX oidc_flow_expiry ON oidc_flows(expires_at,project_id,parent_id,id) WHERE expires_at IS NOT NULL;
CREATE INDEX outbox_retention ON delivery_outbox(delivered_at,outbox_id) WHERE delivered_at IS NOT NULL;
