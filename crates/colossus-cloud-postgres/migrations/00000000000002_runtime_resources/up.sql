-- Independent online management admission; these are never agent tasks.
CREATE TABLE resource_connections (
    project_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    PRIMARY KEY(project_id,node_id),
    FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id)
);
CREATE TABLE runtime_resource_requests (
    request_id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    actor TEXT NOT NULL,
    operation JSONB NOT NULL,
    digest TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    dispatched BOOLEAN NOT NULL DEFAULT FALSE,
    reply JSONB,
    reply_digest TEXT,
    FOREIGN KEY(project_id,node_id) REFERENCES runtime_agents(project_id,id),
    CHECK (expires_at > created_at AND expires_at - created_at <= 25)
);
CREATE INDEX runtime_resource_pending ON runtime_resource_requests(project_id,node_id,owner_id,generation,expires_at) WHERE reply IS NULL;
CREATE INDEX runtime_resource_expiry ON runtime_resource_requests(expires_at);
