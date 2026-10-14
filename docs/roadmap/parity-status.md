# Parity status: Open Write and Open Sheet

Written 2026-10-12 at HEAD d000035. Source tags: `[floor]` = I summed the committed `fixtures/reference/**/*floor*.toml` files today;
`[d35]` = build steward's `ribbon_dispatch` run and status report, 2026-10-12, HEAD d000035; `[score]` = `docs/roadmap/parity-scorecard.md` (measured 2026-10-11, not re-run today); `[doc]` = a repo doc; `[backlog]` = the integration
backlog in the session scratchpad `sheets-integration.md` (every ribbon round). Nothing here is a compatibility claim; see `../testing/fidelity-strategy.md`.

**Ribbon re-run succeeded** at d000035 on 2026-10-12 (`cargo test -p open365-preview --test ribbon_dispatch -- --nocapture`), so the ribbon figures below are current as of that commit.

## 1. Measured status

| Metric | Value | Source |
|---|---|---|
| Word layout, 74 MS templates, floor lines matched | 6,264 / 7,998 = 78.3 % (scorecard run: 67.5 %; the floor has been raised since) | `[floor]` `ms-templates-layout-floor.toml`; `[score]` |
| Word layout, own oracle fixtures (277 floor entries) | 26,541 / 27,206 = 97.6 % | `[floor]` `layout/parity-floor.toml` (scorecard run: 95.8 %) |
| Word char-level agreement (charx) | 855,176 / 892,740 = 95.8 % | `[floor]` `layout/charx-floor.toml` |
| Workbook values vs Excel, 62 MS templates | 15,090 / 16,038 = 94.1 % | `[floor]` `ms-templates-values-floor.toml` |
| Workbook text vs Excel | 14,854 / 15,925 = 93.3 % | `[floor]` same file |
| Formula conformance vs Excel | 9,661 known differences in 120,297 cases; 110,636 / 120,297 = 92.0 % | `[d35]` |
| Excel functions implemented | 494 / 521 = 94.8 % | `[score]` |
| Ribbon wired, Write | 161/198 = 81.3 % | `[d35]` |
| Ribbon wired, Sheet | 154/226 = 68.1 % | `[d35]` |
| Preset shapes matching Word | 184 / 187 = 98.4 % | `[d35]` |
| Revisions accept/reject | 490 / 490 | `[doc]` `docx-revisions.md` |
| Fields (format, date, refs, TOC) | 1473/1473, 564/564, 225/225, 134/134 | `[doc]` `docx-fields.md` |
| Chart layout / pivot refresh | 184/184; 28/28 in-sample (10 hold-out pivots differ) | `[score]`, `[doc]` `pivot-tables.md` |
| PDF glyph placement within 0.5 pt | 88.28 % | `[doc]` `pdf-export.md` |
| PDF/A + PDF/UA validator | 68 / 68 (sample: 17 fixtures x 4 modes) | `[doc]` `pdf-export.md` |
| Unedited package round trip | 136 / 136 byte-identical (not re-run) | `[score]` |
| Platform CI | ubuntu + windows only | `[score]` |

Reading: engines sit at 90-100 % on their own corpora; the product layer is closing fast (ribbon 81 % Write, 68 % Sheet) but dialogs, printing and clipboard are still partly open.

## 2. Remaining work per app

Must = needed for everyday parity. Long = long tail.

### Open Write (Word)

Ribbon wired, `[d35]`: Home 37/40; Insert 17/29; Design 9/10; Layout 11/19; References 15/22; Mailings 21/21; View 27/27; Review 19/25; Help 5/5.
Layout and References are in progress (not merged).

Ribbon and dialogs
- Done, `[d35]`: dialog framework; Page Setup with Header/Footer, Margins, Columns, Line Numbers, Hyphenation, Watermark, Page Borders, Zoom, Paragraph.
- Must: Size dialog (not listed as done at d000035; confirm); Fonts, Styles, Lists, Tables, Pictures, Sections dialogs and Header/Footer galleries. `[score]`
- Must: ruler margin drag sending `DocxSectionMargins`; format painter on mouse-up; single Replace via `DocxReplaceOne`; match highlights. `[backlog]`
- Must: Track Changes persisted to `w:trackRevisions`; typing in footnotes, endnotes, comments; revisions pane and balloons. `[backlog]`
- Must: Mailings remainder: printing, email, and the Edit Recipient List and Match Fields dialogs. Mail merge itself exists (recipients, merge fields, rules, finish, envelopes and labels), `[d35]`.
- Long: Design theme gallery, citations, index, table of figures.

Engine fidelity
- Must: layout on real documents (21.7 % of template lines still differ, `[floor]`); kerning, justification, floats in tables.
- Long: bidi and vertical text; equations (none beyond OMML parsing); SmartArt-equivalent; preset shapes, groups, WordArt (184 of 187 preset shapes already match Word, `[d35]`). `[score]`

File format
- Must: edited-output "opens in real Office with no repair" gate over a large corpus. `[score]`
- Long: .doc, .rtf, .odt; tracked column edits and merge/split revisions (`docx-table-editing.md`).

