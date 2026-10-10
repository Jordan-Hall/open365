# Fuzzing

Documents are hostile input (`../security/threat-model.md`, T1). M1 acceptance requires a
1 h fuzz run with no panics or OOM, and zip-bomb and DTD inputs rejected. Two harnesses
cover this.

## 1. Smoke harness (stable Rust, runs in CI)

`crates/testkit/tests/fuzz_smoke.rs` (helpers in `crates/testkit/tests/fuzz/`).

What it does:

- Takes every fixture in `fixtures/docx` and `fixtures/xlsx` as a seed (debug builds skip fixtures over 512 KiB, such as `large-sheet.xlsx`, to keep the 60 s budget; release runs include them).
- Mutates it, then runs open -> save -> reopen through `opc`, `docx` or `xlsx`, plus
  `inspect`, `inspect --hidden`, and a few edits, undo and redo through `open365-commands`.
- Mutates formula text and runs parse -> print -> load -> recalculate in `calc`.
- Accepts `Ok` or `Err`. A panic is a finding.

Mutations (all derived from a splitmix64 stream, so `seed + iteration` reproduces a case):

| Level | Mutations |
|---|---|
| Zip | bit flips, truncation, duplicate entry (same name and case variant), lying central or local size fields, zip bomb (up to 24 MiB of one byte), hostile entry names (`..`, absolute, backslash, control, empty, 5000 segments, bidi), thousands of entries (including over the limit), dropped, swapped or re-stored entries, junk before or after the zip, deleted ranges |
| XML (one part, re-zipped) | bit flips, truncation, DTD and entity declarations (billion laughs, external `SYSTEM`, parameter entities), nesting up to 100 000 levels (balanced and unbalanced), extreme numbers in place of digit runs (`4294967295`, `1048577`, `NaN`, ...), repeated slices, deleted ranges, bad character references, 1-4 MiB text, BOM and invalid UTF-8, swapped bytes |
| Formula | deletions, insertions of operators, references and unicode, nesting up to 50 000 levels, swaps, doubling |

Environment variables:

| Variable | Default | Meaning |
|---|---|---|
| `OPEN365_FUZZ_SECS` | `60` | Time budget. `3600` is the M1 run. |
| `OPEN365_FUZZ_SEED` | fixed | Seed. |
| `OPEN365_FUZZ_ITERS` | unlimited | Stop after this many iterations. |
| `OPEN365_FUZZ_FROM` | `0` | First iteration number (replay with `ITERS=1`). |
| `OPEN365_FUZZ_STRICT` | off | `1`: fail on panics in every crate, not just those owned by the harness's crates. |
| `OPEN365_FUZZ_VERBOSE` | off | Print any mutation or stage slower than 50 ms. |
| `OPEN365_FUZZ_LAST_CASE` | off | Path; the current case is written there before it runs, to triage aborts. |

Commands:

```sh
# default 60 s (use --release: the debug build is ~10x slower)
cargo test -p open365-testkit --test fuzz_smoke --release -- --nocapture
# M1 run
OPEN365_FUZZ_SECS=3600 cargo test -p open365-testkit --test fuzz_smoke --release -- --nocapture
# replay iteration 4711 of seed 7
OPEN365_FUZZ_SEED=7 OPEN365_FUZZ_FROM=4711 OPEN365_FUZZ_ITERS=1 cargo test -p open365-testkit --test fuzz_smoke --release -- --nocapture
```

### Findings and repros

On the first panic at a site (owner crate + file:line of the innermost workspace frame) the
harness minimises the input (drops zip entries, then delta-debugs entry bytes; 20 s budget) and
writes `fuzz/repros/<crate>/<site>.<docx|xlsx|formula>` plus a `.txt` note with the panic
message, seed, iteration and mutation.

Pass or fail:

