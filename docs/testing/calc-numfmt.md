# Calc number formats: built-in ids by locale, non-Latin codes, argument counts

Evidence is Excel 16.0 on an en-GB machine, measured with `tools/office/make-numfmt-calc-refs.ps1` (own Excel instance, macros off,
`TEXT` and `Range.Text` with explicit `[$-xxxxxxxx]` tags, `Range.Formula` for arity). No Windows or Office setting is changed.

## Built-in numFmtId by LCID

`open365-locale` now owns the table (`builtin_number_format_in(id, lcid)`, `resolve_number_format_lcid`, `nls_pattern_to_excel`, a copy of
`crates/xlsx/src/builtin_numfmt.rs` and `builtin_numfmt_nls.rs`). `FormatLocale::builtin_format_code` uses it for every locale that has an LCID.
Test `crates/calc/tests/numfmt_builtin_lcids.rs` compares calc with the xlsx function for ids 0 to 100 and all 418 LCIDs of the locale table
(42,218 comparisons, fixture `fixtures/calc/numfmt-builtin/xlsx-builtin-all-lcids.tsv`, recorded from `open365_xlsx::builtin_number_format_in`).
The evidence class of each code is unchanged (see `xlsx-numfmt-locale.md`): measured for en-GB, specification for East Asian and Thai, derived elsewhere.

xlsx switch (lead): delete `crates/xlsx/src/builtin_numfmt.rs` and `builtin_numfmt_nls.rs`, and in `crates/xlsx/src/lib.rs` replace the
`pub use builtin_numfmt::{...}` by `pub use open365_locale::{builtin_number_format_in, builtin_number_format_lcids, nls_pattern_to_excel, resolve_number_format_lcid};`.
The generator `make-numfmt-locale-refs.ps1 -Nls` should then write to `crates/locale/src/builtin_numfmt_nls.rs`.

## Non-Latin codes (measured, fixtures/calc/numfmt-nonlatin)

* `t` is a literal everywhere (`t0` gives `t1234568`, also under `[$-41E]`). Thai digits come from the tag.
* Number-shape byte (bits 24 to 31 of `[$-xxxxxxxx]`), identical under every language: 0 and 1 ASCII; 2 Arabic-Indic; 3 Persian; 4 to 8 Devanagari,
  Bengali, Gurmukhi, Gujarati, Oriya; 9 Tamil (no zero); A to C Telugu, Kannada, Malayalam; D Thai; E Lao; F Tibetan; 10 Myanmar; 11 Ethiopic (no zero);
  12 Khmer; 13 Mongolian; 1B to 27 CJK numeral sets and full-width; the rest ASCII. Implemented (`shape_digits`).
* Calendar byte (bits 16 to 23): 7 is the Thai Buddhist calendar (year + 543, Thai names under every language), 2 English names. Implemented.
  Implemented: 3 (Japanese era year, two digits for every year token; names English unless the tag is Japanese), 4 (Gregorian year with Chinese names), 5 (year + 2333; English names unless the tag is Korean), 6 (Hijri), 8 (Hebrew), 9 (French names), 10 (Syriac month names, Arabic weekdays), 16 (Saka), 20 (Korean Sino-Korean names), 22 (Persian), 23 (UmAlQura); `numfmt/calendars.rs`. Hebrew matches Excel for every year below 6000 (Excel drifts by a few days after that), Persian for years up to 1535 (the 33-year cycle), Saka for all years; serial 60 is `#VALUE!` and serial 0 shows serial 1 in these calendars; `mmm` equals `mmmm`, `mmmmm` is the first letter except in Hebrew (full name). Measured but not implemented: 11 and 12 (transliterated names for a 409 tag), 13 to 15, 17 to 19 and 21 (Chinese and Korean lunisolar).
* Thai date letters ว ด ป ช น ท (day, month, Buddhist year, hour, minute, second): without a tag Thai digits, Thai names and a Buddhist year; with a tag
  they follow the tag (`[$-41E]` gives Gregorian ASCII). Implemented for a whole section; Excel applies the Thai digits per Thai letter only
  (`h ว` gives `18 ๑๕`) and prints nothing for ช next to `AM/PM` (known differences).
