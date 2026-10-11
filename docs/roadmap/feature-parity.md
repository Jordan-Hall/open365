# Feature parity: Word and Excel desktop vs Open365

Status: draft, 2026-10-11. Research and planning only; no product code changed. Owner: lead session.

## How to read this

- **Checklist sources** (Microsoft public documentation; the page structure was used as the feature list, pages were not re-fetched in this session except where noted):
  - Word help hub, <https://support.microsoft.com/en-us/word> (ribbon tabs: File, Home, Insert, Draw, Design, Layout, References, Mailings, Review, View, Help).
  - Excel help hub, <https://support.microsoft.com/en-us/excel> (ribbon tabs: File, Home, Insert, Draw, Page Layout, Formulas, Data, Review, View, Help).
  - Word object model, <https://learn.microsoft.com/en-us/office/vba/api/overview/word/object-model> and Excel object model, <https://learn.microsoft.com/en-us/office/vba/api/overview/excel/object-model> (index of objects used as a feature checklist: `Styles`, `Fields`, `Tables`, `Revisions`, `Comments`, `MailMerge`, `ChartObjects`, `PivotTables`, `Validation`, `FormatConditions`, `AutoFilter`, `Scenarios`, ...).
  - Excel function list, <https://support.microsoft.com/en-us/office/excel-functions-alphabetical-b3944572-255d-4efb-bb96-c6d90033e188>. Checked 2026-10-11 in `docs/testing/calc-coverage.md`: 521 distinct functions.
  - File formats: ECMA-376 / ISO 29500 (OOXML), checked against the repo's own docs, not re-read here.
  - Dates on all Microsoft pages: accessed by the earlier research that produced the repo docs, 2026-10-10/11. Treat page contents as "current as of that date"; Microsoft changes these pages often. Unverified items are marked **(unverified)**.
- **Open365 status** was taken from the code (`crates/*`, 2026-10-11, about 100k lines of Rust), `docs/testing/*`, the parity floors under `fixtures/reference/`, and `git log` (71 commits, 2026-10-10 18:32 to 2026-10-11 08:26 +0100).
- **Status values**: `done` (evidence cited), `partial` (numbers), `missing`, `n/a` (out of scope, see scope notes), `preserve` (kept byte-identical on round trip, not editable or rendered).
- **Units**: one unit = about one 80-turn focused agent round. Sizes have a plausible error of about -30%/+60%.

## Evidence base: what Open365 has today

| Area | Evidence (2026-10-11) |
|---|---|
| OPC/zip round trip | 136/136 Microsoft templates (74 docx, 62 xlsx) byte-identical when unedited; 544 Office-verified outputs (commit fb9509e); 1 h fuzz gate passed (5a6cdca); `docs/testing/round-trip.md` |
| Formula engine | 318 of 521 functions registered (61 %); 53,965 conformance cases, 53,744 match Excel (99.6 %); `docs/testing/calc-coverage.md`, `calc-conformance.md` |
| Number formats | Excel-matching engine, 8,306 cases / 8,251 match at introduction (c6e1723); locale: all 418 Office LCIDs (ADR-011) |
| Workbook semantics | Microsoft templates: values 15,090/16,038 (94.1 %), text 14,854/15,925 (93.3 %); formula-only 8,575/9,523 (90 %); `ms-template-values.md` |
| Word layout (own fixtures) | Lines matching Word: bundled fonts 7,540/8,426 (89.5 %), system fonts 7,021/7,189 (97.7 %); `fixtures/reference/layout/parity-floor.toml` |
| Word layout (real templates) | Lines, cells, pictures matching Word on 74 templates: 3,238/8,245 (39.3 %), system fonts; `fixtures/reference/ms-templates-layout-floor.toml`. This is the honest fidelity number. |
| Commands | 8 docx/xlsx edit commands (insert text, bold, delete range, split paragraph, run props, paragraph style, alignment, replace range), set cell value/formula, column width, row height, hide/unhide rows and columns, save, export PDF; effect classes; undo (snapshot, incremental) |
| Agent surface | CLI (`inspect`, `apply`, `roundtrip`, `export-pdf`, `new-from-template`, `layout`, `xlsx eval`) and stdio MCP (12 tools) with effect tiers, fencing, audit |
| Desktop | egui app: paged Word view with caret/selection/IME/AccessKit, virtualised grid with formula bar, in-cell edit, resize/auto-fit (37/39 columns pixel-exact), merged cells, frozen panes, CF drawing, header context menu; File: Open/Save/Save As/Export PDF/Print; Edit: Undo/Redo; Format: Bold/Italic/Underline/style picker |
| Not present anywhere | charts (any), pivot tables, images insert, tables edit, find/replace, clipboard (rich), spell check, track-changes recording, comments UI, fields update, ribbon |

