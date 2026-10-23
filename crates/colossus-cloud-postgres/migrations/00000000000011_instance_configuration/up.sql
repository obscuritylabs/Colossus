-- Instance custody is independent of project identity and project administration.
CREATE TABLE hosted_chat_instance_gate (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK(singleton),
    owner_id TEXT NOT NULL,
    expires_at BIGINT NOT NULL
);
CREATE TABLE hosted_instance_audit (
    sequence BIGSERIAL PRIMARY KEY,
    actor_id TEXT NOT NULL,
    operation TEXT NOT NULL,
    ciphertext_sha256 TEXT NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
