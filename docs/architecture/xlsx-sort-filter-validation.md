# XLSX sort, AutoFilter and data validation

Status: implemented in `crates/xlsx` (`src/sheet/{sort,filter,validation}.rs`), measured against Excel 16.0 through
COM (`tools/office/make-sortfilter-refs.ps1`). Reference data: `fixtures/reference/xlsx/sortfilter-*/scenarios.json`;
fixtures: `fixtures/xlsx/sortfilter-{sort,filter,validation}.xlsx` (made up, no personal data).

## Model and round trip

The three elements stay raw XML in the worksheet (`autoFilter`, `sortState`, `dataValidations` after `sheetData`).
The typed models (`AutoFilter`, `SortState`, `DataValidations`) are read from, and written to, those elements in
schema position; unknown attributes (`xr:uid`) and `extLst` are kept. A sheet nobody edits is written back byte for
byte (ADR-001). Excel's `xlrd2` date groups and the `x14` data validations (cross-sheet lists) are read; the `x14`
list is read only.

## Sort

`Worksheet::sort_range` / `Workbook::sort_range(sheet, &SortSpec, &Locale)`. Value order: numbers, text, logical
(FALSE first), errors (all equal), blanks last in both directions; descending reverses the first four. Stable.
Text: locale collation (ICU through `open365-locale`) at accent strength, case level added for a case-sensitive
sort (lower case first); hyphen and apostrophe carry no weight; ASCII symbols use Windows' measured order; the
ligatures ae, oe, ss compare as pairs. Custom lists first (case-insensitive), colour keys (cell fill, font colour)
put matches first or last. Left-to-right sorts and `pinyin`/`stroke` methods are supported. Header: yes, no, or
Excel's guess (type or format differs between the first two lines; the Sort object of COM never guesses, Range.Sort
does). Cells move whole; moved formulas are shifted like a copy; cached results stay until the application
recalculates (`fullCalcOnLoad` is set on the first edit).

## AutoFilter

`AutoFilter::evaluate` returns the hidden rows; `Workbook::apply_auto_filter(sheet, today, &Locale)` sets
`hidden` on the rows ("Reapply"). Values (shown text, blanks, date groups), custom (numeric, text with collation,
wildcards, "not blank"), top/bottom items and percent, above/below average, all dynamic date filters (weeks start on
Sunday), cell and font colour filters.

## Data validation

`DataValidation::check(row, col, &Value, &ValidationContext)` and `Workbook::validate_cell`. Whole, decimal, list
(literal and range/formula), date, time, text length, custom; formulas evaluate with `open365-calc`, relative to the
top-left cell of the first `sqref` range, and see the candidate in the validated cell. Messages and error style are
in the model.

## Open

- Icon sort/filter keys are parsed and written but not applied.
- Conditional-format colours and table-style colours are not considered by colour keys and colour filters.
- Table (`tableN.xml`) auto filters and sort states are not exposed through `Table`.
- Sorting a range with merged cells or array formulas is refused (Excel allows equal-size merges).
- Sort of numbers that agree to 15 digits: Excel's result is not a total order; we compare exactly.
- Turkish dotless i ordering; non-ASCII symbols follow CLDR, not Windows.
- Non-English collation is ICU/CLDR; Excel was measured in en-GB only.
- Moved formulas keep stale cached values; no recalculation is done here.
- Creating the `dxf` a colour key needs in `styles.xml` is not implemented (the key carries the colour directly).
