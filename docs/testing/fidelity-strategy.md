# Word and Excel fidelity test strategy

Status: proposed, 2026-10-10. Detailed sources: `../research/agent-reports/word-docx.md`, `../research/agent-reports/excel-xlsx.md`.

## Principle
We claim nothing without fixture evidence. The primary metric is **"no silent loss"**, not "looks like Word". A save that would lose content must either preserve it or tell the user.

## Fixture corpus (`fixtures/`, licence-audited per file)
| Tier | Source | Purpose |
|---|---|---|
| A | User-made in Word/Excel/LibreOffice, one feature per file, with a manifest that names the feature and the expected invariant | Regression and feature coverage |
| B | Generated (rust_xlsxwriter, scripted docs), including stress files: 1M rows, 500-page docs, 100k shared strings | Performance and limits |
| C | Public corpora (Apache POI, LibreOffice test files). **Kept outside the repo** until the licence of each file is audited | Breadth / smoke |
| D | Adversarial: zip bombs, DTD/XXE, deep nesting, malformed rels, fuzz-minimised crashers | Security regression |

Required feature fixtures in the first 20: styles, numbering, tables with merges, images, header/footer, comments, tracked changes, footnotes, fields, unknown customXml (DOCX); multi-sheet, shared formulas, defined names, cross-sheet refs, dates in 1900 and 1904 systems, number formats, merged cells, a chart, a pivot, a VBA project (XLSX).

## Invariants (automated, run per fixture)
1. **Part superset:** every input part and relationship exists in the output. Untouched parts are byte-identical.
2. **Canonical XML equality** for parts that were re-serialised but not semantically edited (C14N, then attribute order normalised).
3. **Semantic model diff:** read(write(read(x))) == read(x) (idempotence). Property tests do the same for generated models.
4. **Relationship integrity:** no dangling rIds. Content types cover all parts.
5. **Dropped-content log empty** unless the fixture manifest expects an entry.
6. **External validation:** LibreOffice headless opens and converts the output to PDF. Extracted text equals the original's. Open XML SDK validator (MIT; check the tool's licence) reports no *new* errors.
7. **Formula correctness:** recomputed values equal the Excel-cached `<v>` values (oracle 1) within 1e-9 relative tolerance or exact for text/bool/error. Disagreements with the LibreOffice recalculation (oracle 2) are triaged, not assumed to be our bug.
8. **Visual (layout, P1+):** render to PDF, compare against the LibreOffice PDF by line breaks per page (tolerance N lines/page), not pixels.

## Silent-corruption detection
- The dropped-content log is wired into save. CI fails if a fixture's log is non-empty without a manifest entry.
- Hash every part on load, and on save diff the hashes against the dirty-part set. Any changed part not marked dirty fails the test.
- Periodic "open in real Office" checks on a Windows runner (manual at first). File-repair prompts are recorded as failures.

## Large files and performance (targets to measure, not claims)
Reference hardware is stated in every result. Initial targets: open a 50 MB XLSX (1M cells) in under 3 s, single-cell recalc in a 100k-formula workbook under 50 ms, open a 500-page DOCX to first page under 1 s, typing latency p95 under 50 ms. Results are tracked in CI benchmarks over time.

## Where parity is expensive (defer or preserve-only)
DOCX: floating-object wrap polygons, table autofit, field reflow, font-metric differences, equations rendering. XLSX: dynamic-array spill, LAMBDA/LET, pivot refresh, chart rendering, structured refs, conditional-format icon sets, locale number formats, VBA. Policy: preserve losslessly first, then render, then edit.

## Error handling
Corrupt input opens read-only with a repair report and never crashes. Fuzz targets (zip, XML, shared strings, formula parser) run in CI for 10 minutes per PR and 1 hour nightly.
