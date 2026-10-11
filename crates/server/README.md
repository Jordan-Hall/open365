# open365-server

Stage 1 of the sharing design (`docs/architecture/adr-sharing.md`): one Rust binary with SQLite, a local-disk blob store,
local accounts, versioned documents, named-user ACLs, share links and a tamper-evident audit log.

Also here: OpenID Connect sign-in (cargo feature `oidc`, default on), TOTP for local accounts, soft delete and purge,
ownership transfer, `specific` link audiences, backup and restore. Not in this stage: co-editing, federation,
server-side document parsing. Uploads are stored as opaque bytes and returned as attachments. The only outbound
requests are OIDC discovery, key and token calls to the configured issuer (none unless OIDC is configured).

## Run it locally

```sh
# 1. Secrets (once). Each key is 256 random bits; keep keys out of backups of the data directory.
cargo run -p open365-server -- gen-key ./audit.key
cargo run -p open365-server -- gen-key ./secret.key   # encrypts TOTP secrets at rest; without it TOTP is unavailable
head -c 24 /dev/urandom | base64 > ./bootstrap.token   # any long random value

# 2. Start on loopback. Plain HTTP is only allowed on loopback; elsewhere set the TLS variables below.
export OPEN365_DATA_DIR=./data
export OPEN365_AUDIT_KEY_FILE=./audit.key
export OPEN365_SECRET_KEY_FILE=./secret.key
export OPEN365_BOOTSTRAP_TOKEN_FILE=./bootstrap.token
cargo run -p open365-server
```

| Variable | Meaning |
|---|---|
| `OPEN365_BIND` | Listen address, default `127.0.0.1:8443` |
| `OPEN365_DATA_DIR` | SQLite file `open365.db` and `blobs/`, default `./data` |
| `OPEN365_AUDIT_KEY_FILE` / `OPEN365_AUDIT_KEY` | Hex audit HMAC key (file preferred; mode 0600 enforced on Unix) |
| `OPEN365_AUDIT_KEY_ID` | Key id stored on each audit row, default `k1` |
| `OPEN365_BOOTSTRAP_TOKEN_FILE` / `OPEN365_BOOTSTRAP_TOKEN` | Enables tenant provisioning; leave unset to disable it |
| `OPEN365_TLS_CERT`, `OPEN365_TLS_KEY` | PEM files; serve HTTPS with rustls |
| `OPEN365_ALLOW_PLAIN_HTTP` | Allow HTTP on a non-loopback address (only behind a TLS proxy) |
| `OPEN365_MAX_UPLOAD_BYTES` | Upload limit, default 64 MiB (JSON routes are limited to 64 KiB) |
| `OPEN365_REQUEST_TIMEOUT_SECS`, `OPEN365_UPLOAD_TIMEOUT_SECS` | Whole-request limits, default 30 and 300 |
| `OPEN365_TRUSTED_PROXIES` | Comma-separated CIDRs allowed to set `X-Forwarded-For`. Default: none |
| `OPEN365_HSTS` | Send HSTS when a proxy terminates TLS (automatic when this process serves TLS) |
| `OPEN365_ARGON2_CONCURRENCY` | Concurrent password hashes, default one per CPU |
| `OPEN365_SECRET_KEY_FILE` / `OPEN365_SECRET_KEY`, `OPEN365_SECRET_KEY_ID` | Hex key (64 digits) for XChaCha20-Poly1305 encryption of TOTP secrets; id default `s1` |
| `OPEN365_SECRET_KEY_FILES` | `s1=old.key,s2=new.key`, oldest first; only the last seals. Then run `rotate-secrets` |
| `OPEN365_HEADER_READ_TIMEOUT_SECS` | Time to send a full request head, default 10 (slowloris) |
| `OPEN365_MAX_CONNECTIONS`, `OPEN365_MAX_CONNECTIONS_PER_IP` | Open sockets in total (4096) and per peer address (64); 0 = unlimited. Trusted proxies skip the per-address cap |
| `OPEN365_AUDIT_KEY_FILES` | `k1=old.key,k2=new.key`, oldest first; only the last signs new rows |
| `OPEN365_OIDC_TENANT`, `_ISSUER`, `_CLIENT_ID`, `_CLIENT_SECRET(_FILE)`, `_REDIRECT_URI`, `_EXTRA_HOSTS` | One OIDC provider; the issuer must be https (http only on loopback) |

## Operator commands

```sh
open365-server gen-key ./audit.key
open365-server backup ./backups/2026-10-11        # online snapshot of open365.db plus blobs/; keys are not included
open365-server restore ./backups/2026-10-11       # into an empty OPEN365_DATA_DIR; verifies integrity, blob hashes, audit chains
open365-server rotate-secrets                      # seal plaintext and re-seal under the newest OPEN365_SECRET_KEY_FILES key
open365-server gc-blobs [--dry-run] [--min-age-secs N]  # delete blob files no version refers to (default: older than 1 h; safe while running)
open365-server audit-verify                        # every chain, including the system chain "-"; exit 1 if any is broken
```
A backup holds password hashes, document bytes and TOTP secrets as ciphertext only (the secret key is not in it, so keep
the key file in a separate store and back it up separately: without it enrolled TOTP users need an admin reset). Keep
the backup as private as the live data.

