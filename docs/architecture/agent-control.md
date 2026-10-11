# Agent control surface

Status: implemented (M3 slice, extended). Decision basis: ADR-002 (the command layer is the only mutation path) and ADR-007 (developer CLIs are external agents). The operator guide for the MCP server is `docs/agents/mcp.md`; the CLI reference is `docs/reference/cli.md`.

The agent channel must never be a way around user permissions. Everything here follows from that: an agent gets what the operator grants, and a person, not the agent, decides anything beyond it.

## Surfaces

| Surface | Use | Transport |
|---|---|---|
| `open365` CLI | scripts, developers, agents that run shell commands | argv, stdout (`--json` envelope), stderr audit lines |
| `open365 mcp` | MCP clients such as Claude Code | JSON-RPC 2.0, newline-delimited, stdio only; no network listener |

Both call `open365_commands::Target::apply`. Nothing edits a document any other way. The CLI and the MCP server share two pieces of code so they cannot disagree:

- `open365_mcp::registry`: the command catalogue. One entry per `Command` variant with its domain, summary and a runnable JSON example. `open365 commands`, `open365 run`, the MCP `list_commands` tool and the `open365://commands` resource all read it.
- `open365_mcp::policy`: for a command, its effect class, the scope that covers it and whether it needs confirmation. `assess_in` classifies against the document as it is now and `assess_sequence` classifies a batch, each command against the state the earlier ones leave (a scratch copy). The MCP tools and the CLI gate both call them. `assess` (no document) is for commands whose class does not depend on content, such as `export_pdf`.

## Coverage

Every command of `crates/commands` (31 today) is reachable from both surfaces for documents and workbooks:

| Action | CLI | MCP |
|---|---|---|
| open | every `docx`/`xlsx` subcommand opens its file | `open_document` |
| inspect structure | `inspect`, `docx|xlsx inspect`, `outline`, `docx layout` | `inspect`, `get_outline`, `layout_summary` |
| inspect text | `docx text` | `read_text` |
| inspect cells and formulas | `xlsx cells`, `xlsx formulas` | `read_cells`, `read_formulas` |
| inspect styles | `docx styles`, `xlsx styles [--range]` | `get_styles` |
| edit | `run <command>`, `apply <file>` | `apply_commands` (`dry_run` available) |
| recalc | `xlsx recalc`, `xlsx eval` | `recalc`, `evaluate_formula` |
| save | `run save`, `apply -o` | `save_as` |
| export PDF | `docx export-pdf`, `run export_pdf` | `export_pdf` |
| new from template | `docx|xlsx new-from-template` | `new_from_template` |
| list commands | `commands` | `list_commands`, resource `open365://commands` |

`recalc` recalculates a sheet's formulas from scratch in a scratch engine and reports cells whose stored value differs. It changes and saves nothing; edits made through commands already recalculate their dependents, and an edited workbook asks the application to recalculate on load.

### Keeping coverage complete

`registry.rs` has a test that reads the source of `enum Command` and fails when a variant has no catalogue entry (or an entry names a removed variant). `crates/mcp/tests/coverage.rs` runs every catalogue example through MCP and `apps/cli/tests/surface.rs` runs every example through `open365 run`, both on real fixtures. A new command therefore fails the build until it has an entry that executes on both surfaces. The count is asserted equal to the catalogue length, and at least 31.

## MCP protocol

Checked 2026-10-11 against the specification at <https://modelcontextprotocol.io/specification> (revision **2026-07-28**; changelog at `/specification/2026-07-28/changelog`).

The server is dual-era, as that revision allows:

- **2026-07-28 (modern, stateless).** A request with `params._meta["io.modelcontextprotocol/protocolVersion"]` is served statelessly. `server/discover` returns `supportedVersions`, `capabilities`, `instructions` and `_meta` server info. Results carry `resultType: "complete"`; `tools/list`, `resources/list`, `resources/templates/list` and `resources/read` carry `ttlMs` and `cacheScope`. An unsupported version returns error `-32022` with `data.supported` and `data.requested`. `ping` is removed in this revision and answers `-32601` on a modern request. A missing resource is `-32602`.
- **2025-11-25, 2025-06-18, 2025-03-26, 2024-11-05 (legacy).** `initialize` negotiates the version; `ping` works.

