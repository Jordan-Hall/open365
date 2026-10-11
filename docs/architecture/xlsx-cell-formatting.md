# XLSX cell formatting

Status: engine and commands implemented; measured against Excel by `tools/office/format-oracle.ps1`
(see "Oracle results"). Clean-room: the rules below were measured on generated workbooks, not copied.

## Model

- `Styles` (crates/xlsx/src/styles*.rs) edits the raw `styles.xml` tree. A formatting action derives a new
  `cellXfs/xf` from the cell's current one (`derive_xf`), interning the font, fill, border and number format.
- Worksheet side (`sheet/cellformat.rs`): a blank cell has the style of its row (`row/@s` with `customFormat`),
  else its column (`col/@style`), else 0. A new cell created by typing starts with that style.
- `Workbook` (`workbook/cellformat.rs`) is the public API. Every action returns a `FormatSnapshot` (worksheet
  and style sheet before the action) for undo. `crates/commands` wraps the actions as `FormatCells`,
  `SetBorders`, `ApplyCellStyle`, `ClearFormats`, `CopyFormat`, `ChangeDecimals`.

## Rules measured in Excel (en-GB, Office 2023 theme, Aptos Narrow)

- Apply flags: a group a change touches gets `applyFont`/`applyFill`/`applyBorder`/`applyAlignment`/
  `applyNumberFormat`/`applyProtection="1"` even when the value ends up equal to the default. An xf is reused
  only when identical including flags (bold then un-bold gives a new xf with `applyFont="1"`, not xf 0).
- Saving writes only what is used: unused xfs, fonts, fills, borders, custom number formats and unused
  built-in cell styles are dropped (`Styles::compact`). The live model keeps its numbering; compaction runs on copies in `Workbook::save`.
- Font: bold/italic/strike are `<b/>` etc., removed when off; `<u/>` single, `val` for the others; a font picked by
  name loses `scheme`; colour `auto` removes the `color` child. Child order is Excel's.
- Fill: solid is `fgColor` + `<bgColor indexed="64"/>`; patterns write only the colours given; no fill is fill 0.
- Border: edges are `<color indexed="64"/>` when automatic; diagonals share one `diagonal` line with
  `diagonalUp`/`diagonalDown` flags. Excel does not copy a border onto the neighbouring cell's file record.
  Inside lines set both sides of the shared edge in the range. Whole columns ignore top/bottom, whole rows left/right.
- Alignment: general removes the attribute (the xf keeps `applyAlignment`); an indent on anything but
  left/right/distributed turns the text left aligned; rotation -90..90 is stored 0..180 (255 stacked).
- Tint is stored as `trunc(tint * 32767) / 32767`, written with 17 significant digits.
- Custom number format codes are normalised (`\-`, `\ `, `\(`, `\)`, and `\.` in dates); a code equal to a
  built-in one uses the built-in id.
- Cell styles: a built-in style is added on first use (`cellStyleXfs` + `cellStyles`, kept sorted by name). The
  style's font is a separate record (searched from font 1), the cell xf points at the first identical font and gets
  `applyFont="1"` when that is not the style's own record. Applying a style drops every other format of the cell.
- Whole rows/columns: the row `s`/`customFormat` or `col style` is written, existing cells are restyled, and where
  a styled row crosses a restyled column Excel writes the intersection cell. New `col` entries get the default width.
- Increase/Decrease Decimal is computed from the top-left cell and applied to the whole range; a blank General
  cell is left alone; General uses the decimals shown in a default-width column.
- Percent, Comma and the accounting button apply the `Percent`, `Comma` and `Currency` styles.
- Format Painter tiles the source over the destination; Clear Formats sets style 0.

## Oracle

`tools/office/format-oracle.ps1` (own hidden Excel, aborts if COM attaches to a running instance,
`AutomationSecurity=3`, Quit in `finally`). Scenarios: `fixtures/xlsx/format-scenarios.json`; base workbook
`fixtures/xlsx/format-blank.xlsx`; references `fixtures/reference/xlsx/format-*/expected.json`;
built-in style definitions `fixtures/reference/xlsx/format-builtin-styles/styles.json` (generated into
`crates/xlsx/src/styles_builtin.rs`). Test: `crates/xlsx/tests/format_oracle.rs` (`FORMAT_ONLY=<name>` runs one).

The references are from a UK-locale Excel. Locale-dependent built-in ids (14 to 22, 37 to 40, 47) are left out of
the scenarios; `Currency` is recorded as numFmt 44 whatever the symbol.

## Open

See the report that accompanied this change; summarised in the test's allow-list.
