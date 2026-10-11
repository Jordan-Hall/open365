# PivotTables (`crates/pivot`)

Status: first slice. Measured against Excel 16.0 (build 20430, en-GB) with `tools/office/pivot-oracle.ps1`.

## Model

`PivotTable`, `CacheDefinition` and `CacheRecords` are typed views over the lossless XML tree of
`open365-xlsx` (`ParsedXml`). Unread markup (extension lists, formats, styles, calculated
fields) stays in the tree; `to_bytes()` of an untouched part is byte-identical to the input
(test `untouched_parts_round_trip_byte_identical`, all oracle fixtures).

`sheet_pivots(&Workbook, sheet)` finds the pivot parts through the sheet relationships and the
cache through the table's relationships.

## Computation

`compute(table, cache, dataset, strings, mode)` returns a `PivotGrid` (non-empty cells with
sheet coordinates and the `numFmtId` to apply).

* `Dataset::from_cache` uses the stored records; `Dataset::from_workbook` re-reads the source
  range (a refresh). `OrderMode::Stored` keeps the item order saved in the file;
  `OrderMode::Refresh` re-sorts per `sortType` with the en-US secondary-strength collator
  (case-insensitive, numbers < text < logical < error < blank, descending reverses all).
* Aggregates: Sum, Count, Average, Max, Min, Product, CountNums, StdDev, StdDevp, Var, Varp.
  A cell with no non-blank source value is empty; StdDev/Var of one value is `#DIV/0!`.
* Layouts: compact, outline, tabular; subtotals automatic, none or custom (several functions);
  subtotals at top or bottom; blank row after items; repeat item labels (`fillDownLabels`);
  grand totals on rows and columns (`rowGrandTotals` is the bottom row, `colGrandTotals` the
  right column); several data fields on columns; column fields with subtotal columns; page
  fields (area above the table, item filter); hidden items; sort by label and sort by value
  (`autoSortScope`).
* Captions Excel does not store (`Row Labels`, `Column Labels`, `Grand Total`, `(blank)`,
  `(All)`, `Total`, custom subtotal names `Sum`, `Max.` ...) come from `PivotStrings` (English
  defaults); stored captions (`rowHeaderCaption`, `grandTotalCaption`, data field names) win.

`refresh_sheet(&mut Workbook, sheet, strings)` recomputes from the source range and writes
values with `Workbook::set_cell_value`. `open365-xlsx` has no cell-style setter, so number
formats are returned in `GridCell::num_fmt_id` but not applied, and the `location/@ref` of the
pivot part is not rewritten when the output size changes.

## Evidence (oracle, en-GB)

`cargo test -p open365-pivot` compares every non-empty cell of 112 pivots (value; number
format and display text are in the reference JSON but not compared yet). Floors are in
`tests/oracle.rs`.

| set | role | pivots matching |
| --- | --- | --- |
| layouts | in-sample | 27/30 |
| aggregates | in-sample | 26/26 |
| subtotals | in-sample | 27/28 |
| holdout-a | hold-out, random layouts | 14/14 |
| holdout-b | hold-out, random layouts | 11/14 |

`showas` (21 pivots) and `grouping` (10) fixtures exist but are not computed yet.

## Not done

* Data fields on rows (`dataOnRows` with several data fields).
* Show Values As (percent of total/row/column/parent, difference, running total, index, rank).
* Date and number grouping (`fieldGroup`), including the derived group cache fields.
* Outline/tabular rows with several data fields and subtotal columns; outline row labels with
  several data fields shift the column labels by one (holdout-b P5/P6).
* Label and value filters, top-N, calculated fields/items, OLAP and data-model caches,
  `missingCaption`/`errorCaption`, `showAll` empty items, `dataField` number format fallback.
* Text and number-format comparison against the reference `nf` and `t` fields.
* Writing a refreshed cache (`pivotCacheRecords`) back, location resize, cell styles.
