# Microsoft template corpus: Office-oracle references

The corpus is 136 public Microsoft templates (74 docx, 62 xlsx) listed in `fixtures/external/ms-templates.toml`.
The files (`fixtures/external/ms-templates/`) and every output derived from them
(`fixtures/reference/ms-templates/<id>/`) are local and gitignored: Microsoft grants no redistribution
(see `docs/research/corpus-licensing.md`).

## Generate

```powershell
pwsh -NoProfile -File tools/corpus/fetch-ms-templates.ps1 -Reference   # download+verify, then run the oracles for missing outputs
pwsh -NoProfile -File tools/corpus/summarize-ms-references.ps1         # write fixtures/reference/ms-templates-summary.json
```

Per entry the Office 16 COM oracles write:

| Kind | Files | Content |
|---|---|---|
| docx | `pages.json`, `layout.json`, `<id>.pdf` | page count; per-line page, offset, top, left, baseline; table cells; images |
| xlsx | `values.json` | per sheet used range, and Value2, Text and Formula for up to 2,000 non-empty cells |

## Safety

- Each oracle starts its own Word or Excel instance, sets `AutomationSecurity = 3` (force disable: macros never run)
  and opens the file read-only (`Documents.Open` / `Workbooks.Open`, never saved). It refuses to run if COM
  attached to an instance it did not start. No registry or Trust Center setting is changed.
- Quit runs in a `finally` block; only Office processes the script started are killed.
- Each file runs in a child process with a 90 s timeout; a timeout is reported and the run continues.

## Committed aggregate

`fixtures/reference/ms-templates-summary.json` holds, per template id: kind, category, template sha256, counts
(docx: pages, layout lines, table cells; xlsx: sheets, cells, formula cells, truncated sheets) and the sha256 of each
output file. It holds no titles, text or URLs. Hashes depend on the installed Office build and fonts, so compare
them only on the same machine configuration.
