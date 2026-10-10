# First milestones

Status: proposed, 2026-10-10. Each milestone is a vertical slice with an acceptance command. Durations are estimates for a small team, not commitments.

## Recommended first slice
**M1: headless lossless round-trip.** It tests the highest-risk assumption that most decides the product: can we open, edit, save and reopen real DOCX/XLSX without silent loss? It does this without being blocked on the UI toolkit decision. The UI spike (M2) runs in parallel on the same core.

## M0: Foundation (about 1 week)
- `git init`, Cargo workspace (`crates/*`, `apps/*`, `xtask`), toolchain pin, `cargo xtask ci` = fmt + clippy + test + layering check + `cargo deny`.
- `fixtures/` with a manifest format, and the first 10 user-made fixtures (licence: own work).
- **Accept:** `cargo xtask ci` passes on Windows + Linux CI. The layering check fails a deliberate `docx → render` dependency.

## M1: Headless round-trip core (about 4–6 weeks)
Crates: `opc`, `docx`, `xlsx`, `calc` (own engine: parser, dependency graph, core functions), `commands`, `cli`, `testkit`.
- DOCX: open → `InsertText`/`SetRunBold` on one paragraph → save → reopen.
- XLSX: open → `SetCellValue`/`SetCellFormula` → incremental recalc → save (calcChain removed, recalc flag set) → reopen.
- CLI: `open365 docx|xlsx inspect [--hidden]`, `roundtrip --check`, `apply <commands.json>`.
- Commands carry an effect class. Thumbnail and signature are invalidated on edit. Unreachable parts are GC'd with a log entry.
- **Accept:** on 20 tier-A fixtures, invariants 1–7 in `../testing/fidelity-strategy.md` pass. Untouched parts are byte-identical. Our schema validator reports no new errors on any output. Recalc matches Excel-cached values. The dropped-content log is empty except for a deliberate unsupported-feature fixture. A 1 h fuzz run finds no panics or OOM. Zip-bomb and DTD fixtures are rejected.

## M2: Shell spike, Qt/cxx-qt vs Rust-native (about 2–3 weeks, parallel to M1)
- One paged rich-text view (plain layout, no floats) and one 100k-row grid view on the shared core, in both stacks.
- **Accept:** the pass/fail sheet from `../research/agent-reports/native-ui.md` is completed for both (IME, NVDA/VoiceOver/Orca, scroll p95, typing p95, print-to-PDF, clipboard, high-DPI, packaging). ADR-004 is decided with that evidence.

## M3: Native single-user editor alpha (about 8–10 weeks)
- Chosen shell, basic paged layout (paragraph/run styles, lists, simple tables, inline images), grid editing with formulas, undo/redo through commands, PDF export.
- Local MCP server: stdio or user-only pipe/socket, fenced roots, read/suggest-only by default, native-UI confirmations for destructive and egress effects, hidden content excluded from reads, `no-egress` labels honoured.
- "Sanitize for sharing" command with tests.
- **Accept:** open → edit → save → reopen of all tier-A fixtures from the UI. Scripted UI and MCP sessions produce identical command logs. Accessibility and IME checks from M2 still pass.

## M4: Beta hardening
Printing, rich clipboard, out-of-process parser sandbox, signed installers and updates, performance targets measured on stated hardware, and an "open in real Office" check on a Windows runner.

## Later phases (planned boundaries, not scheduled)
P2 co-editing (own sync service; CRDT primitives spike Loro vs Yrs) + CoWorker MVP (provider trait, Ollama/OpenAI-compatible + one BYOK API, read/suggest-only tools). P3 OIDC org identity, own chat and meetings (WebRTC), admin and audit. P4 federation (allowlist, OCM-style sharing, Matrix federation).

## Non-goals for M0–M4
Server of any kind, co-editing, chat/meetings, CoWorker UI, .doc/.xls/ODF, PowerPoint, macros execution, pivot refresh, chart editing, mobile/web clients, claims of "full Office compatibility".

## Risk register
| Risk | Likelihood | Impact | Mitigation / signal |
|---|---|---|---|
| Word-like layout parity costs far more than estimated | High | High | Target "N lines/page" parity. Grow features by fixture. Watch fixture pass rate per sprint. |
| Accessibility of a custom canvas is not achievable in the Rust stack | Medium | High | M2 tests it first. Qt or per-OS shell as fallback. |
| Own formula engine takes longer than expected (Excel has 500+ functions) | High | High | Prioritise functions by frequency in the fixture corpus; cached values keep unsupported formulas displaying correctly. |
| Qt LGPL obligations conflict with distribution or licensing goals | Medium | Medium | Legal review before M2 ends. |
| Lossless model complexity slows editing features | Medium | Medium | Preserve-first policy, and keep the `extras` bag generic. |
| Fixture licensing contaminates the repo | Low | High | Own fixtures only in the repo. Audit each external file. |
| Malicious documents exploit parsers | Medium | High | Limits + fuzzing in M1, sandbox in M4. |
| Provider terms change (CoWorker) | Medium | Medium | Adapters isolated. Re-verify terms per release. |
| Scope creep into server/Teams before the editors are good | High | High | This roadmap's non-goals, plus an owner review point per milestone. |

## Unresolved questions
Formula function priority list; Qt licence fit; parser sandbox mechanism; CoWorker default context scope; fixture corpora licences; target OS priority; Open365's own licence.
