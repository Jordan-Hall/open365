# Parity scorecard: Open365 Writer and Sheets vs Word and Excel

Measured 2026-10-11 on the working tree (`git log`: 80 commits, first 2026-10-10 18:32, last 2026-10-11 19:53).
Supersedes the figures in `feature-parity.md` (written earlier the same day; its 39.3 % layout and 318-function numbers are stale).

## How to read this

- Every percentage has a source tag. `[run]` = I ran the command today with `CARGO_TARGET_DIR=D:\dev\open365\target-score`.
  `[doc]` = taken from a repo doc, not re-run. `[count]` = my script or grep over the files named.
- Status: none, partial, near. "Engine only" = code and tests exist in a crate, but no app, command or MCP tool calls it.
- Size is in agent-weeks (one agent working one calendar week). Sizes are my judgement, not measured: error band -30 % / +60 %.
- Calendar weeks = agent-weeks / 12 (midpoint of 10-15 agents), then widened for serial work. The Word/Excel COM oracle runs on one
  machine, so layout, calc and number-format work caps at 2-3 concurrent agents. UI work is serialised through `apps/preview/src/shell.rs`
  (2,365 lines), `apps/writer/src/main.rs` and `apps/sheets/src/main.rs`.
- The current pace was observed for about one day (80 commits, 241,623 lines of Rust, 52,814 of them in tests; `[count]` `find crates apps -name '*.rs' | xargs cat | wc -l`).
  One day is a weak baseline. Do not read the ETA as a commitment.

## Measured figures

| Measure | Result | Source |
|---|---|---|
| Excel conformance, exact match | 99,754 / 109,751 = **90.9 %**; 0 unexpected | `[run]` `cargo test -p open365-calc --test excel_conformance -- --nocapture` |
| ...the 9,997 known differences | ulp-noise 7,593; algorithm-unknown 1,397; precision-loss 885; domain-rule 121; nondeterministic 1 | same run, "known differences by category" |
| Excel functions implemented | 494 / 521 = **94.8 %** (27 missing, 23 named in the doc: GETPIVOTDATA, RTD, FILTERXML, WEBSERVICE, 7 CUBE*, IMAGE, STOCKHISTORY, TRIMRANGE, GROUPBY, PIVOTBY, PERCENTOF, CALL, DETECTLANGUAGE, EUROCONVERT, JIS, REGISTER.ID, TRANSLATE) | `[doc]` `docs/testing/calc-coverage.md`, "Missing by category" (2 + 25 missing) |
| Workbook values vs Excel, 62 MS templates | values 15,174 / 16,038 = **94.6 %**; text 14,938 / 15,925 = **93.8 %**; formula cells: values 8,659 / 8,798 = 98.4 %, text 8,475 / 8,798 = 96.3 % | `[run]` `cargo test -p open365-testkit --test ms_template_values -- --nocapture` |
| Word layout, 74 real MS templates, system fonts | 5,396 / 7,998 = **67.5 %**; 17 of 74 templates at 100 % | `[run]` `cargo test -p open365-layout --test ms_template_parity -- --nocapture` |
| Word layout, own oracle fixtures, bundled fonts | 12,262 / 12,801 lines = **95.8 %** (3 fixtures skipped, need Symbol/Arial) | `[run]` `cargo test -p open365-layout --test word_parity -- --nocapture` |
| Word glyph placement in PDF export | 99.37 % paired, 88.28 % within 0.5 pt, 185 of 347 pages exact | `[doc]` `docs/architecture/pdf-export.md` line 99 (125 fixtures) |
| Ribbon commands wired, Writer | 20 / 198 = **10.1 %** | `[count]` ribbon ids in `crates/ui-shell/spec/writer.json` (tabs home..help) whose quoted id appears in `apps/writer/src/*.rs` |
| Ribbon commands wired, Sheets | 37 / 226 = **16.4 %** | `[count]` same method, `spec/sheets.json` vs `apps/sheets/src/*.rs` |
| Tab-by-tab wiring | Writer: Home 21/40, View 3/27, every other tab 0 (Insert 29, Design 10, Layout 19, References 22, Mailings 21, Review 25, Help 5). Sheets: Home 37/43, View 2/31, every other tab 0 (Insert 32, Draw 8, Page Layout 24, Formulas 28, Data 34, Review 18, Automate 3, Help 5) | `[count]` per-tab split from `ribbon.py` (scratch); spec ids vs `apps/preview/src` |
| Edit command surface | 44 `Command` variants (14 docx, 4 xlsx cell/paste/replace, 8 row/column, 7 format, Save, ExportPdf, 5 Share) | `[count]` `crates/commands/src/command.rs` |
| Docx revisions accept/reject | 490 / 490 states exact; `KNOWN_GAPS` is empty | `[doc]` `docs/architecture/docx-revisions.md` line 53; `crates/docx/tests/revisions.rs:332` |
| Docx fields vs Word | format 1473/1473, date 564/564, refs 225/225, TOC 134/134, props 69/69 | `[doc]` `docs/architecture/docx-fields.md` line 126 |
| Charts | layout rects 140/140 + 44/44 (+-1 pt); linear axis 346/346; end-to-end 299/300 | `[doc]` `docs/architecture/charts.md` lines 19, 32, 34 |
| Pivot tables | refresh and styles 28/28, 40/40, 63/63 in-sample; 10 hold-out pivots and 12 custom-style pivots still differ | `[doc]` `docs/architecture/pivot-tables.md` lines 47-117, 193-211 |
| PDF/A, PDF/UA | 68 / 68 (17 fixtures x 4 modes), sample only | `[doc]` `docs/architecture/pdf-export.md` line 92 |
| Package round trip | 136 / 136 MS templates byte-identical when unedited | `[doc]` `docs/roadmap/feature-parity.md` evidence table; not re-run |
| Platform CI | ubuntu + windows only; no macOS, iOS, Android or wasm job | `[count]` `.github/workflows/ci.yml` lines 12-14 |
| Consumers of engine crates | `open365-chart` and `open365-pivot` are in no app's `Cargo.toml`; `open365-spell` only in `ui-egui` (`proofing.rs`), not `apps/writer` | `[count]` `grep -rn "open365-chart" --include=Cargo.toml` and `grep -rln open365_spell apps crates/ui-*` |

