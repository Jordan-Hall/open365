# Formula engine conformance against Excel

`crates/calc` is checked against Microsoft Excel itself. Excel is the oracle; CI only reads committed files and
never needs Excel.

| File | Role |
|---|---|
| `fixtures/calc/conformance.json` | The corpus: 8,306 cases. Edited by hand or by script; one case per line. |
| `tools/office/calc-oracle.ps1` | Local-only tool. Runs every case in Excel over COM and writes the results. |
| `fixtures/reference/calc/conformance-results.json` | Excel's answers, committed. Includes Excel version, build and decimal separator. |
| `fixtures/calc/known-differences.toml` | Cases where Open365 intentionally or temporarily differs, each with a reason. |
| `crates/calc/tests/excel_conformance.rs` | Replays the corpus through `Engine` and compares with Excel. |

## Regenerate

```
pwsh -NoProfile -File tools/office/calc-oracle.ps1          # about 2 minutes, needs desktop Excel
cargo test -p open365-calc --test excel_conformance -- --nocapture
pwsh -NoProfile -File tools/office/calc-oracle.ps1 -Update   # only cases without a stored result (about 20 s per 1,000)
pwsh -NoProfile -File tools/office/calc-oracle.ps1 -Update -Only '^sin\.'   # rerun matching cases, keep the rest
```

The script starts its own hidden Excel, remembers that process id and stops only that process; it restarts the
instance if COM drops. `-Only <regex> -Output <file>` and `-Limit <n> -Output <file>` run a subset without
touching the committed results. Excel decides the answers, so results depend on the installed build and on
regional settings (decimal and thousands separators are recorded). Do not add cases that depend on locale
(currency symbols, `1/2` as a date, month names).

## Corpus format

```json
{"id": "sumif.basic", "formula": "=SUMIF(A1:A6,\"a\",C1:C6)",
 "cells": {"A1": "a", "C1": 10, "D1": true, "E1": {"err": "#N/A"}, "F1": {"f": "=\"\""}},
 "sheet2": {"A1": 5}, "at": "C2", "date1904": true}
```

* `id` is `<function or topic>.<name>`; the part before the first dot groups the summary table.
* `cells` fill `Sheet1`, `sheet2` fills `Sheet2` (always present). A value is a number, a string (always text,
  never parsed), a boolean, `{"err": "#N/A"}` or `{"f": "=formula"}`. Blank cells are left out.
* `formula` is en-US syntax and goes into the cell `at` (default `J20`, away from the data). `at` matters for
  implicit intersection.
* `date1904` evaluates the case in a workbook using the 1904 date system.
* Formulas are entered with `Range.Formula`, not `Formula2`: Excel evaluates them as a pre-dynamic-array formula
  (implicit intersection, no spilling). That is the semantics of an ordinary cell loaded from an XLSX file.
* Results have `kind` `number`, `text`, `bool`, `error` or `rejected` (Excel refused the formula text, for
  example a wrong argument count). Open365 passes a rejected case if its parser refuses the formula or the
  formula evaluates to an error value; a file loader must keep such a cell instead of failing.

## Comparison rules

Numbers are compared bit for bit, text exactly, errors by code. A case missing from `known-differences.toml`
that differs fails the test; a listed case that now passes also fails, so the list only shrinks.

A case with `"format"` also compares the displayed text: `numfmt::format_value(result, format, date1904)` must equal
`Range.Text` of the formula cell (a format code Excel refuses is recorded as `rejected` with hresult `NumberFormat` and
`validate_format` must refuse it too). The formula is a literal such as `=1234.5` or `="abc"`, so the group tests the
number format engine on its own.

## Locale of the oracle

Excel's output depends on the machine: the recorded header names the Windows culture (`en-GB` here), date order,
list separator and decimal/thousands separators. Format codes are en-US syntax and `NumberFormat` is
locale-independent, so results match en-US Excel, except for system-linked formats: `[$-F800]` (system long date),
`[$-F400]` and `g`/`gg`/`ggg` (era names, empty here) follow the machine locale. Open365 implements the en-US
forms; the corpus has no case for them.

## Current state

8,306 cases, 8,251 pass (99.34%), 55 known differences (the 50 below plus 5 fill-width cases of negative accounting formats) (14 older ones plus 36 in the new math/statistics groups: large COMBIN, trigonometric last digits after argument reduction, variance accumulation order). Before the fixes driven by this corpus 3,311 of 3,700
passed (89.49%). The remaining differences are listed with reasons in `known-differences.toml`: the `**`
parse quirk, XLOOKUP array results (no spill), a few undocumented Excel quirks (`DATE` with huge day numbers,
CONCAT over the cell limit returning an unnamed error code, a 17-digit literal) and two formulas Excel
refuses (`IFERROR` as a range operand).

## Behaviour measured against Excel

Findings that the corpus now locks in (see the module comments in `crates/calc/src` for details):

* Number to text: 15 significant digits, positional up to 20 characters (`1E+15` is `1000000000000000`,
  `1E-10` is `0.0000000001`), scientific beyond (`1E+20`, `1.23456789012345E-05`).
* Number literals keep only 15 significant digits (cut, not rounded) and `1E308` or more is rejected.
* `=` and `<` equate values that agree to 15 significant digits; `MATCH`/`VLOOKUP` exact matching and
  `COUNTIF` differ (lookups are bit exact, `COUNTIF` uses 15 digits).