* `b`/`bb`/`bbbb` without a tag are Buddhist years, with a tag Gregorian (existing behaviour, measured again).
* `B1` (Gregorian) and `B2` (Hijri) prefix, upper or lower case; `B0`, `B3` to `B9` are refused (implemented as a format error). `B2` and calendar byte 6 render the Hijri calendar (`numfmt/hijri.rs`): tabular civil calendar with the Kuwaiti leap pattern, measured on 1,990 serials (all match); serial 60 is `#VALUE!`, serial 0 shows serial 1. Month names: Arabic (no tag or an Arabic language), English transliteration for most other tags, own tables for fr, tr, id, fa; `mmm` is always Arabic. Quirks reproduced: without a tag a weekday token before `mmmm` replaces it, and a weekday after a first `mmmm` shows weekday number min(month-1, 6). UmAlQura (calendar byte 23) uses a month-length table measured for 1318 to 1450 (2,226 serials, all match) and the tabular calendar outside it. Open: the month/weekday tables of about 25 other languages (`hj5.*` probes in known.txt).
* `[DBNum1..4]`: only CJK language tags change digits (existing tables); DBNum4 and all other languages leave them. New: under DBNum1 and DBNum2 a
  two-digit part of 10 or more (`dd`, `hh`) is also written in tens (`十八`, `壱拾八`).

Cases: 1,028 (`cases.json`, `results.json`); 872 match Excel, 156 are listed in `known.txt` (a shrink-only list, rewrite with `NUMFMT_WRITE_KNOWN=1`):
135 calendar-byte cases, the Hijri `B2`/UmAlQura cases, the Thai letter quirks above and two Korean DBNum2 date strings.

## Argument counts

`fixtures/calc/arity/names.txt` is the registry's `function_names()` (490 names when measured, more as the registry grows; re-run the script). `make-numfmt-calc-refs.ps1 -Mode Arity` enters
`=NAME(A1,...)` with 0 to 40 (41 to 260 for variadic) arguments through `Range.Formula`, records the accepted counts (`excel-arity.tsv`) and
generates `crates/calc/src/parser_arity.rs` (min, max, step; SORTBY has one extra count). The parser rejects other counts with
`ParseErrorKind::ArgumentCount`. Names outside the registry are not checked. `crates/calc/tests/arity_excel.rs` checks the parser against the
measured list. Conformance case `randarray.c0169` (six RANDARRAY arguments) is now rejected as in Excel.

## Round 20 (lunisolar, Gregorian name calendars, Persian, neutral languages)

* Bytes 14, 15, 17, 18, 19 (lunisolar): measured for every serial 1 to 2958465 (`make-numfmt-calc-refs.ps1 -Mode Series`), stored as one byte per month
  (`numfmt/lunisolar_{f,e}.bin`, built by `tools/office/calc-calendar-tables.py`). The month number counts a leap month; names clamp at 12; AM/PM follow the tag.
  Excel's calendar is real only from about 1960 to 2100; elsewhere it has 1-day and 28-day months and a mid-month year bump on 1 January of 1960+91k.
  F (15, 17) and E (14, 18, 19) differ on 80 month starts. Byte 14 uses Japanese names (Korean tag: Korean), 15 zh-CN (404: zh-TW), 17 zh-CN, 18/19 English.
* Bytes 11 and 12: Arabic transliterated month names and weekdays for every tag; serial 60 `####`, serial 0 shows 1 January 1900. Bytes 13 and 21: English names for every tag.
* Languages 0, 1, 2 are not cultures: default names, and Hebrew/Persian/Saka show the English name of the month number (nothing for 13).
* Persian: Excel's leap pattern leaves the 33-year cycle near year 1502; `numfmt/persian_leap.bin` holds the measured leap years (8,099 bits).
* Thai letters: Thai digits only for the Thai letters; the hour letter prints nothing with AM/PM. Korean `[DBNum2]` tens use U+F973.
* Open: Hebrew from year 5999 (month lengths and year starts follow a mean-year approximation with day glitches; 85 known cases). Note: consecutive-serial
  series in Excel are path dependent for the Hebrew calendar (the same serial differs between a step-1 and a step-7 run), so only single-cell probes are evidence there.
  The lunisolar tables came from consecutive runs; single-cell probes (p5 cases, 2,143) agree, a random-access check of the tables was not finished.
