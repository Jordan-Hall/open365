# office-oracle

Local developer tool that drives the installed Microsoft Word/Excel 16.0 (COM) as a reference oracle.
Not for CI: Microsoft does not support unattended server-side Office automation. CI consumes the committed
outputs in `fixtures/reference/`.

```powershell
# verify: open files read-only with no repair; prints PASS/FAIL per file, exit 1 on any FAIL
pwsh -NoProfile -File tools/office/office-oracle.ps1 -Mode verify -Path fixtures/docx/table-merged-cells.docx

# reference: write fixtures/reference/<docx|xlsx>/<name>/ for every fixture under the path
pwsh -NoProfile -File tools/office/office-oracle.ps1 -Mode reference -Path fixtures
```

What they prove:
- `verify`: Office itself opens the file as-is, with no repair or conversion. Run it on Open365 output.
- `reference`: what Office renders. DOCX gets `<name>.pdf` and `pages.json` (page count, first line per page).
  XLSX gets `values.json` (after CalculateFull: used range plus Value2/Text/Formula for up to 2,000 non-empty
  cells per sheet) and, only with `-XlsxPdf`, a PDF when under 50 pages (off by default: Excel PDF export hung on the dev machine).

Fixtures are opened read-only and never saved. JSON holds repo-relative paths only. Failing fixtures are
reported as `SKIPPED` and the run continues. Only Office processes started by the script are killed.

## Word layout parity

```powershell
pwsh -NoProfile -File tools/office/make-layout-fixtures.ps1          # (re)creates fixtures/docx/layout-*.docx
pwsh -NoProfile -File tools/office/word-layout-oracle.ps1 -Path fixtures/docx/layout-wrap.docx
```
`word-layout-oracle.ps1` writes `fixtures/reference/layout/<name>.json`: per main-story line the page, first
character offset, top, left, lastX (screen-quantised +-0.3 pt) and exact baseline/inkLeft read from Word's PDF.
Method and speed are documented in the script header. References exist for every docx fixture. The
fixtures use Calibri (theme minor font), Calibri Light, Cambria and Arial, not Aptos. Use `make-layout-fixtures.ps1 -Only <name>`
to create a single probe fixture. The parity test scores `x` from the PDF `inkLeft` (the screen `left`/`lastX`
are unreliable for aligned text). `OPEN365_PARITY_SYSTEM_FONTS=1` runs it with the machine's real fonts
(`[system.*]` floors); the default bundled mode skips fixtures whose fonts Carlito/Caladea cannot stand in for
and only approximates Cambria (Caladea is narrower than Cambria).
