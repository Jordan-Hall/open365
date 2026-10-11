-- TOTP recovery codes: only a SHA-256 of each 80-bit code is stored; a row is spent by setting used_at.
-- (users.totp_secret now holds "v1:<key id>:<ciphertext>"; see src/secrets.rs.)
CREATE TABLE totp_recovery_codes (
    user_id TEXT NOT NULL REFERENCES users(id),
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    code_hash TEXT NOT NULL,
    used_at INTEGER,
    PRIMARY KEY (user_id, code_hash)
);

-- OIDC: the flow is bound to the browser that started it (a cookie whose SHA-256 is stored here), so a victim who
-- finishes an attacker's authorization URL cannot complete the attacker's flow (login CSRF / session fixation).
ALTER TABLE oidc_pending ADD COLUMN browser_hash TEXT;

-- OIDC sign-in for a native client without a browser callback: the app sends the SHA-256 of a secret it keeps when it
-- starts the sign-in; the browser finishes the sign-in and the user confirms the short code shown by the app; the app
-- then redeems the secret once (a device-style hand-off). The session is created at redemption, so no token is stored.
ALTER TABLE oidc_pending ADD COLUMN handoff_hash TEXT;

CREATE TABLE oidc_handoff (
    handoff_hash TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    browser_hash TEXT NOT NULL,
    confirm_hash TEXT NOT NULL,
    confirmed INTEGER NOT NULL DEFAULT 0,
    expires_at INTEGER NOT NULL
);
