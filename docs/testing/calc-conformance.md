# Formula engine conformance against Excel

`crates/calc` is checked against Microsoft Excel itself. Excel is the oracle; CI only reads committed files and
never needs Excel.

| File | Role |
|---|---|
| `fixtures/calc/conformance.json` | The corpus: 3,690 cases. Edited by hand or by script; one case per line. |
| `tools/office/calc-oracle.ps1` | Local-only tool. Runs every case in Excel over COM and writes the results. |
| `fixtures/reference/calc/conformance-results.json` | Excel's answers, committed. Includes Excel version, build and decimal separator. |
| `fixtures/calc/known-differences.toml` | Cases where Open365 intentionally or temporarily differs, each with a reason. |
| `crates/calc/tests/excel_conformance.rs` | Replays the corpus through `Engine` and compares with Excel. |

## Regenerate

```
pwsh -NoProfile -File tools/office/calc-oracle.ps1          # about 2 minutes, needs desktop Excel
cargo test -p open365-calc --test excel_conformance -- --nocapture
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

## Current state

3,690 cases, 3,676 pass (99.62%), 14 known differences. Before the fixes driven by this corpus 3,311 of 3,700
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
