# Open365 project operating principles

## Product mission

Build Open365: an open, self-hostable productivity and collaboration suite for Office 365-oriented workflows. Prioritize Word-like documents and Excel-like spreadsheets first. The longer-term ecosystem may include presentations, identity/workspaces, real-time collaboration, a Teams-like communication product, federation between independently operated servers, and Open CoWorker (provider-agnostic AI assistance).

The product should learn from the clean-room, native, Rust-first approach and agent interfaces documented by the Storytold/ArtCraft projects, without copying proprietary code, assets, branding, or unlicensed implementation details. VectorCraft is a useful reference for a layered Rust workspace, separate format/render/engine crates, fixture/property-based round-trip tests, and an agent-accessible CLI plus JSON/MCP control interface. Evaluate these patterns against office editing requirements; do not assume its egui UI or rendering architecture is automatically a fit. References: https://github.com/storytold/vectorcraft, https://github.com/storytold/vectorcraft/blob/main/Cargo.toml, and https://getartcraft.com/apps.

## Non-negotiable engineering principles

- **Native desktop UX**: no Electron and no embedded browser/WebView as the primary desktop application UI. Evaluate genuinely native cross-platform UI frameworks and native drawing/input/accessibility/IME/printing. A custom GPU-rendered document canvas is acceptable if hosted in a native application and it meets accessibility and platform integration requirements. Prove the trade-offs before selecting a toolkit.
- **Agent-native control surface**: make core user actions automatable through a stable, documented CLI and/or permission-scoped JSON/MCP control interface. Design authorization, confirmations, auditability, and sandbox boundaries intentionally; do not make an agent control channel a way around user permissions.
- **Office interoperability first**: define measurable import/export fidelity for DOCX, XLSX, OOXML styles, tables, formulas, number formats, charts, relationships, and embedded content. Keep real-world fixture-based round-trip tests. Do not claim full compatibility without test evidence.
- **Reuse before rewrite**: audit existing engines and open-source projects, maturity, extension points, performance, license obligations, project governance, security history, and product fit. Prefer a modular strategy if it beats rebuilding every subsystem.
- **Clean-room implementation**: work from public standards, documented behaviors, user-created fixtures, and lawful interoperability tests. Do not reproduce Microsoft source code, private protocols, trademarks, UI assets, or proprietary implementation details.
- **Self-hosting and federation**: make a clear distinction between client, storage/sync service, identity, real-time collaboration, messaging, meeting/media services, administration, and federation. Separate tenant data, permission checks, key management, and trust boundaries. Design for operator-owned infrastructure and clear data export.
- **Open CoWorker**: model provider adapters, BYOK secrets, user/org policy, subscription entitlements, budgets, audit trails, and local models as separate concerns. Consider Ollama and supported hosted APIs. Treat Claude Code, Codex CLI, and Grok CLI as developer/agent tools unless the current provider's terms and supported integration explicitly permit their use as production inference backends. Never scrape CLIs, reuse personal sessions as a service, or bypass provider billing/authentication.
- **Security and privacy**: least privilege, tenant isolation, secure update/signing, sandboxing for document parsers/macros/plugins, SSRF-safe URL fetching, secrets redaction, audit events, backup/restore, and secure defaults. Treat DOCX/XLSX/attachments as untrusted input.
- **Reliability**: tests before claims. Add regression fixtures for bug fixes. Document performance targets and the hardware/test corpus behind any performance claim.
- **License-aware**: review licenses, CLA/governance, patents where relevant, trademarks, and dependency notices before copying/adopting code. Do not assume an open-source license automatically permits every use.

## How the lead session operates

The main conversation is the accountable product/architecture lead. It owns prioritization, architecture decisions, task breakdown, integration, and the user-facing plan. Use custom subagents for focused research or bounded work; keep subagent descriptions short, prompts specific, and output structured. Delegate independent investigations in parallel when helpful. Ask each agent for findings, evidence/links, trade-offs, uncertainties, and recommended next action. The lead reconciles disagreement and does not blindly accept agent recommendations.

Avoid duplicated effort. Assign one owner per file or code boundary. Subagents should default to research/review and return findings rather than editing overlapping product code. Only implement after the lead has established scope, ownership, and acceptance criteria.

## Decision and delivery discipline

- Inspect the repository, current branch, git status, existing architecture, tests, build commands, license, and project conventions before suggesting sweeping changes.
- Preserve existing user modifications. Never discard or reset user work.
- Prefer small vertical slices with explicit acceptance criteria and a demonstrable path from opening a file to editing/saving/reopening it.
- For each architectural choice, record context, alternatives, decision, consequences, confidence, and what evidence would change the decision.
- Separate confirmed facts, assumptions, hypotheses, and unresolved questions.
- Research current model names, provider terms, standards, licensing, and project maintenance status against primary sources. Link sources and dates in research notes.
- Do not add infrastructure or a self-hosted server just because it is technically interesting; prove the MVP needs it and document operational cost.
- Do not start broad product-code rewrites before producing a repository audit, a proposed architecture, a sequence of small milestones, and a user review point.

## Expected initial artifacts

Where these do not already exist, create and keep concise:

- `docs/research/competitor-landscape.md`
- `docs/architecture/system-context.md`
- `docs/architecture/decision-log.md`
- `docs/roadmap/first-milestones.md`
- `docs/security/threat-model.md`

Adapt paths if the repository already has a coherent documentation convention. Avoid duplicate docs and do not overwrite existing files without reviewing them first.

## Model effort policy

Use medium effort as the default for Opus and Sonnet. Escalate to high or xhigh only for a clearly difficult, high-impact task (for example, a subtle concurrency bug, a security threat-model dispute, or an architecture decision blocked by contradictory evidence). Prefer bounded investigations and explicit acceptance criteria over globally increasing effort. Model/provider support for effort levels varies.
