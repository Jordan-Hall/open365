# `open365` command-line reference

`open365` runs the same command layer as the desktop apps and the MCP server (ADR-002). `open365 --help` and `open365 <command> --help` list every option. How the CLI and MCP fit together, the permission model and the audit log are in `docs/architecture/agent-control.md`. The short agent-oriented page is `docs/agents/cli.md`.

## Commands

| Command | Does | Effect |
|---|---|---|
| `commands [--domain docx\|xlsx]` | List every command of the command layer with domain, effect and a JSON example. | read |
| `run <name> <input> [--args JSON \| --args-file F] [-o out] [--dry-run] [--allow-destructive] [--root DIR]...` | Run one command and save. `save` and `export_pdf` write to the `path` in their arguments. | by command |
| `apply <commands.json> <input> [-o out] [--dry-run] [--allow-destructive]` | Run a JSON array of commands and save. | by command |
| `inspect <file>` | Parts, content types, integrity issues of any OPC package. | read |
| `docx inspect`, `xlsx inspect [--hidden]` | Same, checked against the format; `--hidden` adds what a user cannot see. | read |
| `docx outline`, `docx text [--start N --count N]`, `docx styles` | Headings, paragraph text, style sheet. | read |
| `docx layout [--max-pages N]` | Pages, lines and table cell boxes. | read |
| `xlsx outline` | Visible sheets and used ranges. | read |
| `xlsx cells --sheet S --range A1:D20` | Values and formulas. | read |
| `xlsx formulas --sheet S [--range R]` | Formula cells with stored values. | read |
| `xlsx styles [--sheet S --range R]` | Cell styles, number formats; with a range, the formatting of those cells. | read |
| `xlsx recalc --sheet S [--range R]` | Recalculate from scratch, report cells whose stored value differs. Saves nothing. | read |
| `xlsx eval --sheet S --cell C [--formula F]` | Evaluate a formula against the workbook. | read |
| `docx export-pdf <in> -o <out.pdf> [--overwrite]` | Tagged PDF 1.7 export. | export |
| `docx\|xlsx new-from-template <tpl> -o <out> [--overwrite --allow-destructive --root DIR]` | New document from a template; a VBA project is kept and never run. Replacing a file needs both flags. | export |
| `roundtrip --check <files>` | Verify read, write, read is lossless. | read |
| `mcp --root DIR --audit-log F [...]` | Serve the MCP tools over stdio. | see agent-control.md |

Read commands leave hidden text, sheets, rows and columns out unless `--hidden` is given.

### Policy

`run`, `apply`, `new-from-template` and `mcp` use `open365_mcp::policy`. A command is classified by its actual effect on the document (`assess_in`; in `apply`, each command against the state the earlier ones leave): a command that is destructive or egress is refused with exit code 3 unless `--allow-destructive` is given. Destructive includes deleting or replacing a range, replace all, deleting rows or columns, writing a value or formula over a filled cell, pasting over non-empty cells, a merge that drops values, clearing formats that exist, and writing over an existing file (`save`, `apply -o`, `run -o`, `export_pdf --overwrite`, `new-from-template --overwrite`, the opened file included). Nothing runs and one audit line with `status: "refused"` goes to stderr. `--root DIR` (repeatable, on `run`, `apply`, `new-from-template` and `docx export-pdf`) fences the output path to those folders, symlinks followed, and the file is written at the resolved path; `--allow-destructive` never overrides the fence. All writes pass one gate (`apply_cmd::gate_write`). `save` and `export_pdf` are not allowed inside an `apply` command file.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | error (bad input, I/O, a failed check) |
| 2 | usage error |
| 3 | refused by policy |

## `--json` output, schema `open365.cli/1`

Every command except `mcp` accepts `--json` and then prints exactly one JSON object on stdout:

```json
{"schema":"open365.cli/1","command":"xlsx cells","ok":true,"result":{...}}
{"schema":"open365.cli/1","command":"xlsx cells","ok":false,"error":{"code":"error","message":"no such sheet"}}
```

