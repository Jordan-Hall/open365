# XLSX built-in number formats by locale

A cell format that names a built-in `numFmtId` has no `numFmt` element. The code is implied, and Excel resolves it from the
Windows locale of the machine that opens the file. `crates/xlsx/src/builtin_numfmt.rs` does the same for any LCID:
`builtin_number_format_in(id, lcid)`. `Styles::set_number_format_locale(Some(lcid))` makes `Styles::format_code` and
`number_format` use it; `None` (the default) keeps the specification's codes (`builtin_number_format`).

Evidence is kept in three classes. Do not read a "derived" row as an Excel measurement.

## Measured (Excel 16.0, build 20430, Windows locale en-GB)

`tools/office/make-numfmt-locale-refs.ps1 -Measure` writes a workbook whose cell formats reference `numFmtId` 0 to 100 with no
`numFmts` part, opens it read-only in Excel's own new instance and records `Range.NumberFormat`, `NumberFormatLocal` and the
`Range.Text` of four probe values per id, plus `Application.International` flags. Result:
`fixtures/reference/xlsx/numfmt-locale/measured-en-GB.json`.

The script aborts if COM attached to an Excel process that was already running, sets `AutomationSecurity = 3`, quits in `finally`
and only stops a process it started. It never changes Windows regional or Office settings, so only the current locale can be
measured. Findings (all ids 0 to 100 match, `numfmt_locale_oracle.rs`):

- 14 is the system short date (`dd/mm/yyyy`); 15 and 16 take a two-digit day (`dd-mmm-yy`, `dd-mmm`); 20 to 22 take a two-digit hour.
  Both follow `International` day and time leading-zero flags. 18 and 19 stay `h:mm AM/PM`.
- 5 to 8 and 37 to 44 follow the currency negative pattern. en-GB (pattern `-$n`): `£#,##0;-£#,##0`, `#,##0;-#,##0`,
  `_-* #,##0_-;-* #,##0_-;_-* "-"_-;_-@_-`. The en-US codes (parentheses) are the specification's.
- 27 to 31, 36, 50 to 58, 71 and 72 are the short date; 32 to 35 are the time with seconds; 59 to 81 repeat 1 to 22 and 45 to 47.
- 23 to 26 and 82 to 100 are General. 47 is `mm:ss.0` (the specification's `mmss.0` is a typo).
- The codes render the probe values as Excel did (`format_value_in` against `Range.Text`, over 300 comparisons).

## Public specification

ECMA-376 Part 1 18.8.30, as republished by Microsoft (Open XML SDK reference for `NumberingFormat`, 2024): the tables for
zh-TW, zh-CN, ja-JP, ko-KR (ids 27 to 36, 50 to 58) and th-TH (59 to 62, 67 to 81). Used verbatim; the test checks them against the
specification's own Unicode listing. zh-HK and zh-MO use the zh-TW table, zh-SG and zh-Hans the zh-CN table (derived assignment).

## Derived

The measured rules applied to the Windows locale data (NLS, `CultureInfo(lcid, useUserOverride = false)`, Windows PowerShell 5.1):
short date, short and long time, currency symbol, positive and negative currency pattern, for 417 LCIDs
(`windows-nls.json`, generated `builtin_numfmt_nls.rs`; regenerate with `make-numfmt-locale-refs.ps1 -Nls`). Every LCID of
`open365-locale`'s table resolves: unknown sub-locales fall back to the language's default, then en-US.

Derivation choices, none verified against Excel outside en-GB:

- Day and hour widths come from `dd` in the short date and `hh`/`HH` in the short time.
- Currency symbols: `$` quoted, a lone non-ASCII symbol bare (the measured `£`), others quoted; spaces escaped (`\ `).
- Accounting (41 to 44) places the symbol by the positive pattern; the padding is `_(`/`_)` for parenthesis locales, `_-` otherwise.
- 32 to 35 outside the East Asian tables use the system long time (en-US gives `h:mm:ss AM/PM`; the other candidate is `h:mm:ss`).
- The short date is taken literally (ja-JP `yyyy/mm/dd`). Excel may show `yyyy/m/d`.

## Locales covered

Measured: en-GB (0x0809). Specification tables: zh-TW, zh-HK, zh-MO, zh-CN, zh-SG, ja-JP, ko-KR, th-TH. Derived: every other LCID of the
table (see above). Per-locale expectations: `crates/xlsx/tests/numfmt_locale.rs`.

## Open

- Measure a second locale (en-US at least) in a separate profile or VM, per ADR-011; this settles the derivation choices above.
- `Workbook` has no setter for the locale (`Workbook::styles()` is read-only); the application must call it through a new method.
- `open365-calc`'s `FormatLocale::builtin_format_code` derives only 14, 22 and the currency from CLDR and should use this module.
- `open365-calc` does not parse the Thai codes (`t0`, Thai date letters, `bb`).