UI/platform
- Must: spell check wired into the Write app (engine only today); print dialog and preview (printing in progress, not merged); screen-reader and IME checks with real tools (unmeasured).
- Long: multi-window; macOS, iOS, Android, web builds; grammar.

### Open Sheet (Excel)

Ribbon wired, `[d35]`: Home 40/43; Insert 12/32; Draw 0/8; Page Layout 18/24; Formulas 24/28; Data 26/34; Review 11/18; View 23/31; Automate 0/3; Help 0/5.
Insert picture and shapes, Page Layout, Review and View are in progress (not merged).

Ribbon and dialogs
- Done, `[d35]`: Format Cells (all tabs, including Border), Page Setup with Header/Footer, Margins, Zoom, Sort, Data Validation, Name Manager, Function Arguments, Evaluate Formula, Protect Sheet/Workbook, Insert Table/Link/Symbol/PivotTable. Sort is assumed to cover Custom Sort; confirm.
- Must: Row Height, Column Width, Print Titles, Merge confirm and data-entry alerts dialogs; Insert Function (confirm).
- Must: dialogs whose models exist but are not drawn: Advanced Filter, Text to Columns, Remove Duplicates, Consolidate, Goal Seek, Subtotal, Group. `[backlog]`
- Must: grid drawing for Show Formulas, AutoFilter arrows, outline gutter, validation drop-down, note/comment markers, trace arrows, page-break and page-layout views; route entry through `review_enter`. `[backlog]`
- Must: Backstage commands wired (`file.new/open/save/save-as/print/export-pdf/close`), zoom sync, startup choosers. `[backlog]`
- Done, `[d35]`: pictures, shapes and charts drawn at Excel's positions.
- Long: slicers, timelines, Solver, Power Query.

Engine fidelity
- Must: workbook values from 94.1 % towards 98 %; formula differences (9,661 known in 120,297 cases, `[d35]`) to be classified, including algorithm-unknown and precision-loss groups. `[score]`
- Must: pivot charts and pivot views must reach users through the app; charts now draw, pivot UI is not re-measured. `[score]`
- Long: surface, treemap, sunburst, histogram, box, waterfall, funnel, map charts; pivot grouping, calculated items, label/value filters, top-N (`pivot-tables.md`).
- Long: 27 missing functions (cube, web, GETPIVOTDATA, RTD, STOCKHISTORY, GROUPBY, PIVOTBY and others). `[score]`

File format
- Must: edited-output Office-open gate. Long: .xls, .xlsb.

UI/platform
- Must: Sheet printing (in progress, not merged; PDF export has no xlsx path yet), rich clipboard formats (in progress), grid accessibility, IME in cell edit.
- Long: multi-window; other OSs.

## 3. Not started

Equations; SmartArt-equivalent; VBA execution (files are preserved, never run); Power Query; Data Model/DAX; add-in host; iOS, Android and web targets (no CI job); macOS CI and signing; real-time co-authoring and version history (the sharing server exists, `[score]`); Open Teams client and server (`adr-teams.md`, next in the owner's order).

## 4. Deviations kept on purpose

- Names and branding: "Open Write", "Open Sheet", "Open Teams" (`crates/branding`), own icons, no Microsoft names or logos. Legal.
- PDF/UA tags: we emit PDF/UA-1 structure where Word's tagged PDF differs (for example link annotations carry `/Contents` only in PDF/UA mode, `pdf-export.md`). Accessibility conformance wins over byte parity.
- No Copilot, Graph, OneDrive/SharePoint, Translator, stock or geography data types, IRM: replaced by Open CoWorker, our sharing server and Drive (`[score]` tier 4).
- Macros are preserved, not executed, until a sandbox exists. Security.
- Calibri and Cambria are not bundled (licence); metric-compatible fonts shift some layout results.

## 5. Rough ETA

Basis: the lead's estimate at d000035 (2026-10-12), at the current pace: Open Sheet everyday parity about 3-5 weeks; Open Write everyday parity about 5-8 weeks; full 1-for-1 parity 3-6 months for each app. This is the lead's estimate, not a commitment.

The earlier, more conservative scorecard figure (`[score]`, measured 2026-10-11) is kept alongside: Tier 1 (everyday open, edit, save, print) about 4 months best case and 6-7 likely; Tier 2 14-26 more weeks. Its pace baseline was one day of 80 commits, which is weak.

| App | Everyday parity (lead estimate) | Full 1-for-1 (lead estimate) | Scorecard Tier 1 (earlier, conservative) |
|---|---|---|---|
| Open Write | 5-8 weeks | 3-6 months | 4 months best, 6-7 likely |
| Open Sheet | 3-5 weeks | 3-6 months | 4 months best, 6-7 likely |

Assumptions: 10-15 parallel agents; the Office COM oracle runs on one machine, so layout and calc work caps at 2-3 concurrent agents; UI work serialises through `shell.rs`; mobile and web (about 40 agent-weeks) are outside Tier 1. The scorecard gives one Tier 1 figure, so it is shared by both rows. Ribbon wiring is now measured (section 1). Treat all figures as a range, not a commitment. Teams (T0-T4) starts after this parity point.
