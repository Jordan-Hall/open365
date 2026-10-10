# Formula engine conformance against Excel

`crates/calc` is checked against Microsoft Excel itself. Excel is the oracle; CI only reads committed files and
never needs Excel.

| File | Role |
|---|---|
| `fixtures/calc/conformance.json` | The corpus: 24,696 cases. Edited by hand or by script; one case per line. |
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

24,696 cases, 24,638 pass, 58 known differences (listed with reasons in `known-differences.toml`: the `**` parse
quirk, XLOOKUP array results (no spill), a few undocumented Excel quirks, large COMBIN, trigonometric last digits, variance
accumulation order, accounting fill, and two `IFERROR` over arrays of references). The test also fails for a corpus case without an Excel
result or a stored result without a case.

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

Added with the text batch (TEXT, FIXED, DOLLAR, CHAR, CODE, UNICHAR, UNICODE, EXACT, PROPER, CLEAN, T, N, NUMBERVALUE,
TEXTBEFORE, TEXTAFTER, TEXTSPLIT, VALUETOTEXT, ARRAYTOTEXT, REPLACE and the B variants; about 1,700 cases):

* `TEXT` is the cell display without the column: no `*x` fill, no `####` (a negative date or an unmatched condition is
  `#VALUE!`), result longer than 255 UTF-16 units is `#VALUE!`. Numeric text is converted first, blank is 0.
  `TEXT` accepts a few format codes `Range.NumberFormat` style parsing does not (known differences `text.fmt_*`).
* `FIXED`/`DOLLAR` round half away on the 15-digit value, decimals above 127 are `#VALUE!`, negative decimals round
  left; `FIXED(x,,TRUE)` means 2 decimals but `DOLLAR(x,)` means 0. The oracle is en-GB: `DOLLAR` prints `£` and `-£1.00`.
  The corpus normalises the symbol with `SUBSTITUTE` and leaves negatives out; Open365 implements the documented en-US form `($1,200)`.
* `CHAR`/`CODE` use Windows-1252 (`CODE` gives 63 for other characters); `UNICHAR` gives `#N/A` for surrogates and
  noncharacters (`U+FDD0..FDEF`, `U+xFFFE/xFFFF`). `CLEAN` removes `U+0000..001F` and `U+0080..009F`.
* `PROPER` works on BMP letters only; its first letter is `UPPER(LOWER(c))`. `LOWER` turns a capital sigma into `ς`
  unless a letter or digit follows, and keeps the Kelvin, Ohm and Angstrom signs.
* `TEXTBEFORE`/`TEXTAFTER`/`TEXTSPLIT`: non-overlapping matches, longest delimiter wins, case-insensitive mode lower-cases
  text and every delimiter separately (asymmetric for sigma). `|instance| > LEN` is `#VALUE!`, within it and absent `#N/A`.
* `NUMBERVALUE` ignores space, tab, CR and LF (not NBSP), allows group separators only before the decimal one, divides by 100 per `%`,
  and falls back to date and time text. Currency symbols and day-first dates are locale-dependent and not in the corpus.
* `VALUETOTEXT` never returns an error (errors and non-intersecting ranges are shown as text); `ARRAYTOTEXT(blank cell)` is `#VALUE!`.
* Byte functions (no double-byte language): `LENB`, `MIDB`, `FINDB`, `SEARCHB`, `REPLACEB` count UTF-16 units (`MIDB` can split
  a surrogate pair, shown as U+FFFD); `LEFTB`/`RIGHTB` equal `LEFT`/`RIGHT`.

Added with the date and time batch (TIME, HOUR, MINUTE, SECOND, WEEKDAY, WEEKNUM, ISOWEEKNUM, EDATE, EOMONTH, DATEDIF, DAYS,
DAYS360, NETWORKDAYS(.INTL), WORKDAY(.INTL), YEARFRAC, DATEVALUE, TIMEVALUE; about 8,700 cases):

* Serial 1 of the 1900 system is a Sunday; the weekday is `(serial - 1) mod 7`. The time of day is rounded to the nearest
  second before the date part is taken (`DAY(44999.999995)` is 15; `YEAR`, `WEEKDAY`, `WEEKNUM`, `ISOWEEKNUM`, `DAYS360`,
  `YEARFRAC` do it, `EDATE`, `EOMONTH`, `DAYS`, `DATEDIF`, `NETWORKDAYS` floor instead). Negative serials are `#NUM!`.
* `TIME` truncates each argument (limit 32767), `#NUM!` for a negative total, and wraps at 24 hours. `HOUR`/`MINUTE`/`SECOND` round to the second.
* Analysis ToolPak style functions (`EDATE`, `EOMONTH`, `WEEKNUM`, `NETWORKDAYS(.INTL)`, `WORKDAY(.INTL)`, `YEARFRAC`): an omitted
  argument is `#N/A`, a boolean or multi-cell range is `#VALUE!` (no implicit intersection). Holiday cells: blank skipped,
  boolean `#VALUE!`, negative `#NUM!`. `WORKDAY` rounds the day count down (`-0.9` is one day back).