## Scope decisions assumed by this document

1. Macros: **preserve only** (VBA project kept byte-identical, never executed). Matches the existing non-goal list.
2. Power Query / Power Pivot / DAX / 3D Maps / Ideas (Analyze Data) / Copilot: **out of scope** for parity (Power Query: preserve the `customXml`/connection parts only). Open CoWorker is the AI line.
3. Legacy binary formats (.doc, .xls, .xlsb), RTF, ODF, PDF import: after MVP.
4. Co-authoring and the server (ADR roadmap P1/P2) are costed separately in the "Collaboration" row, not in the MVP.
5. Printing is OS print of our own layout and PDF export (done); printer-driver fidelity is out of scope.

## Word: status table

| Area | Public-doc feature set | Open365 status | Gap | Units |
|---|---|---|---|---|
| File formats | docx, dotx, docm, dotm, doc, rtf, odt, pdf, txt, html | docx/dotx/docm/dotm open+save, lossless, templates instantiate, macros preserved. PDF export done (own writer, tagged). | doc/rtf/odt/html/txt import-export, PDF import missing | 12 (post-MVP) |
| Open/save fidelity | round trip with no loss | **done** for untouched parts (136/136); every edit command preserves unknown XML (ADR-001) | per-feature round-trip fixtures as features land | in each row |
| Character formatting | font, size, bold/italic/underline, colour, highlight, effects, caps, kerning | partial: model + layout for caps/small caps, char spacing; edit only B/I/U and run props via command; no font/size/colour UI | Format UI, font dialog, Format Painter | 4 |
| Paragraph formatting | alignment, indent, spacing, borders, shading, tabs, keep rules | partial: layout done (spacing collapse, borders, shading, tabs, keep/widow); edit: alignment only | paragraph dialog, tab/indent UI, ruler | 3 |
| Styles | gallery, new/modify style, style inspector, themes | partial: styles.xml resolver, paragraph style set; no create/modify, no theme/gallery | style edit commands + UI | 4 |
| Lists and numbering | bullets, multilevel, restart, legal | partial: typed numbering model, labels laid out; no list edit commands | list commands and UI | 3 |
| Tables | insert, merge/split, borders, shading, autofit, styles, sort, formulas | partial: typed model, layout (grid widths, vMerge, borders, header rows, nested), 1208/1208 table pages in oracle fixtures; no editing | table edit commands + UI, table styles, sort | 5 |
| Sections, columns, page setup | margins, size, orientation, breaks, columns, line numbers, borders | partial: sections, landscape, page size, section breaks parsed and laid out; columns layout **(unverified)**, line numbers and page borders missing | columns, page setup UI | 4 |
| Headers/footers | different first/even, page numbers | **done** layout (first/even/inheritance, distances, PAGE/NUMPAGES/SECTIONPAGES); no edit | edit in view | 1.5 |
| Footnotes and endnotes | insert, separators, numbering | **done** layout (placement, splitting, separator); no insert/edit | insert/edit, numbering formats | 2 |
| Fields | TOC, PAGE, REF, DATE, SEQ, MERGEFIELD, IF, XE/INDEX, citations | partial: parsed, field-safe edits, PAGE/NUMPAGES/SECTIONPAGES computed; TOC/REF/SEQ/DATE update missing | field update engine, TOC build | 4 |
| Images | insert, resize, crop, wrap, effects, alt text | partial: inline and floating render (top/bottom, square, tight; anchor bases), hardened decode, alt text node; no insert/resize/crop | insert and edit, crop, effects | 3 |
| Shapes, SmartArt, WordArt, icons, 3D models | DrawingML shapes, groups, SmartArt, ink | partial: text boxes laid out (txbx-basic 22/22); other shapes, groups, SmartArt, WordArt missing (preserved) | DrawingML shape renderer, SmartArt (large) | 8 (post-MVP; basic shapes 4 in MVP) |
| Text boxes | insert, link, wrap | partial: laid out and drawn; no edit | edit | 1.5 |
| Equations | OMML insert, edit, ink | missing (preserved) | OMML layout (MVP render 4), editing 4 | 4 MVP, 4 post |
| Charts in Word | embed chart parts | missing (preserved) | via shared chart engine | see Excel |
| Comments | add, reply, resolve, show | partial: model, field-safe edits, fixture; no UI | comment pane and commands | 2.5 |
| Track changes | record, accept/reject, balloons, reviewing pane | partial: parsed and stripped for props; no recording, accept/reject or render | revision model, record, render, accept/reject | 5 |
| Compare / combine | document compare | missing | diff engine over revisions | 3 |
| Find / replace / go to | navigation pane, wildcards, formatting search | missing (command `replace_range` exists) | engine + UI | 2 |
| Spelling and grammar | proofing, autocorrect, thesaurus, word count | missing | dictionary integration (licence audit), squiggles, autocorrect | 3 spell, 4 grammar |
| Mail merge / Mailings | data source, envelopes, labels, preview, finish | missing | MERGEFIELD update + data sources | 3 |
| Citations and bibliography | sources, styles, bibliography | missing (fields preserved) | custom-XML sources + fields | 2 |
| Bookmarks, hyperlinks, cross-references | insert, edit, jump | partial: hyperlink fixture round-trips; no bookmarks or cross-ref UI | commands + UI | 2 |
| Indexes, tables of figures, TOA | mark entry, build | missing | after fields | 2 |
| Content controls and forms | rich text, dropdown, date, checkbox, legacy forms | partial: block/inline laid out like Word (placeholders, offsets), addressable for editing; no insert/dropdown UI | insert + properties | 1.5 |
| Protection | restrict editing, password, mark final, IRM, signatures | missing (preserved) | enforcement, password hash, signatures (needs crypto review) | 1.5 enforce, 3 signatures |
| Templates | new from template, themes, building blocks | partial: new-from-template (dotx/dotm) CLI, MCP, UI path; building blocks/Quick Parts missing | building blocks | 1.5 |
| Macros | record, run, VBA editor | **preserve** (VBA byte-identical); run = n/a | none | 0 |
| Views | print, read, web, outline, draft, focus, zoom, rulers, split, side by side | partial: paged print view; zoom **(unverified)**; others missing | views, ruler, status bar, navigation pane | 3 |
| Accessibility | checker, alt text, reading order, tagged PDF | partial: AccessKit tree, tagged PDF 1.7, image nodes; no checker | checker UI | 1.5 |
| Printing | print preview, page ranges, duplex | partial: Print and Export PDF in File menu | print dialog, preview, ranges (shared) | in shared |
| Complex text | RTL, bidi, East Asian, ligatures, OpenType | partial: shaping via `open365-text` (rustybuzz-style, **unverified**), IME works; bidi/vertical text not evidenced | bidi + vertical + font fallback | 5 |
| Layout fidelity to Word | line, page breaks identical | partial: 89.5 % own-fixture lines, **39.3 % real templates** (3,238/8,245) | long tail: floats, tables in floats, kerning/justify, fields, fonts | 12 to reach ~85 %, 15 more to ~99 % |
| Collaboration | co-authoring, share, version history, @mentions | missing (server not started; sharing Stage 1, then Teams T0–T4, then co-editing; see `first-milestones.md`) | sync service + CRDT | 12 (separate track) |
| Hidden content / privacy | inspect document, remove metadata | partial: `inspect --hidden`, hidden text excluded from MCP reads; sanitize command not yet built **(unverified)** | sanitize command | 1 |

