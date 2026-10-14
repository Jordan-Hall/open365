# First milestones

Status: proposed, 2026-10-10; Teams ordering added 2026-10-11 (ADR-014). Each milestone is a vertical slice with an acceptance command. Durations are estimates for a small team, not commitments.

Current measured status and remaining work for Open Write and Open Sheet: `parity-status.md`.

## Product scope (owner requirement, 2026-10-10)
The target is the **full Microsoft 365 suite at 1:1 parity, including Teams**, built entirely by us (ADR-009). Everything below is in scope. The phases set order, not whether something gets built.

| Open365 app | M365 equivalent | Phase starts |
|---|---|---|
| Documents | Word | P0 |
| Sheets | Excel | P0 |
| Drive (files, sync, sharing, versions) | OneDrive / SharePoint files | P1 |
| Teams (chat, channels, threads, presence, file tabs, meetings with audio/video/screen share, calendar) plus its server | Teams | **Next after Word/Excel parity**: T0–T4 (owner, 2026-10-11) |
| Slides | PowerPoint | P2 |
| Mail and Calendar | Outlook / Exchange | P3 |
| Admin, identity, tenancy, audit, retention | Entra ID / admin centre / Purview basics | P1 → P3 |
| Open CoWorker | Copilot | P2 |
| Notes, Forms, Planner/tasks, Sites | OneNote, Forms, Planner, SharePoint sites | P4+ |
| Federation between deployments | (no M365 equivalent) | P4 |
| Web app (native DOM UI in Rust/WASM) | Office for the web | spike after M3, ships P2 |
| iOS/iPadOS and Android apps (native UIKit / Android views driven from Rust): Writer, Sheets, Teams | Office and Teams mobile apps | spike M5; Writer/Sheets M6; Teams from T2 |

## Order of work (owner decision, 2026-10-11)
1. **Word + Excel to parity** (M0–M6 below, including web and mobile Writer/Sheets). This is the current work.
2. **Teams client and server** (T0–T4 below), cross-platform including mobile. The server is the sharing server (`adr-sharing.md`, Stage 1) extended, one Rust binary. Decisions are in `../architecture/adr-teams.md` (ADR-014).
3. **The rest**: Slides, Mail and Calendar (full), Open CoWorker UI, Notes, Forms, Planner, Sites, federation.

Teams is an owned product line. Its server foundation (identity, tenancy, storage, realtime transport) is shared with Drive and co-editing. Sharing Stage 1 is the prerequisite for T0 and may run in parallel with M4–M6.

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

## M2: Desktop shell on the PhotoCraft stack (egui/eframe + wgpu), parallel to M1
- `open365-ui-egui` + `apps/desktop`: one paged rich-text view (plain layout, no floats) and one 100k-row grid view on the shared core.
- **Accept:** the pass/fail sheet from `../research/agent-reports/native-ui.md` is completed (IME, NVDA/VoiceOver/Orca, scroll p95, typing p95, print-to-PDF, clipboard, high-DPI, packaging); failures become tracked work items.

## M3: Native single-user editor alpha (about 8–10 weeks)
- Chosen shell, basic paged layout (paragraph/run styles, lists, simple tables, inline images), grid editing with formulas, undo/redo through commands, PDF export.
- Local MCP server: stdio or user-only pipe/socket, fenced roots, read/suggest-only by default, native-UI confirmations for destructive and egress effects, hidden content excluded from reads, `no-egress` labels honoured.
- "Sanitize for sharing" command with tests.
- **Accept:** open → edit → save → reopen of all tier-A fixtures from the UI. Scripted UI and MCP sessions produce identical command logs. Accessibility and IME checks from M2 still pass.

## M4: Beta hardening
Printing, rich clipboard, out-of-process parser sandbox, signed installers and updates, performance targets measured on stated hardware, and an "open in real Office" check on a Windows runner.

## M5: Web and mobile native UI spike (ADR-010)
- The Rust core builds for `wasm32-unknown-unknown`, `aarch64-apple-ios` and `aarch64-linux-android` in CI from M3 on.
- Spike each platform's native UI (web: Rust-driven DOM; iOS: UIKit from Rust; Android: Android Views from Rust), each with open → edit → save → reopen.
- **Accept:** the per-platform pass/fail sheet is complete (IME, screen reader, selection/clipboard, scroll p95, size, cold start), and ADR-010's stack is chosen per platform.

