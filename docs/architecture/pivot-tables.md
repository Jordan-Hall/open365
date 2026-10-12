# PivotTables (`crates/pivot`)

Status: slices 1-5 plus refresh regeneration, including user formats, groups (dates with a time of day, fractional numbers, groups a blank drops) and calculated items (slice 4 styles: built-in pivot and worksheet table styles and custom table styles fitted, user `formats` partly open). Measured against Excel 16.0 (build 20430, en-GB) with `tools/office/pivot-oracle.ps1`.

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

`refresh_sheet(&mut Workbook, sheet, strings)` recomputes from the source range, writes values
(`Workbook::set_cell_value`) and number formats (`format_range`), then regenerates the pivot parts
(`regen.rs`) and writes them with `Workbook::replace_part`: the pivot table (location, field
`items`, page selections, `rowItems`/`colItems`), the cache definition (`sharedItems`, `recordCount`,
`refreshedDate`, no `refreshOnLoad`) and the cache records. `Workbook::save` writes every other part
byte-identical. A pivot the writer does not model (a source that is not a worksheet range, a cache
shared by several pivots) keeps its old cache and gets
`refreshOnLoad="1"` so Excel rebuilds it (`tests/refresh.rs`).

Refresh with user formats, groups and calculated items (`tests/refresh_ext.rs`, fixtures
`pivot-refresh-{formats,formats-removed,grouped,grouped-numbers,grouped-blank-date,calcitems,calcitems-values}`
with `before.xlsx`, `edits.json`, `after.xlsx` and `pivot.json`, made by
`tools/office/make-pivot-refresh-ext.ps1`; the test compares parts, cells, cell formats and display format):

* **User formats.** `formats/format` stays as stored, and the `x/@v` positions are written as Excel
  writes them (an explicit `v="0"` stays; the old code dropped it). Items that left the source stay
  as missing items (`m="1"`, `u="1"`) so the positions hold. Measured: table, cache and records parts
  equal Excel's on 10 pivots.
* **Cell formats (`survive.rs`).** The borders, fills and fonts Excel keeps on the pivot's cells are not
  moved with the cells: Excel lays the `formats` areas over the new layout on every refresh. We do the
  same: the user layer (the area engine of `style/formats.rs` without the table style) is written into
  the cell formats of the new area, cells the table left lose their formats. All 10 format pivots equal
  Excel's cell formats (`xf`) and display format after the refresh (rules below, "User formats").
* **Calculated items.** Regenerated: the calculated item is a shared item with `f="1"` after the items
  of the source, keeps its place in the pivot field `items`, and its `calculatedItem` reference gets
  the new shared index. 8 of 8 pivots equal Excel's parts, cells, cell formats and display format.
* **Grouped fields.** Regenerated (`grouping::regroup`): automatic start and end follow the data (date
  groups: first date to the day after the last date; number groups: smallest to largest value; a bound
  set by hand stays), the group items are rebuilt (`<dd/mm/yyyy` and `>dd/mm/yyyy` markers, month and
  quarter names, years, `lo - hi` day ranges clipped to the end, `lo-hi` number ranges where the last
  group ends at the end bound when `lo + step` equals it, `<start or (blank)` when the field has
  blanks), the pivot field `items` of a derived group field keep the stored order of the items with
  the same label and add the new ones in index order (months always in calendar order), and the base
  field lists its items in stored order plus new ones. Measured: 11 of 11 date and number pivots
  (months, quarters, years, years and months, days by 30, number groups with explicit and automatic
  bounds, with blanks).
