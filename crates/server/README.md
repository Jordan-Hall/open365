# open365-server

Stage 1 of the sharing design (`docs/architecture/adr-sharing.md`): one Rust binary with SQLite, a local-disk blob store,
local accounts, versioned documents, named-user ACLs, share links and a tamper-evident audit log.

Not in this stage: OIDC sign-in, TOTP, co-editing, federation, server-side document parsing. Uploads are stored as
opaque bytes and returned as attachments. The server never makes an outbound request.

## Run it locally

```sh
# 1. Secrets (once). The audit key is 256 random bits; keep it out of backups of the data directory.
cargo run -p open365-server -- gen-key ./audit.key
head -c 24 /dev/urandom | base64 > ./bootstrap.token   # any long random value

# 2. Start on loopback. Plain HTTP is only allowed on loopback; elsewhere set the TLS variables below.
export OPEN365_DATA_DIR=./data
export OPEN365_AUDIT_KEY_FILE=./audit.key
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
| `OPEN365_MAX_UPLOAD_BYTES` | Upload limit, default 64 MiB |

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
```

## API summary

`POST /api/v1/login`, `/logout`, `GET /me` · `POST|GET /api/v1/users` (admin) · `PUT /api/v1/admin/policy` (admin kill switch
for Anyone links) · `POST|GET /api/v1/documents`, `GET|PUT /documents/{id}` (`?base_version=N` for optimistic
concurrency), `/versions`, `/content?version=N` · `GET|PUT /documents/{id}/acl`, `DELETE /acl/{user}` ·
`POST|GET /documents/{id}/links`, `DELETE /api/v1/links/{id}` · `GET /api/v1/audit`, `/audit/verify` (admin),
`GET /documents/{id}/audit` (owner).

## Security notes

- Tenant comes from the server-side session only. Sessions are opaque values stored as SHA-256; share tokens likewise.
- Unknown, expired, revoked and policy-blocked links all return the same `404`.
- The audit log is HMAC-chained per tenant. Deleting the newest rows is not detectable from the database alone: record
  `GET /api/v1/audit/verify` `head_mac` somewhere the server cannot write.
- Rate limits are in memory and per process. The client address is the TCP peer; there is no `X-Forwarded-For` trust, so
  behind a proxy all clients share one bucket until that is configured (open item).
- On Windows the key file mode is not checked; protect the data directory with an ACL.

## Tests

`cargo test -p open365-server` runs unit tests and `tests/api.rs`, which drives the real router against SQLite on disk
(tenant isolation on every route, scopes, expiry/revocation, password lockout, rate limits, audit chain and tamper
detection, size limits, no plaintext tokens in the database).
