-- Product communications are instance-wide; receipts belong to persisted human
-- accounts. They do not share runtime journals, project membership or execution.
CREATE TABLE announcements (
    project_id TEXT NOT NULL CHECK(project_id='__identity'),
    parent_id TEXT NOT NULL DEFAULT '' CHECK(parent_id=''),
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    publication_revision BIGINT NOT NULL CHECK(publication_revision>=0),
    title TEXT NOT NULL CHECK(char_length(title) BETWEEN 1 AND 120),
    summary TEXT NOT NULL CHECK(char_length(summary)<=240),
    body TEXT NOT NULL CHECK(octet_length(body) BETWEEN 1 AND 16384),
    tone TEXT NOT NULL CHECK(tone IN ('info','success','warning','danger')),
    banner BOOLEAN NOT NULL,
    priority BIGINT NOT NULL CHECK(priority BETWEEN 0 AND 100),
    publication_status TEXT NOT NULL CHECK(publication_status IN ('draft','published','archived')),
    -- Fixed nine-digit UTC precision preserves exact publication ordering in the
    -- indexed text key and canonical audit encoding, without a lossy SQL cast.
    published_at TEXT CHECK(published_at ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{9}Z$'),
    expires_at TEXT,
    announcement_created_at TEXT NOT NULL,
    announcement_updated_at TEXT NOT NULL,
    creation_sha256 TEXT NOT NULL CHECK(creation_sha256 ~ '^[0-9a-f]{64}$'),
    last_change_sha256 TEXT CHECK(last_change_sha256 ~ '^[0-9a-f]{64}$'),
    published_content_sha256 TEXT CHECK(published_content_sha256 ~ '^[0-9a-f]{64}$'),
    PRIMARY KEY(project_id,parent_id,id),
    UNIQUE(project_id,id),
    CHECK((publication_revision=0 AND published_at IS NULL AND published_content_sha256 IS NULL)
       OR (publication_revision>0 AND published_at IS NOT NULL AND published_content_sha256 IS NOT NULL)),
    CHECK(publication_status<>'published' OR publication_revision>0)
);
CREATE INDEX announcements_page
    ON announcements(project_id,domain_created_at DESC,id DESC) WHERE NOT deleted;
CREATE INDEX announcements_active_page
    ON announcements(project_id,priority DESC,published_at DESC,id DESC)
    WHERE NOT deleted AND publication_status='published';
CREATE INDEX announcements_banner_page
    ON announcements(project_id,priority DESC,published_at DESC,id DESC)
    WHERE NOT deleted AND publication_status='published' AND banner;

CREATE TABLE announcement_receipts (
    project_id TEXT NOT NULL CHECK(project_id='__identity'),
    parent_id TEXT NOT NULL,
    id TEXT NOT NULL,
    revision BIGINT NOT NULL CHECK(revision>0),
    audit_hash TEXT NOT NULL,
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    domain_updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    read_revision BIGINT NOT NULL CHECK(read_revision>0),
    dismissed_revision BIGINT NOT NULL CHECK(dismissed_revision>=0 AND dismissed_revision<=read_revision),
    receipt_updated_at TEXT NOT NULL,
    PRIMARY KEY(project_id,parent_id,id),
    FOREIGN KEY(project_id,id) REFERENCES announcements(project_id,id) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY(parent_id) REFERENCES cloud_users(id) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX announcement_receipts_account_page
    ON announcement_receipts(project_id,parent_id,id) WHERE NOT deleted;
