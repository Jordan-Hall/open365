# Open365 command line

`open365` is the CLI over the same command layer the MCP server and the UI use (ADR-002). This page lists the commands an agent or script is likely to call. `open365 --help` and `open365 <command> --help` are the full reference. For the MCP equivalents and the security model see `mcp.md`.

Exit codes: 0 success, 1 error (message on stderr, nothing on stdout), 3 `apply` refused by the effect policy.

## Inspect and edit

| Command | Effect | Notes |
|---|---|---|
| `open365 inspect <file>` | read | Parts, content types and integrity issues of any OPC package. |
| `open365 docx inspect [--hidden] [--json] <file>` | read | Also `xlsx inspect`. `--hidden` lists what a user cannot see. |
| `open365 apply <commands.json> <input> -o <out> [--dry-run] [--allow-destructive]` | edit / export / destructive | Applies the JSON command log and saves. Prints each command's effect class. |
| `open365 roundtrip --check <files...>` | read | Read, write, read; reports any loss. |
| `open365 mcp --root <dir> --audit-log <file> [--allow-edit] [--allow-destructive] [--include-hidden]` | see `mcp.md` | Stdio MCP server. |

## PDF export

`open365 docx export-pdf <in.docx> -o <out.pdf> [--overwrite --title --author --subject --keywords --language --system-fonts]`

Lays the document out and writes a tagged PDF 1.7. Bundled fonts unless `--system-fonts`. No author or date unless given. Effect: export. The CLI runs the `export_pdf` command (`{"command": "export_pdf", "path": ..., "overwrite": false}`), the same one the desktop's File, Export PDF and the MCP `export_pdf` tool use. Without `--overwrite` the output is created exclusively and an existing file is an error (exit 1, file untouched). `export_pdf` is refused inside an `apply` command file, like `save`; the MCP tool adds the fence and the `--allow-destructive` rule (see `mcp.md`).

## New document from a template

`open365 docx new-from-template <template.dotx|.dotm> -o <out.docx|.docm> [--overwrite]`
`open365 xlsx new-from-template <template.xltx|.xltm> -o <out.xlsx|.xlsm> [--overwrite]`

The template content type becomes the plain document type (macro-enabled when a VBA project is present). The VBA project is kept byte for byte and never run. The output extension must match the template (`dotx`->`docx`, `dotm`->`docm`, `xltx`->`xlsx`, `xltm`->`xlsm`). A file that is not a template is an error. An existing output is never replaced without `--overwrite`. The save goes through the `save` command, so its effect is export.

## Layout query

`open365 docx layout <file.docx> [--json] [--max-pages N]` (read-only)

Without `--json`: page count and one line per page (lines, cells, page size in points). With `--json`:

```
{"page_count": 2, "truncated": false, "notes": [],
 "pages": [{"number": 1, "width": 612.0, "height": 792.0,
   "lines": [{"paragraph": 0, "start": 0, "end": 12, "top": 90.0, "baseline": 109.2,
              "height": 26.3, "left": 72.0, "role": "heading", "level": 1}],
   "cells": [{"table": 0, "row": 0, "col": 0, "row_span": 1, "col_span": 1, "header_row": false,
              "x": 72.0, "y": 100.0, "width": 144.0, "height": 20.0, "fill": null}]}]}
```

Units are points with the origin at the top-left of the page. `role` is `plain`, `heading` (with `level`) or `list_item`. `start` and `end` are story offsets in UTF-16 units. Line text is not included; read it with the paragraph index. Output is capped at `--max-pages` pages (default 200); `truncated` says when pages were left out.

## Formula evaluation

`open365 xlsx eval <file.xlsx> --sheet <name|index> --cell <A1> [--formula <f>]` (read-only)

Evaluates the formula as if entered in the cell, against the workbook as stored, in a scratch engine. The file is never written. Without `--formula` the cell's own formula is recalculated. Prints one JSON object:

```
{"sheet": 0, "cell": "Z1", "formula": "SUM(A1:A3)", "saved": false, "truncated": false,
 "result": {"type": "number", "value": 6.0}}
```

`result.type` is `number`, `text`, `bool`, `error` (value like `#DIV/0!`), `empty` or `array` (rows of results, at most 100 x 100). An error value is a result with exit code 0; a formula that does not parse is an error with exit code 1. Formulas are limited to 8192 characters. Unlike the MCP tool, the CLI sees hidden sheets, rows and columns: the user running it owns the file.
