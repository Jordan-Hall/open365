# Fuzzing

`cargo-fuzz` targets for the untrusted-input parsers. They need a **nightly** toolchain and
libFuzzer, so this directory has its own `[workspace]` and is listed in the root `exclude`.
The stable-Rust regression suite is `crates/robustness`; see
`docs/security/parser-robustness.md`.

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz list
cargo +nightly fuzz run opc_open  -- -max_total_time=300 -rss_limit_mb=1024 -timeout=20
cargo +nightly fuzz run docx_load ../fixtures/docx -- -max_total_time=300 -rss_limit_mb=1024
cargo +nightly fuzz run xlsx_load ../fixtures/xlsx -- -max_total_time=300 -rss_limit_mb=1024
cargo +nightly fuzz run calc_formula -- -max_total_time=300 -rss_limit_mb=1024 -timeout=20
```

Targets: `opc_open`, `docx_load`, `xlsx_load`, `calc_formula`. On Windows use the MSVC
nightly toolchain (the sanitizer runtime ships with it); Linux and WSL are better supported.
Crashing inputs land in `fuzz/artifacts/<target>/`; minimise with `cargo +nightly fuzz tmin`
and copy the result to `fixtures/robustness/` with a finding id. Corpora (`fuzz/corpus/`) and
artifacts are not committed.