## M6: Mobile Writer and Sheets (after the M5 spike; about 8–12 weeks per platform)
- iOS/iPadOS (UIKit) and Android (Views) apps for Writer and Sheets on the shared Rust core: open, view, edit text and cells, save, reopen, share sheet. Web Writer/Sheets (Rust/WASM DOM) ship in the same window.
- **Accept:** on the tier-A fixtures, open → edit → save → reopen passes on one phone and one tablet per OS and in two browsers. VoiceOver, TalkBack and IME checks from M5 pass. Output is validated by the same invariants as desktop.

## Teams track (starts after M5; M6 may overlap T0–T1)
Each slice is demonstrable end to end. Rationale and alternatives: `../architecture/adr-teams.md`.

### T0: Server messaging core (about 3–4 weeks)
- `open365-server` gains teams, channels, threads and messages (append-only events), a WebSocket sync endpoint with resume tokens, and a CLI/MCP surface (`open365 teams send|read|watch`).
- **Accept:** two CLI clients exchange messages in a channel across a forced reconnect with no loss or duplicates. Tenant A cannot read, write or subscribe to tenant B (a test per route and per WebSocket subscription). Agent sends are `egress`-class and audited. SQLite and Postgres both pass.

### T1: Chat client on desktop and web (about 6–8 weeks)
- Team and channel list, threads, replies, edit/delete, reactions, @mentions, unread counts, attachments from Drive. Native UI per ADR-010 (own stack on desktop, Rust/WASM DOM on web).
- **Accept:** a scripted two-user session passes on Windows, Linux, macOS and two browsers. IME and screen-reader checks pass (messages announced, composer usable). Pasted HTML is sanitised. A 50k-message channel scrolls inside a stated p95 frame-time target on stated hardware.

### T2: Presence, file tabs, search, mobile chat and push (about 8–10 weeks)
- Presence, typing, read receipts. File tabs: a channel tab opens a Drive document in Writer or Sheets, with permissions checked per user. Message search. Notification settings.
- iOS and Android chat clients on the shared core. A push gateway delivers content-free wake-ups (APNs, FCM, UnifiedPush).
- **Accept:** a message reaches a locked phone within a stated latency on both OSes. The push payload carries no message text. A user without Drive access sees the tab but not the content. Search never returns a channel the user left.

### T3: Calls and meetings core (about 8–10 weeks)
- 1:1 and group audio/video through the SFU behind a `MediaProvider` trait; join by link; lobby; host mute and remove; in-meeting chat; desktop, web and mobile clients.
- **Accept:** 8 participants across desktop, web, iOS and Android hold a 30 min call within stated loss and latency on stated networks. A room token cannot be reused for another room. A host removal ends that participant's media within 2 s.

### T4: Screen share, calendar and governance (about 8–10 weeks)
- Screen and window share on every platform (ReplayKit, MediaProjection, ScreenCaptureKit, Windows Graphics Capture, PipeWire portal). Scheduled meetings with iCalendar import/export and a basic calendar view. Retention policy, legal-hold basics, admin audit view. Spikes: opt-in E2EE channels (MLS) and a Matrix bridge.
- **Accept:** screen share runs on all client targets with an explicit picker each time. A scheduled meeting round-trips through a standard .ics file. A retention rule deletes messages and their search entries, verified by test. Spike reports are written.

## Later phases (planned boundaries, not scheduled)
After T4: document co-editing (own sync service; CRDT primitives spike Loro vs Yrs; pull it earlier only if Teams file tabs need it) and the CoWorker MVP (provider trait, Ollama/OpenAI-compatible + one BYOK API, read/suggest-only tools). Then Slides, full Mail and Calendar, OIDC org identity hardening, admin and audit depth, and federation (allowlist, OCM-style sharing, Matrix bridge per ADR-014).

## Non-goals for M0–M4
Shipping (not designing) the server beyond sharing Stage 1, co-editing, Teams (T0–T4 start after M5), Slides or CoWorker UI; .doc/.xls/ODF; macro execution, pivot refresh, chart editing, mobile/web clients before M5/M6, claims of "full Office compatibility".

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
| Scope creep into server/Teams before the editors are good | High | High | Teams starts only after M5; non-goals above; an owner review point per milestone. |
| Real-time media quality and cross-platform capture cost far more than estimated | High | High | Proven SFU first (ADR-014); T3 acceptance measures loss and latency; `MediaProvider` stays swappable. |
| Push on self-hosted installs (APNs/FCM credentials belong to the app publisher) | Medium | Medium | Project-run content-free push gateway plus UnifiedPush; documented operator options. |

## Unresolved questions
Formula function priority list; Qt licence fit; parser sandbox mechanism; CoWorker default context scope; fixture corpora licences; target OS priority; Open365's own licence.
