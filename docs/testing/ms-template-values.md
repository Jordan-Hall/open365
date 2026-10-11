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

## Result (2026-10-11, after names, arrays and tables)

| | matched / total |
|---|---|
| values, all cells | 15,090 / 16,038 (94.1 %) |
| text, all cells | 14,854 / 15,925 (93.3 %) |
| values, formula cells only | 8,575 / 9,523 |
| text, formula cells only | 8,391 / 9,523 |

Before: 12,582 values (6,067 formula cells). The engine is seeded from the file's cached values and then recalculates
every formula, so constants always match; the formula-only rows are the real measure. The test applies the oracle
locale (en-GB) to the engine too (`FormulaEngine::set_format_locale`), because date text such as `JAN 1` reads by locale.

## Root causes of the 3,456 value mismatches (all fixed except where noted)

| # | Cells | Cause | Status |
|---|---|---|---|
| 1 | 2,189 | Relative references in defined names | fixed: names move with the formula cell (`Engine::define_name`) |
| 2 | 948 | Legacy array formulas | fixed: `Engine::set_array_formula` (array mode, members show elements) |
| 3 | 86 | Structured table references | fixed: `Engine::define_table`, `Ast::Structured` |
| 4 | 95 | Sheet-scoped names | fixed: `Engine::define_name_scoped` |
| 5 | 3 | 2D range in a scalar cell | fixed: intersection by row and column |
| 6 | 8 | `A1:INDEX(...)` in a name | fixed (inside an array formula, needs array-mode IF) |

Left (formula cells, 948 values in all): 864 constant cells in two calendar templates whose references were taken on another
date (TODAY-dependent cached values, not an engine fault), 84 cells `TEXT(date,"aaa")` (`#VALUE!`; Excel: abbreviated
weekday), and the text-to-date items below. `DATEVALUE("October 1, 2026")` is `#VALUE!` in en-GB Excel but a date here.

The numbered table above describes the state before this work; the "Left" list is current.

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
