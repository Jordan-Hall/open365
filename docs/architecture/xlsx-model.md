# XLSX workbook model: tables, names, arrays, shared formulas

Read APIs in `crates/xlsx` for the calc engine. All are read-only views; the parts they
come from are unchanged on save (ADR-001, round trip checked in `tests/model.rs`).

## API

- `Workbook::tables() -> impl Iterator<Item = (sheet, &Table)>`, `Workbook::table_by_name(name)`,
  `Worksheet::tables()`, `Worksheet::table_at(row, col)`. Tables come from the `table`
  relationships of each sheet part. `Table`: `name`, `display_name`, `range_text`/`rect`
  (header and totals rows included), `header_row_count` (default 1), `totals_row_count`
  (default 0), `columns: Vec<TableColumn>`, `style: Option<TableStyle>`, `data_rect()`,
  `data_row_count()`, `column(name)`. `TableColumn`: `id`, `name`, `totals_row_function`,
  `totals_row_label`, `totals_row_formula`, `calculated_column_formula`,
  `calculated_column_array`. An unreadable table part is skipped.
- `Workbook::defined_names()`, `sheet_defined_names(sheet)`, `resolve_defined_name(sheet, name)`
  (sheet scope first, then workbook scope). `DefinedName`: `name`, `formula`,
  `local_sheet_id` (index into `sheets()`), `hidden`, `comment`, `is_sheet_scoped()`.
- Legacy arrays: `Worksheet::array_formulas()` (`ArrayFormula { master, range, text, dynamic }`),
  `array_formula_at(row, col)`, `array_role(row, col) -> Option<ArrayRole>`
  (`Master` or `Member { master }`). `formula_text` is now `None` for a member that stores an
  empty `<f/>` (it used to be `Some("")`). Excel itself stores no `<f>` for members.
- Shared formulas: `Worksheet::shared_formula(row, col) -> Option<SharedFormulaRef>` with
  `si`, `master`, `master_text`, `master_range`, `is_master`, `row_offset`, `col_offset`.
  `formula_text` already expanded dependents by shifting the master.
- `MAX_ROW_HEIGHT` is 409.5 (Excel's limit); `SetRowHeight` accepts 0..=409.5.

Fixtures: `fixtures/xlsx/model-tables.xlsx`, `model-arrays.xlsx`, made with
`tools/office/make-xlsx-fixtures-model.ps1` (Excel COM); references in
`fixtures/reference/xlsx/model-*`.

## What `build_engine` passes today

Workbook-scoped names, every formula cell with text (array masters as ordinary scalar
formulas), cached values for array members. Sheet-scoped names are not passed: the engine
has only `define_name(name, formula)` (global).

## What calc needs (not done here)

1. Sheet-scoped names: `Engine::define_name_scoped(sheet: &str, name, formula)`, lookup order
   sheet then workbook. Feed from `sheet_defined_names(i)`.
   Relative references in names must resolve against the formula cell (report rank 1,
   e.g. `Sheet!$B1`): `define_name` needs the name's anchor `(sheet, row 1, col 1)` as in
   Excel, and evaluation must offset by the calling cell.
2. Legacy arrays: `Engine::set_array_formula(sheet, range: (r1,c1,r2,c2), formula)`; evaluate
   once as an array, write each element into its cell, treat member cells as formula-owned
   (no per-cell formula). Feed from `array_formulas()`; example `D4:F4 = A1:C1*2` and
   `A6:A8 = TRANSPOSE(A1:C1)` in `model-arrays`. Single-cell `B10:B10` `SUM(A1:C1*A2:C2)`
   needs array evaluation of the operands too.
3. Structured references: `Engine::define_table(sheet, TableDef { name, rect, header_rows,
   totals_rows, columns })` from `tables()`, resolving `Tbl[Col]`, `Tbl[[#This Row],[Col]]`
   (`[@Col]`), `Tbl[[#Totals],[Col]]`, `Tbl[#Headers]`, `Tbl[#Data]`. Example in
   `model-tables`: `SUBTOTAL(109,SalesTbl[Qty])` at B7; calculated column
   `SalesTbl[[#This Row],[Qty]]*SalesTbl[[#This Row],[Price]]`. Excel reports the short form
   (`[@Qty]`), the file stores the long form.

## Gaps

No table or name edits (read-only); tables are not shifted by row/column edits; table
`autoFilter`, `sortState` and column `dxf` ids are not exposed; dynamic-array spill
(`cm` metadata) is only flagged via `ArrayFormula::dynamic`.
