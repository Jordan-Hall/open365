# MCP and CLI fixtures

- `schemas/cli-v1.json`: the `open365.cli/1` `--json` contract (envelope plus one result schema per command). Checked by `apps/cli/tests/surface.rs`; documented in `docs/reference/cli.md`.
- `transcripts/modern-readonly.jsonl`: a request sequence in the MCP 2026-07-28 shape, one request per line, for piping into `open365 mcp --root <dir> --audit-log <file>` by hand. Replayed by `apps/cli/tests/mcp.rs`.