## Known gaps lists

- `KNOWN_GAPS` in `crates/docx/tests/revisions.rs:332` is `&[]`. Empty means the 22 `rev-*`/`cmt-*` fixtures match, not that revisions are complete.
- `docs/architecture/pdf-export.md` "Known gaps" (line 285): PDF/A and PDF/UA validated on a sample; structure differences (TOC entries, CommentAnchor, no `/Pg` on struct elements,
  no text-box probe); `w:eastAsia`/`w:bidi` language by script not read; CFF subroutines not pruned. PDF export exists for Word only (no xlsx path in `crates/pdf`).
- `docs/architecture/pivot-tables.md` "Not done" (line 193): grouped fields, calculated items, shared caches, user `formats`, label/value filters, top-N, GETPIVOTDATA wiring.
- `docs/architecture/docx-table-editing.md` "Not done" (line 56): tracked column edits, merge/split revisions, `TableGrid` style.

## Area scorecard

Status describes what a user can do in the app today, not what a crate can do. AW = agent-weeks.

| Area | Status | Measured | Main missing | AW | Conf. |
|---|---|---|---|---:|---|
| File fidelity (docx/xlsx) | near | 136/136 byte-identical `[doc]` | edited-output "opens in Office with no repair" gate over a large corpus; .doc/.xls/.xlsb/.rtf/.odt | 4 (+8 legacy) | med |
| Word layout | partial | 67.5 % templates, 95.8 % own `[run]` | bidi/vertical, floats in tables, kerning/justify; 32.5 % of template lines still differ; serial oracle | 30 | low |
| Word editing UI | partial | ribbon 20/198 `[count]` | fonts dialog, paragraph dialog, lists, styles, images, tables, sections, headers/footers | 40 | med |
| Sheets editing UI | partial | ribbon 37/226 `[count]` | fill/autofill, sort/filter, paste special, names, split/group, formula tools | 40 | med |
| Ribbon commands (shared tail) | partial | 57/424 = 13.4 % `[count]` | Insert, Layout, References, Mailings, Review, Data, Formulas, Draw, Help tabs have no handlers | 30 | med |
| Formulas | near | 90.9 % exact, 494/521 `[run]`/`[doc]` | cube/web/pivot functions; 1,397 algorithm-unknown + 885 precision cases | 3 | high |
| Number formats | near | 3,266 `numfmt` corpus cases in the run, 5 known differences `[run]` | Format Cells dialog; locale function names | 3 | high |
| Real-workbook fidelity | near | 94.6 % values `[run]` | raise to 98 % | 3 | med |
| Charts | partial (engine only) | layout 184/184 `[doc]` | no app/PDF consumer; surface, treemap, sunburst, histogram, box, waterfall, funnel, map; trendlines; edit UI | 14 | med |
| Pivot tables | partial (engine only) | 28/28 refresh `[doc]` | no UI, grouping, calculated items, slicers, timelines, pivot charts | 10 | med |
| Tables (Word and Excel) | partial (engine only) | `table_edit.rs`, `tables.rs` exist; no `Command` | commands, UI, styles gallery, structured-ref editing | 8 | med |
| Conditional formatting | partial | `commands/conditional.rs`; render matched Excel in docs only | rule manager UI | 4 | med |
| Data validation | partial (engine only) | `xlsx/src/sheet/validation.rs`; no ribbon id wired (`data` tab 0/34) | enforcement in grid, dropdown, dialog | 4 | med |
| Comments and track changes | partial (engine only) | 490/490 `[doc]`; no `Command`, no UI | record while typing, pane, balloons, compare/combine | 11 | med |
| Fields, TOC | partial (engine only) | 1473/1473, 134/134 `[doc]`; no `Command` | update UI, index, table of figures, citations | 5 | med |
| Equations | none | OMML only in `docx/src/props.rs` | layout, edit | 10 | low |
| Shapes, text boxes | partial | text boxes laid out (`layout/src/textbox.rs`); no `a:prstGeom` anywhere in `crates` | preset shapes, groups, connectors, WordArt, editing | 12 | low |
| SmartArt-equivalent | none | no `smartArt` match in `crates` | layout engine for ~200 SmartArt types | 14 | low |
| Mail merge | none | `MERGEFIELD` parsed in `docx/src/fields.rs` only | data sources, preview, finish, labels, envelopes | 5 | med |
| Macros | none (preserve only) | `vbaProject` kept byte-identical | execution is tier 3 | 0 | high |
| Printing | partial | Writer File>Print and Export PDF; Sheets: none (`apps/sheets/src/main.rs` has neither) | print dialog/preview, Sheets pagination, scaling, titles | 6 | med |
| PDF export | near (Word) | 88.3 % glyphs within 0.5 pt `[doc]` | xlsx path, remaining structure diffs | 3 | med |
| Accessibility | partial | AccessKit tree, tagged PDF, PDF/UA 68/68 `[doc]` | checker, alt-text UI, reading order | 4 | med |
| Spell and grammar | partial (engine only) | `crates/spell` 59 tests; not in `apps/writer` | wire squiggles, autocorrect, thesaurus; grammar | 3 + 8 | med |
| Collaboration, sharing | partial | server 11,999 lines, 74 tests, share dialog `[count]` | real-time co-authoring, version history, mentions | 30 | low |
| Mobile (iOS, Android, web) | none | no target in `ci.yml` | M5 spike then M6 apps | 40 | low |
| Mac and Linux builds | partial | Linux in CI, macOS absent `[count]` | macOS CI, signing, packaging, platform QA | 6 | med |

