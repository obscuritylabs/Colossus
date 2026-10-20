-- Hosted runtime ownership is independent from the connector's stream lease.
-- Journal adapters fence every write against this exact generation and DB clock.
CREATE TABLE hosted_chat_leases (
    namespace TEXT PRIMARY KEY CHECK(length(namespace) BETWEEN 1 AND 256),
    project_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    generation BIGINT NOT NULL CHECK(generation > 0),
    expires_at BIGINT NOT NULL
);
CREATE INDEX hosted_chat_leases_project ON hosted_chat_leases(project_id,expires_at);
CREATE TABLE hosted_chat_project_gates (
    project_id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL DEFAULT '',
    expires_at BIGINT NOT NULL DEFAULT 0
);
-- Values are authenticated ciphertext. Keys and plaintext never enter this adapter.
-- Tombstones retain revisions so deletion cannot create a compare-and-swap ABA.
CREATE TABLE hosted_chat_records (
    namespace TEXT NOT NULL CHECK(length(namespace) BETWEEN 1 AND 256),
    record_id TEXT NOT NULL CHECK(length(record_id) BETWEEN 1 AND 256),
    revision BIGINT NOT NULL CHECK(revision > 0),
    ciphertext BYTEA CHECK(octet_length(ciphertext) <= 1049600),
    expires_at BIGINT,
    admission_scope TEXT,
    PRIMARY KEY(namespace,record_id)
);
CREATE INDEX hosted_chat_records_expiry ON hosted_chat_records(expires_at)
    WHERE expires_at IS NOT NULL;
-- Closed, encrypted owner-routed management. Claimed external work is never replayed.
CREATE TABLE hosted_chat_requests (
    id TEXT PRIMARY KEY,
    namespace TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    generation BIGINT NOT NULL,
    ciphertext BYTEA NOT NULL CHECK(octet_length(ciphertext)<=1049600),
    expires_at BIGINT NOT NULL,
    dispatched BOOLEAN NOT NULL DEFAULT FALSE,
    response BYTEA CHECK(octet_length(response)<=1049600)
);
CREATE INDEX hosted_chat_requests_pending ON hosted_chat_requests(namespace,owner_id,generation,expires_at)
    WHERE NOT dispatched;
