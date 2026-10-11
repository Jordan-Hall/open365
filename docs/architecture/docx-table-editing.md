# DOCX table editing

Code: `crates/docx/src/table_edit.rs` (operations), `table.rs` (`AutoFitMode`, `Table::grid_twips`,
`Table::autofit_mode`). Oracle: `tools/office/table-edit-oracle.ps1`. Reference output:
`fixtures/reference/docx/tbledit-<scenario>/table.tsv`. Test: `crates/docx/tests/table_edit.rs`.

## Method

The oracle builds base fixtures (`fixtures/docx/tbledit-base-*.docx`, generated, no personal data),
copies one per scenario to a temp folder, applies the edit in Word through COM (own instance,
`AutomationSecurity = 3`, quit and killed by PID in `finally`), saves, and dumps the saved
`word/document.xml` as property strings (grid, `tblPr`, `trPr`, `tcPr`, first `pPr`, text). The Rust
test applies the same edit to the same base and compares the same dump, field by field and property
by property. Word 16.0 on this machine, 128 scenarios.

Result: 109 of 125 modelled scenarios identical, 7507 of 7610 compared fields equal. Three
scenarios are skipped (Word 97 table behaviour x2; Word's tracked column insert, see below).

## Measured Word rules (implemented)

* New table (Insert Table, `Tables.Add(range, r, c, wdWord9TableBehavior, wdAutoFitFixed)`): style
  `TableGrid`, `tblW` auto 0, `tblLook` 04A0, no `tblLayout`; columns share *text width - 10 twips*
  (floor each, the remainder goes +1 to the last columns); `tcW` in twips. Content: grid 222 per
  column and `tcW` auto. Window: `tblW` pct 5000 and `tcW` pct share 5000 the same way. At offset 0 the
  table goes before the paragraph; elsewhere the paragraph splits and the table goes between halves.
* New row/rows: deep copy of the reference row (`trPr`, `tcPr`, paragraph `pPr`), text emptied;
  inside a vertical merge the new cell continues it, otherwise it is plain.
* New column: new cell has only `tcW` (width of the reference column), paragraph `pPr` copied.
  The grid grows; `tblW` in twips follows the sum, an `auto` width becomes twips when wider than the
  text area, a percentage scales and the cell percentages follow the grid.
* Delete row: vertical merges are repaired (a restart row's successor becomes the restart; a lone
  start/continue becomes a plain cell). Delete column: spanning cells narrow.
* Merge: text of the non-empty cells joins as paragraphs; the first cell keeps its properties; width is
  the sum; rows left holding only continuations are dropped; the grid loses unused boundaries.
* Split: columns divide width with cumulative floor and refine the grid (other rows span the new
  columns); a spanning cell split into its span uses the existing grid; rows add rows below and
  the other cells of the row merge down; a vertically merged cell re-divides its rows.
* Column width: `wdAdjustNone` only that column; proportional/equal/next absorb to the right.
* Border set on a cell is mirrored onto the neighbour that shares the edge.
* Alignment writes `jc` in `tblPr` and in every `trPr`. Fixed auto-fit sets `tblLayout fixed` and
  `tblW` auto; Content removes the layout, `tcW` auto; Window sets pct widths.
* Track changes: inserted row = `trPr/ins` + `pPr/rPr/ins` on each cell paragraph; deleted row =
  `trPr/del` + `w:del` runs + `pPr/rPr/del`; a row the same author inserted is removed outright.
  Inserted table = every row inserted. Tracked table delete = every row deleted.

## Known divergences (listed in `known_divergence` in the test)

| Scenario | Difference | Reason |
|---|---|---|
| `autofit-content*`, `col-right-content` | grid (609 vs ours) | Word measures content |
| `table-pref-*`, `table-borders-outside`, `col-width-window`, `autofit-window-narrow` | grid | Word re-lays the table out |
| `merge-all`, `split-2x1`, `split-2x2` | `trHeight` | Word writes measured heights |
| `row-shading`, `cell-border-none` | extra `bottom` borders on cells above | Word materialises style borders |
| `col-*-merged`, `delete-col-merged` | whole row structure | Word works cell by cell with ragged rows; we keep the grid rectangular and widen/narrow spanning cells |

## Not done

* Tracked column insert/delete, merge and split (Word writes `tblPrChange`/`tcPrChange`,
  `cellIns`/`cellDel`/`cellMerge`).
* `TableGrid` is not in `Styles::ensure_builtin` (styles.rs is not owned here): a document without
  that style gets direct single borders instead of `tblStyle`.
* Cell-level width changes, nested-table tracked deletion, converting with track changes on (an error).
