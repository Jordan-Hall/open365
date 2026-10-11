# Sharing API (Stage 1 server, `crates/server`)

Hand-written reference for `open365-server`. The typed Rust client is `open365_server::api::SharingClient`
(`crates/server/src/api.rs`); every method below has one. Base path `/api/v1`. Bodies are JSON unless noted; errors are
`{"error": "<code>"}` with a fixed code. Authentication is `Authorization: Bearer <token>` (non-browser clients sign in
with `"bearer": true`; browsers get an HttpOnly cookie only). Status: Stage 1, not yet a stability promise.

## Sign-in

| Route | Body | Result |
|---|---|---|
| `POST /login` | `{tenant, email, password, bearer?, totp?, recovery_code?}` | `{tenant_id, user_id, expires_in, token?}`. `401 totp_required` when the account has TOTP and neither `totp` nor `recovery_code` was sent; `401 invalid_code` for a wrong, replayed or spent code. |
| `POST /logout` | | `204`; the session is revoked. |
| `GET /me` | | `{tenant_id, user_id, email, is_admin}` |
| `POST /me/totp/enroll` | `{}` | `{secret, otpauth_uri}`; the secret is stored encrypted and inactive. `503 totp_unavailable` when the operator set no secret key. |
| `POST /me/totp/confirm` | `{code}` | `{recovery_codes: [10 strings]}`, shown once. |
| `POST /me/totp/recovery-codes` | `{code}` | New set of ten codes; the previous set stops working. |
| `DELETE /me/totp` | `{code}` | `204`; also deletes recovery codes. |
| `DELETE /users/{id}/totp` | | Admin reset of a lost authenticator. |

### OpenID Connect from a native app (device-style hand-off)

The server is the relying party; the app never sees the authorization code. Flow:

1. The app makes a random secret `S` and computes `H = hex(sha256(S))` and the user code `H[0..4]-H[4..8]` (upper case).
2. The app opens `GET /auth/oidc/{tenant}/start?handoff=H` in the system browser. The server sets an HttpOnly,
   `Secure`, `SameSite=Lax` cookie (`__Host-` prefixed when TLS is served; only that one name is accepted) and stores its hash with the flow.
3. After the issuer redirects back, `GET /auth/oidc/callback` requires that same cookie (constant-time compare against
   the stored hash); a missing or different cookie is refused as `400 invalid_state`, exactly like an unknown state. This
   binds the flow to the browser that started it (no login CSRF or session fixation).
4. For a hand-off flow the browser shows an HTML page with the user code and a button. Nothing can be redeemed until the
   person presses it (`POST /auth/oidc/confirm`, same cookie, one-time token from the page). The person must see the same
   code in the app.
5. The app polls `POST /auth/oidc/poll {"handoff": S}`: `202 {"status":"pending"}` until confirmed, then one `200`
   with a session (as `/login` with `bearer`). The hand-off row is deleted when redeemed; it expires after two minutes.

Without `handoff`, the callback completes in the browser as before (cookie, or the token in the body with `bearer=true`).
A loopback-listener variant is not needed; the hand-off opens no local port and puts no secret in a URL.

## Documents

| Route | Notes |
|---|---|
| `POST /documents?name=N` | Raw bytes body (limit `OPEN365_MAX_UPLOAD_BYTES`). `201 {id, version: 1}`. |
| `PUT /documents/{id}?base_version=V` | New version; `409` when the document moved past `V`. `{id, version}`. |
| `GET /documents` | `[{id, name, version, updated_at, role}]` for documents you own or were granted. |
| `GET /documents/{id}` | `{id, name, version, role, owner_id}` |
| `GET /documents/{id}/versions` | `[{n, size, sha256, created_by, created_at}]` |
| `GET /documents/{id}/content?version=N` | Bytes as an attachment. |
| `DELETE /documents/{id}`, `POST /documents/{id}/restore|purge|transfer`, `GET /documents/trash` | Soft delete, restore, purge, ownership transfer `{email}`. |

## Links

