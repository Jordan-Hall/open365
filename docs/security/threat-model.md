# Threat model (first pass)

Status: draft, 2026-10-10. Scope: P0–P1 in detail, later phases at boundary level. Architecture: `../architecture/system-context.md`.

## Assets
User documents and their hidden content (comments, tracked changes, metadata); provider API keys and OIDC tokens; audit logs; tenant data (P1+); server signing keys (P4).

## Trust boundaries and threats

| # | Boundary | Main threats | Controls (phase) |
|---|---|---|---|
| T1 | Untrusted DOCX/XLSX → `opc` parser | Zip bombs, path traversal in part names, XXE/DTD entity expansion, deeply nested XML, huge shared-string tables, malformed relationships, parser panics/OOM | Hard limits on entry count, uncompressed size, ratio, depth and string length; DTDs rejected; part names normalised; no `unsafe` in format crates; cargo-fuzz targets in CI (P0). Out-of-process parser sandbox with no network or FS write (P1/P2). |
| T2 | External references in files | Remote images, external workbook links, `INCLUDETEXT`/`HYPERLINK` fields, OLE objects → SSRF, tracking pixels, credential leaks (UNC paths on Windows) | Never auto-fetch. Consent is bound to specific hosts/URLs (not a whole document), and new references added by co-editors prompt again. UNC/`file:` blocked. When the server fetches (P1+), use an SSRF-safe fetcher that blocks private/link-local ranges and re-checks after redirects. |
| T3 | Active content | VBA, DDE, macros, ActiveX, embedded executables | Preserve as opaque bytes and never execute. Show the user an indicator. Optionally strip on export (P0). |
| T3b | Silent retention | Users believe content is gone, but lossless save keeps it: hidden text (`w:vanish`), comments, veryHidden sheets, orphaned media, stale `docProps/thumbnail`, pivot/chart/externalLink caches, custom props, rsids, printer settings. Re-saving "launders" attacker payloads (OLE, altChunk, DDE, VBA in a mis-typed `.docx`) | `inspect --hidden` report; logged GC of unreachable parts; invalidate thumbnail and signatures on edit; VBA presence must match content type and extension (P0). Tested "Sanitize for sharing" command and an active-content indicator (P1). |
| T4 | Agents → command layer (CLI/MCP/CoWorker) | Prompt injection from document text (including hidden text, comments, alt-text, customXml) driving tool calls; out-of-scope file access; mass destructive edits; exfiltration via the model itself (external MCP agents are cloud models) | **Transport:** stdio by default, otherwise a named pipe or Unix socket restricted to the current user. If HTTP is ever used: strict `Origin` and `Host` checks (DNS rebinding), no cookies or CORS, token only in a header, never in argv/URL/tool output (P1). **Effects:** every command carries an effect class. Destructive and egress effects need confirmation in Open365's *native* UI, never via MCP elicitation the agent could answer (P0 schema, P1 enforcement). **Reads:** tag visibility and exclude hidden content by default; wrap document text as delimited untrusted data; `no-egress` labels block or confirm MCP reads as well as CoWorker (P1). **Parity:** MCP defaults to read/suggest-only, like CoWorker; edit scope is granted explicitly per session (P1). **Scope limit:** this contains injected agents, not same-user malware; the CLI records the parent process and does not grant ambient full access (P0). |
| T4b | Exfiltration via generated content | An injected agent plants beacons that fire later in Word/Excel: external-target images, `INCLUDEPICTURE`/`INCLUDETEXT`, `WEBSERVICE()`, `HYPERLINK`, external workbook refs, DDE, data connections | Commands that create these are `egress` class. Agent-origin egress needs native confirmation showing the exact URL (P0 schema, P1 enforcement). |
| T5 | App ↔ provider APIs | Key theft, keys leaking into docs/logs/crash reports, sending classified docs to a cloud model | Keys stored in the OS keychain; redaction in logs and telemetry; org policy for provider allowlist and "no-egress" labels; local-only mode; per-request audit of model, tokens and doc IDs (not content by default) (P2). |
| T6 | Client ↔ server | Broken authz (IDOR), token theft, cross-tenant reads, CRDT updates merging below command validation, attribution forgery | OIDC with PKCE; server checks permissions on every op/snapshot; tenant ID in every key path plus Postgres RLS defence in depth; per-document authz before joining a sync room (P1–P3). Server decodes CRDT updates and enforces roles (comment-only and suggest-only cannot be enforced on opaque deltas). CRDT client IDs bound to the authenticated user. Merged state fuzzed as untrusted input. Sessions closed on revoke. History, deleted content and awareness withheld from newly granted readers (P2). |
| T7 | Retention/legal | CRDT history keeps "deleted" content; backups keep data past retention | Snapshot compaction and history truncation; legal hold; documented backup expiry; deletion propagates to backups within a stated window (P2–P3). |
| T8 | Federation | Malicious or compromised remote server, spam, data residency | Allowlist-only by default; signed server keys; per-server rate limits; remote content treated as untrusted input (T1) (P4). |
| T9 | Supply chain / update | Malicious crate, compromised release | `cargo deny` (licences + advisories), `cargo vet` or audits for parser deps, reproducible builds where feasible, signed releases and updates (P1). |
| T10 | Share-link token entropy | Guessable or leaked capability URLs; tokens in logs, referrers or the database | 256-bit CSPRNG token (base64url, 43 chars); only the SHA-256 is stored; constant-time compare after the hash lookup; `Referrer-Policy: no-referrer`, `Cache-Control: no-store`; the server never logs request URIs, and a test greps the database and audit log for plaintext tokens (ADR-013, P1; `crates/server`) |
| T11 | Link revocation and expiry | Revoked or expired links still work; sessions outlive revocation | Revocation, expiry, deleted-document and policy state are read from the database on every request, so revocation is immediate. Residual risk: anyone who already downloaded keeps the content, and "view, no download" is a policy, not DRM; the UI must say so. Streams dropped on revoke arrive with co-editing (P2) |
| T12 | Server-side request forgery | Server fetches attacker-supplied URLs (links, OIDC discovery, webhooks, federation) | Stage 1 makes no outbound requests at all and has no HTTP client dependency. Later fetchers must share one egress client: block private, loopback and link-local ranges after DNS resolution, pin the address, cap redirects and size, allowlist the IdP (P1 OIDC, P4) |
| T13 | Malicious uploads | Hostile DOCX/XLSX stored and served; stored XSS via inline rendering; parser attacks on the server | Uploads are opaque bytes: no server-side parsing in Stage 1; body-size limit and per-tenant quota; names are display-only (blob paths come from validated hex ids); every download is `application/octet-stream`, `Content-Disposition: attachment`, `nosniff`, CSP `sandbox`. Later parsing runs as T1 input in a resource-limited worker; optional AV hook (P1) |
| T14 | Tenant separation | Cross-tenant read or write by IDOR, shared caches, mixed blob paths | `TenantCtx` comes only from the server-side session; every query filters on `tenant_id`; blobs live under a per-tenant directory; users named in an ACL are resolved inside the caller's tenant; a test walks the route table (`ROUTES`) with another tenant's session and expects 404 on each, with a drift test against the router. Postgres RLS when that backend lands (P1) |
| T15 | Abuse and enumeration | Token or password guessing, oracle on link state, upload flooding, anonymous-link exposure | Per-address and per-link rate limits on `/s/*`, failed-lookup and wrong-password lockouts, sign-in lockouts; unknown, expired, revoked and policy-blocked links return an identical `404`; upload size limit and quota; admin switch that disables Anyone links. Limits are per process: a multi-node deployment needs a shared store (P1) |
| T16 | Key and secret management | Audit or session secrets in the database, backups or process listings; undetected audit tampering | Audit HMAC key and bootstrap token come from a file (mode 0600 checked on Unix) or the environment, never the database; key ids on every audit row allow rotation; sessions are random opaque values stored hashed; the audit chain is per tenant and verifiable, but truncating the newest rows needs an external anchor of the head MAC; backups exclude keys and a restore drill is still to be tested (P1) |

