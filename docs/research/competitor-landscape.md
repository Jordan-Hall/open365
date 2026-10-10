# Competitor and engine landscape

Status: first pass, 2026-10-10. Owner: lead session. Detailed evidence: `agent-reports/landscape.md`, `agent-reports/native-ui.md`, `agent-reports/word-docx.md`, `agent-reports/excel-xlsx.md`, `agent-reports/collaboration.md`.
Legend: **F** = checked against a primary source on 2026-10-10. **I** = inference. **U** = unverified.

## Positioning in one paragraph

LibreOffice is the fidelity benchmark among open-source suites, and Collabora and ONLYOFFICE own the self-hosted *web* office market. None of them is a native, Rust-built, agent-controllable suite with permissive licensing (I). Open365 should not compete on format breadth in year one. It should compete on four things: (1) native UX quality, (2) a lossless "never silently drop content" guarantee that is proven by tests, (3) a first-class, permission-scoped agent/CLI/MCP control surface, and (4) a simple self-host with clean service boundaries. Format breadth is something we measure and grow over time, not something we claim.

## Full suites and engines

| Project | Licence | Health | Fidelity | Embeddable in a native Rust app? | Relevance to Open365 |
|---|---|---|---|---|---|
| LibreOffice core | MPL-2.0 (F, libreoffice.org/about-us/licenses) | Very active (F) | Broadest OSS OOXML (I) | LibreOfficeKit is a C++ in-process API that is heavy and has a large attack surface (I) | Competitor only. Not used in product or tests (ADR-005). |
| Collabora Online | MPL-2.0 (F, COPYING) | Active (F) | LibreOffice engine | Web UI; WOPI-based | Competitor for the self-hosted web case. Its WOPI integration pattern is worth matching. |
| ONLYOFFICE Docs | AGPL-3.0 + s.7 terms: no logo/trademark rights; modified versions must state "based on ONLYOFFICE" (F) | Active (F) | Strong OOXML-native (I) | Web/JS; AGPL network clause | Competitor. Do not embed, because licence and branding terms conflict with a permissive core. |
| Calligra, Gnumeric | U | U | Lower (I) | GPL family (U) | Low. Gnumeric's function semantics are a possible reference only (licence permitting). |

## Rust building blocks (reuse candidates)

| Area | Candidate | Licence (F) | Verdict |
|---|---|---|---|
| OPC/zip/XML | own thin layer on `zip` + `quick-xml` | permissive (U per crate) | **Build**, because lossless part-graph preservation is the core guarantee and no crate provides it (I). |
| DOCX read/write | docx-rs 0.4.22 | MIT | **Reference/test comparison only.** Its reader maps into a writer model, and its README says support is "not exhaustive" (F). |
| XLSX read/write | calamine (read), rust_xlsxwriter (write), umya-spreadsheet (r/w) | MIT / Apache-2.0 / MIT | Useful oracles and baselines. Our own model is still needed for lossless preservation (I). |
| Formula engine | IronCalc v0.8.3 (2026-08-03) | MIT OR Apache-2.0 | **Wrap behind a `CalcEngine` trait and contribute upstream.** It is pre-1.0, and function coverage and recalc design are U. |
| Text shaping/layout | parley 0.12, cosmic-text 0.19, harfrust 0.14, skrifa, ICU4X | Apache-2.0 / MIT (ICU4X U) | **Reuse.** Actively maintained (F). |
| Page layout (Word-like) | none found | — | **Build.** This is the largest single engineering item (I). |
| Rendering | vello 0.11, wgpu 30 | Apache/MIT | Reuse for the canvas. A separate PDF/print path must be built (I). |
| CRDT | Loro, Yrs, Automerge | MIT | Reuse one, chosen after a rich-text + grid spike (later phase). |

## Sync, chat, media, federation building blocks

| Component | Licence | Use |
|---|---|---|
| Matrix: Tuwunel (official conduwuit successor, Rust) | Apache-2.0 (F) | Preferred chat server to integrate rather than build. |
| Matrix: Synapse | AGPL-3.0 (F) | Acceptable as a separately deployed service. Never link it. |
| LiveKit (SFU) | Apache-2.0 (F) | Meetings media. Deploy separately. |
| OpenCloud (oCIS fork) | Apache-2.0 (F) | Optional file-platform interop. |
| WOPI | protocol | Implement as a *host* later to interoperate with existing file servers (I). |

## Reference project: Storytold VectorCraft

VectorCraft is a clean-room Rust vector editor, not an office suite. It is MIT OR Apache-2.0, v0.8.0 (F). Patterns we adopt: a layered workspace (`doc`/`format`/`render`/`engine`/`ui-*` crates), a UI crate that can be replaced, `cargo xtask ci` with layering checks, pinned fixtures plus property-tested round trips, a CLI with `mcp` stdio, and a desktop JSON control port with read/write root fencing for agents (F, README/Cargo.toml). We **do not** adopt its egui UI, because egui is immediate-mode, has weak support for very large scroll areas, and its AccessKit support is limited to Windows and macOS (F). That is a poor fit for paged rich text and large grids.

## Reuse / wrap / fork / build summary

| Subsystem | Decision (proposed) |
|---|---|
| OPC package + DOCX/XLSX model and serialisers | Build (thin, lossless) |
| Formula engine | Build our own (ADR-009) |
| Text shaping | Reuse (parley/harfrust/skrifa/ICU4X) |
| Page layout, grid renderer, PDF export | Build |
| UI shell | Decided by spike (see `../architecture/decision-log.md`, ADR-004) |
| Conversion and rendering oracle | None. Cached values, own schema validator, golden snapshots (ADR-005) |
| CRDT | Reuse (spike Loro vs Yrs in a later phase) |
| Chat / meetings | Build our own (ADR-009) |
| Storage / identity | Thin own service + OIDC; details in `../architecture/system-context.md` |

## Open questions
- IronCalc's actual function coverage and recalc design. Measure it against our fixtures.
- Licence texts for Qt (LGPL obligations), Slint, ICU4X and the OOXML validator tooling.
- Licences of public fixture corpora (Apache POI, LibreOffice test files). Audit each file before committing it.
