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

## ADR-003 Formula engine behind a `CalcEngine` trait, IronCalc first — Proposed
- **Context:** IronCalc is MIT/Apache-2.0, pre-1.0 (v0.8.3), with XLSX I/O. Its function coverage and recalc design are unverified.
- **Alternatives:** build our own engine; fork IronCalc; LibreOffice Calc via LOK.
- **Decision:** Use IronCalc through our trait, contribute upstream, and keep our own workbook model for preservation.
- **Consequences:** Two models must be kept in sync (ours ↔ engine). Fixture coverage measures the gaps.
- **Confidence:** Medium. **Would change if:** coverage is below about 70% of functions in our corpus, the recalc design cannot do incremental updates, or governance stalls.

## ADR-004 Desktop shell chosen by a head-to-head spike — Proposed (decision needed)
- **Context:** Every option requires a custom document canvas. What differs is what the toolkit provides *around* the canvas: IME, accessibility, printing, clipboard. AccessKit 0.25 lacks rich-text support. No Rust toolkit documents printing. Dioxus desktop is a WebView, and dioxus-native/Blitz is alpha/beta with no IME or accessibility yet.
- **Alternatives:** (A) Qt 6 via cxx-qt with a Rust canvas item; (B) winit + wgpu + vello + parley + AccessKit, with Slint for chrome; (C) per-OS native shells; (D) egui; (E) Dioxus native.
- **Decision (proposed):** Spike A vs B on the same Rust core for 2–3 weeks against the pass/fail criteria in `../research/agent-reports/native-ui.md`. C is the long-term option if A and B both fail accessibility. D and E are rejected for the editor.
- **Consequences:** The core must not depend on the UI (enforced by an xtask layering check).
- **Confidence:** Medium. **Would change if:** Qt LGPL review fails (rules out A), or AccessKit gains text-range/rich-text support (strengthens B).

## ADR-005 LibreOffice is a test oracle, not a product dependency — Proposed
- **Context:** LibreOffice (MPL-2.0) has the broadest open-source OOXML fidelity, but it is a large C++ attack surface.
- **Decision:** Run it headless, out of process, in CI only, for conversion/PDF render/recalc comparisons.
- **Confidence:** High. **Would change if:** users demand legacy formats (.doc/.xls) in the MVP. A sandboxed conversion sidecar would then be considered.

## ADR-006 Server deferred; integrate before build — Proposed
- **Context:** P0/P1 value is file-local editing. Real-time co-editing needs a demand case.
- **Decision:** No Open365 server in P0. P1 sync integrates an existing WOPI/WebDAV host. P2 adds a sync service with a CRDT (Loro vs Yrs spike). Chat/meetings integrate Matrix (Tuwunel) + LiveKit.
- **Confidence:** Medium-high. **Would change if:** the target first users are teams who need co-editing on day one.

## ADR-008 Open365 is MIT-licensed; dependencies must be permissive — Accepted (owner, 2026-10-10)
- **Context:** The owner chose "same as VectorCraft" and then specified MIT.
- **Decision:** The workspace is `license = "MIT"`. `deny.toml` allows only permissive dependency licences (MIT, Apache-2.0, BSD, ISC, Unicode-3.0, Zlib).
- **Consequences:** Copyleft engines (LibreOffice MPL-2.0, ONLYOFFICE AGPL, Synapse AGPL) may only be used out of process: as a test oracle, or as a separately deployed service. Qt (LGPL) as a desktop dependency would need an explicit exception recorded here. This weakens ADR-004 option A.
- **Confidence:** High.

## ADR-007 Developer CLIs are external agents, not inference backends — Proposed
- **Context:** Consumer CLI logins and subscriptions are not licensed as embedded production inference (details in `../research/agent-reports/coworker-providers.md`).
- **Decision:** Open CoWorker ships provider adapters for APIs (BYOK) and local runtimes (Ollama/OpenAI-compatible). Claude Code, Codex CLI and Grok CLI connect *to* Open365 as user-launched MCP clients under the user's own permissions. Open365 never shells out to them for inference.
- **Confidence:** High.
