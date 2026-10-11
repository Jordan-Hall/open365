# ADR-013 Sharing: local send-a-copy first, then a self-hosted share-link server

Status: proposed, 2026-10-11. Related: ADR-002, ADR-006, ADR-009, ADR-012, `system-context.md`, `../security/threat-model.md`, `pdf-export.md`, `app-shell.md`.

## 1. Context and requirements

Owner request: a Share button like Office's, with link or people sharing, view/edit, link expiry, copy link, send a copy (attachment or PDF), and see who has access.

Repo facts: the shell reserves a top-right Share control (`app-shell.md`). `pdf` exports PDF. No server exists (ADR-006). ADR-009 forbids depending on third-party servers but allows open standards (OIDC, WOPI, WebDAV). P1 plans single-org sync and OIDC; P2 plans co-editing; P4 federation.

### Office parity checklist
Sources, retrieved 2026-10-11 (Microsoft only): [Share OneDrive files](https://support.microsoft.com/en-us/office/share-onedrive-files-and-folders-9fcc2f7d-de0c-4cec-93b0-a82024800c07), [Block downloads](https://support.microsoft.com/en-us/onedrive/block-downloads-for-view-only-files-in-sharepoint-and-onedrive), [Sharing settings (admin)](https://learn.microsoft.com/en-us/sharepoint/turn-external-sharing-on-or-off), [Unauthenticated sharing](https://learn.microsoft.com/en-us/microsoft-365/solutions/best-practices-anonymous-sharing?view=o365-worldwide). Exact labels came from a search summary; re-check the live pages before freezing the dialog.

| Feature | Stage |
|---|---|
| Share button opens a dialog | 0 |
| Send a copy as attachment or PDF | 0 |
| Link scope: anyone, organisation, specific people | 1 |
| Can edit / can view, per link or person | 1 |
| Block download for view-only links | 1 (best effort) |
| Expiry date; password on a link | 1 |
| Add people by name/email, message | 1 (email notice optional) |
| Copy link | 1 |
| Manage access: list, change, stop sharing | 1 |
| Admin policy: disable Anyone links, force expiry or view-only | 1 |
| Co-authoring with presence | 2 |
| Share across servers | 3 |

## 2. Decision: four stages

### Stage 0: local, no server
- **Send a copy / Send as PDF**: write a temp file (PDF via `open365-pdf`) and open the OS share sheet (Windows share contract, macOS NSSharingService, Linux xdg-desktop-portal). Fallback: reveal the file in the folder.
- **Copy file** to the clipboard as a file object, and "Save a copy as".
- **Hygiene**: before a copy leaves, reuse the T3b hidden-content inspector and show a summary (comments, tracked changes, hidden text) with "remove and send".
- Each action is a command of effect class `export`; temp files are deleted on close and next start.
- Link and people sections are shown disabled ("Connect a server"), not hidden.

### Stage 1: `open365-server`, single Rust binary
Scope: storage, identity, share links, audit log, tenant isolation. No co-editing, chat or federation.
- **Framework: axum on tokio with tower.** Both MIT; the largest Rust web ecosystem; tower layers give rate limits, timeouts, body limits and tracing for free; rustls for TLS. Rejected: actix-web (permissive and fast, but less tower reuse), poem/salvo (smaller communities). Throughput is irrelevant at MVP scale and handlers sit behind our own service traits, so switching is cheap.
- **Data**: SQLite (WAL) via `sqlx` by default; Postgres with RLS as scale-up. `tenant_id` on every row and key path.
- **Blobs**: content-addressed local disk behind a `BlobStore` trait; immutable versions.
- **Identity**: local accounts (argon2id, optional TOTP) and OIDC with PKCE. Opaque server-side sessions, not JWTs, for instant revocation. Desktop sign-in uses the system browser and a loopback redirect, never a WebView.
- **Share links**: `https://host/s/<token>`; 256-bit CSPRNG token, base64url; the server stores only its SHA-256. Row: document, scope (`view`, `edit`, `view-no-download`), audience (`anyone`, `org`, `specific`), optional `expires_at`, optional argon2id password, creator, revoked_at. One permission function serves links and accounts.
- **People**: ACL rows `(document, principal, role)`; roles viewer, editor, owner.
- **Audit**: append-only, HMAC-chained log of share changes, link use, sign-ins, downloads and denials; visible to admins and to the owner for their documents.
- **Tenant isolation**: `TenantCtx` derived from the session or token, never from request fields.
- **Control surface**: share actions are commands with effect class `egress`, used by desktop, CLI and MCP alike. Agents cannot raise their own scope.

### Stage 2: real-time co-editing (needs its own ADR)

| | CRDT (Yrs, Loro, Automerge; all MIT) | OT |
|---|---|---|
| Offline/merge | Native convergence | Central transform server; offline is hard |
| Server | Relay plus storage | Transform authority |
| Rust libraries | Mature options | None general; we would write it |
| Rich text, tables | Marks supported; table and range moves are hard | Flat text well understood; tree OT is complex |
| History size | Needs compaction (T7) | Small log |
| Security | Decode and authorise updates before merge (T6) | Server validates each op |

Recommendation: CRDT behind the `DocSync` trait, server as sole peer, library chosen by the Loro-versus-Yrs spike. `docx`: a CRDT tree, one text sequence per paragraph, formatting as marks, unknown `extras` XML as opaque per-node blobs. `xlsx`: cells keyed by `(rowId, colId)` using the stable IDs from `xlsx-model`, last-writer-wins for values and formulas, row/column insert, delete and move as list operations. Formula results are never synced; peers recompute. OOXML stays import/export only.

### Stage 3: federation (needs its own ADR)
Share links already work across servers (plain HTTPS). Named-user sharing across servers adds signed server identity (Ed25519, well-known URL), allowlist-only trust (T8), an invitation handshake modelled on the public Open Cloud Mesh spec, and per-server rate limits.

## 3. Alternatives
ADR-009 excludes depending on third-party servers, so these are interop targets or optional backends. Licence notes are from memory and **must be verified** against each repository; this draft did not check them.

| Option | Role | Licence note | Verdict |
|---|---|---|---|
| WebDAV | Standard file protocol | IETF RFC 4918 | Optional endpoint later; no link semantics, so it cannot replace Stage 1 |
| Nextcloud | Mature sharing, WOPI, OCM | Server AGPL-3.0 | Interop only; never copy code (ADR-008) |
| oCIS / OpenCloud | Go file platform | Apache-2.0 core (verify) | Interop only |
| WOPI | Host serves files to an editor | Public protocol docs; Microsoft terms apply to its clients | Implement the host side later; not the sharing model |
| S3 | Blob backend | API is a de-facto interface; MinIO and Garage AGPL, SeaweedFS Apache-2.0 (verify) | Optional `BlobStore` backend via `object_store` (Apache-2.0); default stays disk |

### 3a. Stage 1 dependency licences (verified 2026-10-11)
Source: the `license` field of each crate on the crates.io registry API (`https://crates.io/api/v1/crates/<name>`, newest published version) and the manifest of the version resolved in `Cargo.lock`; repositories linked from there. All fit `deny.toml` without new allow-list entries (confirmed by `cargo deny check`). The unverified-licence warning above applies only to the alternatives table.

| Crate | Version checked | Licence | Use |
|---|---|---|---|
| axum | 0.8.9 | MIT | HTTP routing |
| axum-server | 0.8.0 | MIT | rustls listener for axum |
| tokio | 1.53.2 | MIT | async runtime |
| tower | 0.5.3 | MIT | service layers |
| rustls | 0.23.45 | Apache-2.0 OR ISC OR MIT | TLS |
| ring | 0.17.14 | Apache-2.0 AND ISC | rustls crypto provider (chosen over aws-lc-rs: no C++/NASM build) |
| rusqlite | 0.40.2 | MIT | SQLite (bundled, public-domain SQLite C source) |
| sqlx | 0.9.0 | MIT OR Apache-2.0 | not used (see below) |
| argon2 | 0.5.x used; 0.6.0 latest | MIT OR Apache-2.0 | argon2id |
| openidconnect | 4.0.1 | MIT | OIDC client, deferred (not built in this slice) |

Deviation: Stage 1 uses `rusqlite` instead of `sqlx`. With SQLite as the only backend the migration runner is small, the dependency tree is far smaller (no async driver set), and the store sits behind our own functions, so a Postgres port can swap it later.

## 4. Threat model delta (proposed rows T10 to T16)
1. **T10 Token entropy:** 256-bit tokens, hash-only storage, constant-time compare, `Referrer-Policy: no-referrer`, tokens redacted from logs.
2. **T11 Revocation and expiry:** checked server-side on every request; sessions and streams dropped on revoke. Anyone who already downloaded keeps the content, and "block download" is a policy, not DRM; the UI says so.
3. **T12 SSRF:** Stage 1 never fetches user-supplied URLs. Later fetchers (OIDC discovery, webhooks, federation) use one egress client that blocks private, loopback and link-local ranges after DNS resolution, pins the address, caps redirects and size, and allowlists the IdP.
4. **T13 Malicious uploads:** treat as T1 input; size limits; parse in a resource-limited worker; serve with `Content-Disposition: attachment`, `nosniff` and a sandboxing CSP; no server-side rendering in Stage 1; optional AV hook.
5. **T14 Tenant separation:** tenant from session only; RLS on Postgres; a cross-tenant test for every route.
6. **T15 Rate limits:** per-IP and per-token limits on link resolution and password tries; identical responses for unknown, expired and revoked tokens (no oracle); upload quotas; admin kill switch for Anyone links.
7. **T16 Key management:** HMAC, session and OIDC secrets from file or environment (0600 check), never in the database; key IDs for rotation; backups exclude keys; restore tested.

## 5. Desktop UI
- **Share** button at the top right of both apps, per `app-shell.md` (icon-only when narrow). Tabler `share` icon (MIT, vendored); no Microsoft names or artwork.
- Modal dialog titled with the document name, mirroring Office's layout:
  1. "Add people" field, permission dropdown (Can edit / Can view), message, Send.
  2. Link row: scope summary, gear for settings (scope, permission, block download, expiry, password), **Copy link** with a "Copied" state.
  3. "Send a copy": Attachment, PDF, Copy file (Stage 0).
  4. "Manage access" list: people and links, role dropdowns, Stop sharing, owner first.
- Full keyboard order, Esc closes, AccessKit labels, "Link copied" announced. Tabler icons: `link`, `copy`, `mail`, `file-type-pdf`, `settings`, `users`, `lock`, `clock`.
- New links default to **edit**, matching Office (owner decision, 2026-10-11). The default lives in the client's Share dialog. The server has no implicit scope: every create-link request names its scope (`links.rs` rejects a missing or unknown scope), so the API never widens access silently.

## 6. Acceptance criteria and test plan

### Stage 0
- AC0.1 Dialog opens from the button in Writer and Sheets, keyboard operable.
- AC0.2 Send as PDF matches `export-pdf` output (page count and text).
- AC0.3 Send a copy is byte-identical to the saved file; "remove and send" output has no comments, tracked changes or hidden text (`inspect --hidden`).
- AC0.4 Share sheet opens on Windows; other platforms behind a trait tested with a fake.
- AC0.5 Temp files removed on close and next start.
- Tests: scope and effect-class unit tests; dialog layout golden; fake share-sheet integration; hidden-content regression fixtures; accessibility tree snapshot.

### Stage 1
- AC1.1 Owner creates view and edit links; a second account's access is enforced on read and write.
- AC1.2 Expired, revoked and unknown tokens give identical responses; revocation is immediate.
- AC1.3 Wrong passwords rejected and rate-limited.
- AC1.4 Tokens have 256 bits; no plaintext token in the database or logs (grep test).
- AC1.5 Cross-tenant access fails on every route (generated from the route table).
- AC1.6 Audit log records every share event; HMAC chain verifies and detects tampering.
- AC1.7 Local and OIDC sign-in work (mock IdP: PKCE, wrong issuer rejected).
- AC1.8 Manage access matches the stored ACL and links.
- AC1.9 Single binary runs on SQLite with no other service; backup restore tested.
- Tests: `tower::ServiceExt` handler tests; property tests (effective rights never exceed grants); fuzzing of token parsing and uploads; SSRF suite for the egress client; link-resolution load test with stated hardware; `cargo deny` in CI; corpus round-trip through upload and download proves bytes unchanged.

## 7. Confidence and what would change it
**Medium-high** on Stage 0 and the Stage 1 design (standard capability-URL pattern); **medium** on axum (cheap to change); **low-medium** on Stages 2 and 3 (deferred, spikes needed).

Evidence that would change this:
- Users need co-editing before sharing: merge Stage 2 into Stage 1.
- First users already run Nextcloud or S3: promote WebDAV or the S3 backend.
- A compliance review rejects capability links: make per-link authentication the default.
- Live Microsoft pages differ from section 1: update the checklist first.
- The CRDT spike fails on table or range-move conflicts: reconsider OT for those structures.
- Operators reject a bundled server: ship a library plus container recipe.

## 8. Implementation status (Stage 1 server, `crates/server`, 2026-10-11)
Done and tested (`cargo test -p open365-server`; end-to-end test starts the binary over TLS with a self-signed certificate made at test time):
- Hardening: per-route time limits (30 s, 300 s for uploads), JSON body limit 64 KiB separate from the upload limit, bounded audit fields, bounded rate-limit memory, argon2 behind a semaphore, per-address limit on every sign-in, attempts counted before verification (no concurrency bypass), per-link upload rate and hourly byte budget, blobs written and fsynced before the database lock, HSTS when TLS is served, private file modes on Unix.
- Trusted proxies (`OPEN365_TRUSTED_PROXIES`, default none): `X-Forwarded-For` is read from the right and only through trusted peers.
- Failed sign-ins go to the tenant's audit chain when the tenant exists, else the system chain `-`.
- Soft delete, restore, purge, ownership transfer (owner only; previous owner stays editor), all audited; the `specific` audience (named people in the same tenant).
- AC1.7 OIDC (feature `oidc`, on by default): Authorization Code with PKCE (S256), single-use hashed state, nonce, exact issuer, JWKS cache with one rate-limited refresh, RS256 and ES256 only, issuer from configuration only, https required except a loopback test issuer, discovery endpoints limited to the issuer host plus listed hosts, egress client refuses private and link-local addresses, no redirects, no automatic account creation. Tested against an in-process mock issuer (ES256).
- TOTP for local accounts with single-use steps and admin reset.
- AC1.9 backup and restore: `backup` (VACUUM INTO snapshot while running, plus blobs) and `restore` (integrity check, blob hashes, audit chains), with a restore test; `audit-verify` verifies every chain including `-`.
- Property tests for token shape, names, rights, scopes, audit chain edits, TOTP window and proxy resolution.

Added 2026-10-11 (second pass):
- TOTP secrets encrypted at rest with XChaCha20-Poly1305 (RustCrypto `chacha20poly1305` 0.11, MIT OR Apache-2.0) under a server key kept outside the data directory; key ids and `rotate-secrets` for rotation; backups hold ciphertext only; plaintext rows are sealed at start-up. Ten single-use recovery codes stored as SHA-256 (`recovery_code` at sign-in, regenerated with a current code).
- Slowloris: HTTP/1 header-read timeout (default 10 s), HTTP/2 keep-alive timeouts, global and per-address connection limits (`serve.rs`, trusted proxies exempt from the per-address cap).
- RS256 verification has a known-answer test (key and signature made by an independent implementation) plus forgery cases.
- `gc-blobs` removes blob files no version refers to (age-gated, re-checked before delete; re-uploads refresh file age).
- Typed client `open365_server::api::SharingClient` (password with TOTP or recovery code, OIDC device-style hand-off, upload/update with `base_version`, links, ACL, access list) with an integration test over a loopback socket; HTTP reference in `docs/api/sharing-api.md` (hand-written; utoipa not adopted, to avoid a macro dependency on every handler).
- OIDC login-CSRF fix: the flow is bound to the starting browser by a cookie (hash stored, constant-time match in the callback), and a native hand-off completes only after the person confirms, in that browser, the short code the app shows (RFC 8628 style).

Added 2026-10-11 (third pass):
- Rate limits are persistent sliding windows in SQLite (survive restart, shared per data directory, hashed keys, fail closed), per address (IPv6 by /64, trusted-proxy `X-Forwarded-For`), per address+account, per account across addresses (binds only addresses never seen signing in, so it cannot lock the owner out), per link and per user; `Retry-After` on every 429; lockout with doubling backoff keyed on address+account, never account alone. Covers sign-in, TOTP, link password, unknown-link lookups, uploads and link creation. Trade-off: users behind one NAT address share that address's lockout.
- `KeyProvider` (`src/keys.rs`): `file` (as before), `env` and `command` (JSON argv, no shell, timeout, output cap, stderr discarded) for an external KMS or vault; keys have ids, the current (last) key writes, older keys read; `rotate-secrets` re-wraps TOTP secrets, `key-status` shows which key ids are still in use. Audit rows are never re-signed (tamper evidence), so an audit key id stays listed while its rows exist.

Security review fixes (2026-10-11): known-device cookie (`__Host-`, HMAC, bound to the account) so an owner on a new address is not held back by the account-wide cap; account cap and link-failure counts taken before the work and cleared on success (no concurrent overshoot); limiter in its own `ratelimit.db`, used through the blocking pool; audit verification rejects a key downgrade within a chain; limiter keys are HMACs under a server-derived key (the file is pseudonymous, not anonymous); key command runs with a cleared environment, a bound on `wait`, and process-tree kill; the current key is named explicitly (`OPEN365_*_KEY_CURRENT`) and must be the last entry; `X-Forwarded-For` entries with ports are accepted.

Open: the rate limiter does not yet cover a multi-node deployment sharing one database over a network file system (it needs SQLite on a local disk); the desktop Share dialog does not call the client yet; the confirm page is minimal HTML without localisation; key material is not zeroized in memory; no native KMS client (the `command` provider calls the operator's own tool); the SSRF suite covers the egress client and OIDC discovery only (no webhooks or federation exist); the `for_user_server` client policy (private networks allowed, https required off loopback) is meant for user-typed server addresses only.

## Sources (retrieved 2026-10-11)
The four Microsoft pages in section 1, and the repository documents in the header. No web check of axum, Yrs, Loro, Nextcloud, oCIS, MinIO or SeaweedFS licensing was done; those statements are unverified.
