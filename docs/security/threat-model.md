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