## Excel: status table

| Area | Public-doc feature set | Open365 status | Gap | Units |
|---|---|---|---|---|
| File formats | xlsx, xlsm, xltx, xltm, xlsb, xls, csv, ods | xlsx/xlsm/xltx/xltm lossless (62/62 templates byte-identical); instantiate template | csv (quick), xls/xlsb/ods | 1 csv, 8 others (post-MVP) |
| Functions | 521 | 318 registered; 99.6 % of 53,965 cases match Excel. By category (Excel / registered / missing): math 82/69/13, statistical 111/30/81, financial 55/23/32, date-time 25/25/0, text 44/40/4, lookup 36/34/2, logical 18/18/0, information 21/21/0, engineering 54/36/18, database 12/12/0, compatibility 41/10/31, web+cube+other 25/0/25 | 205 missing; statistics need Excel-exact gamma/normal CDF routines not yet identified (calc-coverage.md) | 10 MVP (stats, financial, math, compat), 6 more for engineering/rest; web/cube 2 or n/a |
| Dynamic arrays and LAMBDA | spill, FILTER, SORT, LET, LAMBDA family | **done** in engine (LET, LAMBDA, MAP, REDUCE, SCAN, BYROW, BYCOL, MAKEARRAY, XMATCH, SORT, UNIQUE, ...); spill display in grid unverified | spill ranges UI, `#SPILL!` | 1 |
| Calculation | auto/manual, iterative, precision | partial: dependency recalculation, calcChain hygiene; iterative calc and manual mode UI **(unverified)** | options | 1.5 |
| Number formats | built-in + custom, locale | **done**: engine matches Excel (8,251/8,306 at start; later batches); all 418 LCIDs; text parity 93.3 % on templates | Format Cells dialog | 3 (UI) |
| Cell formatting | fonts, fills, borders, alignment, styles, themes | partial: render exact (theme tint, borders, wrap, overflow, `####`); edit: none beyond width/height | format commands + UI | 3 |
| Conditional formatting | rules, colour scales, data bars, icon sets | **done render/eval**: 504/504 DisplayFormat cells match Excel; negative data bars; edit UI missing | rule manager UI and commands | 3 |
| Data validation | list, number, custom, input/error messages | partial: parsed and preserved (fixture `data-validation`); no enforcement or UI | enforce + dropdown + UI | 3 |
| Sort / filter / AutoFilter | sort levels, custom lists, filter by colour/value | missing (autoFilter preserved) | commands, UI | 4 |
| Tables (ListObjects) | styles, structured refs, totals row | partial: model + structured references in calc (86 cells fixed); no create/resize/style UI | commands + UI | 2 |
| Pivot tables | create, refresh, slicers, timelines, pivot charts, GETPIVOTDATA | missing (preserved; GETPIVOTDATA unregistered) | cache reader, render of stored result, refresh engine, create UI | 6 MVP (render + refresh), 10 more |
| Charts | column, bar, line, pie, area, scatter, bubble, stock, surface, radar, treemap, sunburst, histogram, box-whisker, waterfall, funnel, map, combo; trendlines, error bars, secondary axis | missing (chart parts preserved; fixture `chart`) | chart model, layout, renderer, UI; shared with Word/Slides | 12 MVP (6 core types), 8 more for all types |
| Sparklines | line, column, win/loss | missing (preserved extLst) | render + edit | 2 |
| Names | defined names, scope, name manager | partial: relative and sheet-scoped names evaluate (2,189 + 95 cells fixed); no manager UI | UI | 1 |
| What-if | Goal Seek, Data Tables, Scenario Manager | missing (Data Table `{=TABLE()}` not evidenced) | engine features | 1 Goal Seek, 2 Data Table, 1.5 Scenario |
| Solver, Analysis ToolPak | add-in | missing | simplex/GRG solver | 4 (post-MVP) |
| Power Query, Power Pivot | Get and Transform | **out of scope**; connection parts preserved | n/a | 0 |
| Freeze panes / split / views | freeze, split, new window, custom views | partial: frozen panes rendered; no split or UI to set; custom views missing | UI | 2 |
| Rows and columns | insert, delete, hide, group, outline | partial: hide/unhide, resize, auto-fit; insert/delete with reference adjustment, group/outline missing | structural edit with formula rewrite | 4 |
| Fill, autofill, flash fill | drag handle, series, flash fill | missing | fill engine; flash fill is large | 2 fill, 2 flash fill |
| Copy / paste | values, formats, transpose, paste special, cross-app | missing | shared clipboard | in shared |
| Find / replace / go to | formulas, formats, within sheet/workbook | missing | engine + UI | 2 |
| Comments and notes | threaded comments, legacy notes | partial: parsed, preserved (fixture `comments-notes`); no UI | UI | 2 |
| Protection | lock cells, protect sheet/workbook, password, ranges | partial: `locked` flag feeds CELL(); no enforcement | enforce + UI | 2 |
| Hyperlinks, images, shapes | insert, link, drawing layer | missing (preserved; HYPERLINK function done) | drawing layer | 3 |
| External links | `[1]Sheet!A1`, update links | partial: fixture `external-link-formula` cached values; refresh = egress-class command, not built | link manager with permission gating | 2 |
| Grouping, subtotals, text-to-columns, remove duplicates, consolidate | Data tab tools | missing | commands | 3 |
| Print setup | print area, titles, page breaks, scaling, header/footer | missing in grid; PDF export is Word-only so far (`docx export-pdf`; `ExportPdf` command is document-agnostic **(unverified)**) | grid pagination to PDF/print | 4 |
| Multi-sheet UI | tabs, add/rename/move/colour, hidden sheets | partial: hidden sheet model; tab bar **(unverified)** | UI | 1.5 |
| Locale | formula separators, date systems, function names in 100+ languages | partial: format locale done (418 LCIDs); localised function names and list separators missing | function-name tables | 2 |
| Performance | 1M rows | partial: virtualised grid, large-sheet fixture round trip 360 ms; calc speed not benchmarked at scale | benchmark + fix | 2 |
| Accessibility | grid AccessKit | **done** grid tree + IME (51d0ca7); checker missing | checker | shared |
| Values fidelity on real workbooks | match Excel recalculation | **94.1 %** values, 90 % formula cells on 62 templates | raise to 98 % | 3 |
| Macros | VBA | **preserve** | none | 0 |
| Collaboration | co-authoring | missing | server track (after Teams T0–T4; ADR-014) | 12 (separate) |

