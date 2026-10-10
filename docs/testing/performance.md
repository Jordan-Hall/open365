# Package performance

Scope: `opc` read, write, lookup and integrity checks, and the testkit round-trip invariant
(`open365 ... roundtrip --check`). The budgets are enforced by
`crates/testkit/tests/performance.rs` (release builds only; debug builds run the workload
without the clock).

## Hardware

Intel Core i7-13850HX (20 cores, 28 threads), 64 GB RAM, Windows 11 Pro 10.0.26300,
rustc 1.95.0, release profile. Single test thread (`--test-threads=1`), machine otherwise idle
(median of two runs; the before column is one run).

## Corpus

- Generated package: 19 000 zip entries (one `word/document.xml`, 18 996 stored
  `word/media/imageN.png` parts, one `.rels` with 18 996 relationships, a
  `[Content_Types].xml` with 18 997 overrides). 19 000 is just under
  `Limits::max_parts` (20 000). Write it out with `OPEN365_PERF_DUMP=<path>`.
- `fixtures/xlsx/large-sheet.xlsx`: 2.2 MB zip, one 20 MB worksheet part, a 1 MB calcChain.

## Before and after

The 19 000-entry package, before (commit `bfdcc51`) and after:

| Step | Before | After |
|---|---|---|
| `Package::read` | 15 ms | 13 ms |
| `check_integrity` | 9 570 ms | 36 ms |
| 19 000 case-insensitive `Package::part` lookups | 9 210 ms | 3.0 ms |
| `Package::write` | 31 ms | 21 ms |
| `roundtrip` (read, write, read, compare, integrity) | 18 950 ms | 66 ms |
| `inspect` | 10 680 ms | 89 ms |

`large-sheet.xlsx` `roundtrip`: 380 ms before, 360 ms after (unchanged within noise). About 300 ms of that is
deflating the 20 MB sheet in `Package::write`; reading it takes 25 ms. The remaining cost is
the zip codec, not package code.

## What changed

- `Package` keeps a lower-cased name to position map next to the part vector, so `part`,
  `set_part` and `remove_part` find a part without scanning. Zip order is the vector order;
  nothing about writing changed, so untouched parts stay byte-identical.
- `Relationships::parse` and `ContentTypes::parse` stream the XML and decode only the
  attributes they use, instead of building an element tree first.
- `check_integrity` parses `[Content_Types].xml` once and no longer lower-cases every part
  name to find `.rels` parts.
- `roundtrip` skips the second integrity pass when the re-read package equals the first
  (names, bytes, compression and order), and uses hash sets for the issue comparison.

## Budgets (release)

| Step | Budget | Measured |
|---|---|---|
| read 19 000 entries | 500 ms | 13 ms |
| `check_integrity` | 500 ms | 36 ms |
| write | 500 ms | 21 ms |
| 19 000 lookups | 100 ms | 3.0 ms |
| `roundtrip` | 1 s | 66 ms |
| `inspect` | 1 s | 89 ms |
| `roundtrip` of `large-sheet.xlsx` | 3 s | 360 ms |

Budgets are 5 to 10 times the measurement so they catch a return to quadratic behaviour and
not machine noise. Raise them only with a new measurement and a note here.

## Reproduce

```sh
cargo test --release -p open365-testkit --test performance -- --nocapture --test-threads=1
OPEN365_PERF_DUMP=big.docx cargo test --release -p open365-testkit --test performance
```