Because the modern revision has no sessions, `resources/list` returns the same two resources whatever documents are open (`open365://commands`, `open365://session`). Documents are addressed by the server-minted handle from `open_document`, passed as an ordinary argument or URI segment (`open365://document/{document}/outline`, `/structure`, `/styles`, `/text/{start}`, `/cells/{sheet}/{range}`, `/formulas/{sheet}`; listed by `resources/templates/list`). Unsupported and deprecated features (roots, sampling, logging, subscriptions, tasks, prompts) are not offered.

Framing: one JSON object per line, UTF-8, no embedded newlines; batches are rejected; request line at most 4 MiB.

## Permission model

### Scopes

| Scope | Allows | Default |
|---|---|---|
| `read` | open documents, read text, cells, formulas, styles, structure, layout, formula evaluation, recalc, resources | always held |
| `edit` | `apply_commands` on the open document (in memory) | off |
| `save` | `save_as` | off |
| `export` | `export_pdf`, `new_from_template` | off |
| `execute-external` | commands whose effect is `egress` (send data off the machine). None exists yet; the scope is reserved so the first one is denied by default | never, unless the operator names it at start-up |

The operator sets scopes at start-up: `--allow-edit` (shorthand for edit, save, export) and `--grant <scope>` (repeatable). A host embedding the library can call `Server::grant` and `revoke`. `read` cannot be revoked.

### Per-session grants

Scopes belong to one server process, which is one MCP stdio session. An agent can call `request_scope {scope, reason}` for `edit`, `save` or `export`. The request goes to the confirmation hook; the server grants the scope only if a person approves it. `execute-external` cannot be requested: the call is refused without asking anyone. The grant lasts until the process ends.

### Confirmation hook

`scope::Confirm` is called before:

1. a destructive or egress command, classified by its actual effect (`Command::effect_in`): deleting or replacing a non-empty range, replace-all, deleting rows or columns, writing a value or formula over a filled cell, pasting over non-empty cells, merging cells so that values are dropped, clearing formats that exist. The match in `Command::effect` and `overwrites_content` has no catch-all arm, so a new command does not compile until it is classified. A scan that fails counts as destructive. The static `Command::effect` is not for agent decisions; an interactive UI, where the person at the keyboard is the authority, may use it,
2. replacing a file that exists (`save_as` to any existing path, the file the document was opened from included, `export_pdf` or `new_from_template` with `overwrite: true`),
3. widening the session's scopes (`request_scope`).

Implementations: `DenyAll` (default: nobody is asked, so the action is refused), `OperatorFlag` (`--allow-destructive`: destructive actions are approved in advance for the session, scope requests are still refused), and `TerminalConfirm` (`--confirm terminal`: asks `[y/N]` on the operator's console, `CONIN$`/`CONOUT$` on Windows and `/dev/tty` elsewhere, which is separate from the agent's pipe). The prompt text is built by the server from command names, counts and resolved paths; control, format (bidirectional overrides, zero-width and tag characters), line and paragraph separator and non-ASCII space characters from the agent are replaced and the length is capped, so an agent cannot forge a prompt. The one agent sentence, the `reason` of `request_scope`, is shown only quoted and labelled untrusted, cut to 120 characters, and never logged. Document content never reaches the prompt. The answer, who gave it and the reason are in the audit line. The agent cannot supply a confirmation in any argument.

All commands of a call are classified first; one refusal means none run. `dry_run` reports each command's effect class and whether it would be permitted without asking.

### Path sandbox

Every path an agent names must resolve inside a `--root` folder. Paths are canonicalised with symlinks and junctions followed; the parent of a new file is resolved the same way and an existing symlink at the target is refused. Refused: `..` escapes, links leaving a root, UNC and device paths, alternate data streams, reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `COM0-9`, `LPT0-9` and the superscript forms `COM¹²³`, `LPT¹²³`), NUL bytes. Resource URIs cannot name files at all. The CLI's `run`, `apply` and `new-from-template` take optional `--root` folders and apply the same check to `save`, `export_pdf` and `-o`; `save`, `-o` and the template output are written at the resolved path, never at the argument as typed. Every CLI write goes through one gate (`apply_cmd::gate_write`): fence, classification, refusal without `--allow-destructive`, one audit line on stderr.

