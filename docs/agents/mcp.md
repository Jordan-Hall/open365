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
| `--audit-log <file>` (required) | Appended; JSON lines: an intent line before each call that can change something, then one result line per tool call, plus session starts and rejected requests. |
| `--allow-edit` | Permit `apply_commands`, `save_as`, `export_pdf` and `new_from_template`. |
| `--allow-destructive` | Permit destructive and egress commands, and `save_as`, `export_pdf` or `new_from_template` over an existing file (the call must also pass `overwrite: true` for the last two). |
| `--include-hidden` | Include hidden text, hidden sheets, rows and columns, and hidden-content findings in reads. |
| `--timeout-secs <n>` (default 30, 1 to 3600) | Longest `layout_summary`, `export_pdf` or `evaluate_formula` may run (see Run time). |

## Tools

| Tool | Effect | Purpose |
|---|---|---|
| `open_document {path}` | read | Opens a `.docx`, `.docm`, `.dotx`, `.dotm`, `.xlsx`, `.xlsm`, `.xltx` or `.xltm` inside a root. A VBA project is kept byte for byte and never run. A macro-enabled file or template can only be saved with its own extension.  Returns a document id (`doc1`, ...) and a summary. At most 8 open at once; files up to 32 MiB. |
| `get_outline {document}` | read | DOCX: headings with paragraph index and level. XLSX: visible sheets with used ranges. |
| `read_text {document, start?, count?}` | read | DOCX paragraph text. At most 1000 paragraphs and about 256 KiB per call; `next_paragraph` continues. |
| `read_cells {document, sheet, range}` | read | XLSX values and formulas for a range such as `A1:D20` (at most 10000 cells). `sheet` is an index or a name. |
| `inspect {document}` | read | Parts, relationships, content types, integrity issues. Comment, custom XML, macro, thumbnail and `docProps` parts are not listed unless `--include-hidden`, which also adds hidden-content findings. |
| `apply_commands {document, commands, dry_run?}` | edit / destructive | Applies the same JSON commands as `open365 apply`. `dry_run` runs them on a scratch copy and reports each command's effect class and whether it would be permitted; it needs no flag. A `save` or `export_pdf` command is refused; use `save_as` or the `export_pdf` tool. Sheet sizes: `{"command":"set_column_width","sheet":0,"first":2,"last":3,"width":24.5}` (stored characters, 0..=255) and `{"command":"set_row_height","sheet":0,"first":2,"last":2,"points":40}` (0..=409); both are `edit` and undoable. |
| `save_as {document, path}` | edit / export | Writes the document through the `save` command. Same path as opened: `edit`. New path: `export`. |
| `close {document, discard?}` | read | Refused while there are unsaved changes unless `discard` is true. |
| `export_pdf {document, path, overwrite?}` | export | DOCX only. Lays the open document out (bundled fonts, no author or date) and writes a tagged PDF 1.7 to `path` (must end in `.pdf`, inside a root). Needs `--allow-edit`. A new file is created exclusively; an existing file is never replaced unless `overwrite: true` is passed and the server has `--allow-destructive`. PDF at most 64 MiB. Runs the `export_pdf` command (effect `export`) on a private copy of the document. Returns bytes, page count and up to 20 warnings. Times out like the other slow tools; a timed-out export does not write its file. |
| `new_from_template {path, output, overwrite?}` | export | Makes a new file from a `.dotx`, `.dotm`, `.xltx` or `.xltm` inside a root: `.docx`, `.docm`, `.xlsx` or `.xlsm` (the output extension must match). Uses the same instantiate step as the CLI and writes through the `save` command. A VBA project is kept byte for byte and never run. Needs `--allow-edit`; same overwrite rule as `export_pdf`. A file that is not a template is `invalid`. The result is not opened; call `open_document` on it. |
| `layout_summary {document}` | read | DOCX: page count, line count per page (at most 1000 pages), headings with page, paragraph and level (at most 200, 200 characters each), tables with first and last page, rows and columns (at most 100), layout notes (at most 20) and the count of fonts that fell back to the default. Heading text honours `--include-hidden` like `get_outline`. |
| `evaluate_formula {document, sheet, cell, formula?}` | read | XLSX: evaluates `formula` as if entered in `cell` (relative references resolve from there) against the workbook as it is now, including unsaved edits. Nothing is changed or saved. Without `formula` the cell's own formula is recalculated. Formula at most 8192 characters; text results at most 32 KiB; array results at most 100 x 100. Hidden sheets, rows and columns are invisible to the formula unless `--include-hidden`; the result says `hidden_content_excluded`. |

Permission tiers: `read` tools need nothing. `export` tools create a file and need `--allow-edit`; replacing a file also needs `--allow-destructive` and `overwrite: true` in the call. Read-only is the default for a server started without flags.

Responses are JSON text. Reads carry a `notice` that document content is untrusted data. Tool failures return `isError: true` with a `status` of `refused` (policy or fence), `invalid` (bad arguments), `timeout` (see Run time) or `error`.

## Run time

`layout_summary`, `export_pdf` and `evaluate_formula` are bounded by `--timeout-secs` (default 30 s; `Config::deadline` in the library). The work runs on a worker thread on a private copy of the document and the call returns `status: "timeout"` when the deadline passes. The timeout also sets the request's cancellation flag, which layout pagination, the PDF writer and recalculation check (see `cli.md`, Cancellation), so the worker stops at its next check instead of running on. A step that does not check, such as opening a very large file, can still finish in the background; its result is dropped. A cancelled `export_pdf` removes a file it created and never touches an existing one. Edited documents are copied by saving them in memory, so a document whose edits dropped content that cannot be saved cannot be copied and these tools report an error for it.

