# Open365 MCP server (local, stdio)

`open365 mcp` lets an agent such as Claude Code read and edit documents through the command layer (ADR-002, ADR-007). The agent connects to Open365 as a user-launched MCP client under the user's own permissions. Open365 never calls the agent for inference.

Status: first version (M3 slice). Stdio only; there is no network listener.

## Setup for Claude Code

Build the CLI (`cargo build --release -p open365-cli`), then register the server. Everything after `--` is the command Claude Code launches:

```
claude mcp add open365 -- open365 mcp --root C:\Users\me\Documents\open365 --audit-log C:\Users\me\open365-audit.jsonl
```

That registration is read-only. To allow edits, add `--allow-edit`. To allow destructive commands as well, add `--allow-destructive` (it requires `--allow-edit`). Put the audit log outside every `--root`.

Flags:

| Flag | Meaning |
|---|---|
| `--root <dir>` (repeatable, required) | Folders the agent may open and save in. Relative paths resolve against the first root. |
| `--audit-log <file>` (required) | Appended; one JSON line per tool call. |
| `--allow-edit` | Permit `apply_commands` and `save_as`. |
| `--allow-destructive` | Permit destructive and egress commands, and `save_as` over a different existing file. |
| `--include-hidden` | Include hidden text, hidden sheets, rows and columns, and hidden-content findings in reads. |

## Tools

| Tool | Effect | Purpose |
|---|---|---|
| `open_document {path}` | read | Opens a `.docx`, `.xlsx` or `.xlsm` inside a root. Returns a document id (`doc1`, ...) and a summary. At most 8 open at once; files up to 32 MiB. |
| `get_outline {document}` | read | DOCX: headings with paragraph index and level. XLSX: visible sheets with used ranges. |
| `read_text {document, start?, count?}` | read | DOCX paragraph text. At most 1000 paragraphs and about 256 KiB per call; `next_paragraph` continues. |
| `read_cells {document, sheet, range}` | read | XLSX values and formulas for a range such as `A1:D20` (at most 10000 cells). `sheet` is an index or a name. |
| `inspect {document}` | read | Parts, relationships, content types, integrity issues. Comment, custom XML, macro, thumbnail and `docProps` parts are not listed unless `--include-hidden`, which also adds hidden-content findings. |
| `apply_commands {document, commands, dry_run?}` | edit / destructive | Applies the same JSON commands as `open365 apply`. `dry_run` runs them on a scratch copy and reports each command's effect class and whether it would be permitted; it needs no flag. A `save` command is refused; use `save_as`. |
| `save_as {document, path}` | edit / export | Writes the document through the `save` command. Same path as opened: `edit`. New path: `export`. |
| `close {document, discard?}` | read | Refused while there are unsaved changes unless `discard` is true. |

Responses are JSON text. Reads carry a `notice` that document content is untrusted data. Tool failures return `isError: true` with a `status` of `refused` (policy or fence), `invalid` (bad arguments) or `error`.

The commands are the serialised `Command` enum in `crates/commands`; the CLI and MCP take the same JSON, and the same commands give byte-identical files and the same per-command effect and changed-cell results (tested in `apps/cli/tests/mcp.rs`).

## Security model

Threat context is T4 in `docs/security/threat-model.md`: a prompt-injected agent. This contains agents, not malware running as the same user.

1. **Fenced roots.** Every path is canonicalised (symlinks followed) and must lie inside a `--root`. Refused: `..` escapes, symlinks or linked folders that leave a root, UNC and device paths (`\\server\share`, `\\?\`, `\\.\`, `//host`), alternate data streams (`name:stream`) and reserved device names (`NUL`, `CON`, ...). `save_as` resolves the parent folder the same way and refuses an existing symlink as the target.
2. **Read-only by default.** Without `--allow-edit`, `apply_commands` and `save_as` are refused, and the refusal is audited. `dry_run` is the suggest path.
3. **Effect classes.** `Command::effect` decides. Edit and export need `--allow-edit`. Destructive and egress (for example deleting or replacing a non-empty range) also need `--allow-destructive`. All commands in a call are classified first; one refusal means none run. Overwriting a different existing file needs `--allow-destructive`.
4. **Hidden content is not returned.** `w:vanish` runs, hidden sheets (including very hidden; they answer "no such sheet"), hidden rows and columns, comments and author metadata never appear in reads or in `inspect` unless the operator passes `--include-hidden`. The agent cannot change this.
5. **Audit.** Every `tools/call` appends one JSON line: `seq`, `ts_ms`, `origin: "mcp"`, `pid`, `tool`, `status`, `reason` on refusal, the document id, resolved path, command names and effect classes. Document text and cell values are never written to it. If a line cannot be written, the server refuses all later calls.
6. **Limits.** Request line 4 MiB; response 1 MiB; document file 32 MiB; 8 open documents; 1000 commands per call; 10000 cells and 256 KiB of text per read. Package parsing uses the `open365-opc` limits (zip bombs, part counts).
7. **Untrusted text.** The server tells the client that document text is data. Honouring that is up to the client; the flags above are what actually bound the damage.

## Known gaps

- **No native confirmation UI.** The threat model wants destructive and egress effects confirmed in Open365's own UI, never through MCP. That UI does not exist yet, so the only gate is the `--allow-destructive` start-up flag. Operators decide once per server start, not per action.
- **Hidden-content coverage.** Only direct `w:vanish` on a run is detected. Hidden character styles and style-level hidden text are not, and offsets in `read_text` are marked `offsets_match_text: false` for paragraphs where hidden runs were dropped. Text inside content controls and alternate-content blocks is not returned at all. Tracked-change deletions are not returned (they are not text in the model).
- **Race between check and use.** Paths are checked, then the file is read; a process that can write inside the root could swap a file in between. Out of scope for the injected-agent threat.
- **No `no-egress` label handling**, per-session scopes, or CoWorker parity yet; no resources or prompts (tools only).
- **Partial apply.** A command that fails mid-call leaves earlier commands of the same call applied in the open document (the error says how many). Nothing reaches disk until `save_as`.
- The server cannot undo; there is no undo tool yet.