- Panics in `opc`, `testkit`, `cli` or a third-party dependency fail the test.
- Panics in `docx`, `xlsx`, `calc`, `commands` and the other crates are saved and printed but do
  not fail the default run, because other owners fix them. `OPEN365_FUZZ_STRICT=1` fails on all.
  Turn strict on in CI for the M1 gate once the repros directory is empty.
- When a bug is fixed, delete its repro and add a regression test in the owning crate (project
  rule: regression fixture per bug fix).

### Limits of the harness

- Out-of-memory cannot be caught in-process (`unsafe` is forbidden, so no counting allocator).
  An allocation failure or stack overflow aborts the process, which shows as a crashed test
  run. Use `OPEN365_FUZZ_LAST_CASE` and the `progress:` lines (every 30 s) to find the case.
  Memory is bounded by `opc::Limits`, and the zip-bomb mutations stay under 24 MiB.
- Slow cases (over 10 s in one stage) are counted and reported as `SLOW`; they do not fail.
- The harness drives library APIs. The CLI has its own hostile-input tests in
  `apps/cli/tests/cli.rs`.

## 2. cargo-fuzz targets (nightly, local or scheduled only)

`fuzz/` is a separate workspace (the root `Cargo.toml` has `exclude = ["fuzz"]`), so stable CI
never builds it. Targets: `opc`, `docx`, `xlsx`, `calc_formula`.

```sh
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz run opc  ../fixtures/docx ../fixtures/xlsx
cargo +nightly fuzz run docx ../fixtures/docx
cargo +nightly fuzz run xlsx ../fixtures/xlsx
cargo +nightly fuzz run calc_formula
cargo +nightly fuzz run opc -- -max_total_time=3600
```

The fixtures directories seed the corpus. `fuzz/corpus` and `fuzz/artifacts` are git-ignored.
Copy any crash artifact into `fuzz/repros/<crate>/` after minimising it with
`cargo +nightly fuzz tmin <target> <artifact>`.

The targets use the same operations as the smoke harness with smaller `Limits` so executions
stay fast. `cargo check --manifest-path fuzz/Cargo.toml` type-checks them on stable.

## M1 gate result

| Item | Value |
|---|---|
| Command | `OPEN365_FUZZ_SECS=3600 OPEN365_FUZZ_STRICT=1 cargo test --release -p open365-testkit --test fuzz_smoke -- --nocapture` |
| Seed | 1918371904 |
| Iterations | 43 336 (7 109 formula cases; 36 227 package mutants, of which 21 985 still opened as valid documents) |
| Duration | 3600 s (finished in 3604.8 s with the build already warm) |
| Panics | 0 distinct panic sites (strict mode: any crate) |
| Slow cases | 0 (none over 10 s in one stage) |
| Aborts (OOM, stack overflow) | 0 |
| Repros | none: `fuzz/repros/` is empty |
| Machine | Intel Core i7-13850HX (20 cores, 28 threads), 64 GB RAM, Windows 11 Pro 10.0.26300, rustc 1.95.0 |
| Date | 2026-10-10 23:12 to 2026-10-11 00:12 (local) |
| Code | `opc` and `testkit` with the M1 hardening changes (part index, streaming parsers, content-type checks); other crates as they stood in the working tree |

The harness is single-threaded; the machine was shared with other builds.

An earlier attempt (seed 1454911672) aborted after 15 036 iterations in 1 600 s with
`memory allocation of ... bytes failed`. The cause was not the harness: another process on
the host (a Python script) had taken about 41 GB of commit, leaving the machine with
under 1 GB free. Replaying iterations 15 000 to 15 199 of that seed with the same binary
on a machine with free memory finished with no panic. The result above is a fresh seed on
the full hour.

Limits: seeded mutation, not coverage-guided. It shows that the listed mutations do not
panic, not that none exists. Start `cargo fuzz` (section 2) for depth.

## What the harness does not prove

Coverage-guided fuzzing finds deeper bugs than seeded mutation. The smoke harness is a
regression net; the claim "1 h with no panics" needs the 3600 s run on a stated machine, with
the log kept next to the M1 result.