## Shared substrate (counted once)

| Item | Status | Units |
|---|---|---|
| Ribbon / command surface, dialogs framework, keyboard shortcuts, status bar | menu bar with 3 menus only | 6 |
| Rich clipboard (text, HTML, images, cells, Office formats) | missing | 4 |
| Chart engine (model, layout, render, PDF/print) | missing; counted in Excel row | 0 here |
| Spell-check engine (dictionaries, licensing) | missing | 3 (counted in Word row) |
| Print dialog, preview, page ranges | menu items exist | 3 |
| Accessibility checker | missing | 2 |
| Theme/font management UI | partial (installed fonts + OFL Carlito/Caladea) | 2 |
| Sanitize for sharing, document inspector | CLI inspect only | 1 |
| **Shared total (MVP)** | | **about 16** |

## Sizing and calibration

- Observed: 71 commits in about 14 hours of repository history (38 on 10-10, 33 on 10-11). The brief puts the landing rate at about 10 units per 24 h. That is roughly 7 commits per unit. Target rate for planning: 25 to 40 units per day with 6 concurrent agents; below this is the observed rate.
- Sizes above are totals per row. Where a row says "MVP" and "post-MVP", only the MVP part is in the MVP sum.

| Bucket | Word | Excel | Shared | Total |
|---|---:|---:|---:|---:|
| MVP-complete units | 70 | 80 | 16 | 166 |
| MVP range (-30% / +60%) | 50 to 110 | 55 to 125 | 11 to 26 | 116 to 260 |
| Post-MVP to 100 % (excl. collaboration, macros execution, Power Query) | about 70 | about 60 | about 6 | about 136 |
| Collaboration / server (both apps) | 12 | 12 | | 24 |
| Layout fidelity long tail (Word 85 % to 99 %) | 15 | | | 15 |
| **100 % parity, total** | about 155 | about 150 | about 22 | about 330 |

