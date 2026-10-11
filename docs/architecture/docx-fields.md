# DOCX fields (crates/docx, `fields*.rs`)

Status: partial. Measured against Word 16.0 build 20430 through `tools/office/fields-oracle.ps1`.

## Model

- `Document::fields()` parses `w:fldSimple` and complex fields (nested, multi-paragraph) into
  `Field { instruction, depth, parent, result_text, ... }`. Parsing never changes the tree, so an
  untouched document saves byte-identical (tested on the oracle fixtures).
- `Instruction` holds the field name, a `FieldKind`, positional args and switches (`\*`, `\#`,
  `\@`, `\h`, `\o`, `\t`, ...). Which switches take an argument is decided per field type.
- `Document::update_fields(&FieldContext)` rewrites results in document order, innermost first
  (fields nested in an instruction update before their parent). A result that already equals
  the new one is not touched. Locked, unsupported and input-starved fields are reported in
  `FieldUpdateReport::skipped`.
- Inputs from outside come through `FieldContext`: clock and UTC offset, file name/path, a
  `PageProvider` (layout lives elsewhere), `MergeData`, and `DateNames` to override locale names.

## Measured Word behaviour (all in tests)

- Default result formatting is the formatting of the first character of the field code;
  `\* MERGEFORMAT` keeps the old result's formatting (by word); `\* CHARFORMAT` equals the default.
- `=` keeps a decimal "scale": add/multiply/power use the largest operand scale, division at
  least 2, `%` adds 2, `ROUND(x,n)` has scale n; trailing zeros are dropped; a non-zero value that
  rounds to zero prints `0.0`. Results carry 14 significant digits. `-2^2` is 4, `^` is right
  associative. Errors: `!Zero Divide`, `!Syntax Error, x`, `!Unexpected End of Formula`,
  `!Missing Operator`, `!Undefined Bookmark, NAME`.
- `\#` pictures: digits fill placeholders from the right, `#` pads with a space, `,` groups all
  digits in threes, `-`/`+` are the sign slot, sections are positive;negative;zero, `\` is ignored.
- `\*`: CardText/OrdText up to 999999 (0.5 to 0.99 is an error), DollarText cents are rounded
  without carry, Roman up to 32767, ALPHABETIC repeats the letter up to 30 letters, Hex up to
  65535, Ordinal up to 2147483647; `Caps` capitalises after whitespace and `-` only.
- Dates: `MMMMM` = month name + number, `dddddd` = day name + `dd`, `yyy` is an error, `AM/PM`,
  `am/pm`, `A/P`, `a/p` all print the locale's AM/PM string, a backslash is ignored. Document
  dates (CREATEDATE/SAVEDATE/PRINTDATE) show minutes only; custom date properties show midnight.

## Oracle match counts

| set | fixture | match |
|---|---|---|
| format (`=`, QUOTE, `\#`, `\*`) | `fixtures/docx/fields-format.docx` | 1473 / 1473 |
| date (DOCPROPERTY dates, CREATE/SAVE/PRINTDATE) | `fixtures/docx/fields-date.docx` | 564 / 564 (see Update 5) |

Later updates closed the date mismatches (see Update 5); the other sets are listed there.

## Open

REF/PAGEREF/NOTEREF/SEQ/STYLEREF/IF/MERGEFIELD/HYPERLINK/TOC evaluation (stubbed: they report
`UnsupportedType`), table `SUM(ABOVE)` formulas, fields in headers/footers, TOC styles missing from
`styles.xml` (needs a `Document` hook to add styles), generated locale table. The oracle script
already defines the `props`, `refs` and `toc` sets; they have not been run.
`update_fields` rescans the story after each field (about 65 s for the 1473-field fixture in a
debug build); a batch path is open work.

## Update 2 (REF family)

- Batch path: `UpdateState` keeps the scan, paragraph slots and snapshot; a result written inside one
  paragraph rescans only that paragraph. The 1473-field `format` set updates in about 0.5 s (debug).
  Paragraph insertions are tracked so the `PageProvider` always sees original paragraph numbers.
- Done: REF (`\n \r \w \p`), PAGEREF (`\p`, page labels), NOTEREF (`\p`), SEQ (`\c \n \r \h \s`),
  STYLEREF (`\n \r \w \p \l`), IF (numeric/text, `*`/`?` wildcards on the right side, dictionary
  ordering, case-sensitive equality), MERGEFIELD (`\b \f`, `MergeData`), HYPERLINK (result kept),
  table `SUM(ABOVE|LEFT|BELOW|RIGHT)` and `A1:B3` ranges (directions stop at a blank or non-number cell).
- Oracle: refs 221/225, props 59/69 (before the last two small fixes), format 1473/1473, date 470/564.
- Open: TOC generation (`fields_toc.rs` is still a stub, TOC n styles not added), locale name table
  (`-LocaleTable`), REF source formatting, `IF` with literal braces, thousands separators in table sums,
  `1e2` table cells, `MERGEFIELD \m \v`.