## Security non-negotiables for P0
1. Every format crate has fuzz targets and resource limits before it accepts real-world fixtures.
2. No network access from the core. External references are inert.
3. The command layer records origin (`ui`/`cli`/`mcp`/`coworker`) on every command from day one.
4. `cargo deny` runs in CI from the first commit.

5. Commands carry an effect class from day one. The CLI records the parent process.
6. `inspect --hidden` and logged GC of unreachable parts ship with the first round-trip.

## Further controls (security review, 2026-10-10)
- Audit records use a keyed HMAC of content, never a plain hash (short selections can be brute-forced). A local log is writable by same-user code, so tamper evidence needs signing or server-side copies (P1).
- Org provider keys only work through a server-side proxy. Client-side policy is advisory (P2–P3).
- Until the parser sandbox lands, agent `open` is limited to fenced roots (P1). The sandbox remains an M4 item.

## Open questions
- Is the P1 parser sandbox a separate process (portable) or OS-specific (AppContainer/sandbox-exec/seccomp)? Decide during the shell spike.
- Do we sign the agent audit log locally (tamper evidence) or only server-side?
- What does CoWorker send by default: selection only, or the whole document? (Proposed: selection plus explicit user-added context.)

## Teams additions (proposed, 2026-10-12; see `../architecture/teams.md`, ADR-016)
Phase: T0 to T8 slices. Rows T17 to T25 come from ADR-014; T26 to T30 are new. T6 and T8 apply unchanged and gain a Teams test per route.

