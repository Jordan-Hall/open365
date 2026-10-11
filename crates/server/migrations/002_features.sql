-- TOTP for local accounts. The shared secret must be readable to verify codes, so protect the database and its backups.
ALTER TABLE users ADD COLUMN totp_secret TEXT;
ALTER TABLE users ADD COLUMN totp_enabled INTEGER NOT NULL DEFAULT 0;
-- Highest time step already accepted, so a code cannot be replayed.
ALTER TABLE users ADD COLUMN totp_last_step INTEGER NOT NULL DEFAULT 0;

ALTER TABLE documents ADD COLUMN deleted_by TEXT;

-- Add the `specific` audience (named people). SQLite cannot alter a CHECK, so the table is rebuilt.
CREATE TABLE share_links_new (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    document_id TEXT NOT NULL REFERENCES documents(id),
    token_hash TEXT NOT NULL UNIQUE,
    scope TEXT NOT NULL CHECK (scope IN ('view', 'edit', 'view-no-download')),
    audience TEXT NOT NULL CHECK (audience IN ('anyone', 'org', 'specific')),
    expires_at INTEGER,
    password_hash TEXT,
    created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
);
INSERT INTO share_links_new SELECT id, tenant_id, document_id, token_hash, scope, audience, expires_at,
                                   password_hash, created_by, created_at, revoked_at FROM share_links;
DROP TABLE share_links;
ALTER TABLE share_links_new RENAME TO share_links;
CREATE INDEX share_links_doc ON share_links (tenant_id, document_id);

CREATE TABLE share_link_users (
    link_id TEXT NOT NULL REFERENCES share_links(id),
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    PRIMARY KEY (link_id, user_id)
);

-- OpenID Connect: one row per sign-in in flight. Only the hash of `state` is stored; rows are single use.
CREATE TABLE oidc_pending (
    state_hash TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    nonce TEXT NOT NULL,
    code_verifier TEXT NOT NULL,
    bearer INTEGER NOT NULL DEFAULT 0,
    expires_at INTEGER NOT NULL
);

CREATE TABLE oidc_identities (
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    issuer TEXT NOT NULL,
    subject TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (tenant_id, issuer, subject)
);
