# Open365 self-hosting, collaboration, identity, federation (research, 2026-10-10)

Legend: [F] verified fact, [I] inference, [U] unverified. MVP = P0/P1 only.

## 1. Phasing
| Phase | Why | Operational cost |
|---|---|---|
| P0 file-local (MVP) | Proves open/edit/save/reopen of DOCX/XLSX. No server justified. | None. |
| P1 single-org sync/storage + async share (MVP+) | Orgs need shared files, versions, links. [I] Cheapest path: integrate an existing file host via WOPI/WebDAV (OpenCloud Apache-2.0) before building one. | One service binary, SQLite/Postgres, filesystem or S3-compatible store, nightly DB+blob backup, OIDC to existing IdP. |
| P2 real-time co-edit (later) | Only after P1 demand. | Adds stateful WebSocket doc-session service, snapshot store, sticky routing; DB+object storage already present. |
| P3 chat/meetings (later) | Teams-like; do not build early. | Matrix homeserver (own DB, media store) + SFU. Separate trust zone. |
| P4 federation (last) | Cross-org sharing/chat. | Public DNS/TLS, key management, abuse tooling, on-call. |

## 2. Co-editing model (P2)
- [F, prior research] Loro, Yrs, Automerge are MIT. Loro is Rust-native with rich-text (Peritext-style) and movable tree/list; Yrs is Rust port of Yjs (mature ecosystem, text/XML); Automerge has Rust core. [U] Current relative performance/rich-text maturity: benchmark on our own corpus.
- [I] Recommendation: Loro or Yrs behind a `DocSync` trait; choose by spike (rich-text marks, 10k-paragraph doc, 100k-cell grid, snapshot size, API stability). Spreadsheets: model each cell as a map entry keyed by (sheet, row-id, col-id) with stable row/column IDs (not indexes) and last-writer-wins per cell; formulas stored as strings, recomputed locally. Concurrent structural edits (insert row vs edit cell) need stable IDs.
- OOXML is import/export only, never the wire format [I]: lossy features stay in an opaque "preserved parts" blob attached to the doc.
- OT/server-authoritative: simpler permission and retention story, no offline merge. [I] Hybrid is best: CRDT for convergence, server as authority that validates, authorises and persists every update.
- Offline: clients buffer CRDT updates, sync on reconnect; server re-checks permission per update, rejects after revocation.
- Snapshots/compaction: periodic full snapshot + update log truncation; keep named versions as snapshots.
- Permissions: server is the only peer; clients never gossip peer-to-peer. Check authz on connect and per message; per-range ACLs are hard in a CRDT [I] so P2 scope = document-level permission.
- Retention/legal delete: CRDT history retains deleted content. Policy: compaction to a snapshot that drops tombstoned content (supported mechanisms differ per library, [U]) and re-key/rewrite when legal delete needed; document that old exported copies/offline clients retain data.

## 3. Identity/tenancy
- OIDC bring-your-own-IdP (Keycloak/Authentik/Entra) is the default; P1 needs OIDC login only. SCIM only when orgs demand it (P2+). Local accounts only for small installs.
- Better Auth: [F] MIT licensed; repo has plugin ecosystem and oidc/sso topic tags; [U] production status of its OIDC-provider, SSO and SCIM plugins (page did not state). Verify before relying. [I] Use it only as an OIDC relying party/session layer if server is Bun+Hono; a Rust server should use a mature OIDC client crate instead. Open365 should not become an IdP.
- Tenancy: [I] MVP is one org per deployment (isolation by deployment). For multi-tenant hosting, start with shared DB + tenant_id + Postgres row-level security; escalate to DB-per-tenant for regulated customers. Schema-per-tenant has migration cost.
- Authz: [I] simple ACL (owner/editor/commenter/viewer on file/folder, group grants) is enough for P1-P2. ReBAC (OpenFGA/SpiceDB) only if nested sharing/spaces become complex [U: current licences/maturity not verified here].
- Audit: append-only event log (who/what/when/IP), hash-chained, exportable. Keys: per-tenant data keys wrapped by an operator KMS/master key; encryption at rest via disk/object-store SSE first.
- E2EE trade-off [I]: E2EE conflicts with server-side search, preview, AI (Open CoWorker) and server validation of CRDT updates. Default is server-trusted; optional E2EE vault for specific folders with search/AI disabled.

## 4. Teams-like surface
- Recommendation [I]: integrate Matrix; do not build a chat protocol. Run Tuwunel (Apache-2.0, Rust, prior research) as default homeserver; Synapse (AGPL-3.0) only as separate process, never linked. XMPP is mature but weaker for threads/spaces/ecosystem fit [I].
- Matrix: threads and spaces are in the spec [F, general knowledge; not re-fetched]. MLS: [F] "Matrix over MLS" is still research/PoC, test accounts only, so Matrix E2EE remains Olm/Megolm for now. MatrixRTC (MSC4143) was in active spec work in Dec 2025; 2026 status [U].
- Meetings: LiveKit (Apache-2.0, prior research) SFU, which MatrixRTC clients also use [U version]. Jitsi Meet is Apache-2.0 [F, github.com/jitsi/jitsi-meet]; pick LiveKit for SDK/Rust fit [I].
- Chat requires TURN/SFU, media storage, and push gateways: P3 only.

## 5. Federation (P4)
- Chat: Matrix federation (signed server keys, server-name verification). Documents: share-by-link first (no federation, just public/external-guest links); then Open Cloud Mesh (OCM) for server-to-server shares. OCM spec status/version [U]: the spec site fetched but returned no content; check cs3org/OCM-API.
- Trust model [I]: explicit allowlist by default for enterprise; open federation opt-in. Server identity: per-server signing keypair published via well-known, rotation supported. Controls: allow/deny lists, per-peer rate limits and quotas, size limits, remote content never executed, SSRF-safe fetch, Matrix ACL events and moderation tooling. Data residency: shared documents stay hosted at origin; remote users get access, not copies (copies on download are out of our control). 

## 6. Ops
- Retention: per-folder/org policies, trash with TTL, legal hold flag overriding delete; version pruning. Backup: DB dump + object store snapshot, tested restore, key backup separate. Export: standard DOCX/XLSX plus JSON manifest, per-user and per-org. Abuse: upload size/type limits, parser sandboxing, malware scan hook, share-link expiry/passwords.
- Packaging [I]: P1 = single static binary + SQLite/Postgres + local/S3 storage (also Docker image). Compose for P3 (adds Matrix, LiveKit, TURN). Kubernetes only as optional Helm chart; do not require it.

## Sources (accessed 2026-10-10)
- https://github.com/better-auth/better-auth (MIT; plugin status not shown)
- https://github.com/jitsi/jitsi-meet (Apache-2.0)
- https://cs3org.github.io/OCM-API/docs.html (no spec content retrieved)
- https://arewemlsyet.com/ and https://matrix.org/blog/2025/02/14/this-week-in-matrix-2025-02-14/ (Matrix over MLS research status; search snippets)
- Prior verified research supplied by lead: Loro/Yrs/Automerge MIT, Tuwunel, Synapse AGPL, LiveKit, OpenCloud, WOPI.