## Totals, four tiers

Tier sizes are sums of the rows above plus items the table folds together. Calendar weeks assume 12 agents, then show a widened range for serial
oracle and UI-file bottlenecks.

| Tier | Contents | AW | Calendar (12 agents) | Plan range |
|---|---|---:|---:|---|
| 1. Everyday documents and spreadsheets: open, edit, save, print identically | Word layout to ~95 % (30), core ribbon wiring in both apps (60), Word edit features (36), Excel charts, sort/filter, validation, CF, tables (37), formulas and value fidelity (6), Office-open gate (4), Mac/Linux (6), accessibility (3) | about 180 | 15 weeks | 15 to 28 weeks |
| 2. Power-user features | pivot UI, remaining chart types, equations, shapes, SmartArt, mail merge, co-authoring, mobile, legacy formats, bidi, grammar, what-if/Solver, protection | about 170 | 14 weeks | 14 to 26 weeks |
| 3. Long tail: VBA, Power Query, Data Model, add-ins | VBA interpreter with sandbox (about 50), Power Query M (about 40), Data Model and DAX (about 40), add-in host (about 20) | about 150 | 12 weeks | 12 to 24 weeks |
| 4. Cannot be 1:1 (legal or security) | Copilot, Microsoft Graph, OneDrive/SharePoint, Ideas, stock/geography data types, Translator, IRM/AD RMS, Microsoft trademarks, logos, Calibri/Cambria files | 0 (replace, not clone) | n/a | n/a |
| **Tiers 1-3** | | **about 500** | **about 42 weeks** | **about 40 to 75 weeks** |

Tier 1 should be sequenced first: it is the only tier where the owner's "opens, edits, saves and prints identically" claim could be tested.
Tier 1 ETA from this baseline is **about 4 months at best, 6 or 7 months likely**. The earlier "10 to 24 days" in `feature-parity.md` does not survive the measurements above.

## Honest assessment

1. The engine layer is ahead of the product layer. Calc, number formats, docx fields/revisions, charts, pivots and PDF are each measured at 90 to 100 % against Office
   on their own corpora. The shell exposes 13.4 % of the ribbon and 44 commands. Tier 1 is dominated by wiring, which is slower to parallelise than engine work.
2. The single largest user-visible risk is Word layout on real documents: 67.5 % of template lines match. Own-fixture 95.8 % overstates fidelity because the fixtures
   were authored to probe known rules.
3. Charts and pivots have no consumer. A user cannot see a chart in either app today (`grep` finds no `open365_chart` in `apps/` or `ui-*`).
4. 90.9 % exact conformance undersells the function engine (7,593 of the differences are last-digit) and oversells nothing, but 1,397 `algorithm-unknown` cases are
   real divergences from Excel's routines and need black-box work.
5. Doc-sourced figures marked `[doc]` were not re-run. Round-trip 136/136 and the chart, pivot, fields and PDF counts should be re-run before any compatibility claim (CLAUDE.md).
6. Not measured at all: Word/Excel feature completeness by checklist (the checklist in `feature-parity.md` is unverified against current Microsoft pages),
   performance beyond `docs/testing/performance.md` (opc only), accessibility with real screen readers, and any macOS, mobile or web behaviour.