## Dependencies and recommended order

1. **Shared chart engine** has no dependency and unblocks Excel charts, Word charts and later Slides. Start early; it is the biggest zero-coverage gap.
2. **Structural edit commands** (insert/delete rows and columns with formula reference rewrite; Word table edit; list edit) all need the command layer plus snapshot undo, which exist. They are independent per app.
3. **Rich clipboard** depends on the table and cell edit commands for useful paste. Start the text and cell-range copy in parallel; paste-special after step 2.
4. **Find/replace** depends on nothing (document traversal exists in `ranges.rs`).
5. **Ribbon / format UI** depends on the commands it calls; build UI after each command group lands, not first.
6. **Fields update** (TOC, REF, SEQ) depends on bookmarks and headings (styles); then citations, index, mail merge.
7. **Track changes** depends on the run/paragraph edit commands recording revisions; compare depends on it.
8. **Pivot tables** depend on tables, the function set (GETPIVOTDATA), and charts (pivot charts).
9. **Statistics functions** are independent but gated by identifying Excel's gamma and normal-CDF routines (recorded in calc-coverage.md). Start with those not needing them (CORREL, SLOPE, LINEST, DEVSQ, and so on: about 25 functions).
10. **Layout fidelity** is a parallel continuous track on the template corpus, shared by every Word feature. It needs the single-machine Word oracle, so it is serial (one oracle run at a time).
11. **Collaboration** needs the server decision and starts after both apps hit MVP.