* **Ungrouping.** Excel drops a number group when the source gets a blank and the group was made
  without one (the field is a plain field again: shared items in order of first appearance, no
  `fieldGroup`); modelled and equal to Excel. Excel drops a date group when a source date is blank
  (or text): the derived fields leave the cache and the pivot (`cacheFields`, `pivotFields`, `rowFields`,
  `colFields`, `pageFields`, `dataFields` and `formats` renumbered or dropped), the base field loses its
  `fieldGroup` and returns to inline records (no shared items unless it is on an axis itself) and the
  pivot with no axis field is one value cell (`rowItems` `<i/>`). `ungroup.rs` rewrites the pivot and cache
  that way before the refresh runs (`SheetPivot::without_dropped_date_groups`), so `regen.rs` sees a plain
  pivot. Table, cache and records parts, cells, cell formats and display format equal Excel's on the 2 pivots
  of `grouped-blank-date`. Not measured: the base field on an axis itself, calculated items or filters
  that name a dropped field, a group dropped for only some of the derived fields of one base.
* **Time-of-day and fractional groups** (`make-pivot-refresh-time.ps1`, sets `grouped-time`,
  `grouped-fraction`, `grouped-time-maxwhole`, `grouped-time-onetime`; 24 of 24 pivots equal Excel's
  parts and cells). Measured rules: hours items are `00`..`23`, minutes and seconds `:00`..`:59`, days with
  no interval (alone or beside other parts) are the 366 days of a leap year `01-Jan`..`31-Dec`; the
  `<dd/mm/yyyy` and `>dd/mm/yyyy` markers carry the date of the bounds. A date group's automatic start is
  the first value as it is (time of day kept) and its end the last value as it is, a day later only when
  that last value is a whole date (midnight): `maxDate` of the shared items follows the same rule (the
  edge cases `maxwhole`, a time of day on every value but the latest, and `onetime`, a time of day on one
  middle value, both end a day after the latest whole date). A number group is `lo-hi` with
  `hi = lo + step - 1` only when the data, the step and the bounds are all whole numbers (the last group ends
  at the end bound when `lo + step` equals it), else the groups touch, `lo-(lo+step)`; numbers print with 15
  significant digits. The first item of a number group with blanks is `(blank)` when the start is
  automatic and `<start or (blank)` when it was set by hand, else `<start`.

Measured rules (`tests/refresh_oracle.rs`: `pivot-refresh-{none,values,items,removed}` edited, refreshed by
`refresh_sheet` and compared part by part with Excel's `after.xlsx`, ignoring `refreshedDate`, `cacheId`,
`xr:uid`; 28/28 pivots, plus every cell against `pivot.json` and the stored-cache grid against the refreshed grid):

* Excel shares items (`sharedItems count=..` with item elements) only for fields that were on an axis at
  cache creation; other fields carry type flags and the value range, and their records hold values inline.
* A refresh rebuilds the shared items in order of first appearance in the source rows. Text is merged
  case-insensitively under the first spelling met, in every field (records and labels use that spelling).
* An item that left the source stays at the end of the shared list as `u="1"` (and `m="1"` in the pivot field
  items) for text and number fields whatever the sort type; a date item is dropped. Retained items of a sorted
  field sit at their sorted position in the field items.
* Field items keep the stored order of the items still present; new items follow in order of first appearance
  (manual sort), else the field is sorted. `x` in `rowItems`/`colItems` is the position in the field items.
* Flags: `containsSemiMixedTypes="0"` without text and blanks, `containsNonDate="0"` for date-only fields,
  `containsString="0"`, `containsBlank`, `containsMixedTypes`, `containsNumber`, `containsInteger`,
  `minValue/maxValue`; `maxDate` is the last date plus one day.