Saves write a temporary file with a random name in the destination folder (`create_new`, so a planted file or link is never opened), flush it, re-check that the destination folder still resolves inside a root, then rename. Documents opened through MCP carry the roots into the save (`Target::restrict_writes_to`).

### Limits

Request line 4 MiB; response 1 MiB; document file 32 MiB; PDF 64 MiB; 8 open documents; 1000 commands per call; 10000 cells and 256 KiB of text per read; formula 8192 characters. `layout_summary`, `export_pdf`, `recalc` and `evaluate_formula` run under `--timeout-secs` (default 30) with cooperative cancellation; a timed-out export leaves no file.

## Audit log

`--audit-log` is a file opened for append; each line is one JSON object (`schema: 1`):

| Field | Meaning |
|---|---|
| `seq`, `ts_ms`, `origin`, `pid`, `session` | ordering and identity |
| `event` | `session_start`, `tool_intent`, `tool_call`, `resource_read` or `protocol_error` |
| `scopes` | scopes held when the line was written |
| `tool` / `uri`, `status` | `ok`, `refused`, `invalid`, `timeout`, `error` |
| `reason` | why, for anything but `ok`: a fixed code (`path_denied`, `scope_not_granted`, `confirmation_denied`, `invalid_commands`, `limit_exceeded`, `target_exists`, `audit_unavailable`, `parse_error`, `unsupported_version`, ...), never error text |
| `intent_seq` | on a `tool_call` line: the `seq` of its `tool_intent` line |
| `effect`, `effects`, `commands`, `path`, `document`, `dry_run`, `applied`, `bytes_written` | what it touched |
| `confirmation` | `{kind, decision, by or reason}` when the hook was asked |

One line is written for every `tools/call` (including unknown tools and non-object arguments), every `resources/read` and the first request of a session (`initialize` for the legacy era, any request for the stateless one; `stateless: true`). A call that can change something (`apply_commands` without `dry_run`, `save_as`, `export_pdf`, `new_from_template`, `request_scope`, `close` with `discard`) writes a `tool_intent` line first; if that line cannot be written the call is refused and does not run. Requests turned away by the protocol layer (`parse_error`, `invalid_request`, `unsupported_version`, `tools_call_without_name`, `method_not_found`, `request_too_large`, `request_not_utf8`) are `protocol_error` lines. Document text, cell values, formula text and the agent's free-text `reason` are never written. Every agent-supplied value (path, document id, range, cell, URI, client name) is kept as it is up to 256 characters, with control and format characters replaced; a longer one is cut to 64 characters and gets `<field>_len` and `<field>_fp`, a 64-bit SipHash fingerprint (not cryptographic) for correlation. If a line cannot be written the server refuses all further calls and reads. Keep the log outside every `--root`. The CLI writes one JSON line per command to stderr for `apply`, `run` and `new-from-template`, including refusals.

A result too large for the 1 MiB response limit does not turn a completed call into an error: `apply_commands` trims its command log and trailing entries and sets `truncated: true`; any other completed effectful call reports success with `truncated: true`.

## Threat model

Contained: a prompt-injected agent (T4 in `docs/security/threat-model.md`) that tries to read outside the roots, write without a scope, destroy content, replace files, grant itself rights or hide its actions. Not contained: malware running as the same user, which can edit the files directly or run the CLI with all flags.

## Open items

- Cooperative cancellation: a single step that never checks can overrun the deadline.
- A check-then-use race on paths needs a local writer inside the root. The save re-checks the destination folder right before the rename, which narrows the window but cannot close it: that needs directory handles (`openat`/`renameat`, or the NT equivalents), which `std` lacks and the workspace cannot reach without `unsafe`. A root must not be writable by anyone the operator does not trust. `export_pdf` creates its file exclusively without that re-check.
- Scope grants are per process; there is no persisted policy or per-tool expiry.
- No undo tool; no `execute-external` command exists yet to exercise that scope.
- `TerminalConfirm` cannot run headless (no console); it then refuses.
- The CLI does not require `--root`; its user is the operator.