## ETA

Assumptions:

1. 6 concurrent agents; throughput 25 to 40 units/day (brief). Observed 10/day is shown as the pessimistic case.
2. Throughput is lower than the headline when work needs the Windows Word/Excel COM oracle: it runs on one machine and one instance at a time, so oracle-bound units (layout, calc batches, number formats) cap at about 2 to 3 concurrent.
3. Integration overhead and rework multiply the raw time by 1.3 to 2.0 (merge of shared files such as `command.rs`, `app.rs`; flaky floors).
4. Word MVP and Excel MVP draw on the same shared 16 units and compete for the same agents.
5. MVP definition: daily-use editing (typing, formatting, structure, find, clipboard, undo, print/PDF) with open/save fidelity on the 136-template corpus (round trip byte-identical when unedited; edited outputs open in real Office with no repair prompt; value parity at least 98 % for Excel; layout at least 85 % lines on templates for Word).
6. No external blockers (licences for spell-check dictionaries, signing certificates, hardware).

| Target | Units | Days at 40/day | Days at 25/day | Days at 10/day | With 1.3 to 2.0 overhead (25 to 40/day) | Confidence |
|---|---:|---:|---:|---:|---|---|
| (a) Word MVP (70 + 8 shared share) | 78 | 2.0 | 3.1 | 7.8 | **3 to 6 days** | Low-medium. Unknown: how fast real-template layout goes 39 % to 85 %. |
| (b) Excel MVP (80 + 8 shared share) | 88 | 2.2 | 3.5 | 8.8 | **3 to 7 days** | Medium. Calc is measured; charts and pivots are the risk. |
| (a)+(b) together | 166 | 4.2 | 6.6 | 16.6 | **6 to 13 days** (about 2 weeks) | Low-medium |
| (c) 100 % parity (incl. long tails, excl. collaboration) | about 306 | 7.7 | 12.2 | 30.6 | **10 to 24 days** raw, plan **4 to 10 weeks** | Low. The last percent of layout and function fidelity is asymptotic. |
| (c) plus collaboration/server | about 330 | 8.3 | 13.2 | 33 | plan **5 to 12 weeks** | Very low |

Reading the table: the first column set is arithmetic. The plan figures are the lead's recommended commitments, widened because (1) the observed rate was 10/day, a quarter to 40 % of the target; (2) parity work on real files has shown a long tail (Word layout 89.5 % on own fixtures but 39.3 % on real templates); (3) human review points are required between milestones (CLAUDE.md). If the observed 10/day persists: Word MVP about 8 days of pure throughput (about 2 weeks with overhead), both MVPs about 3 to 5 weeks.

"100 % parity" is not a testable statement. Define it as: all rows above `done` or `n/a` by decision, floors at or above 98 % on both corpora, and no `missing` in the feature rows. Do not claim full compatibility without that evidence (CLAUDE.md).

## Top 10 gaps by user impact

