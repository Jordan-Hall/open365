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
- New links default to **view** (deliberately safer than Office's reported edit default).

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

## Sources (retrieved 2026-10-11)
The four Microsoft pages in section 1, and the repository documents in the header. No web check of axum, Yrs, Loro, Nextcloud, oCIS, MinIO or SeaweedFS licensing was done; those statements are unverified.