`error.code` is `io`, `format`, `refused`, `error` or `check_failed` (the last also carries `result`, as `roundtrip` does). Human text goes to stdout only without `--json`; errors go to stderr in that mode. Per-command audit lines (`apply`, `run`) stay on stderr in both modes.

### Versioning

- The schema string `open365.cli/N` changes only for a breaking change to an existing field: removal, rename, type change, or a changed meaning. A new version ships beside the old one for at least one release, selected by a future `--schema` flag.
- Adding fields or commands is not breaking; consumers must ignore unknown fields.
- The machine-readable definition is `fixtures/mcp/schemas/cli-v1.json`: the envelope and one result schema per command, in a JSON Schema subset (`type`, `required`, `properties`, `items`, `enum`, `const`). `apps/cli/tests/surface.rs` runs each command and validates its output against it.
- The command catalogue has its own identifier, `open365.commands/1`.

### Result shapes (v1)

| Command | Result fields |
|---|---|
| `commands` | `schema`, `count`, `commands[]` of `name`, `domain`, `summary`, `example_effect`, `example`, `cli`, `mcp` |
| `run`, `apply` | `input`, `dry_run`, `output`, `bytes_written`, `commands[]` of `seq`, `command`, `effect` (`read` `edit` `destructive` `egress` `export`), `status` (`applied` `dry_run_ok`), `changed_cells`, `notes` |
| `inspect`, `docx inspect`, `xlsx inspect` | `parts[]`, `relationships[]`, `content_types{defaults,overrides}`, `integrity_issues[]`, `hidden[]` with `--hidden` |
| `docx outline` | `kind`, `paragraph_count`, `headings[]` of `paragraph`, `level`, `text` |
| `docx text` | `paragraph_count`, `paragraphs[]` of `paragraph`, `text`, `truncated`, `next_paragraph` |
| `docx styles` | `kind`, `count`, `styles[]` of `id`, `name`, `type`, `based_on`, `next`, `default`; `truncated` |
| `docx layout` | `page_count`, `pages[]`, `truncated`, `notes[]` |
| `xlsx outline` | `kind`, `sheets[]` of `sheet`, `name`, `state`, `worksheet`, `used_range` |
| `xlsx cells` | `range`, `sheet`, `cells[]` of `ref`, `value`, `formula`; `truncated` |
| `xlsx formulas` | `range`, `sheet`, `count`, `formulas[]` of `ref`, `formula`, `value`; `truncated` |
| `xlsx styles` | `kind`, `cell_styles[]`, `cell_format_count`, `default_font`, `number_formats[]`; with a range `cells[]` of `ref`, `style_name`, `number_format`, `font`, `fill`, `alignment` |
| `xlsx recalc` | `sheet`, `range`, `formulas_checked`, `stale`, `differences[]` of `ref`, `formula`, `stored`, `recalculated`; `truncated`, `saved: false` |
| `xlsx eval` | `formula`, `result{type,value}`, `truncated`, `sheet`, `cell`, `saved: false` |
| `docx export-pdf` | `output`, `pages`, `bytes_written`, `warnings[]` |
| `new-from-template` | `output`, `bytes_written` |
| `roundtrip` | `files[]` of `file`, `passed`, `parts` or `detail`; `passed`, `failed` |

Changes in this version: `docx layout --json` and `inspect --json` now return the envelope (before, the bare object); `xlsx eval` without `--json` still prints its compact JSON line as before.

## `mcp` options

| Option | Meaning |
|---|---|
| `--root DIR` (repeatable, required) | Folders the agent may touch. |
| `--audit-log FILE` (required) | JSONL audit log, appended. |
| `--allow-edit` | Grants scopes `edit`, `save`, `export`. |
| `--grant SCOPE` (repeatable) | Grants one of `edit`, `save`, `export`, `execute-external`. |
| `--allow-destructive` | Approves destructive actions and file replacement in advance. Needs `--allow-edit` or `--grant edit`. |
| `--confirm deny\|terminal` | Who confirms what is not pre-approved: nobody (default) or the console of the machine. |
| `--include-hidden` | Include hidden content in reads. |
| `--timeout-secs N` | Deadline for slow tools, 1 to 3600 (default 30). |
