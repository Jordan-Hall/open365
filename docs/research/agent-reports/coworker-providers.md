# Open CoWorker: architecture and provider/terms risk (research, 2026-10-10)

Legend: [F] = verified fact from a primary source fetched 2026-10-10; [I] = inference; "unverified" = not confirmed from a primary source. Not legal advice; have counsel review before shipping.

## 1. Provider contract [I, design]
- Trait `Provider` (async, object-safe): `capabilities() -> Caps{ctx_tokens, max_out, tools, parallel_tools, vision, streaming, json_schema, reasoning, local, data_region}`; `stream(req, cancel: CancelToken) -> Stream<Event>`.
- Events: `TextDelta | ToolCallDelta | ToolCallDone | Usage{in,out,cached,reasoning} | Done{stop_reason} | Error{kind, retry_after}`. Cancel = drop stream + close connection (HTTP/2 RST); record partial usage.
- Errors normalised: `RateLimited(retry_after) | Auth | Quota | ContextExceeded | ContentFilter | Transient | Fatal`. Retry only idempotent calls, jittered backoff, hard per-request and per-turn timeouts, global circuit-breaker per provider.
- Cost: provider-reported usage x price table (versioned, org-overridable); local models cost 0 but count tokens/time.
- Tools: CoWorker sees only Open365 document commands (typed JSON schema, same registry as the CLI/MCP surface). Provider adapters translate schemas; capability-less models get no tools (read-only chat).
- Adapters: native Anthropic, native Gemini, plus one OpenAI-compatible adapter (OpenAI, xAI, Mistral, Ollama, llama.cpp, vLLM). Ollama's OpenAI-compatible API supports chat completions with streaming, tools, vision (base64 only), JSON mode; Responses only non-stateful; no logprobs [F: docs.ollama.com]. Per-server quirks need conformance tests.
- MCP: two separate roles. (a) Open365 as MCP server: external agents drive documents via the same permission-scoped command registry (local stdio/loopback socket, per-session token). (b) CoWorker as MCP client: off by default, org allow-list of servers, each tool call goes through the same policy engine; MCP tool output is untrusted data.

## 2. Secrets [I]
- Desktop: OS keychain (Windows Credential Manager/DPAPI, macOS Keychain, Secret Service); fall back to explicit passphrase-encrypted file, never plaintext. Server: envelope encryption with KMS/operator-supplied key; keys write-only in UI.
- Never in documents, logs, crash dumps, agent/MCP output or prompts. Redaction: typed `Secret<T>` (no Debug/Serialize), pattern scrubber on logs and transcripts, canary test.
- Per-user BYOK default; org keys optional with admin-managed rotation, scoped per workspace; usage attributed per user either way. Org keys are never visible to users or to agent tools.
- Reference practice: Codex warns its `auth.json` holds tokens "like a password" and supports keyring storage [F: learn.chatgpt.com/docs/auth].

## 3. Policy [I]
- Org policy (signed, server-distributed, enforced client-side and server-side): provider/model allow-list, region, max tokens, tools allowed, MCP servers allowed.
- Data classification: labels on workspace/document (`public/internal/restricted/no-egress`). `no-egress` => local-provider-only or hard block; content to hosted providers only after label check; send minimal context (selection, not whole doc) by default.
- Budgets: per-user/org/day/month token+currency caps, pre-flight estimate, hard stop, alerts.
- Audit events (append-only, content-hash not content by default): provider, model, user, doc ID, classification, tool calls, approvals, token/cost, policy decision.
- Prompt injection: document text, comments, linked files and tool results are untrusted data, never instructions. Tool scopes least-privilege per session (read / suggest / edit-range). Edits as reversible tracked proposals; confirmation for destructive/bulk edits, sharing, export, network, deletion; no tool can read secrets or make arbitrary URL fetches (SSRF-safe fetcher only); rate-limit tool calls; log every call.