| # | Boundary | Main threats | Controls (slice) |
|---|---|---|---|
| T17 | Message content | Injection through rich text, pasted HTML, deceptive links | Typed tree rendered by native widgets or DOM text nodes; pasted HTML sanitised against a payload corpus; links show the real target (T1) |
| T18 | Subscription authz | Cross-tenant or removed-member reads over WebSocket; resume replays | Every subscribe and resume goes through the permission function; membership change closes live subscriptions; a test per event type (T0) |
| T19 | Link previews and webhooks | SSRF, tracking, internal-network scans | Only the shared egress client from T12: private ranges blocked after DNS, redirects and size capped (T2, T7) |
| T20 | Metadata | Presence, typing and read receipts leak activity; push leaks content | Scoped to shared channels; admin switch; push carries no text, sender or channel name, verified by payload capture (T2, T3) |
| T21 | SFU and TURN abuse | Open relay, token reuse, stream floods | Single-room, short-lived, identity-bound tokens; TURN denies private and loopback peers; per-room and per-user limits; host remove and lock (T4) |
| T22 | Capture consent | Silent screen capture or recording | Picker every time; recording needs host consent and shows an indicator; recordings are Drive files with ACLs (T5) |
| T23 | File-tab permissions | Channel membership widening document access | A tab grants nothing; document ACL decides per viewer; leaving a channel never widens access (T2) |
| T24 | Retention and legal hold | Deleted messages survive in search, thumbnails, push queues or backups | Delete removes derived stores (extends T7); holds are admin-only and audited (T6) |
| T25 | Abuse and spam | Floods, bulk mentions, guest abuse | Rate limits per user, channel and tenant; guest limits; audit of bulk mentions and invites (T0, T6) |
| T26 | Recording storage | Recordings readable by non-participants; consent bypass | Recording ACL equals meeting roster at start; consent event audited; retention applies (T5, T6) |
| T27 | Apps and bots | Over-privileged bot tokens; prompt injection from message text into CoWorker or MCP | App principals with per-channel grants, expiring tokens, rate limits, audit; message text delimited as untrusted data; agent send and attach need native confirmation (T7) |
| T28 | E2EE channels | Key or membership errors; plaintext leaking to search, logs, push or bots | MLS group membership tied to channel membership; removed members cannot decrypt later messages; grep test of the database and logs for plaintext; UI states disabled features (T8) |
| T29 | Retention across derived stores | Search index, attachments, caches and mobile local stores outlive deletion | A retention job covers every store listed in a checked inventory; client caches honour server tombstones (T6) |
| T30 | Federation bridge | Compromised remote server, spoofed senders, spam, residency | Allowlisted rooms and servers only; bridged identities are guests with a distinct marker; remote content treated as untrusted (T1); per-server rate limits; off by default (T8) |
