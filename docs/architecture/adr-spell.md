# ADR: Spell-check engine, dictionaries and Word parity (`crates/spell`)

Status: accepted by the lead's brief, 2026-10-11. Supersedes the "Decision record (copy into decision-log.md)" block in `spell-check.md` (engine behaviour and the directive list stay there). `decision-log.md` was not edited; it needs a one-line pointer to this file.

## Context

Writer and Sheets need Word-style proofing for every Office locale. Constraints: MIT distribution, `unsafe` forbidden, clean-room, no network inside the crate, dictionaries are untrusted input.

## Engine alternatives

| Option | Licence | MIT distribution | Fit |
|---|---|---|---|
| spellbook 0.4.x (Helix) | MPL-2.0 | Allowed as a dependency (file-level copyleft: MPL files stay MPL and their source must be offered; our MIT files are unaffected), but MPL-2.0 is not on the `deny.toml` allow list and would need a decision-log entry | Best existing crate; "alpha"; no PHONE; weak on some compound directives |
| zspell 0.5.x | Apache-2.0 (crates.io shows "non-standard" because its test data is MPL) | Compatible | No compounding, suggestions incomplete |
| hunspell-rs / Nuspell | MPL-1.1/GPL-2.0/LGPL-2.1 tri-licence C++; LGPL-3.0 | FFI needs `unsafe` (forbidden); LGPL not adoptable | Rejected |
| **Own clean-room engine (chosen)** | MIT | n/a | Already implemented in this crate; dependencies `encoding_rs` ((Apache-2.0 OR MIT) AND BSD-3-Clause) and `unicode-segmentation` (MIT OR Apache-2.0) pass `cargo deny check licenses` with no new allow entry |

**Decision:** keep the own engine. **Consequences:** we own correctness; compensated by Word-agreement tests below. **Confidence:** medium. **Would change if:** the agreement numbers fall for non-English dictionaries (not yet measured), or the project accepts MPL-2.0 dependencies (then adopt spellbook and contribute fixes).

## Dictionaries

Bundled (feature `bundled-en`, default on): SCOWL 2020.12.07 size-60 Hunspell builds en_US, en_GB (`-ise`), en_CA, en_AU, packaged by wooorm/dictionaries. Licence: SCOWL permissive notice plus Ispell BSD-style, VarCon, Princeton WordNet 1.6 notice, public-domain 12Dicts/ENABLE/Moby; the UKACD clause (verbatim text must accompany) is only triggered at SCOWL size 80+, but each full licence text is shipped next to the data (`crates/spell/dictionaries/*.LICENSE.txt`). Compatible with MIT distribution on condition that notices travel with the data. Hashes and sources: `crates/spell/dictionaries/NOTICE.md`. These are data files, not code, so `cargo deny` does not see them; the application's third-party notices page must list them.

Not bundled: every other language. They load at run time from directories added with `Proofing::install_dir` (user-installed or downloaded by the application with explicit consent). Per-source licences remain as recorded in `docs/research/spell-dictionaries.md` (class A permissive: ru, lt, nl, tr, ka, fa and others; class B weak copyleft: fr, pl, pt, es and others; class C GPL/AGPL: never in an Open365-hosted bundle). Other English regions map to the nearest bundled convention (en-NZ to en-AU; en-IE, en-ZA, en-IN to en-GB; others to en-US) and the result reports `exact = false`.

Language selection: the paragraph or run language tag (`w:lang`, `xml:lang`) drives `Proofing::check_spans`. Order: installed exact match, bundled nearest, installed fallback (`zh-Hant-TW` to `zh`). `und`, `zxx` and `LangSpan::no_proof` are not checked. A tag with no dictionary is reported in `missing_languages`.

## API (no UI code)

`Proofing` (thread-safe, cheap to clone): lazy dictionary load with per-dictionary verdict memo, one user dictionary plus session "Ignore All" list, `generation()` counter for cache invalidation, `check_paragraph` / `check_spans` returning `Misspelling { kind, word, byte_range, char_range, utf16_range }`, `suggest` (max 5 by default). `ProofSession` caches per paragraph id and returns the same `Arc` when nothing changed. `ProofWorker` is a std-thread worker with newest-wins coalescing per paragraph, revision echo and an optional notify callback for repaint. Excel cells use the same calls.

## Word parity rules (observed, not copied)

Defaults: ignore UPPERCASE, words with digits, URLs, e-mail and file paths; repeated words flagged (second word) when separated by plain spaces only; `that that`, `had had`, `in in`, `then then`, `blah blah`, `hush hush` are not flagged; ALL-CAPS repeats (`GE GE`) are flagged; numbers-only pairs are not. Suggestions are capped at 5, lower-case input demotes proper-noun candidates.

## Measurement (Word 16.0.20430, own instance, `tools/office/make-spell-refs.ps1`)

4054 generated words per locale (sampled stems, single-edit typos, case variants) and 65 hand-written paragraphs; references committed in `fixtures/spell/`. Results with `SPELL_PARITY_FULL=1 cargo test --release -p open365-spell --test word_parity -- --nocapture`:

| Locale | Accept/flag agreement | Suggestion: first equal | Word's first in our top 5 | Our first in Word's top 5 |
|---|---|---|---|---|
| en-US | 98.03% | 87.8% | 94.3% | 92.7% |
| en-GB | 97.39% | 87.2% | 93.4% | 92.1% |
| en-CA | 97.51% | 87.4% | 93.3% | 92.2% |
| en-AU | 97.41% | 87.3% | 93.5% | 92.4% |

Paragraph flag sets identical to Word: en-US 67.7% (44 of 65), en-GB 70.8%. Doubled-word pairs (342, en-US): 96.8% identical. Caveats: the sample is dominated by dictionary words with a single typo, so suggestion agreement overstates real-world agreement; verdict differences are mostly vocabulary (Word accepts accented forms and many capitalised names, flags some British and slang words). Word's contextual "confused word" errors (`their/there`) are not reproduced. Word's personal custom dictionary on the measuring machine was not inspected.

## Open

Suggestion ranking is stage-ordered, not frequency- or phonetically ranked; non-English dictionaries are unmeasured; unsupported Hunspell directives are listed in `spell-check.md`; no performance target is set (en-US load about 90 to 140 ms, a bad-word suggestion 3 to 120 ms in release on a loaded machine).