* `EDATE`/`EOMONTH` results below 1900-01-01 are `#NUM!`; in the 1904 system serials down to 1900-01-01 (-1461) are valid.
* `DATEDIF`: `MD` adds the length of the month before the end month when the end day is smaller (negative results happen;
  February 1900 has 29 days); `YD` is the days from the start through `YM` months counted inside the start's own year, plus `MD`
  (this reproduces Excel's leap-day oddities, for example 2001-02-28 to 2004-06-15 is 107).
* `DAYS360` US method: only the start date's end of February becomes day 30 (end of February to the same date is -1);
  `YEARFRAC` basis 0 follows NASD with the real calendar (1900 is not leap there) and tests day 31 against the original start day.
* Weekend parameter: a code 1-7 or 11-17, or a 7 character 0/1 string from Monday; other text is `#VALUE!`; `"1111111"` is allowed
  (`NETWORKDAYS.INTL` gives 0, `WORKDAY.INTL` with 0 days returns the start). An error value there becomes `#VALUE!`
  (`NETWORKDAYS.INTL`) or `#NUM!` (`WORKDAY.INTL`).
* Date and time text (`crates/calc/src/datetext.rs`): year-first dates (`2024-03-15`, `2024/3/5`, spaces around separators), day-first
  with an English month name (`15 Mar 2024`, `15-Mar-24`, two-digit years 0-29 are 20xx), times `h:mm`, `h:mm:ss.fff` (rounded to the
  millisecond), `mm:ss.f`, `h AM`, `h:mm PM` (also `A`/`P`; a space is required), hours up to 9999 and at most one of
  hour >= 24, minute >= 60, second >= 60. A date and a time may come in either order. `DATEVALUE`/`TIMEVALUE` reject plain numbers;
  arithmetic does not accept a date or time with leading spaces, `VALUE` does. Not supported (locale dependent or time dependent):
  `1/2/2024`, month-first names, dates without a year.

Not done yet: array lifting of scalar functions over computed arrays in plain formulas (known difference `textsplit.sumv`), the
info/logic/lookup batch (ISEVEN, ISODD, ISLOGICAL, ISNONTEXT, ISERR, ISREF, ISFORMULA, TYPE, ERROR.TYPE, IFS, SWITCH, ROWS, COLUMNS, ROW,
COLUMN, LOOKUP, ADDRESS, OFFSET, INDIRECT and volatile-dependency tests) and the financial batch (PMT ... SYD), and the five accounting-fill differences.

Added with array lifting and the info/logic/reference batch (groups `lift`, `iseven`, `isodd`, `islogical`, `isnontext`,
`iserr`, `isref`, `isformula`, `type`, `errortype`, `ifs`, `switch`, `rows`, `columns`, `row`, `column`, `lookup`,
`address`, `offset`, `indirect`):

* A scalar function given a computed array or array constant runs once per element in any formula; ranges are expanded
  only inside array parameters (`SUMPRODUCT`, `INDEX`, `ROWS`, `LOOKUP` vectors) and elsewhere take the implicit
  intersection. `1xN` and `Nx1` stretch, other size mismatches give `#N/A`. Once an array makes a call lift, ranges
  among its other scalar arguments are read as arrays (`IF({TRUE,FALSE},A1:A2,B1:B2)`).
* `IF`, `IFS`, `CHOOSE`, `SWITCH` lift only when the condition/selector is an array (a scalar condition returns the chosen
  value whole). `IFERROR`/`IFNA` lift only in array contexts. `VLOOKUP`/`HLOOKUP`/`XLOOKUP`/`INDEX` never lift.
  `IF`, `IFS`, `SWITCH`, `IFERROR`, `ISEVEN` do not pass the array context to ranges (`SUMPRODUCT(IF(A1:A3>0,..))` is `#VALUE!`).
* `ROW`/`COLUMN` of a multi-cell reference are arrays only in array contexts. `ROW(A1)` in A1 is no cycle.
  `ROWS`/`COLUMNS`/`ROW`/`ISREF` of a reference to a missing sheet still see a reference.
* `ISEVEN`/`ISODD` refuse booleans, empty text and multi-cell ranges. `TYPE` of an array is 64, of a multi-cell range outside
  the formula's row/column 16. `ISFORMULA` reads the top-left cell of a range.
* `LOOKUP` result vectors are not clipped to their size and a single cell counts as a row; arrays search by row when wider.
* `ADDRESS` quotes sheet names that look like cells, R1C1 references, booleans or contain symbols; relative R1C1 parts
  cannot be 16384 columns. `INDIRECT` rejects leading spaces, operators and several `!`; relative R1C1 wraps at the sheet edge.
* `OFFSET`/`INDIRECT` are re-evaluated (with everything downstream) until values stop changing, so chains settle
  after `set_value` + `recalc_dirty`. `OFFSET`'s base reference is not a dependency.

Added with the error-literal batch: `ErrorKind` now has all 14 `ERROR.TYPE` kinds (`#GETTING_DATA` 8, `#SPILL!` 9, `#CONNECT!` 10,
`#BLOCKED!` 11, `#UNKNOWN!` 12, `#FIELD!` 13, `#CALC!` 14). Only the seven classic errors and `#GETTING_DATA` parse as formula
literals; Excel refuses the others (`ErrorKind::is_formula_literal`). The oracle names unknown error codes `#UNKNOWN(code)`; the test
reads the name from `Range.Text`. `ISFORMULA(A1)` watches A1 without depending on its value, so `ISFORMULA` of its own cell is TRUE.

Financial batch, verified (groups `pv`, `fv`, `sln`, `effect`, `nominal`, `dollarde`, `dollarfr`; 1,768 cases, all bit-exact):

* `(1+r)^n` for integer `n` is right-to-left binary exponentiation, for fractional `n` it is `exp(n*ln x)` and for `n = 0.5` it is
  `sqrt`. `PV` is `-(fv + pmt*a) / q` and `FV` is `-(pv*q + pmt*a)` with `q = (1+r)^n`, `a = (q-1)/r`, times `1+r` for type != 0.
  `rate = -1` is `#DIV/0!` when `q` is 0 and `#NUM!` for `0^0`; the rate 0 case is the linear sum.
* `EFFECT`/`NOMINAL`/`DOLLARDE`/`DOLLARFR` are Analysis ToolPak style (boolean and multi-cell range arguments are `#VALUE!`); `n` and the
  fraction are truncated. `SYD` rejects a negative salvage, not a negative cost.

Implemented but NOT verified (no corpus cases; generators `gen_fin1.py`..`gen_fin3.py` kept outside the repository): `PMT`, `NPER`, `RATE`,
`IPMT`, `PPMT`, `CUMIPMT`, `CUMPRINC`, `NPV`, `XNPV`, `IRR`, `XIRR`, `MIRR`, `SYD`, `DB`, `DDB`, `VDB`. Measured so far:

* `PMT` matches the closed form `-r*(pv + (pv+fv)/expm1(n*log1p(r)))/(1+r*type)` only in about half of the cases; Excel's fv-only results
  carry an error that grows with `(1+r)^n` (80 ulp at q = 140), so its algorithm is not a plain double evaluation. `NPER` is
  `ln(num/den)/ln(1+r)` (93% exact); the rest differ because Excel's `ln(1.05)` is one ulp above the C library's.
* `RATE`/`IRR`/`XIRR` are Newton iterations. Plain Newton (guess 0.1, 20 iterations, step < 1e-7) reproduces about 40% of `RATE` results
  bit for bit; Excel's final iterate is noise-limited, so its exact stopping rule is unknown. Excel's `XIRR` result is the true root
  (`XIRR({-100,150},{43831,44562})` = 0.2244052527 in exact arithmetic, Excel 0.22440524697), so its tolerance is looser than 1e-8.
  `XNPV` needs `rate > 0`. `CUMIPMT` takes `ceil(start)` and `trunc(end)`. `DDB` is `cost*(1-f/life)^(period-1)*f/life`, capped at salvage.

## Locale (ADR-011), arrays of references, long text

* `"locale": "en-GB"` marks a case that depends on the oracle machine's system locale (date order, currency symbol, date
  text); Open365 evaluates it with that locale and the oracle script refuses to run it on another culture. Cases without
  it must be locale-independent. `Engine::set_locale` / `EvalContext::with_locale` take an `open365_locale::Locale`.
* Measured: a `[$-LCID]` prefix changes month, day and AM/PM names only (group `loc`, 40 LCIDs); separators stay.
  en-GB date text: `1/2/2024` is 1 Feb, `15.03.2024`, month-first names and `$1` are refused, `Mar 15` and `3/15` are
  March 2015, two-digit years below 30 are 20xx (group `locgb`, `datevalue.gb_*`).
* Not verified: built-in format ids 14/22/currency in other locales (Excel keeps the en-US code it is given over COM, so the
  oracle cannot show them; derived from CLDR), `[$-F800]`/`[$-F400]` and eras (`ggg`) per locale, `DOLLAR` negatives.
  Locale reaches value coercion through a thread-local scope set by `evaluate_root`.
* Arrays of references (`Arg::Refs`; group `refarr`, 1,989 cases): `OFFSET`/`INDIRECT` with an array argument return one
  reference per element when evaluated in array mode (inside `SUMPRODUCT`, `N`, or two function calls deep). Per-function
  behaviour is in `functions::refs_mode`.
* `CONCAT`/`TEXTJOIN` past 32,767 characters give `#CALC!`.
