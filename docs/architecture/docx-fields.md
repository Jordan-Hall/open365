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
| date (DOCPROPERTY dates, CREATE/SAVE/PRINTDATE) | `fixtures/docx/fields-date.docx` | 470 / 564 |

The 94 date mismatches are non-English language names (the built-in locale table has English
only; `-LocaleTable` generation is not written) and the undocumented `g` and `n` characters.

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
