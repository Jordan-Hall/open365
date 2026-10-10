# Spreadsheet engine strategy and XLSX fidelity (2026-10-10)

Legend: [F] fact with source; [I] inference; [U] unverified.

## 1. IronCalc check
- [F] MIT OR Apache-2.0; GitHub org `ironcalc`; 2,091 commits; README calls itself early-stage and seeks collaborators; bindings folder (Python, WASM/JS, Node); XLSX reader+writer in repo (S1).
- [F] Pre-1.0 (v0.8.3 per package index, S3). NLnet-funded work from 2026-03 on conditional formatting and UI features (S2).
- [U] Function count: third-party pages say 300+ and 500+ (inconsistent). Official count not retrieved (docs URL 404). Must count from the source function enum.
- [U] Dynamic arrays/spill, LET/LAMBDA (one third-party listing only). Structured table references: no source mentions them, treat as absent. Defined names: listed by third party; [U].
- [U] Dependency graph and incremental recalc design, volatile handling, locale/number-format depth, contributor count, company governance. Read the source before deciding.
- [F] Charts and real-time collaboration reported as unsupported (S2-linked listing).
- Recommendation [I]: build the Open365 workbook model and XLSX package layer ourselves (preserve-everything, see 2). Put the calc engine behind a `CalcEngine` trait (set cell, get value, recalc dirty set, function registry). Start with IronCalc as the first adapter and contribute upstream fixes. Fork only if governance stalls. Reason: IronCalc's own model will not preserve unknown parts, pivots, charts or VBA, which Open365 needs.

## 2. Workbook model must-haves [I]
- Package layer: keep every OPC part and relationship byte-for-byte unless edited (unknown parts, extLst, mc:AlternateContent, customXml). Model only what we edit.
- Cell store: sparse chunked map (row-block, column-run) with a typed cell enum, shared-string table (interned), style IDs into deduplicated xf tables. 1M x 16k is a coordinate space, never a dense array. Targets are about memory per populated cell, not grid size.
- Must model: number formats (custom codes kept as text, rendering separate), merged cells, conditional formats (store rules, dxf), data validation, defined names (workbook and sheet scope), tables (ref, columns, autofilter), comments plus threaded comments and persons parts.
- Preserve-only: pivot caches/tables, external links (never auto-fetch; SSRF and data-exfiltration risk), VBA `vbaProject.bin` (never execute, never re-sign), embedded objects, ActiveX.
- Charts: preserve the DrawingML parts first, render later from the chart XML.
- Treat all parts as untrusted: zip-bomb limits, XML entity limits, part-size caps, no external fetches.

## 3. Formula correctness
- Oracle 1 [I]: `<v>` cached values in XLSX files written by Excel. These are the ground truth for fixtures. Compare our recalculation to the cache with a tolerance (relative 1e-12 for floats, exact for strings, errors and booleans).
- Oracle 2 [I]: LibreOffice headless convert-and-recalc. It disagrees with Excel in places, so use it only for tie-breaks and flag disagreements. Do not copy its code (MPL/LGPL obligations); running it as a test tool is fine, check the license [U].
- Semantics sources: ECMA-376 Part 1 section 18.17, OpenFormula for function behaviour, MS-OI29500 for Excel deviations (S4-S6).
- Edge cases to test: 1900 leap-year bug (serial 60 = fictitious 1900-02-29), 1904 date system (`workbookPr date1904`), 15-digit display precision, IEEE-754 results that Excel snaps near zero, `#DIV/0!`, `#N/A`, `#REF!`, `#NAME?`, `#SPILL!` error propagation, text-to-number coercion rules, locale-neutral storage (formulas stored in en-US syntax).
- Circular refs: default is an error/zero warning; iterative calc (`calcPr iterate`, `iterateCount`, `iterateDelta`) must be honoured and deterministic.
- Volatile functions (NOW, RAND, OFFSET, INDIRECT) recalculate on load only when Excel would (`fullCalcOnLoad`). Otherwise keep cached values.

## 4. Fidelity test strategy
- Corpus [I]: (a) user-made fixtures (own copyright, CC0), (b) generated with rust_xlsxwriter (Apache-2.0) and our own writer, (c) upstream test files from permissively licensed projects (check each license; e.g. calamine, IronCalc, SheetJS tests [U]), (d) public corpora such as Enron/Fuse/Euses spreadsheets: licence and privacy status [U], keep out of the repo and run as an external, uncommitted corpus.
- Round-trip invariants: open->save->open gives identical cell values, formula text, style resolution (resolved font/fill/border/numFmt per cell), merges, names, tables; all unparsed parts are byte-identical; part list differs only by documented changes. Treat calcChain as derived: drop it on edit and set a recalc flag, never write a stale one.
- Silent-corruption detection: XML canonical diff of untouched parts; schema validation against ECMA XSDs; reopen the saved file in LibreOffice headless and check for load errors; manual Excel open checks on a small fixture set (Excel "repair" dialog is the failure signal). Excel automation is not available in CI [U].
- Perf targets (proposed, to be measured on stated hardware, e.g. 8-core x86-64 laptop, 16 GB, NVMe): load 100 MB / 1M-cell XLSX < 5 s and < 1.5 GB RSS; edit-one-cell incremental recalc of 100k dependents < 100 ms; scroll render 60 fps on the visible window. These are targets, not claims.
- Fuzzing: cargo-fuzz on zip, XML and formula parser; structure-aware mutation of valid XLSX; property tests (proptest) for the round-trip invariants; sanitizers with part/size limits.

## 5. Where Excel parity is expensive [I]
Dynamic arrays and spill semantics with implicit intersection; LAMBDA/LET; full date/time and locale number-format rendering; pivot table refresh; chart rendering fidelity; conditional-format icon sets and data bars; structured references; Power Query/external data; threaded comment metadata; VBA; exact text-width and font metrics (autofit, print layout); array-formula (CSE) legacy behaviour; precision edge cases in financial/statistical functions.

## 6. Smallest XLSX vertical slice [I]
Open an XLSX with one or more sheets of numbers, strings, booleans, dates and simple formulas (SUM, IF, cross-sheet refs, one defined name); edit a cell; recalc; save; reopen.
Acceptance:
1. Fixtures (>= 10 user-made) load with zero errors, and values match Excel cached values.
2. After a single-cell edit, only dependents recalculate and results equal the oracle within tolerance.
3. Saved file has all untouched parts byte-identical, no calcChain, fullCalcOnLoad set; styles/number formats resolve identically.
4. Pivot, chart, VBA and unknown parts in an extra fixture survive the round trip.
5. LibreOffice headless opens the output with no errors; fuzz run of 1 hour finds no panics.
6. CLI `open365 xlsx roundtrip --check` exposes it for agents.

## Sources (accessed 2026-10-10)
- S1 https://github.com/ironcalc/IronCalc
- S2 https://nlnet.nl/project/IronCalc-conditional/ (via search results)
- S3 https://openapps.pro/packages/ironcalc-base (via search results)
- S4 ECMA-376 https://ecma-international.org/publications-and-standards/standards/ecma-376/ [not re-fetched]
- S5 MS-XLSX https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-xlsx/ [not re-fetched]
- S6 MS-OI29500 https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/ [not re-fetched]
- Search summary: https://cdn.jsdelivr.net/gh/ironcalc/ironcalc@main/README.md, https://dev.co/devops/open-source/ironcalc
