# Parser robustness

Status: first pass, 2026-10-11. DOCX, XLSX and attachments are untrusted input; this note
records how the parsers are exercised and what is still open. It complements
`threat-model.md`.

## Harnesses

| Harness | Where | Toolchain | Purpose |
|---|---|---|---|
| Regression suite | `crates/robustness` (`open365-robustness`) | stable 1.95 | Runs in `cargo test`; fixture mutations and hand-made hostile inputs |
| cargo-fuzz | `fuzz/` (own `[workspace]`, excluded from the root) | nightly | Coverage-guided search; targets `opc_open`, `docx_load`, `xlsx_load`, `calc_formula` |

Run the suite: `cargo test -p open365-robustness`. Known-crash cases are `#[ignore]`d with
their finding id; run them with `-- --ignored`. Set `ROBUST_CASES=200` for a longer mutation
soak, `ROBUST_KEEP=<dir>` to keep every hostile input the suite writes. Run the fuzzers as
described in `fuzz/README.md`.

## What the suite asserts

- No panic, no abort or stack overflow, no hang, no memory blow-up.
- Refusals come back as error values (`Ok` and `Err` are both acceptable outcomes).
- Hostile cases run in a child process (`o365-probe`, 8 MiB worker stack) so a crash cannot
  take the test binary down. The parent enforces **20 s wall clock (90 s in a debug build,
  about 15x slower)** and **1 GiB resident memory** (polled every 20 ms: `tasklist` on
  Windows, `/proc/<pid>/status` on Linux; a spike shorter than the poll can be missed).
- Parsers get tighter-than-default package limits (32 MiB per part, 96 MiB total, 5 000
  parts, ratio 200) so a blow-up below them is a real finding.
- Mutation tests (`tests/mutations.rs`) use a seeded xorshift mutator on every fixture in
  `fixtures/docx` and `fixtures/xlsx`: part-level byte flips, deletions, repeats, truncation,
  token insertion (`<`, `&`, `]]>`, `../`, extreme numbers) and digit-run replacement, then
  rezip so checksums stay valid; plus raw byte flips on the zip container and 1 800 mutated
  formulas. They run in process with `catch_unwind` and a watchdog thread.

## Hostile inputs covered (`tests/hostile.rs`)

Zip bombs (single part, many parts, 30 MiB text run, 100 000 entries), absolute / `../` /
backslash / UNC / NUL / 60 KB part names, duplicate entries (exact and case variants), lying
central-directory sizes and entry counts, truncated and prefixed containers; XML nesting to
300 000 levels (sdt, tables, hyperlinks, unclosed), 200 000 attributes, 2 000 000 siblings;
billion-laughs and quadratic-blowup entities in every part kind, character-reference
extremes; shared-string counts of 2^32-1 and one million strings, out-of-range string
indexes; row and column indexes up to 2^32-1, `XFD1048576` extremes, giant `cols`,
`mergeCell`, conditional-format and validation ranges; cyclic relationships (root, main
part, workbook, sheet to drawing); 200 000 relationships; external-target relationships
(http, file, UNC, ftp) and external entities, with a loopback listener asserting **no
connection is ever made**; about 150 formula shapes (deep nesting to 200 000, huge literals,
whole-grid ranges, array and dynamic-array blow-ups, recursion, volatile and external
functions).

## Findings

All calc findings reproduce on a release build. Files are under `fixtures/robustness/`.

| Id | Crate | File:line | Input | Effect |
|---|---|---|---|---|
| R-CALC-1 | calc | `functions/matrix.rs:90` | `MMULT(ROW(1:100000),COLUMN(A:ZZ))` | Output matrix (70 M cells) is not capped; 2.7 GB resident, 7 s |
| R-CALC-2 | calc | `functions/distrib_reg.rs:520` | `LINEST(ROW(1:100000),COLUMN(A:ZZ)*ROW(1:100000))` | 3.8 GB resident, 20 s |
| R-CALC-3 | calc | `functions/text.rs:283` | `SUBSTITUTE(REPT("a",32767),"a",REPT("a",32767))` | `s.replace` builds a 1 GB string before the text-length check turns it into `#VALUE!` |
| R-CALC-4 | calc | `functions/distrib_reg.rs:58-61` | `FREQUENCY(ROW(1:1048576),ROW(1:1048576))` | O(data x bins) linear scans; over 25 s |
| R-CALC-5 | calc | `functions/distrib_reg.rs:80` | `MODE.MULT(ROW(1:1048576))` | Linear `find` per value, quadratic in distinct values; over 25 s |
| R-CALC-6 | calc | `functions/dynarray.rs:252-258` | `UNIQUE(SEQUENCE(1000000))` | Linear group search per row, quadratic; over 25 s |

Not findings (checked): `MDETERM(MUNIT(2000))` takes 11 s (cubic by nature, bounded by the
array cap); relationship and shared-string tables of 200 000 / 1 000 000 entries parse in a
few seconds on release (they only look slow in a debug build).

No panic, stack overflow, hang or memory blow-up was found in `opc`, `docx` or `xlsx`
within the corpus and cases above, and no external relationship target was fetched.

## Open

- Owners to fix R-CALC-1..6, then remove the `#[ignore]` on the matching tests.
- A general cap on array results (the evaluator caps range operands at 2^20 cells in
  `eval.rs:25` and lambda results at 4 194 304 in `eval/lambda.rs:28`, but function outputs
  such as MMULT are not covered) would close R-CALC-1 and R-CALC-2 together.
- Memory is polled, not capped by the OS (no job object or cgroup; `unsafe` is forbidden).
  A real sandbox for parsers is a separate item in the threat model.
- The coverage-guided targets have been built but not run for a meaningful duration: they
  need a nightly toolchain and libFuzzer. A scheduled run (for example 30 minutes per target
  in CI) with the fixture corpora as seeds is the next step.
- Not covered yet: charts, pivots, drawings and images (`crates/chart`, `crates/pivot`),
  the PDF writer, and DOCX field-code evaluation with hostile field text beyond what the
  mutator reaches.