| Route | Notes |
|---|---|
| `POST /documents/{id}/links` | `{scope: view|edit|view-no-download, audience: anyone|org|specific, expires_in_secs?, password?, emails?}`. `emails` is required for `specific` and refused otherwise (`400 invalid_emails`). `201 {id, token, url_path, scope, audience, expires_at}`; the token is shown once. |
| `GET /documents/{id}/links` | `[{id, scope, audience, expires_at, has_password, created_by, created_at, revoked_at, emails?}]` |
| `DELETE /links/{id}` | Revoke. |
| `GET /s/{token}`, `GET|PUT /s/{token}/content` | Open, download, and (edit links) upload. Link password in `x-share-password`. |

## Access

| Route | Notes |
|---|---|
| `PUT /documents/{id}/acl` | `{email, role: viewer|editor}` for a person in the same tenant. |
| `DELETE /documents/{id}/acl/{user_id}` | |
| `GET /documents/{id}/acl` | Owner first, then `[{user_id, email, role}]`. |

## Administration and audit

`POST /admin/tenants` (bootstrap token), `PUT /admin/policy`, `POST|GET /users`, `GET /audit`, `GET /audit/verify`,
`GET /documents/{id}/audit`. See `crates/server/README.md`.

## Rate limits

Any route can answer `429 {"error":"rate_limited"}` with a `Retry-After` header (whole seconds, at least 1). The limits
are sliding windows kept in SQLite in their own file, `ratelimit.db` in the data directory (never in `open365.db`, so
a busy main database cannot cause refusals, and backups of the main database do not carry it). They survive a restart
and are shared by every process on the same data directory (the directory must be on a local disk). The limiter runs
on the blocking pool, never on an async worker. Stored keys are HMAC-SHA-256 under a server key derived from the
oldest audit key, so a copy of the file cannot be used to test guesses of addresses or e-mail addresses; the file is
still pseudonymous data (it shows how often one pseudonym was seen, and when), so treat it as personal data. IPv6
clients are keyed by their /64. The client address is the TCP peer, or the right-most untrusted `X-Forwarded-For`
entry when the peer is in `OPEN365_TRUSTED_PROXIES`; entries may carry a port (`1.2.3.4:5000`, `[2001:db8::1]:443`).
If a trusted proxy sends an entry that is not an address, the previous hop is used and the server logs one fixed
warning per process. If the limiter store fails, the attempt is refused (fail closed).

| Limit | Key | Default | Backoff |
|---|---|---|---|
| Sign-in attempts (any outcome) | address | 60 / 60 s | no |
| Sign-in attempts (password, TOTP, recovery code) | address + tenant + account | 8 / 15 min | yes |
| Sign-in attempts from unknown addresses and devices, all addresses | tenant + account | 40 / h | no |
| TOTP enrolment, confirm, disable, recovery codes | user | 8 / 15 min | yes |
| Share-link requests | address; link | 300 / min; 120 / min | no |
| Link lookups that do not reach a usable link | address | 20 / 5 min | yes |
| Link password guesses | link + address; link | 10 / 15 min; 100 / 15 min | yes (first) |
| Edit-link uploads | link | 60 / h | no |
| Link creation | user | 120 / h | no |

Backoff: the first time a key is over its limit it is locked until its window would have slid; each further lockout
within 24 hours doubles the lock (at most 24 hours), and a successful attempt clears it. Lockouts are keyed on address
plus account, never on the account alone, so an attacker can lock only their own address and not the owner
(account-lockout DoS).

Account-wide cap (`login_account_rate`): guessing spread over many addresses is caught by a count per account that
binds only addresses and devices not known for that account. Known means an address that signed in to the account in
the last 30 days (the last 32 are remembered, as keyed pseudonyms), or a browser that presents the known-device
cookie. A successful sign-in sets `__Host-od-<account pseudonym>=<expiry>.<MAC>`: HttpOnly, Secure, SameSite=Strict,
Path=/, no Domain, 30 days. The MAC is HMAC-SHA-256 over the account and expiry under a key derived from the oldest
audit key; it is compared in constant time, bound to one account, and a forged, expired or other-account value is
ignored. The count is taken BEFORE the password is checked and cleared by a success, so concurrent guesses cannot
overshoot it. The same rule applies to link lookups: each lookup is counted before it is resolved and a lookup that
reaches a usable link clears the count, so only failures accumulate. Trade-offs: users behind the same NAT address as
an attacker share that address's pair lockout; a new device without the cookie can be held back while the account is
under distributed attack (it can retry after `Retry-After`, and any device that signed in before is unaffected).