* `rowItems`/`colItems`: `t` = default/blank/grand/function name, `r` = leading levels shared with the previous
  line (subtotal and blank lines: their own level), `i` = data field index when above 0.
  Not measured: retained items of a field sorted by a data field (placed last).

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
  hours, minutes, seconds and numeric ranges. A refresh regroups (see "Refresh with user formats,
  groups and calculated items"), hours, minutes, seconds and fractional number groups included.

## Location, styles, layout forms (slice 4)

* **Location** (`location.rs`): `PivotGrid::location` holds `ref`, `firstHeaderRow`,
  `firstDataRow`, `firstDataCol`, `rowPageCount`, `colPageCount` as Excel writes them.
  Measured rules: `ref` is the body without the page-field area; `firstDataRow` is the number of
  header rows; `firstDataCol` the number of label columns; `firstHeaderRow` is 1 when there is a
  column field, else 1 except 0 when several data fields sit on the columns; `rowPageCount` is the
  number of page fields and `colPageCount` 1 when there are any. `PivotTable::set_location` writes
  them into the part (page counts of 0 are removed); writing the stored values leaves the part
  byte-identical. The layout forms (compact, outline, tabular, repeat labels, blank rows, subtotals
  top/bottom) were already measured value by value in slices 1-3 (layouts, subtotals, values sets).
* **Table styles** (`style.rs`): `builtin_styles.txt` is the definition of the 84 built-in
  `PivotStyleLight/Medium/Dark` styles (28 elements each: font bold/italic/colour, fill, six
  borders), read from Excel's `TableStyles` object model by `tools/office/make-pivot-styledefs.ps1`
  (theme colours as OOXML theme index plus tint). `style_cells(table, grid, theme)` composes, per
  cell of the pivot range, the elements that apply (whole table, row/column stripes, first/last
  column, header row, total row, subtotal rows/columns, row subheadings, blank rows) in a fitted
  priority order and resolves a border shared by two cells to the higher-priority element.
  Fitted against `Range.DisplayFormat` of every cell (`tests/style.rs`). Element numbers follow
  Excel's enumeration (RowSubheading1 is 23, BlankRow 19, ColumnSubheading1 20), not the order of
  the ECMA names. Worksheet table styles (`TableStyleLight1-21`, `Medium1-28`, `Dark1-11`) format a pivot too: `builtin_table_styles.txt` (read by `make-pivot-styledefs.ps1 -Kind Table`, elements 0-12) is merged into the built-in table, and the set `tablestyles` (`make-pivot-tablestyles.ps1`, 12 styles in compact, tabular and outline layouts with page fields) is 36/36, 2652/2652 cells after two rules: with the Last Column flag the last column also formats a pivot's only header row (not the first of several), and the page-field area of a worksheet table style carries the whole-table border on all four sides (a pivot style: top and bottom only). The Last Column element ranks above the header row (a table style's last column is bold dark text on the header fill). Custom table styles are read by `style::CustomStyles` (`style/custom.rs`): `tableStyles/tableStyle/tableStyleElement` (type, `dxfId`, stripe `size`) and the `dxfs` they point to (font bold/italic/colour, solid fill `bgColor`, six borders with style thin/medium/thick/hair/dotted/dashed/double); `style_cells_with` looks them up before the built-in names. Rules found with the custom sets: `lastColumn` never formats the first header row; stripe `size` makes stripes of that many rows/columns; the page-field area takes the whole-table top and bottom borders.
  Fitted rules (in-sample `pivot-styles`, 40/40): the header row beats every border; subtotal rows tie
  with row subheadings (the lower cell wins); the total row loses to subtotal rows but beats subtotal
  columns; tabular layouts apply subheading level k to label column k of every line of the group
  (subtotal rows then cover only their own and later columns); `showRowHeaders` off drops row
  subheadings; subtotal columns cover header rows from the second; column subheading k formats the
  item cells of header row k+1; the first header cell covers the label columns of all header rows
  but the last; page-field cells take the whole-table fill/font (not its outer borders) plus the
  page label/value elements.
  Block rules (found in styles2 P63 and customstyles): with Values on the rows, the first-label-column subheading of every data line and every subtotal line is a block of its own (its top and bottom edges are outer edges); a data field caption cell under a column field gets the column subheading of its level and is a block of its own horizontally; an inside border of the whole table beats a heavier-or-equal-weight stripe border only when it is heavier.
  User formats (`style/formats.rs`): `formats/format` (`pivotArea` + `dxfId`) are read in document order (= application order) and laid over the table style by `style_cells_full`; `Dxfs` reads fill, bold/italic, font colour, borders and `numFmt` of the style sheet `dxfs`. An area selects cells by type (all, origin, button, topRight), `dataOnly`/`labelOnly`, `grandRow`/`grandCol` and references (field, item positions, `defaultSubtotal`); a reference list selects the lines whose level is the deepest named field. Round 6 rules for `outline="0"` label areas (`Matcher::field_range`, tabular/outline layouts, found in `formats-areas`): a reference list selects the label cells of the deepest named field's own column (column = its level, capped at the last label column) over every line of the named items, including the lines of its subtree whose cells in that column are empty; with `offset` set (Excel writes `offset="IV256"` on a parent item) only the last label column of the item's own line; a `defaultSubtotal` reference with items selects the subtotal line, last label column only; a `defaultSubtotal` reference without items means "all items and their subtotals" and does not count as the deepest field; `grandRow` + `labelOnly` + `outline="0"` selects all label columns of the grand total line; `topRight` with `outline="0"` is the whole top header row right of the origin. Custom style names (`style/custom.rs`, `tests/customstyles_conflicts.rs`, fixtures `pivot-customstyles-conflicts`, made by `tools/office/make-pivot-customstyles-conflicts.ps1` from hand-edited `styles.xml`): the names Excel ships (`PivotStyle|TableStyle` + Light|Medium|Dark + number, any case) always win over a custom style of the same name (8 of 8 cases, every fill, font, bold, italic and border; `TableStyleMedium2` applies the built-in worksheet table style); custom names compare without case; of several custom styles with one name the first in the file applies; a style marked `pivot="0"` (tables only) applies to a pivot that names it. User format border rules (measured with `make-pivot-formats-probe.ps1`, 43 controlled cases (2 table styles) each read before and after a refresh, `pivot-formats-probe/`; `tests/formats_probe.rs` 41/43 cases equal Excel on every cell): a dxf `border` has `outline` (default true): its edges belong to the outline of the cells the area selects, so a side shows only where the neighbour cell is not selected by the same area (a `databody` left, top, bottom and right format gives the body rectangle's outline); `outline="0"` (Excel writes it for `BorderAround`) gives every selected cell all four edges; the dxf's inside vertical and horizontal borders (`Borders(11)`, `Borders(12)`) show between two selected cells. `type="all"` includes the page-field cells. A shared edge shows the same on both cells (`DisplayFormat`; the cell format `xf` holds an edge on the cell the format selected only): the heavier line wins, at equal weight the darker colour (luminance 0.2126 r + 0.7152 g + 0.0722 b: a thin red beats the gray table-style border, a thin green does not, blue beats red); the blank cells right of a parent line's own label in outline form take fill and font of a field's range but no border. Effect: formats 36/58 to 40/58, formats-borders 11/45 to 26/45, holdout-formats 22/38 to 26/38 (confirmation only), formats-areas unchanged 64/66. Not identified: ties between a user edge and an edge of the style of equal weight and equal luminance, line styles other than weight (dashed, double) at a shared edge, the repeated `(blank)` item cell that gets a box in outline form (probe `outline Region field data right`).
  Open: borders of user formats (remaining `formats` failures are mostly fills/fonts of whole-table and label areas, not inspected here); formats of Excel's document order: dxf ids descend in the file, so the file lists the newest format first, yet applying in document order fits every set so far. Regeneration remaps `x/@v` of the format references through the field-item position map (`regen::remap_formats`; an item that is gone drops its format; unit tests in `regen.rs`). Not tested against Excel: the oracle fixtures with formats share one cache among all pivots (the writer falls back to `refreshOnLoad`), so a single-cache formats fixture is needed.
  Second round (`styles2`, 63/63): the exclusion of label columns left of a subtotal line holds with
  row headers off and for outline forms (`label_cols > 1`: a subtotal line or a non-leaf item of
  level k covers columns from k); a blank row joins the group of column c only when the next line
  stays inside that group (`LineInfo::shared_next`); the page-field area takes the whole-table
  borders between its own cells (inside horizontals), not the outer ones. Value rule found
  through the style set: when a group ends together with its parent and both have blank rows on,
  Excel writes one blank row (`compute`), not one per level.

## Calculated fields and GETPIVOTDATA (slice 5)

* **Calculated fields** (`calc.rs`): `cacheField/@formula` with `databaseField="0"`. Excel
  evaluates the formula over the **sums** of the fields it names, per output cell, so a total cell
  is the formula over the summed fields (not the sum of the cells). The evaluator covers constants,
  field names (quoted `'Per unit'`), `+ - * / ^ & % = <> < > <= >=`, and `IF IFERROR AND OR NOT
  ABS SQRT INT ROUND ROUNDUP ROUNDDOWN POWER MOD SUM MAX MIN AVERAGE`; a division by zero is
  `#DIV/0!`. A cell without records is empty. Measured: 6/6 calculated-field pivots.
* **Calculated items** (`calcitems.rs`): Excel stores them in
  `pivotCacheDefinition/calculatedItems` (`formula="Product[a]+Product[b]"`, a `pivotArea`
  reference to the new shared item with `f="1"`); the item shows as a normal item. Its cell is the
  formula over the aggregates of the named items under the cell's other constraints; a calculated
  field on the same table is then evaluated over those item-level sums; any line that does not
  constrain the field (grand totals, outer subtotals) includes the calculated item (Sum and Count add,
  Max and Min compare). Excel refuses Average, StdDev and Var data fields while a table has
  calculated items, and so does `compute`. Measured: 9/9 item pivots, 15/15 with the fields.
  Excel reports a calculated-item cell as a formula (`=Apple+Banana`) that the file does not store.
  Second round (`calc2`): a calculated-item line is evaluated per item of the axis fields the cell does not
  constrain and the results combine with the data field's function (Sum/Count add, Max/Min compare), so the
  grand-total column of `Apple/Banana` is the sum of the line's cells and of a Max item the maximum of its
  cells, not the formula over the totals (`CalcItems::value` `split`).
* **GETPIVOTDATA hook** (`getpivotdata.rs`): `open365-calc` does not depend on this crate, so the
  hook is data only. calc builds a `PivotIndex` per pivot (`PivotIndex::build(&SheetPivot,
  &PivotStrings)`), picks the one whose `contains(row, col)` holds for the `pivot_table` argument
  and calls `get_pivot_data(&index, data_field, &[(field, Value)])`, returning the `GridValue` or
  the error code (`#REF!`). Rules measured: the data field matches the caption (`Sum of Units`) or
  the source field name, case-insensitively; field names are case-insensitive; the pairs are a set
  (order does not matter, a repeated field with the same item is accepted); the pairs must equal
  the field set of a displayed cell (subtotals and grand total included), otherwise `#REF!`; when
  the field set is displayed and all items exist but the intersection has no value, the result is
  0. The index is a snapshot of the stored-cache grid; rebuild it after a refresh.

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
| location (all sets above) | in-sample and hold-out | 238/238 |
| calc (fields and items) | in-sample | 15/15 (items 9/9, stored and refreshed) |
| calc2 (division, Count/Max data fields, inner-level items, fields beside items) | in-sample, second round | 24/24 pivots, 1136/1136 cells (stored and refreshed; 21/24 before fitting; Min data field not generated) |
| getpivotdata | in-sample | 36/36 queries |
| styles (`Range.DisplayFormat`) | in-sample | 40/40 pivots, 8485/8485 cells (was 21/40, 7608) |
| styles2 (row headers off, blank rows over levels, several page fields, subtotals top) | in-sample, second round | 63/63 pivots, 15485/15485 cells (62/63 before the data-on-rows block rule) |
| holdout-styles | hold-out, random styles/flags/layouts | 39/48 pivots, 10771/11052 cells (38/48 before the data-on-rows block rule; not tuned on) (31/48, 10664 before the second round; unchanged by the custom-style fixes; confirmation only, failures not inspected) |
| holdout-calc (calculated items and fields) | hold-out, random formulas/layouts, written once | 38/40 pivots, 2145/2173 cells (stored and refreshed; 29/40 before calc2; confirmation only, failures not tuned on) |
| customstyles (`styles.xml` tableStyles and dxfs) | in-sample, random element sets | 39/48 pivots, 9671/10288 cells (36/48 before the block rules) |
| tablestyles (worksheet table styles on pivots) | in-sample | 36/36 pivots, 2652/2652 cells (26/36 before the page-area and last-column rules) |
| formats (user `formats` over a table style) | in-sample, random area operations | 40/58 pivots, 6297/6922 cells (36/58, 6042 before the outline-edge and shared-edge rules) |
| formats-borders | in-sample | 26/45 pivots, 5569/6147 cells (11/45, 5040 before) |
| formats-areas (one fill per pivot: which cells an area selects, fills only) | in-sample | 64/66 pivots, 9849/9855 cells (56/66, 9484 before round 6) |
| holdout-formats | hold-out, fresh seed, written once | 26/38 pivots, 4736/5330 cells (22/38, 4052 before the outline-edge and shared-edge rules; confirmation only, failures not inspected) |
| holdout-customstyles | hold-out, fresh seed, written once | 32/48 pivots, 7530/7901 cells (28/48 before the last-column rule; confirmation only) |
| refresh regeneration (`pivot-refresh-*`) | in-sample | 28/28 pivots part for part (none, values, items, removed) |
| refresh with formats, groups, calculated items | in-sample | formats 10/10 (parts, cells, cell formats, display format), grouped 11/11, grouped-blank-date 2/2 (parts, cells, cell formats, display format), grouped-time and -fraction 24/24, calcitems 8/8 |

The open hold-out case is `rankDescending` with the base field outermost over three row
levels (the leaf ranks are not among the sibling items; the comparison set Excel uses is not
identified).

## Not done

* Groups: manual (discrete) groups and date parts with the 1904 date system are untested; hours with years or quarters, and three or more derived fields of one base, are untested (measured: hours with days, minutes or months); the base field on an axis of its own when its date group is dropped is not measured.
* Writing: the function wiring of GETPIVOTDATA in `open365-calc`; regeneration for shared caches (they fall back to `refreshOnLoad`).
* Styles: 10 hold-out pivots still differ (not inspected, to keep the set clean). Open in-sample:
  `styles2` P63 (tabular, values on rows, row stripes: Excel draws the medium subheading bottom
  border between every line of a group in the first label column; the same border is absent
  without stripes, rule not identified; a per-line "own block" model fixed P63 but broke 8 other
  pivots; not retried). User `formats` (pivot cell formats layered over the style: `formats/format/pivotArea`
  with `dxfId`) and conditional formats are not read and have no generator. Custom styles: 12 in-sample
  pivots still differ; seen: wholeTable borders on the second header row of compact tables with page fields
  (P15/P20/P39), subtotal-row borders in compact layouts with two data fields (P1), border colour/weight
  conflicts between subheading and subtotal elements of random styles (P22/P29).
* Refresh regeneration is done for single-cache pivots, including groups, calculated items and user
  formats (see above). Open: caches shared by several pivots, retained items of fields sorted by a data field.
* Calculated items: 2 hold-out pivots still differ (a Max item over one field, a Count item with a product formula; not tuned on). The Min data field is not in the generated set.
* Label and value filters, top-N, OLAP and data-model caches, `missingCaption`/`errorCaption`,
  `showAll` empty items.
* Rank with the base field outermost over three or more row levels (see above).
