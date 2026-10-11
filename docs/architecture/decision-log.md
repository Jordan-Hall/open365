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

## ADR-010 Web and mobile get their own native UIs, written in Rust — Accepted direction (owner, 2026-10-10); stack chosen by spike
- **Context:** The owner wants Open365 on web and mobile, each with "its own UI" and "a full on native solution", ideally with Rust writing it. Not a canvas renderer shipped everywhere, and not a WebView wrapper.
- **Decision:** One shared Rust core: the OPC/docx/xlsx models, calc, layout, the command layer and later sync. It sits under three UI families, each built from that platform's native building blocks:
  - **Web:** Rust compiled to WebAssembly that builds and updates the real DOM, so browser text selection, IME, accessibility, find, spellcheck and printing work natively. Candidates: Leptos (MIT), Dioxus web (MIT/Apache) and direct `web-sys`. Document pages are DOM elements positioned by our layout engine, not a single canvas.
  - **iOS/iPadOS:** native UIKit views driven from Rust. Candidates: `objc2` + `objc2-ui-kit` bindings (MIT/Apache, Rust calls UIKit directly), or a thin Swift shell over a Rust core via a permissively licensed binding generator. The document view uses UITextInput for IME and UIAccessibility, plus the native print path.
  - **Android:** native Android Views driven from Rust over JNI (`jni` crate, MIT/Apache). The alternative is a thin Kotlin/Compose shell over the Rust core. The document view uses InputConnection for IME and AccessibilityNodeInfo.
- **Rules:** the UI layer stays thin and replaceable on every platform (as with ADR-004). All mutations go through the command layer (ADR-002). Platform shells share no UI code with each other, only the core. Dependency licences stay permissive (ADR-008). UniFFI is MPL-2.0 and needs review before use.
- **Spike (scheduled after M3, before P2):** for each platform, open, edit, save and reopen a tier-A DOCX and XLSX. Pass/fail rows cover IME, the screen reader (VoiceOver, TalkBack, browser plus NVDA), selection and clipboard, scroll p95 on large-sheet, app size and cold start. Pick the per-platform stack with that evidence.
- **Consequences:** The core must stay free of desktop-only assumptions: no egui types, no threads the browser can't run, and no blocking I/O in the core. File access sits behind a storage trait. CI gains a wasm32 build of the core crates now, so the core stays portable.
- **Confidence:** High on direction. The stack is open until the spike.

## ADR-011 Every Office locale is supported, not just en-US/en-GB — Accepted (owner, 2026-10-10)
- **Context:** The owner said "We need to support all settings not just UK and US". Excel and Word change behaviour by locale:
  - number display: decimal and grouping separators, currency, negative patterns, digit substitution;
  - built-in date/time formats, plus day, month and era names;
  - date and number text parsing (`DATEVALUE`, `VALUE`, cell entry);
  - list and argument separators and localised function names in the formula bar (files always store en-US invariant formulas);
  - collation for sorting and lookup, `UPPER`/`LOWER` casing, first day of week;
  - calendars: Gregorian variants, Hijri, Japanese era, Thai Buddhist, Korean;
  - in Word: hyphenation, line breaking, East Asian and complex-script layout, RTL and proofing language.
- **Alternatives:** hand-written per-locale tables (they don't scale); copying Windows NLS data (not ours to copy); Unicode CLDR via ICU4X.
- **Decision:**
  - Add a new core crate, `open365-locale`. Its data comes from Unicode CLDR through ICU4X (Unicode-3.0 licence, already allowed in deny.toml). The data is baked in at build time, so the crate stays wasm- and mobile-portable.
  - Where Office deviates from CLDR, a reviewed override table records the difference. Each entry is backed by an oracle measurement.
  - The engine takes an explicit `Locale` (BCP-47 tag plus Office LCID). There is no global state: the document, the user and the system locale are separate inputs.
  - Excel's in-format locale tags (`[$-xxxx]`) take precedence, as in Excel.
- **Evidence and oracle:**
  - Formatting is tested per locale without touching the machine's settings, using Excel's `[$-LCID]` format prefixes and `TEXT()`.
  - Behaviour that follows the system locale (parsing, separators, built-in formats) needs the oracle to run under each culture. That runs in a separate Windows user profile or VM per culture, never by changing the owner's own settings.
  - Target matrix: every locale Office supports. The first ratchet covers the top 40 by user population, then all of them.
- **Consequences:** calc's numfmt, date-text parsing, collation and the grid/formula bar all take a `Locale`. Layout gains script and language-aware line breaking (ICU4X segmenter) and later complex-script shaping and RTL. The conformance corpus tags each case with its locale.
- **Confidence:** High. **Would change if:** CLDR and Office disagree so often that the override table grows larger than the CLDR-derived data. Even then, CLDR stays as a fallback.

## ADR-012 One app per product with an Office-identical chrome shell — Accepted (owner, 2026-10-11)
- **Context:** The owner decided each product ships as its own app (Open365 Writer, Open365 Sheets) that looks exactly like the Office app it replaces. `apps/desktop` was one window with a basic menu.
- **Alternatives:** keep one app with a mode switch (rejected by the owner); restyle egui widgets generally (cannot give Office geometry); embed a web UI (forbidden: native only); copy Office icons (forbidden).
- **Decision:** a chrome-only crate, `open365-ui-shell`, driven by data (`spec/*.json`) generated from oracle measurements (`tools/office/ribbon-oracle.ps1`, UI Automation rectangles at 125 %). Two thin binaries host the existing `DocView`/`GridView` through a `ShellHost` trait. Icons: Tabler Icons v3.49.0 (MIT), rasterised with resvg; no Microsoft artwork or product names. Segoe UI is read from the OS, never bundled.
- **Consequences:** geometry is testable (`geometry.rs`: 100 % of measured tab, group and control rects within 1 px at 125 %). 100 % and 150 % are derived, not measured. Light-theme colours are estimates. New dependency: `resvg` (BSD/MIT/Apache tree, allowed by deny.toml). `GridView` still draws its own formula bar, sheet tabs and font table: API changes are listed in `app-shell.md`.
- **Confidence:** Medium-high on structure; medium on light colours and non-125 % scales. **Would change if:** a second oracle run at 100 % or 150 % (separate profile) disagrees with the derived values by more than 1 px, or egui text metrics force per-control width estimates.
