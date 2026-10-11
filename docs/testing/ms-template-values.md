# Microsoft template corpus: value and text fidelity

`crates/testkit/tests/ms_template_values.rs` recalculates every formula of the 62 local `.xltx` templates
(`open365-xlsx` + `open365-calc`) and compares each cell with the Excel reference
(`fixtures/reference/ms-templates/<id>/values.json`, see `ms-template-corpus.md`) on two axes:

- **values**: Value2 (numbers within 1e-12 relative, text, booleans, errors by code);
- **text**: the displayed text, our number formatting applied to our value with the cell's number format.

The corpus is local and gitignored; the test skips when it is absent. Per-template counts (no content) are the
floors in `fixtures/reference/ms-templates-values-floor.toml`; a template may not match fewer cells than its
floor. Refresh after an improvement: `OPEN365_UPDATE_FLOORS=1 cargo test -p open365-testkit --test ms_template_values`.
`OPEN365_MISMATCH_OUT=<file>` writes one line per mismatch for local analysis.

Left out of the totals: volatile formulas (TODAY, NOW, RAND, RANDBETWEEN: 139 cells, counted as `volatile`) and, for
text only, width-dependent display (`###`, `*` fill padding: 113 cells, `narrow`). References were taken on a Windows
en-GB machine, so the test uses LCID 0x809 for built-in formats: short date `dd/mm/yyyy`, ids 20 and 21 as
`hh:mm` and `hh:mm:ss`, ids 37 to 40 with a minus sign instead of parentheses.

## Result (2026-10-11)

| | matched / total |
|---|---|
| values, all cells | 12,582 / 16,038 (78.5 %) |
| text, all cells | 12,392 / 15,925 (77.8 %) |
| values, formula cells only | 6,067 / 9,523 |
| text, formula cells only | 5,929 / 9,523 |

The engine is seeded from the file's cached values and then recalculates every formula, so constants always match; the
formula-only rows are the real measure.

## Root causes of the 3,456 value mismatches (ranked)

Cells that depend on a failing cell carry the root cause of that cell.

| # | Cells | Cause | Class | Owner |
|---|---|---|---|---|
| 1 | 2,189 | Defined names with relative references (`Sheet!$B1`, `Calculations!$I1048576`: resolved against the formula cell) | something else (names) | calc, commands |
| 2 | 948 | Legacy array formulas (`t="array" ref=...`): the non-anchor cells have empty `<f/>`; no array evaluation or spill | something else (arrays) | calc, commands |
| 3 | 86 | Structured table references (`Table[Col]`, `Table[[#Totals],[Col]]`, `[#This Row]`) in cells and names | something else (tables) | xlsx (no table model), calc, commands |
| 4 | 95 | Sheet-scoped names (`localSheetId`): `build_engine` skips them | something else (names) | commands |
| 5 | 183 | Remaining, see below | function difference | calc |

The 183 remaining:

- text to date coercion: `MONTH("March1")`, `VALUE("January 1")`, `EDATE("January 2022",-1)` fail (about 150 cells with dependents);
  `DATEVALUE("October 1, 2026")` is `#VALUE!` in en-GB Excel but a date here (about 14 cells).
- `TEXT(date,"aaa")` returns `#VALUE!` (Excel: abbreviated weekday): 12 cells.
- `INDEX(...)` returning a reference inside the range operator, in a name (`Calculations!$AC$15:INDEX(...,n)`): 8 cells.
- a range in a scalar cell (`='PLANNED EXPENSES'!B2:D3`) needs implicit intersection or top-left: 3 cells.

No value mismatch came from a missing function (no `#NAME?` for an unknown function name) or a date system (no 1904 workbook failed).

## Text-only mismatches (value matched, 207 cells)

| Cells | Cause | Class |
|---|---|---|
| 88 | Excel text is empty for a formula with a non-empty value: a conditional-format number format (such as `;;;`) | number format (conditional formatting) |
| 12 | `mmmm\ \'yy` shows `April` in Excel: a conditional-format number format overrides it | number format (conditional formatting) |
| rest | followed from the locale rules above; none remain unexplained after them | locale |

Locale was the largest text cause before the oracle locale was applied (about 560 cells: short date, `h:mm`, ids 37 to 40,
trailing `_)` padding). The calc crate's `FormatLocale::builtin_format_code` follows the locale only for ids 14 and 22; the
test overrides 20, 21 and 37 to 40 itself.

## Open items for the calc and commands owners

See the report that accompanied this file; each entry names the function or format, an example formula and the expected result.
