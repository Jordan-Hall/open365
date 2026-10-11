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

## Values field, Show Values As, grouping

* **Values field** (`x="-2"` in `rowFields`/`colFields`) is a level of the axis whose items are the
  data fields, so `dataOnRows`, several data fields and the Σ Values position (first, between,
  last; rows or columns) share one axis builder. A field directly above Values has no subtotal
  lines; a field with Values below it gets one subtotal line per data field (`East Sum of
  Revenue`); grand totals become `Total <data field>` lines. With only Values on the rows there is
  no grand total line.
* **Show Values As** (`showas.rs`): `showDataAs` and the x14 `pivotShowAs` extension (percent of
  parent, rank). Every line carries its item constraints; a cell's denominator or base cell is
  re-aggregated from the records under adjusted constraints. Measured rules: an empty numerator
  counts as 0; `% of total/row/column` over an empty denominator is `#DIV/0!`, `% of parent` is
  blank; `% of` and `% difference` give `#NULL!` for an empty cell and blank for a value over an
  empty base; `previous`/`next` skip items that have no line; the named base item shows blank for
  difference and 100% for `% of`.
* **Grouping** (`grouping.rs`): a grouped field (derived `databaseField="0"` date fields or a
  numeric field with `rangePr`) reads the stored `groupItems` as its items; each source value is
  bucketed (below start/blank, in range, above end) for months, quarters, years, days by N,
  hours, minutes, seconds and numeric ranges. Group items are not regenerated on refresh: the
  stored start/end and labels are used.

## Evidence (oracle, en-GB)

`cargo test -p open365-pivot` compares every non-empty cell of the pivots below (value; number
format and display text are in the reference JSON but not compared yet). Floors are in
`tests/oracle.rs`.

| set | role | pivots matching |
| --- | --- | --- |
| layouts | in-sample | 30/30 |
| aggregates | in-sample | 26/26 |
| subtotals | in-sample | 28/28 |
| values (Σ Values positions) | in-sample | 48/48 |
| showas | in-sample | 21/21 |
| grouping (one cache per pivot) | in-sample | 17/17 |
| holdout-a | hold-out, random layouts | 14/14 |
| holdout-b | hold-out, random layouts | 14/14 |
| holdout-showas | hold-out, random Show Values As | 39/40 |

The open hold-out case is `rankDescending` with the base field outermost over three row
levels (the leaf ranks are not among the sibling items; the comparison set Excel uses is not
identified).

## Not done

* Group items are not regenerated on refresh (auto start/end, changed bounds); manual
  (discrete) groups and date parts with the 1904 date system are untested; the oracle has no
  time-of-day source data yet (hours/minutes/seconds are implemented but not measured).
* Writing: number formats and styles of refreshed cells, `location/@ref` rewrite and a refreshed
  cache (needs a cell-style setter in `open365-xlsx`).
* Calculated fields and items, GETPIVOTDATA (needs a hook in `open365-calc`).
* Label and value filters, top-N, OLAP and data-model caches, `missingCaption`/`errorCaption`,
  `showAll` empty items.
* Rank with the base field outermost over three or more row levels (see above).
