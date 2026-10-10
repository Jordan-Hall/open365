# Fixtures

Tiers and invariants are defined in `docs/testing/fidelity-strategy.md`.

- `docx/`, `xlsx/`: tier A, one feature per file, **own work only** (made by a project member in Word, Excel or LibreOffice). Licence: MIT, same as the repo.
- `adversarial/`: tier D, hand-built malicious inputs.
- `external/`: tier C public corpora. Git-ignored. Never committed until each file's licence is audited.

Each fixture `name.docx` has a sibling `name.toml`:

```toml
feature = "tracked-changes"          # what this file exercises
made_with = "Word 365 16.0 (Windows)" # application and version that saved it
author = "project member"            # own work; no third-party content
expect_dropped = []                  # dropped-content log entries this file is expected to produce
notes = "one insertion, one deletion, one formatting change"
```