`evaluate_formula` keeps one calculation engine per open workbook on its own thread. The first call builds it from a copy of the workbook (that time counts against the deadline); later calls reuse it, and each call puts the scratch formula in and takes it out again. It is rebuilt when the document is edited or undone, or when the opened file's modified time or size changes; a timeout does not: the cancelled job puts the scratch formula back and the worker keeps the engine. The audit line carries `engine: "built"` or `"reused"`.

The commands are the serialised `Command` enum in `crates/commands`; the CLI and MCP take the same JSON, and the same commands give byte-identical files and the same per-command effect and changed-cell results (tested in `apps/cli/tests/mcp.rs`).

## Security model

Threat context is T4 in `docs/security/threat-model.md`: a prompt-injected agent. This contains agents, not malware running as the same user.

1. **Fenced roots.** Every path is canonicalised (symlinks followed) and must lie inside a `--root`. Refused: `..` escapes, symlinks or linked folders that leave a root, UNC and device paths (`\\server\share`, `\\?\`, `\\.\`, `//host`), alternate data streams (`name:stream`) and reserved device names (`NUL`, `CON`, ...). `save_as` resolves the parent folder the same way and refuses an existing symlink as the target.
2. **Read-only by default.** Without `--allow-edit`, `apply_commands`, `save_as`, `export_pdf` and `new_from_template` are refused, and the refusal is audited. `dry_run` is the suggest path.
3. **Effect classes.** `Command::effect_in` decides, by actual effect on the document. Edit and export need `--allow-edit`. Destructive and egress also need the operator's confirmation (`--allow-destructive` or `--confirm terminal`): deleting or replacing a range, replace all, deleting rows or columns, writing a value or formula over a filled cell, pasting over non-empty cells, a merge that drops values, clearing formats that exist. All commands in a call are classified first, each against the document as the earlier ones leave it; one refusal means none run. Overwriting any existing file, the opened one included, needs the same confirmation.
4. **Hidden content is not returned.** `w:vanish` runs, hidden sheets (including very hidden; they answer "no such sheet"), hidden rows and columns, comments and author metadata never appear in reads or in `inspect` unless the operator passes `--include-hidden`. The agent cannot change this.
5. **Audit.** Every `tools/call` appends one JSON line: `seq`, `ts_ms`, `origin: "mcp"`, `pid`, `tool`, `status`, `reason` on refusal (a fixed code, never error text), the document id, resolved path, command names and effect classes. A call that can change something first writes a `tool_intent` line, and is refused if that line cannot be written; the result line carries `intent_seq`. The first request of a session (either protocol era) writes `session_start`; rejected requests (parse errors, unsupported version, missing tool name, oversize lines) write `protocol_error` lines. Agent-supplied values are capped at 256 characters; longer ones are cut and get a length and a fingerprint. The file tools also log `effect: "export"`, `overwrite` and `bytes_written`; `evaluate_formula` also logs `engine`; `new_from_template` logs the template (`path`) and the new file (`output`); `evaluate_formula` logs the sheet cell and `formula_chars` (the length, not the formula). Document text and cell values are never written to it. If a line cannot be written, the server refuses all later calls.
6. **Limits.** Request line 4 MiB; response 1 MiB; document file 32 MiB; PDF 64 MiB; formula 8192 characters; 8 open documents; 1000 commands per call; 10000 cells and 256 KiB of text per read. Package parsing uses the `open365-opc` limits (zip bombs, part counts).
7. **Untrusted text.** The server tells the client that document text is data. Honouring that is up to the client; the flags above are what actually bound the damage.

## Known gaps

- **Cancellation is cooperative.** See Run time: the checks sit in the long loops, once per block, page item, page, or recalculated cell, so a single very large cell range, table or file open can overrun the deadline slightly.
- **PDF fonts differ between front ends.** The command lays out with the bundled fonts (and installed fonts when the caller sets `system_fonts`); the desktop passes `system_fonts: true`, so its PDF can differ from the CLI's on a machine with other fonts.
- **Layout is measured against Word with bundled fonts only.** `layout_summary` page and line counts differ from Word where a document uses fonts the bundle does not stand in for (`fallback_fonts` says so). Hidden text may still take part in layout; only returned text honours `--include-hidden`.
- **Opened templates and macro files stay as they are.** `open_document` edits `.dotx`, `.dotm`, `.xltx` and `.xltm` as the template they are (it does not instantiate one; use `new_from_template` for that) and saves them under the same extension.

- **No native confirmation UI.** The threat model wants destructive and egress effects confirmed in Open365's own UI, never through MCP. That UI does not exist yet, so the only gate is the `--allow-destructive` start-up flag. Operators decide once per server start, not per action.
- **Hidden-content coverage.** Only direct `w:vanish` on a run is detected. Hidden character styles and style-level hidden text are not, and offsets in `read_text` are marked `offsets_match_text: false` for paragraphs where hidden runs were dropped. Text inside content controls and alternate-content blocks is not returned at all. Tracked-change deletions are not returned (they are not text in the model).
- **Race between check and use.** Paths are checked, then the file is read or written; a process that can write inside the root could swap a file or folder in between. Saves use a random temporary file (`create_new`) and re-check the destination folder just before the rename, which narrows but cannot close the window (that needs directory handles, which need `unsafe`). Out of scope for the injected-agent threat.
- **No `no-egress` label handling**, per-session scopes, or CoWorker parity yet; no resources or prompts (tools only).
- **Partial apply.** A command that fails mid-call leaves earlier commands of the same call applied in the open document (the error says how many). Nothing reaches disk until `save_as`.
- The server cannot undo; there is no undo tool yet.