## 4. Provider risk matrix
| Provider | Integration | Auth | Embedding/BYOK ToS | Retention/training default | Risk |
|---|---|---|---|---|---|
| Ollama | HTTP localhost, OpenAI-compat | none (local) | MIT licence [F: github.com/ollama/ollama LICENSE]; model licences vary per model [I] | local; cloud-hosted models: unverified | Low (check each model licence) |
| Anthropic API | native Messages API | API key (Console) | Devs building products must use API keys; no Claude.ai login in own apps [F] | Commercial: not trained on; 30-day retention, ZDR for qualified [F: Claude Code data-usage, commercial context]. API-only retention page unreachable: unverified | Low with BYOK |
| OpenAI API | Responses/Chat API | API key | Commercial terms page not fetched: unverified | Not used for training; abuse logs up to 30 days; ZDR/MAM by approval [F] | Low |
| xAI API | OpenAI-compat | API key | Terms not fetched: unverified | Not trained without permission; 30 days stored; ZDR available [secondary search of docs.x.ai, partly unverified] | Low-Med |
| Gemini API | native / OpenAI-compat | API key | Additional ToS: unverified in full | Unpaid tier: used to improve products and human review; paid: not used, logged for abuse checks [F via search of ai.google.dev/terms] | Med: warn/block free-tier keys for restricted data |
| Mistral | OpenAI-compat | API key | unverified | Reported no training on API, 30-day logs, EU hosting; only third-party sources: unverified | Low-Med |
| llama.cpp / vLLM | OpenAI-compat server | optional token | Server licences (MIT / Apache-2.0): unverified this session | local | Low; local server must bind loopback |

Policy fix for Gemini: classify free-tier keys as "may train" and block for non-public labels.

## 5. Developer CLIs
- Claude Code: OAuth is for subscribers using Anthropic's native apps; developers building products (incl. Agent SDK) "should use API key authentication"; Anthropic does not permit third parties to offer Claude.ai login, route requests through Free/Pro/Max credentials for their users, or collect/store Claude.ai credentials. A user may sign in to the unmodified binary with their own subscription [F: code.claude.com/docs/en/legal-and-compliance]. Embedding the binary needs Commercial Terms, unmodified, each user on their own credentials [F].
- Codex CLI: API keys recommended for automation; ChatGPT sign-in is supported inside Codex itself; custom providers supported [F: learn.chatgpt.com/docs/auth]. Third-party reuse of the ChatGPT login: sources conflict (a Sept 2026 "Login with ChatGPT" for third-party apps reported only by secondary press; earlier guide says unsanctioned): unverified, treat as not allowed.
- Grok CLI (Grok Build): subscription OAuth and API key reportedly both exist; only third-party sources found, official terms unverified.

| Tool | Classification |
|---|---|
| Claude Code / Agent SDK | Do not ship with subscription login. Only API-key (BYOK) in product, or user-launched external agent |
| Codex CLI | Ship only as user-launched external agent; API-key BYOK for any in-product use |
| Grok CLI | Developer workflow only until terms verified |

Safe pattern [I, consistent with the Anthropic text above]: the user installs and signs in to their own CLI agent, which connects to Open365's local MCP server as a client under user permissions. Open365 never sees, stores or relays the CLI's credentials or tokens, never spawns/scrapes the CLI's session, and bills nothing. Conditions: loopback/stdio only, per-session token, scoped consent screen, audit label "external agent", same policy and confirmations as CoWorker, no auto-launch.

## 6. MVP scope [I]
Build: provider trait; OpenAI-compatible adapter + Ollama; one native hosted adapter (Anthropic) with BYOK; keychain storage; streaming/cancel; read/suggest-only tools through the command registry; per-user budgets; audit log; local-only and no-egress flag; MCP server (loopback) shipped before the CoWorker UI.
Defer: org/KMS key vaults, server-side routing and entitlements, MCP-client, multi-step autonomous edits, Gemini/Mistral/xAI adapters, resale/subscription billing, any CLI embedding.

## Sources (accessed 2026-10-10)
- https://code.claude.com/docs/en/legal-and-compliance
- https://code.claude.com/docs/en/data-usage
- https://developers.openai.com/api/docs/guides/your-data
- https://learn.chatgpt.com/docs/auth (Codex auth)
- https://docs.ollama.com/api/openai-compatibility
- https://raw.githubusercontent.com/ollama/ollama/main/LICENSE
- Search-only/secondary: ai.google.dev/terms; docs.x.ai/docs/faq/security; Mistral summaries (meetily.ai, conductatlas.com); runtimewire.com and developer.puter.com (OpenAI third-party login); amplifying.ai/coding-agents/grok-build
- Not reachable this session: anthropic.com commercial terms, docs.x.ai direct, ai.google.dev direct, legal.mistral.ai, OpenAI terms.
