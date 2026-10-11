# Round trip on the Microsoft template corpus

ADR-001: parts the model does not touch come back byte-identical; unknown XML is preserved; every
drop is logged. This page records how that is proved on real-world files and what is known to fail.

## Corpus

136 public Microsoft templates (74 docx, 62 xlsx), listed in `fixtures/external/ms-templates.toml` and
fetched by `tools/corpus/fetch-ms-templates.ps1` into `fixtures/external/ms-templates/` (gitignored). They
are all-rights-reserved (`docs/research/corpus-licensing.md`): nothing derived from them is committed,
including the outputs below and any regression fixture. Regressions use minimal inputs built in code.

## Running

```
cargo xtask corpus-roundtrip
```

(Set `CARGO_TARGET_DIR` as usual.) It needs Windows with desktop Word and Excel and PowerShell 7 (`pwsh`).
Without the corpus the test prints `SKIPPED` and the command ends. The Rust half alone runs in
`cargo test -p open365-testkit --test corpus_roundtrip`; it also runs, skipping cleanly, in the normal suite.

For each file the test (`crates/testkit/tests/corpus_roundtrip.rs`):

| Stage | Action | Rust-side check |
|---|---|---|
| `unchanged` | open, save with no edit | every part byte-identical, same order, nothing dropped |
| `edited` | docx: insert a marker in the first non-empty paragraph. xlsx: write a text and a number below the used range of the first worksheet | every part outside the edited ones byte-identical; any part removed or added is named in the hygiene log; the edit is read back after reopening |
| `template` | content type swapped to the template kind (real templates are used as they are) | none; this is the input to the next stage |
| `instantiated` | open the template, `instantiate_template()`, save | only `[Content_Types].xml` differs from the template; reopened kind is the plain (or macro-enabled) document kind |

Outputs go to `fixtures/external/roundtrip-out/<stage>/` (override: `OPEN365_CORPUS_OUT`). Then
`tools/office/office-oracle.ps1 -Mode verify` opens every output read-only in Word or Excel. Word is opened
without repair; Excel opens it normally and a `[Repaired]` workbook name fails. Any failure to open is a failure.

## Result (2026-10-11)

`cargo xtask corpus-roundtrip` on this machine (Word and Excel 16.0): 136 files, 544 outputs, 0 failures.

| Stage | Files | Rust checks passed | Opened in Office without repair |
|---|---|---|---|
| unchanged | 136 | 136 | 136 |
| edited | 136 | 136 | 136 |
| template | 136 | n/a | 136 |
| instantiated | 136 | 136 | 136 |

The oracle was checked against truncated packages (one docx, one xlsx): both fail as intended.
Macro-enabled files are not in the corpus (the catalog serves only `.docx` and `.xlsx`); they are covered by the
fixtures `template-macro.dotm` and `template-macro.xltm` and by the fuzz seeds (`docs/testing/fuzzing.md`).

## Bugs found and fixed

| Cause | Fix | Regression |
|---|---|---|
| xlsx: `<v xml:space="preserve"> text</v>` (string formula result with edge spaces) put "attributes on v" in the dropped-content log, so 3 workbooks could not be saved | `v` attributes are kept on the cell and written back; they are cleared when the cell's content is replaced | `crates/xlsx/tests/corpus_regressions.rs` |

## Known exceptions

None.

## Fuzz with template and macro-enabled seeds

`OPEN365_FUZZ_SECS=600 cargo test -p open365-testkit --test fuzz_smoke --release` (2026-10-11, default seed):
17,449 iterations (2,929 formula), 0 panics, 0 slow cases. Seeds include 16 `.dotx`, 16 `.dotm`, 15 `.docm`,
7 `.xltx`, 7 `.xltm` and 6 `.xlsm` files, built from our own fixtures.
