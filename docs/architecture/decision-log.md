# Decision log

Format: context, alternatives, decision, consequences, confidence, and what evidence would change the decision. Status is **Proposed** until the owner approves it.

## ADR-001 Lossless OPC part graph as the persistence foundation — Proposed
- **Context:** Silent content loss is the main way office-file compatibility fails. No Rust crate guarantees that unknown XML and parts are preserved (docx-rs README: support "not exhaustive").
- **Alternatives:** adopt docx-rs/umya as the model; convert through LibreOffice; build our own thin OPC layer.
- **Decision:** Build our own OPC part graph. Untouched parts stay byte-identical. Typed nodes carry raw-XML `extras`. Unknown parts are kept as blobs. A dropped-content log blocks silent save.
- **Consequences:** More upfront work. The "never silently drop" guarantee becomes testable and a differentiator. The flip side is *silent retention*: hidden text, orphaned parts, stale thumbnails, pivot and external-link caches, VBA. Mitigations: `inspect --hidden`, logged GC of unreachable parts, invalidating thumbnails and signatures on edit (P0), and a tested "Sanitize for sharing" command (P1).
- **Confidence:** High. **Would change if:** an existing crate passes our preservation invariants on the fixture corpus.

## ADR-002 Command layer is the only mutation path — Proposed
- **Context:** The UI, CLI, MCP, CoWorker and sync all mutate documents, and authorisation and audit must not be bypassable.
- **Alternatives:** separate APIs per surface; UI-internal mutation with agent APIs bolted on.
- **Decision:** Every mutation is a typed, serialisable `Command` with an effect class (read/edit/destructive/egress/export), checked against a permission scope and audited. All Open365 surfaces call it.
- **Consequences:** The command schema is a stable contract and needs versioning. Undo/redo and the CRDT mapping build on it. Scopes contain injected agents, not same-user malware (security review 2026-10-10). Remote CRDT updates must be decoded and authorised server-side, because they otherwise bypass this layer.
- **Confidence:** High. **Would change if:** command granularity proves too coarse for interactive typing latency. In that case batch at the input layer, but keep the single path.

## ADR-003 Formula engine behind a `CalcEngine` trait — revised by ADR-009: we build our own engine (`open365-calc`); IronCalc is not used
- **Context:** IronCalc is MIT/Apache-2.0, pre-1.0 (v0.8.3), with XLSX I/O. Its function coverage and recalc design are unverified.
- **Alternatives:** build our own engine; fork IronCalc; LibreOffice Calc via LOK.
- **Decision:** Use IronCalc through our trait, contribute upstream, and keep our own workbook model for preservation.
- **Consequences:** Two models must be kept in sync (ours ↔ engine). Fixture coverage measures the gaps.
- **Confidence:** Medium. **Would change if:** coverage is below about 70% of functions in our corpus, the recalc design cannot do incremental updates, or governance stalls.

## ADR-004 Desktop shell follows the Storytold PhotoCraft stack (egui/eframe on wgpu) — Accepted (owner, 2026-10-10)
- **Context:** The owner named Storytold (PhotoCraft, VectorCraft) as the gold standard and asked to base the UI on PhotoCraft. PhotoCraft (MIT OR Apache-2.0) uses an engine-first design: pure-data document model + command engine, a thin swappable `ui-egui` crate on eframe/egui 0.36 + winit 0.30 + wgpu, `egui_kittest` UI tests, `rfd` file dialogs, a shared command registry used by UI/CLI/MCP, and an authenticated loopback `--control` JSON channel (sources: github.com/storytold/photocraft README and Cargo.toml, read 2026-10-10).
- **Alternatives:** Qt/cxx-qt (LGPL, conflicts with ADR-008); winit+vello+parley+AccessKit hand-built shell; per-OS native shells.
- **Decision:** Adopt the PhotoCraft pattern. Chrome (ribbon, panels, dialogs) in egui via `open365-ui-egui`; the document canvas (paged text) and grid are our own renderers drawn as custom egui widgets/wgpu paint callbacks, fed by the format-agnostic layout crates. The UI crate stays thin and replaceable; all mutations go through the command layer (ADR-002).
- **Consequences / risks we test, not assume:** egui is immediate-mode, so large documents need virtualised layout caches; accessibility of the custom canvas (AccessKit text ranges), IME composition, and printing need explicit work and measurement on Windows/macOS/Linux. The native-UI pass/fail sheet in `../research/agent-reports/native-ui.md` remains the acceptance gate.
- **Confidence:** Medium-high. **Would change if:** the canvas cannot meet screen-reader or IME criteria inside egui; then only the canvas host is swapped (the UI crate boundary exists for this).
- **Native accessibility, IME and printing are ours to build (owner, 2026-10-10: "add screen reader support… if we have to do our own for pure native cross platform we will").** egui already exposes its widgets through AccessKit (UIA on Windows, NSAccessibility on macOS, AT-SPI on Linux). For the document canvas and grid we build our own AccessKit subtree: paragraphs/cells as nodes, `TextRun` nodes with character lengths and positions, caret and selection, and announcements of edits, so NVDA, Narrator, VoiceOver and Orca read and navigate text natively. Gaps in AccessKit's platform adapters are fixed by us and contributed upstream (MIT/Apache). IME uses winit's composition events with our own preedit rendering and candidate-window positioning. Printing goes through our own PDF output to each OS's native print path. Each item has a pass/fail test in the M2 sheet; none is waved through.

