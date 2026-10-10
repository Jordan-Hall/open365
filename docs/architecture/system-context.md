# System context

Status: proposed, 2026-10-10. Decisions are tracked in `decision-log.md`. Evidence is in `../research/`.

## Core idea

There is one Rust core and one **command layer**. Every user-visible action is a typed, serialisable `Command`, for example `InsertText`, `SetCellFormula` or `Save`. Commands are validated against a permission scope, applied to the document model, recorded for undo/redo and audit, and emitted as change events. The UI, the CLI, the MCP/JSON control surface, Open CoWorker and (later) collaboration sync are all *clients of the same command layer*. This gives three properties together: agent-native control, auditability, and a single place for authorisation. Each command carries an **effect class** (`read`/`edit`/`destructive`/`egress`/`export`) that drives scopes and confirmations.

Limit, stated plainly: on a desktop, scopes protect against *prompt-injected or misbehaving agents* that use our surfaces. They do not protect against malicious code running as the same OS user, which can edit files directly. The command origin is recorded by the surface (stdio/pipe/process), never supplied by the caller. Remote CRDT updates (P2) are a second mutation path and must be decoded and authorised server-side (see threat model T6).

```
        egui shell (ADR-004)            CLI     MCP / JSON control    Open CoWorker (P2)
                    \                    |               |                 /
                     +------------- Command layer (typed, scoped, audited) -------------+
                                         |                         |
                       Document models (DOCX-like, workbook)    Calc engine (own, behind a trait)
                                         |
            Layout (paged text, grid) -> Render scene (vello) -> screen | PDF/print
                                         |
                OPC package graph (lossless) <-> DOCX / XLSX files   [untrusted input boundary]
                                         |
                      (P1+) Sync client -> Open365 server / WOPI file host
```

## Components and phases

| Component | Responsibility | Phase | Build/reuse |
|---|---|---|---|
| `opc` | Zip/OPC part graph, relationships, content types; resource limits (zip-bomb, entity expansion, part count/size); untouched parts byte-identical on save | **P0** | Build on `zip`/`quick-xml` |
| `docx` | Typed document model plus an `extras` raw-XML bag per node, reader/writer, dirty-part tracking, dropped-content log that blocks silent save | **P0** | Build |
| `xlsx` | Sparse workbook model (stable row/col IDs), shared strings, styles/numFmts, preservation of pivots/charts/VBA as opaque parts | **P0** | Build |
| `calc` | `CalcEngine` trait: dependency graph, incremental recalc, error semantics | **P0** | Build (ADR-009) |
| `text` / `layout` | Shaping, line breaking, bidi (parley/harfrust/ICU4X); paged layout; grid viewport virtualisation | **P0 basic → P2 rich** | Reuse shaping, build layout |
| `render` / `pdf` | Scene → GPU (vello/wgpu) and → PDF; same layout used for both | **P1** | Reuse vello; build PDF path |
| `commands` | Command schema, validation, undo/redo, permission scopes, audit events | **P0** | Build |
| `cli` | `open365 docx|xlsx <cmd>`: inspect, roundtrip --check, apply-commands, export-pdf | **P0** | Build |
| `mcp` | Local stdio/loopback MCP server exposing commands; read/write path fencing (VectorCraft pattern); confirmation for destructive commands | **P1** | Build |
| Desktop shell | App chrome, canvas host, IME, accessibility, clipboard, printing | **Spike in P0, product in P1** | egui/eframe on wgpu, PhotoCraft pattern (ADR-004) |
| `testkit` / `xtask` | Fixture corpus, cached-value oracles, ECMA-376 schema validator, invariants, fuzz targets, layering checks | **P0** | Build |
| Sync/storage | Single-org file sync and sharing; own sync/storage service | P1–P2 | Build (ADR-009) |
| Real-time co-edit | CRDT (Loro or Yrs) behind a `DocSync` trait, server as sole peer; OOXML stays import/export only | P2 | Reuse CRDT |
| Identity / tenancy | OIDC with the customer's IdP; simple ACL; per-tenant Postgres with RLS later; audit log | P1 (single-org) → P3 (multi-tenant) | Integrate IdP |
| Chat / meetings | Own chat and meeting services (WebRTC; Matrix protocol optional for interop) | P3 | Build (ADR-009) |
| Federation | Share-by-link, then OCM-style document sharing; Matrix federation for chat; allowlist trust | P4 | Integrate/standards |
| Open CoWorker | Provider trait, BYOK in OS keychain, policy/budget/audit; calls the command layer like any other client | P2–P3 | Build contract; reuse providers |

**P0** = headless core + CLI + UI spike. **P1** = usable single-user native editor. **P2** = co-editing and CoWorker MVP. **P3** = org features, chat, meetings. **P4** = federation.

## Trust boundaries (see `../security/threat-model.md`)

1. Untrusted files → `opc` parser. Run in a resource-limited parser, and later in a sandboxed process.
2. Agents (MCP/CLI/CoWorker) → command layer: scoped, fenced, confirmed, audited.
3. Client ↔ server (P1+): OIDC tokens, server-side permission checks on every op.
4. Client (BYOK, P2) or server proxy (org keys, P3) ↔ provider APIs: egress policy, secrets never inside documents or logs. Org keys live only server-side, and client-side policy is advisory.
5. Server ↔ federated server (P4): signed server identity, allowlist.

## Workspace layout (proposed, VectorCraft-style)

`crates/{opc, docx, xlsx, calc, text, layout, render, pdf, commands, cli, mcp, testkit}`, `apps/{desktop-*, cli}`, `xtask` (CI + layering check: format crates must not depend on UI/render crates), `fixtures/` (licence-audited) and `docs/`.

## Non-goals until a later phase
Macro/VBA execution (preserve only), pivot refresh, chart editing (preserve and render-later), PowerPoint, mobile, a browser client, multi-tenant SaaS, E2EE, and building our own chat or media server.