The numbers are `Config` fields (`login_fail_rate`, `login_account_rate`, `link_create_rate`, ...).

## Key management

Two key sets, each a list of `(id, key)` oldest first, with the current key named explicitly.

| Purpose | Used for | Key | Current key is used for | Old keys are used for |
|---|---|---|---|---|
| audit | HMAC-SHA-256 chain over audit rows | at least 32 bytes | signing new rows | verifying old rows (key id is stored in each row) |
| secret | XChaCha20-Poly1305 over TOTP secrets | exactly 32 bytes | sealing new values | opening values `v1:<key id>:...` |

The current key is named by `OPEN365_AUDIT_KEY_CURRENT` and `OPEN365_SECRET_KEY_CURRENT`. It is required whenever more
than one key is listed (with a single key it defaults to that key). The named key must exist and must be the LAST
entry; the server refuses to start otherwise, so a provider that returns keys in another order cannot make the server
silently sign with, or seal under, the wrong key. Audit verification also checks the order: a row signed with an older
key than an earlier row in the same chain (a forgery with a retired key after a rotation) fails verification.

`OPEN365_KEY_PROVIDER` selects where the keys come from (`file` by default):

| Provider | Configuration |
|---|---|
| `file` | `OPEN365_AUDIT_KEY_FILES=k1=/p/old,k2=/p/new` or `OPEN365_AUDIT_KEY_FILE` / `_KEY` / `_KEY_ID`; `OPEN365_SECRET_KEY_FILES` / `_KEY_FILE` / `_KEY` / `_KEY_ID` likewise. Files must be mode 0600 on Unix. |
| `env` | `OPEN365_AUDIT_KEYS=k1=hex,k2=hex`, `OPEN365_SECRET_KEYS=s1=hex,s2=hex`. Environment variables are visible to same-user processes; prefer `file` or `command`. |
| `command` | `OPEN365_KEY_COMMAND='["/usr/local/bin/kms-fetch","--vault","prod"]'` (JSON argv, run without a shell). It prints `id=hex` lines, oldest first. Exit 0 required, output at most 64 KiB, the whole run (including waiting for exit) limited to 10 s, standard error discarded. The program runs with a cleared environment: only `PATH`, `SystemRoot`, `HOME`, `OPEN365_KEY_PURPOSE` (`audit` or `secret`) and the names listed in `OPEN365_KEY_COMMAND_ENV` (comma-separated, for example a KMS credential) are passed on, so the server's other secrets never reach it. |

On a timeout or oversized output the command's whole process tree is killed: `taskkill /T /F` on Windows, `kill -KILL`
on its process group on Unix (the child is started in its own group). The server contains no `unsafe`, so it cannot call
a Job Object or `killpg` directly; if that tool is missing, only the direct child is killed and the pipe is closed.
A grandchild that detaches itself from the group or job can survive; run the key command under a supervisor if that
matters.

The limiter and known-device keys are derived from the oldest audit key, so retiring it (which is not possible while its
audit rows exist) or replacing it would reset limiter state and invalidate known-device cookies, nothing else.

Rotation: (1) add the new key last, set `OPEN365_..._KEY_CURRENT` to its id, restart (new rows and values use it at
once; old ones stay readable); (2) run `open365-server rotate-secrets` to re-wrap every TOTP secret under the current
secret key (plaintext rows from before encryption are sealed too; exits 1 if a value cannot be opened with the listed
keys); (3) run `open365-server key-status` to see how many audit rows and secrets still use each key id; (4) retire a
secret key when it shows 0. An audit key must stay listed for as long as rows signed with it exist: re-signing history
would erase the tamper evidence, so there is deliberately no audit re-wrap. Errors from providers never contain key
material, and nothing a provider returns is logged. Key bytes are not zeroized in memory (open).

## Listener limits

A client must send a complete request head within `OPEN365_HEADER_READ_TIMEOUT_SECS` (default 10) or the connection is
closed. `OPEN365_MAX_CONNECTIONS` (4096) and `OPEN365_MAX_CONNECTIONS_PER_IP` (64) cap open sockets; peers in
`OPEN365_TRUSTED_PROXIES` are exempt from the per-address cap only.