1. **Word layout on real documents: 39.3 % of lines, cells, pictures** match Word on the 74 templates. Everything a Word user sees depends on it.
2. **Charts: none, in either app.** Most Excel workbooks and many Word reports contain them. Chart parts are preserved but cannot be seen or edited.
3. **Word editing breadth.** Only bold, italic, underline, alignment, style pick. No fonts, colours, lists, indents, tabs UI.
4. **Word tables cannot be edited** (layout is strong; editing is zero).
5. **No rich clipboard and no find/replace** in either app. Daily use is impossible without them.
6. **Excel structural edits and data tools**: no insert/delete rows/columns, fill/autofill, sort, filter, copy/paste of formulas with reference adjustment.
7. **205 of 521 functions missing**, mainly statistics (81), financial (32), compatibility names (31). Workbooks with those show cached values until edited, then break.
8. **Pivot tables** are not rendered or refreshable.
9. **Track changes and comments** are not usable (collaboration and review workflows).
10. **Fields (TOC, REF, SEQ) do not update**; footnotes and headers cannot be edited; no spell check. Close behind: data validation enforcement, print setup for sheets.

## Recommended next 6 agent units (disjoint file ownership)

`crates/commands/src/command.rs`, each crate's `lib.rs`, root `Cargo.toml` and `crates/ui-egui/src/app.rs` are serialised registration points: the lead adds the one-line registrations; agents add new files only.

| # | Agent | Task | Owns (new or exclusive files) | Acceptance |
|---|---|---|---|---|
| 1 | o365-calc | Statistics batch 1 without special-function routines (CORREL, SLOPE, INTERCEPT, RSQ, STEYX, DEVSQ, AVEDEV, GEOMEAN, HARMEAN, SKEW, KURT, STDEVA, VARA, TRIMMEAN, PERCENTRANK, FREQUENCY, LINEST/TREND/FORECAST.LINEAR, MMULT/MINVERSE/MDETERM, SUMX2MY2) plus legacy aliases | `crates/calc/src/functions/stats_regression.rs`, `functions/matrix.rs` (new), `fixtures/calc/conformance.json` (append), `docs/testing/calc-coverage.md` | Conformance: no new mismatch; coverage doc updated; calc tests pass |
| 2 | o365-layout | Real-template layout ratchet: top 3 root causes from `PARITY_DETAIL` on the lowest templates (0/57, 0/118, 3/178) | `crates/layout/src/flow.rs`, `para.rs`, `table.rs`, `float.rs`, `fixtures/reference/ms-templates-layout-floor.toml`, `docs/architecture/word-layout.md` | Floor rises with no template falling; `cargo test -p open365-layout` |
| 3 | focused-builder (xlsx) | Insert/delete rows and columns with reference rewrite, merged cells, names, CF ranges, tables; undo | `crates/xlsx/src/sheet/structure.rs` (new), `crates/calc/src/refshift.rs` (new), `crates/commands/src/xlsx_structure.rs` (new), fixture `fixtures/xlsx/structure-*` | Round-trip + Excel oracle compare on the template corpus; tests pass |
| 4 | focused-builder (docx) | Table edit commands: insert/delete row and column, merge/split cells, cell shading and borders, undo | `crates/docx/src/table_edit.rs` (new), `crates/commands/src/docx_table.rs` (new), fixtures `fixtures/docx/table-edit-*` | Edited outputs validate and open in Word (oracle); tests pass |
| 5 | o365-ui | Find/replace (both apps) plus text and cell-range copy/paste (plain, HTML) | `crates/ui-egui/src/find.rs`, `clipboard.rs` (new), `crates/docx/src/search.rs` (new) | kittest UI tests; replace goes through commands with undo |
| 6 | focused-builder (new crate) | Chart model + parser + preserve, plus layout to vector primitives for column, bar, line, pie (xlsx and docx chart parts) | `crates/chart/**` (new crate), `fixtures/xlsx/chart-*`; Cargo.toml registration by lead | Parse the `chart` fixture and 10 template charts; render matches Excel image export by SSIM threshold set in the brief; no edit yet |

Conflicts: 3 and 4 both add modules to `crates/commands` (new files, separate); 1 and 3 both touch `crates/calc` in different files (`functions/*` vs `refshift.rs`); `fixtures/calc/conformance.json` belongs to unit 1 only.

## Unresolved questions

- Does the grid already paginate for print/PDF? `ExportPdf` is generic but only the Word path is evidenced.
- Whether columns layout and zoom exist (not found in a quick scan).
- Which spell-check dictionary has a licence compatible with MIT (to audit).
- Excel's gamma/normal routines, blocking about 56 statistical functions bit-exact.
- Real Microsoft page contents were not re-fetched in this session; refresh the checklist before using it for acceptance.