## ADR-005 No other office suite anywhere, not even as a test oracle — Accepted (owner, 2026-10-10)
- **Context:** The owner wants a complete suite of our own: "a full 365 not using anything else". The earlier proposal used LibreOffice headless as a test-only oracle.
- **Decision:** No LibreOffice, Collabora, ONLYOFFICE or similar in the product, the tooling or CI. Fidelity evidence comes from (1) values that Excel/Word already wrote into fixtures (cached `<v>` values, field results), (2) our own ECMA-376 schema validation built from the published XSDs, (3) our own renderer with reviewed golden snapshots, and (4) manual "opens cleanly in Microsoft 365" checks, which are the real interoperability target.
- **Consequences:** The schema validator becomes an M1 deliverable.
- **Update (owner, 2026-10-10):** Microsoft 365 (Word/Excel 16.0) is installed locally and scriptable via COM, so it is **the** oracle. `tools/office/` generates fixtures, opens our output (repair prompt = failure), exports reference PDFs, and recalculates for reference values. It is a local developer tool only: Microsoft does not support unattended server-side Office automation, so it does not run in CI. CI uses the stored reference outputs instead.

## Product target: 1:1 parity with Microsoft 365 — Accepted (owner, 2026-10-10)
- **Goal:** everything a user does in Word and Excel works identically in Open365: same file behaviour, same calculation results, same layout.
- **How we hold ourselves to it:** parity is measured, never claimed. Each feature is "done" only when its Office-generated fixtures pass every invariant (round trip, recalculation equal to Excel, pagination equal to Word, opens in Office without repair). A parity dashboard is generated from the fixture results, and the corpus only grows.
- **Honest scale:** Word layout and Excel's full function library are each multi-year efforts for a large team. The order is preserve everything (no loss, day one) → calculate and render identically → edit everything.
- **Confidence:** High.

## ADR-009 Self-contained suite; libraries and open standards allowed — Accepted (owner, 2026-10-10)
- **Decision:** Open365 does not depend on other office products or third-party servers: office suites, formula engines, chat/meeting servers or file platforms. Every office capability is ours: calc engine, layout, sync, chat and meetings. Low-level permissive libraries (zip, XML, text shaping, fonts, GPU, crypto, CRDT primitives) and open standards/protocols (OOXML, OIDC, WebRTC, and optionally Matrix/WOPI for interop) are allowed.
- **Consequences:** This supersedes "wrap IronCalc" in ADR-003, and "integrate Tuwunel/LiveKit/OpenCloud" in ADR-006 and system-context. The scope and timeline grow, mainly from the formula engine and, in later phases, the media server.

## ADR-006 Server deferred; integrate before build — Proposed
- **Context:** P0/P1 value is file-local editing. Real-time co-editing needs a demand case.
- **Decision:** No Open365 server in P0. P1 sync integrates an existing WOPI/WebDAV host. P2 adds a sync service with a CRDT (Loro vs Yrs spike). Chat/meetings integrate Matrix (Tuwunel) + LiveKit.
- **Confidence:** Medium-high. **Would change if:** the target first users are teams who need co-editing on day one.

## ADR-008 Open365 is MIT-licensed; dependencies must be permissive — Accepted (owner, 2026-10-10)
- **Context:** The owner chose "same as VectorCraft" and then specified MIT.
- **Decision:** The workspace is `license = "MIT"`. `deny.toml` allows only permissive dependency licences (MIT, Apache-2.0, BSD, ISC, Unicode-3.0, Zlib).
- **Consequences:** Copyleft engines (LibreOffice MPL-2.0, ONLYOFFICE AGPL, Synapse AGPL) may only be used out of process: as a test oracle, or as a separately deployed service. Qt (LGPL) as a desktop dependency would need an explicit exception recorded here. Superseded in practice by ADR-004 (egui stack).
- **Confidence:** High.

## ADR-007 Developer CLIs are external agents, not inference backends — Proposed
- **Context:** Consumer CLI logins and subscriptions are not licensed as embedded production inference (details in `../research/agent-reports/coworker-providers.md`).
- **Decision:** Open CoWorker ships provider adapters for APIs (BYOK) and local runtimes (Ollama/OpenAI-compatible). Claude Code, Codex CLI and Grok CLI connect *to* Open365 as user-launched MCP clients under the user's own permissions. Open365 never shells out to them for inference.
- **Confidence:** High.