* A result within binary noise of zero becomes 0 only for the outermost `+`/`-` of a formula and for the last
  addition of `SUM`: `=0.1+0.2-0.3` is 0, `=(0.1+0.2-0.3)` and `=ABS(0.1+0.2-0.3)` are not.
* Operators and scalar parameters apply implicit intersection to ranges; only the array parameters of `INDEX`
  and `XLOOKUP` evaluate over whole ranges.
* Ascending approximate lookups are Excel's binary search (unsorted data gives Excel's answer); `MATCH` type -1
  scans forward and returns an exact hit first.
* Lookups return references: a blank target is blank (`ISBLANK`, `&""` give TRUE and `""`), `INDEX` returns a
  reference usable in `A1:INDEX(...)`.
* Text comparison is a linguistic sort (accents break ties, hyphen and apostrophe are ignored first, symbols sort
  before digits before letters), not code point order.
* Text positions count Unicode scalar values (`LEN("😀")` is 1) but wildcards see UTF-16 units (`"?"` does not
  match an emoji).
* `VALUE` and arithmetic coercion accept ISO dates, times, `(12)`, `- 1`, `% 1` and misplaced thousands commas.
* `ROUND` cuts the shortest decimal form at 15 digits (`ROUND(0.49999999999999994,0)` is 0); `ROUNDUP` and
  `ROUNDDOWN` round it first (`ROUNDDOWN(0.7+0.1,1)` is 0.8); `INT` snaps to 15 digits (`INT(0.57*100)` is 57).

Added with the second function batch (math and statistics):

* `SIN`, `COS` and `TAN` refuse `|x| >= 2^27` and reduce the argument with a pi of 64 mantissa bits
  (`0xC90FDAA22168C235 * 2^-62`): `SIN(PI())` is `1.22514845490862E-16`, `COS(PI()/2)` is `6.1257422745431E-17`.
* `ACOS(x)` is `PI()/2 - ASIN(x)`; `COSH` is `(e^x + e^-x) / 2` and `SINH`/`COSH` return `#NUM!` when `e^|x|` overflows
  (`SINH(710)`); `DEGREES` is `x * (180/PI)`, `RADIANS` is `x * (PI/180)`; `LOG(x, base)` is `ln(x)/ln(base)`.
* `FACT` multiplies from the top down, `PERMUT` from the bottom up. `FACT(-0.5)` is `#NUM!`.
* The Analysis ToolPak functions (`GCD`, `LCM`, `QUOTIENT`) reject booleans with `#VALUE!`, `GCD(,4)` is `#N/A`.
* `RANDBETWEEN` rounds both limits up. `SMALL` floors `k`; `LARGE` floors `n + 1 - k`.
* `MODE` considers only numbers, even among direct arguments. `PERCENTILE.INC` forms the 1-based position first.
* `SUBTOTAL`/`AGGREGATE` skip any cell whose formula contains `SUBTOTAL` or `AGGREGATE` anywhere (`AGGREGATE` options 4-7
  keep them). `AGGREGATE` with function 1-13 refuses direct numbers and array constants.
* `SUMPRODUCT` evaluates all arguments as arrays, and element-wise functions (`ISNUMBER`, `ABS`, ...) are lifted over
  arrays inside array parameters.

Added with the number format engine (`crates/calc/src/numfmt`, 3,271 cases in group `numfmt`):

* `Range.Text` of a cell in the oracle's 60-digit column. General shows at most 11 characters (sign excluded),
  rounded half up on the 15-digit value: `0.333333333`, `123456789012` is `1.23457E+11`. Values below 0.0001 stay
  positional only when they need no rounding (`0.000012345`; `0.0000123456` is `1.23456E-05`).
* Digits are rounded half away from zero on the 15-significant-digit decimal (`2.675` with `0.00` is `2.68`). A negative
  value that rounds to zero shows no minus sign (`-0.001` with `0` is `0`).
* Sections: a section with an explicit or implied condition that only matches negative numbers drops the minus
  sign; the third section of `[=1]..;[=2]..;..` keeps it. With conditions and no matching section the cell shows `####`.
  A last section containing only `@` is the text section; `@` alone formats numbers as General.
* Fractions follow a continued-fraction walk that stops at the denominator limit (`# ?/?` shows 0.1 as `0`, not
  `1/9`); a zero numerator blanks the fraction with spaces (`5` is `5    `).
* Scientific: the exponent is a multiple of the number of integer placeholders (`##0.0E+0`, `12345.6789` is `12.3E+3`).
* Dates: the time is rounded to the nearest second (or to the `.0` digits) before it is split, `0.999999999` with
  `h:mm` is `0:00`. The 1900 system shows `####` for negative values and above 9999-12-31; the 1904 system shows a
  minus sign followed by the magnitude. `[m]`/`[s]` counters ignore the date limit. `m` is a minute after `h`, after
  `s` or before `s`, otherwise a month. `AM/PM` is always upper case, `a/p` keeps its case.
* `_x` is one space whatever `x` is. `*x` fill repeats `x` until the text fills 540.5 glyph units; widths of
  Excel's default font (a digit is 9, a space 3) are in `numfmt::glyph_units`. Fill is emulation: it matches the
  oracle's column, not a user-resized one.
* Excel accepts most unquoted ASCII letters as literals; it refuses `b d e g h m n s y` (format letters) in numeric
  sections, `E` without a sign, `0:0`, `0/` and `/0`, lowercase `e+`.