## Update 3 (TOC, props)

- TOC (`fields_toc.rs`): `\o \u \t \h \z \n \p \w \x \b \l \f \c` and `TC` fields, measured with the `toc` set
  (17 TOC variants, 134/134 fields and every paragraph's style and text match Word). Entries are `TOC n` /
  `Table of Figures` paragraphs, each with a `_Toc` bookmark (reused when one already wraps the heading), a
  `w:hyperlink` with `\h`, and a nested `PAGEREF \h`. Tab and line break in a heading become a space unless `\w` / `\x`.
  `TC` identifier defaults to `C`; `\l` filters TC levels; `\n` on a TC drops its page. An empty TOC reads
  "No table of contents entries found." in one paragraph. Pages come from the `PageProvider`; without one the TOC is
  skipped (`MissingInput`). Left tab position for numbered entries and the right tab (text width - 10) are estimates.
- Missing styles `TOC1-9`, `TOC Heading`, `Table of Figures`, `Hyperlink` are added from `fields_toc_styles.rs`
  (measured with `fields-oracle.ps1 -TocStyles`; reference in `fixtures/reference/docx/fields-toc/styles.xml`)
  through `Document::styles_mut`, which swaps the styles part.
- Props: DOCPROPERTY built-in dates show minutes, custom dates only the date, Yes/No properties show Y/N,
  `INFO FileName`, `Caps` capitalises after any non-alphanumeric except an apostrophe, `IF` without an operator
  (text is true when not empty, the next two words are the answers; a number is an error).
- `fields-oracle.ps1`: `-TocStyles`, `-LocaleTable <rust file>` (see below), reference output no longer records the
  scratch path.
- Open: locale name table (generator written; see status in the report), date pictures `g` (a Japanese era initial)
  and `n` (weekday number), thousands separators in table sums (`1,003`), `1e2` cells, REF source formatting,
  SECTIONPAGES (4 vs 5 in the test provider), `MERGEFIELD \m \v` (accepted, no effect without data).

## Update 4 (date eras, language slots, section pages)

Measured with `fields-oracle.ps1 -Probe` (new: `#date NAME=ISO` custom date properties, `#table`/`#cell` table probes,
`@tag|code` to write any BCP 47 tag into the code run, `@o1025|` / `@f1041|` to use `LanguageIDOther` /
`LanguageIDFarEast`).

- Date pictures `g` and `n` are not weekday letters: this Word reads them as Japanese-era letters whatever the field
  language. `g` = era initial (M T S H R), `gg` = first kanji, `ggg` = era name, longer runs repeat in threes (`gggg` =
  `ggg` + `g`). `e` and `n` = year of the era (Reiwa 6 for 2024, Heisei 31 on 2019-04-30; the first year prints 1, not
  an "gannen" word); pairs are zero padded to two digits, a lone one is not (`nnn` -> `066`). Era starts used: Meiji
  1868-09-08 (assumed), Taisho 1912-07-30, Showa 1926-12-25, Heisei 1989-01-08, Reiwa 2019-05-01. Implemented in
  `fields_date.rs` (unit test `japanese_era_letters_follow_word`; the oracle `date` set has 2024 only, so the other eras
  are covered by the unit test, whose values come from probes).
- Other letters seen in the same probes (not implemented): `G`/`GG` kanji era, `A` `O` `R` `r` `I` `C` kanji numerals of
  day, month, hour (24/12), minute, second, `rr` = "xia wu" + kanji hour, `b`/`bb`/`bbbb` Buddhist year (odd runs and
  `B`, `E`, `N`, `yyy`, `Y` triples are the picture error), `a` runs collapse (`aaaa` -> `aa`). Runs of `h H m s` pair up
  like `n` (`mmm` -> `055`); `yyyyy` = `yyyy` + `yy`; `MMMMM` and `dddddd` are the existing rules.
- Genitive month names: Word uses them for `MMMM` whenever the picture holds a day number (`d`, `dd`, `D`) anywhere
  (`MMMM d`, `d 'x' MMMM`, `yyyy MMMM d`), not for `dddd MMMM` or `MMMM yyyy`. Measured in lt (kovo), sk (marca), pl, ru,
  cs; fi and hu have none. Not implemented: the full locale scan needs the generator below.
- Languages: `LanguageID` only fills the Latin slot, which is why 58 tags (all `ar-*`, he, fa, ps, hi, ja, ko, th, ...)
  were "not accepted". The XML is the same `w:lang w:val="ar-SA"`; `LanguageIDOther` sets it for complex scripts, and
  patching the file works for every tag (ja-JP, zh-TW, ko-KR give localised names this way; th-TH falls back to English
  names in this installation). None of them switches the calendar: ar-SA, he-IL, fa-IR, th-TH all print Gregorian years
  (`yyyy` = 2024) without a switch, so there is no "default non-Gregorian calendar" to implement. `\h` and `\s` are
  not DOCPROPERTY switches (Error! Unknown document property name.). Untested: `DATE \h` / `\s`.
- Draft generator (unrun, not in the script): copy one block of 34 fields per culture with the tag written into the
  file, add `d'|'MMMM'|'MMM` per month for genitive names; the existing COM generator needs about 20 minutes.
- SECTIONPAGES (5 vs 4) was a test-provider bug: Word's paragraph list carries an extra paragraph where a REF result ends
  in a paragraph mark, and an end-of-row mark is indistinguishable from an empty cell. `tests/fields.rs` now aligns
  Word's paragraphs with the model's (tables walked, split results skipped). refs: 223/225.
- Open: table formulas (`1,003` with comma cells, `1e2`: probe file `t1`-style cases are not yet run), REF source
  formatting, props rerun, the full genitive/locale regeneration.

## Update 5 (locale table, genitive months, table cells)

Oracle counts now: format 1473/1473, date 564/564, refs 225/225, toc 134/134, props 69/69 (rerun in full against Word
16.0 build 20430; the reference holds `<scratch>` only), tables 169/169 (new set). Every set is asserted exactly in
`tests/fields.rs`.

- Locale table: `fields-oracle.ps1 -LocaleTable` now builds one block of 34 fields (months, genitive months with a day
  number, weekdays, AM/PM, a year check), copies it once per culture with the BCP 47 tag written into the code run's
  `w:lang` (`Replicate-FieldBlock`), and reads all results from one `Fields.Update` (about 4 minutes instead of 20).
  Every tag Word accepts is in `fields_locale_data.rs` (232 tags, 187 records, including ar-*, he, fa, hi, ja, ko; three
  tags are left out because Word falls back to English names: bin-NG, ibb-NG, th-TH). Records carry two more columns,
  `genitive_full` and `genitive_abbr` (empty when equal); `LocaleNames` exposes them as `month_full_genitive` /
  `month_abbr_genitive`.
- Genitive rule (`has_day_number` in `fields_date.rs`): `MMM` and `MMMM` use the genitive list whenever the picture, outside
  quoted text, has a day number run of one or two `d` (or `D`); `ddd` / `dddd` do not count. Fixed the last 4 date misses.
- Pictures: `G` = kanji era (first kanji / name / name + first kanji / name twice); `bb`, `bbbb`, `bbbbbb` = Buddhist
  year (Gregorian + 543; pairs, `bbbbbb` = `bbbb` + `bb`); `BB`, `BBBB` = Gregorian year; an odd run of `b`/`B` and
  `yyy` give `Error! Unknown character in picture string.` as the whole result (a lone `b yyyy` prints only the error).
  Covered by unit tests with probe values, not by the date set.
- Table formulas (`fields_cells.rs`, measured with the `tables` set, 169 cases): a cell is a list of numbers. A comma
  followed by three or more digits is a thousands separator (`1,2345` = 12345), any other comma separates numbers
  (`12,34` = 46), a second `.` starts a number (`1.5.5` = 2), `(5)` = -5, `-(5)` = 5, `5%`/`50 %` percentages, words and
  `$` ignored (`2 apples` = 2). `+5`, `5-`, `1e2`, `1.5e1`, `0x10` are not numbers (`1e+2` is 100). A cell that is more than
  a bare number (brackets, words, symbols, exponent, `1 000`) counts only while no bare number was counted yet; a
  direction walks from the nearest cell, passes over leading blank/non-number cells, and stops at the first blank or
  non-number cell after a number. With no `\#` picture or numbering switch the result is written like its cells: thousands
  separators if any counted cell had one, brackets for a negative sum if a counted cell had brackets (`(5)` -> `(5)`,
  `1,000/5/-2` -> `1,003`, `1,000/(5)` -> `995`). Range references (`A1:B2`) read every cell and do the same styling.
  Not modelled: the locale currency symbol (`£5` -> `£5.00` in this installation; `$` is not a currency here).
- REF source formatting (measured with a COM probe, not implemented): default `REF` copies the character formatting of the
  bookmarked runs into the result (direct run properties, per run; `\* Upper` keeps them); `\* CHARFORMAT` gives the
  formatting of the first character of the field code to the whole result (what the model already does); `\* MERGEFORMAT`
  maps old words to new words as before, but words beyond the old result take the source formatting (not the last old
  word's), and a trailing space takes the old word's trailing-space formatting, or the plain default when the old word had
  none. Needs a rich `Outcome` (runs from the bookmark range) and a formatting-aware reference dump; open.
- Open: REF source formatting (above), locale currency in table results, the kanji numeral letters `A O R r I C`, `rr`, and
  `a` run collapsing.