Key providers (`file`, `env`, `command` for a KMS), rate limits and `key-status`: see `docs/api/sharing-api.md`.

Key rotation: add a new key to `OPEN365_SECRET_KEY_FILES` after the old one, restart, run `rotate-secrets`, then drop the
old key. Rows from before encryption existed are sealed at start-up and by `rotate-secrets`.

## Try it

```sh
B=http://127.0.0.1:8443
curl -s $B/api/v1/admin/tenants -H "x-bootstrap-token: $(cat bootstrap.token)" -H 'content-type: application/json' \
  -d '{"slug":"acme","name":"Acme","admin_email":"admin@acme.test","admin_password":"use a long passphrase here"}'
TOKEN=$(curl -s $B/api/v1/login -H 'content-type: application/json' \
  -d '{"tenant":"acme","email":"admin@acme.test","password":"use a long passphrase here"}' | sed 's/.*"token":"\([^"]*\)".*/\1/')
curl -s "$B/api/v1/documents?name=Budget.xlsx" -H "authorization: Bearer $TOKEN" --data-binary @Budget.xlsx
# share a link (scope: view | edit | view-no-download; audience: org | anyone)
curl -s $B/api/v1/documents/<id>/links -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' \
  -d '{"scope":"view","audience":"anyone","expires_in_secs":86400,"password":"optional link password"}'
# anyone with the token: GET /s/<token> (metadata), GET /s/<token>/content, PUT /s/<token>/content (edit links)
# link password goes in the x-share-password header, never in the URL
# sign-in: browsers get an HttpOnly cookie only; non-browser clients add "bearer": true to receive the token in the body
# 2FA: POST /api/v1/me/totp/enroll, then /me/totp/confirm {"code":"123456"}; later sign-ins add "totp":"123456"
```

## API summary

`POST /api/v1/login`, `/logout`, `GET /me` · `POST|GET /api/v1/users` (admin) · `PUT /api/v1/admin/policy` (admin kill switch
for Anyone links) · `POST|GET /api/v1/documents`, `GET|PUT /documents/{id}` (`?base_version=N` for optimistic
concurrency), `/versions`, `/content?version=N` · `GET|PUT /documents/{id}/acl`, `DELETE /acl/{user}` ·
`POST|GET /documents/{id}/links`, `DELETE /api/v1/links/{id}` · `GET /api/v1/audit`, `/audit/verify` (admin),
`GET /documents/{id}/audit` (owner). `DELETE /documents/{id}` (soft delete), `POST /documents/{id}/restore|purge|transfer`,
`GET /documents/trash`. Links take `audience: specific` with `emails`. `POST /me/totp/enroll|confirm`, `DELETE /me/totp`,
`DELETE /users/{id}/totp` (admin). OIDC: `GET /auth/oidc/{tenant}/start`, `GET /auth/oidc/callback`.

## Security notes

- Tenant comes from the server-side session only. Sessions are opaque values stored as SHA-256; share tokens likewise.
- Unknown, expired, revoked and policy-blocked links all return the same `404`.
- The audit log is HMAC-chained per tenant. Deleting the newest rows is not detectable from the database alone: record
  `GET /api/v1/audit/verify` `head_mac` somewhere the server cannot write.
- Rate limits are in memory, per process and bounded (oldest keys are evicted). Attempts are counted before the check, so
  concurrent guesses cannot exceed a limit. The client address is the TCP peer unless the peer is inside
  `OPEN365_TRUSTED_PROXIES`, in which case the rightmost untrusted `X-Forwarded-For` entry is used.
- On Unix the data directory, database and blobs are created 0700/0600. On Windows file modes are not checked or set:
  give the data directory and key files an ACL that allows only the service account.
- OIDC accounts are never created automatically; a first sign-in links to an existing account with the same verified email.
- TOTP secrets are encrypted in the database (XChaCha20-Poly1305, row-bound); recovery codes (ten, 80 bits, single use) are
  stored as SHA-256 only. OIDC sign-in is bound to the browser that started it by a cookie, and a native-app hand-off
  completes only after the person confirms the short code shown by the app.
- Typed Rust client: `open365_server::api::SharingClient`; HTTP reference in `docs/api/sharing-api.md`.

## Tests

`cargo test -p open365-server` runs unit tests and `tests/api.rs`, which drives the real router against SQLite on disk
(tenant isolation on every route, scopes, expiry/revocation, password lockout, rate limits, audit chain and tamper
detection, size limits, no plaintext tokens in the database). `tests/hardening.rs`: secrets at rest and rotation, recovery
codes, blob GC, header-read timeout and connection limits. `tests/client_api.rs`: the typed client against a loopback server. `tests/features.rs` covers limits, timeouts, proxies,
concurrency, delete/purge/transfer, specific links, TOTP and backup/restore; `tests/oidc.rs` runs against an in-process
mock issuer; `tests/e2e.rs` starts the binary over TLS with a certificate generated at test time. Property tests
(proptest) cover token, scope, rights, audit-chain, TOTP and proxy rules.
