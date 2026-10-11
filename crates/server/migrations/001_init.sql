-- Every table that holds tenant data carries tenant_id; every query filters on it (ADR-013, T14).
CREATE TABLE tenants (
    id TEXT PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    allow_anyone_links INTEGER NOT NULL DEFAULT 1,
    quota_bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE users (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    email TEXT NOT NULL,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    is_admin INTEGER NOT NULL DEFAULT 0,
    disabled INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    UNIQUE (tenant_id, email)
);

-- Opaque sessions: only the SHA-256 of the bearer value is stored.
CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER
);

CREATE TABLE documents (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL,
    owner_id TEXT NOT NULL REFERENCES users(id),
    current_version INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE INDEX documents_tenant ON documents (tenant_id, owner_id);

-- Immutable versions; blob_id is the SHA-256 of the content, scoped to the tenant on disk.
CREATE TABLE versions (
    document_id TEXT NOT NULL REFERENCES documents(id),
    n INTEGER NOT NULL,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    blob_id TEXT NOT NULL,
    size INTEGER NOT NULL,
    created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (document_id, n)
);

CREATE TABLE acl (
    document_id TEXT NOT NULL REFERENCES documents(id),
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    role TEXT NOT NULL CHECK (role IN ('viewer', 'editor')),
    granted_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (document_id, user_id)
);

-- Only the SHA-256 of the 256-bit token is stored.
CREATE TABLE share_links (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    document_id TEXT NOT NULL REFERENCES documents(id),
    token_hash TEXT NOT NULL UNIQUE,
    scope TEXT NOT NULL CHECK (scope IN ('view', 'edit', 'view-no-download')),
    audience TEXT NOT NULL CHECK (audience IN ('anyone', 'org')),
    expires_at INTEGER,
    password_hash TEXT,
    created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
);
CREATE INDEX share_links_doc ON share_links (tenant_id, document_id);

-- Append-only, HMAC-chained per tenant. tenant_id '-' is the system chain (events with no known tenant).
CREATE TABLE audit (
    tenant_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    ts INTEGER NOT NULL,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    target TEXT NOT NULL,
    detail TEXT NOT NULL,
    key_id TEXT NOT NULL,
    prev_mac TEXT NOT NULL,
    mac TEXT NOT NULL,
    PRIMARY KEY (tenant_id, seq)
);
