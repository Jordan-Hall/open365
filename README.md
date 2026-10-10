# Open365

An open, self-hostable, native productivity suite for Office-oriented workflows, starting with Word-compatible documents and Excel-compatible spreadsheets. It is a clean-room implementation in Rust, licensed under MIT.

Status: pre-alpha foundation (milestone M0). No document features yet.

## Build and check

```sh
cargo xtask ci      # fmt, clippy -D warnings, tests, layering check, cargo deny
cargo xtask layers  # layering check only
```

Requires Rust 1.95 (pinned in `rust-toolchain.toml`). Install `cargo-deny` (`cargo install --locked cargo-deny`) to run the licence/advisory check locally. CI fails without it.

## Layout

- `crates/`: libraries, layered `opc` → `docx`/`xlsx` → `commands` → … (enforced by `xtask`)
- `apps/cli`: the `open365` binary
- `fixtures/`: licence-audited test documents
- `docs/`: research, architecture, decisions, roadmap, security, testing
