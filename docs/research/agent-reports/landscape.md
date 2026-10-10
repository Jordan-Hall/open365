# Open365 landscape report (research only)
Accessed 2026-10-10. F = verified fact (GitHub API/raw file); I = inference; "unverified" = not confirmed this session. "Pushed" = last repo push.

## 1. Suites/engines
| Project | License (F) | Pushed | Notes |
|---|---|---|---|
| LibreOffice core | GitHub mirror reports GPL-3.0; MPL-2.0 per brief: unverified, check file headers per module | 2026-10-10 | Read-only mirror, dev on Gerrit. Mature, broadest OOXML fidelity of any OSS. LibreOfficeKit embeds C++ engine; heavy (millions of LOC), not Rust. |
| Collabora Online | GitHub reports "Other"; MPL-2.0 unverified | 2026-10-09 | Issue tracker only; dev on Gerrit. WOPI host-facing server, web UI (I: not native). |
| ONLYOFFICE DocumentServer | AGPL-3.0 + s.7 terms (F) | 2026-07-22 | Terms: no trademark/logo/brand rights; modified versions must state they are based on ONLYOFFICE and show legal notices. Network use triggers AGPL source offer. Web-based. |
| Calligra, Gnumeric | unverified | unverified | Not assessed; I: low relevance (GPL, small communities). |

I: A native Rust app that links LibreOffice (LOK) inherits its weight and a C++ ABI; process-isolating it as an import/convert sidecar is the safer pattern, also good for sandboxing untrusted files.

## 2. Rust building blocks
| Crate | License (F) | Pushed | Notes |
|---|---|---|---|
| docx-rs | MIT | 2026-10-05 | Writer-oriented; reader/fidelity unverified. |
| rust_xlsxwriter | Apache-2.0 | 2026-10-01 | Write-only (I, from design). |
| calamine | MIT | 2026-09-02 | Read-only (I). |
| umya-spreadsheet | MIT | 2026-10-09 | Read+write; round-trip fidelity unverified. |
| IronCalc | README: MIT or Apache-2.0 (API shows Apache-2.0) | 2026-10-09; release v0.8.3 (2026-08-03) | Self-described WIP; has XLSX reader/writer; function coverage not stated in README. Pre-1.0. |
| cosmic-text | Apache-2.0 | 2026-09-30 | Shaping/layout/editing. |
| parley | Apache-2.0 | 2026-10-09 | Linebender; rich-text layout. |
| harfrust | MIT | 2026-10-10 | HarfBuzz port; VectorCraft uses it with skrifa (F). |
| ICU4X | GitHub "Other"; Unicode license unverified | 2026-10-09 | Segmentation/bidi/collation. |
| rustybuzz, swash, skrifa | unverified | unverified | Not individually checked. |
| Loro | MIT | 2026-10-09 | Rich-text CRDT. |
| Yrs | MIT (LICENSE file, F) | 2026-09-30 | Yjs-compatible. |
| Automerge | MIT | 2026-10-09 (core repo) | |

I: No Rust DOCX layout engine (pagination, tables, footnotes, floats) exists in these; that is the largest build item.

## 3. Sync/chat/federation/media
| Project | License | Pushed | Implication |
|---|---|---|---|
| Synapse | AGPL-3.0 | 2026-10-10 | Run as separate service; do not link. |
| Tuwunel (official conduwuit successor) | Apache-2.0 | 2026-10-10 | Rust; embeddable-ish; continuwuity unverified. |
| LiveKit server | Apache-2.0 | 2026-10-09 | Go SFU; deploy separately; permissive. |
| OpenCloud (oCIS fork) | Apache-2.0 | 2026-10-09 | File platform; Nextcloud, Jitsi, XMPP servers, ActivityPub, WOPI spec text: unverified (not fetched). |

I: WOPI (file host protocol) is how Collabora/ONLYOFFICE attach to storage; Open365 implementing a WOPI host gives free interop with existing file servers.

## 4. VectorCraft (F, from README/Cargo.toml)
Clean-room Illustrator-class vector editor in Rust, NOT an office suite. v0.8.0, MIT OR Apache-2.0, 1,671 commits, edition 2024, MSRV 1.95. Layered workspace: geom, color, doc, pathops, text, render, svg, pdf, format, engine, tools, mcp, testkit, plugins (wasmi), ui-egui (UI separable from engine). Apps: desktop, CLI (`mcp` stdio, `run --export`, `bench`), web (WASM). Desktop `--control <port>` JSON channel plus MCP; `--automation-read-root/--automation-write-root` fence agent file access. `cargo xtask ci` runs clippy, tests, layering checks; README claims property-tested round trips and 33 pinned Affinity fixtures. Last commit date: unverified. Stack (kurbo, vello_cpu, skrifa, harfrust) is reusable reference for rendering/text.

## 5. Recommendations
| Subsystem | Call | Rationale |
|---|---|---|
| DOCX I/O | Build own model on a lossless OOXML part/XML layer; borrow from docx-rs; LibreOffice sidecar as oracle in tests | Fidelity needs preserving unknown XML (I). |
| XLSX I/O | Wrap calamine (read) + rust_xlsxwriter (write) short term; evaluate umya/IronCalc for round-trip | Fixture-test before claiming. |
| Formula engine | Wrap/contribute IronCalc, behind trait | Pre-1.0; permissive; coverage must be measured. |
| Layout | Build | No Rust option found. |
| Text shaping | Reuse parley or cosmic-text + harfrust/skrifa; ICU4X for segmentation | Permissive, maintained. |
| CRDT | Reuse Loro or Yrs; spike both on rich text | |
| Chat | Wrap Tuwunel (Apache) or Matrix; avoid linking AGPL Synapse | |
| Media | Reuse LiveKit, separate deploy | |
| Storage | Own thin store + WOPI host; OpenCloud interop optional | Avoid server until MVP needs it. |

Differentiation (I): native Rust UI, agent control surface with scoped permissions, permissive licensing, self-host simplicity; not format breadth versus LibreOffice.

## Uncertainties
MPL status of LibreOffice/Collabora, crate fidelity/coverage, Calligra/Gnumeric, Nextcloud/XMPP/ActivityPub/WOPI/Jitsi details, ICU4X/swash/skrifa/rustybuzz licenses, VectorCraft last-commit date.

## Sources (accessed 2026-10-10)
github.com/storytold/vectorcraft (+ raw Cargo.toml); api.github.com/repos/{ironcalc/IronCalc, CollaboraOnline/online, ONLYOFFICE/DocumentServer, LibreOffice/core, MathNya/umya-spreadsheet, bokuweb/docx-rs, tafia/calamine, jmcnamara/rust_xlsxwriter, loro-dev/loro, y-crdt/y-crdt, automerge/automerge, pop-os/cosmic-text, linebender/parley, harfbuzz/harfrust, unicode-org/icu4x, element-hq/synapse, matrix-construct/tuwunel, livekit/livekit, opencloud-eu/opencloud}; raw.githubusercontent.com/ONLYOFFICE/DocumentServer/master/LICENSE; raw.githubusercontent.com/y-crdt/y-crdt/main/LICENSE; raw.githubusercontent.com/ironcalc/IronCalc/main/README.md; raw.githubusercontent.com/CollaboraOnline/online/main/README.md
