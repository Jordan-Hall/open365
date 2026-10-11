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

| Locale | Accept/flag agreement | Our first = Word's first (top-1) | Word's first in our top 3 | Word's first in our top 5 | Our first in Word's top 5 |
|---|---|---|---|---|---|
| en-US | 98.03% | 90.5% (was 87.8%) | 94.2% | 94.7% (was 94.3%) | 94.9% (was 92.7%) |
| en-GB | 97.39% | 89.6% (was 87.2%) | 93.3% | 93.8% (was 93.4%) | 94.4% (was 92.1%) |
| en-CA | 97.51% | 90.0% (was 87.4%) | 93.2% | 93.7% (was 93.3%) | 94.6% (was 92.2%) |
| en-AU | 97.41% | 89.6% (was 87.3%) | 93.3% | 93.9% (was 93.5%) | 94.5% (was 92.4%) |

Verdict agreement is unchanged by the ranking work (it only reorders suggestions). `word_parity` asserts floors of 88.5% top-1 and 91.5% top-3 (when at least 500 suggestion rows are measured) besides the verdict and top-5 floors.

Paragraph flag sets identical to Word: en-US 67.7% (44 of 65), en-GB 70.8%. Doubled-word pairs (342, en-US): 96.8% identical. Caveats: the sample is dominated by dictionary words with a single typo, so suggestion agreement overstates real-world agreement; verdict differences are mostly vocabulary (Word accepts accented forms and many capitalised names, flags some British and slang words). Word's contextual "confused word" errors (`their/there`) are not reproduced. Word's personal custom dictionary on the measuring machine was not inspected.

## Suggestion ranking model (2026-10-11)

Context: suggestions used to come out in generation order (case, `REP`, `MAP`, transpositions, key neighbours, deletions, insertions, substitutions, splits, n-gram). Alternatives: keep stage order (87.8% top-1 en-US); a hand-tuned score; a fitted linear model; a model driven by a word-frequency table (none is available without licence work: SCOWL size 60 carries no frequencies).

Decision: all stages fill a pool of up to 40 candidates (`POOL_LIMIT`) and a linear model ranks it. Features (21 numeric): Damerau/OSA distance, length difference, same first and last letter, shared prefix and suffix, bigram Dice, a consonant-skeleton match and distance (a cheap phonetic proxy), capitalised-when-typed-lower-case, extra apostrophe, whether the candidate is a dictionary entry and how many affix flags it has (a popularity proxy), discovery order, anagram, accented-only, n-gram candidate longer than the input; plus one bias per generation stage. The old hard rule that demotes proper nouns for lower-case input became a learned feature. The weights (`WEIGHTS`, `STAGE_BIAS` in `crates/spell/src/suggest.rs`) come from pairwise logistic regression against Word's suggestion order: candidates absent from Word's list rank last, pair weight 1/(1+rank).

Reproduce: `cargo run --release -p open365-spell --example pool_dump > pool.tsv`, then `python -I crates/spell/tools/fit_ranking.py pool.tsv` (needs scikit-learn); paste the printed arrays into `suggest.rs`. The feature order in `Dictionary::features` must match.

Evidence: 4068 flagged reference queries over four locales. On a 25% holdout of words (crc32 of the word, all locales) top-1 went from 85.8% to 89.6% and top-3 from 94.0% to 94.1% (training set: 87.6% to 89.9% top-1). The ceiling is candidate recall: Word's first suggestion is in our pool for about 94% of queries. Always running the n-gram stage lifted recall to 94.1% (from 93.8%) at roughly twice the suggestion time, so it still runs only when fewer than 3 candidates were found.

Consequences and caveats: the references are Word's answers to generated single-edit typos of dictionary words, so natural misspellings (phonetic errors, several mistakes in one word) are under-represented, and the holdout shares locales and stems with the training data, so it is optimistic. Confidence: medium-low that the 2.7-point top-1 gain transfers in full. It would change if a corpus of natural misspellings run through the oracle shows a different optimum, or if a permissively licensed frequency table is adopted (the next feature to try).

## Non-English dictionaries: plan

Status: nothing non-English is bundled or measured. Licence classes come from `docs/research/spell-dictionaries.md` (LibreOffice dictionaries repository, fetched 2026-10-11); engineering triage, not legal advice.

1. **Tiers by licence.**
   - Class A, permissive (ru and lt BSD-3-Clause; nl BSD-3-Clause or CC-BY-3.0, choose BSD; tr and ka MIT; fa, ie, tk, tlh Apache-2.0): bundle-eligible after the same review as the English set. Each added dictionary needs a section in `THIRD-PARTY-NOTICES.md` with verbatim licence text, an entry in `crates/spell/dictionaries/NOTICE.md` with upstream commit and SHA-256, and a `<tag>.LICENSE.txt` beside the data. Each gets its own Cargo feature (`bundled-ru`, and so on), never folded into `bundled-en`, so a build can leave it out.
   - Class B, weak copyleft (fr MPL-2.0; pl and pt, choose MPL-2.0; es, hu, ro, sk, da, el and others with an MPL-1.1 option; ca, sl, hr LGPL options; cy, et, lv, sv, en-ZA LGPL only): optional language packs, unmodified, as separate files outside the MIT binary, source being the upstream URL. The installer shows the licence and needs a click. Choose the MPL option where several are offered.
   - Class C, strong copyleft (de, it, cs, uk, nb, nn, gl, eu and others GPL only; he AGPL-3.0), and is (CC-BY-SA) and mn (LPPL): never in an Open365-hosted bundle or download list. Cover them through (a) the operating-system proofing service behind the same `Proofing` interface (Windows `ISpellChecker`, macOS `NSSpellChecker`), and (b) `Proofing::install_dir`, where the user supplies a folder; Open365 then neither distributes nor modifies the data.
2. **Delivery.** A language pack is a directory with `.aff`, `.dic`, a `manifest.json` (tag, upstream URL and commit, SPDX, SHA-256, size) and the licence file. The application layer, not this crate (which does no networking), downloads only on explicit user action from a fixed host allow-list, with SSRF-safe fetching, a size cap and checksum verification. A document never triggers a download. An agent or JSON/MCP caller may list packs; installing one needs the same user confirmation.
3. **Order.** es, fr, de, pt, it, nl, pl, ru first (largest Office markets), then the rest. de and it are class C, so they depend on the OS backend or user-supplied packs. That is the main product risk, and the OS-backend interface should exist before any German or Italian promise is made.
4. **Engine readiness.** Real dictionaries exercise directives the English set does not: `COMPOUNDRULE` and `COMPOUNDFLAG` (de, nl), `ICONV`/`OCONV`, `BREAK`, `PHONE`, long and numeric flags, dotted-I casing (tr, az), heavy affix tables (hu, el). Before shipping a language: load its upstream test dictionary in `cargo test`, fuzz the loader with its affix file, record load time and memory.
5. **Measurement.** `tools/office/make-spell-refs.ps1 -Tags <tag>` already sets the document proofing language, so per-language references need only Word's proofing tools for that language on the measuring machine (check `Languages(id).ActiveSpellingDictionary`; skip a language Word cannot check). Acceptance before bundling or advertising a language: verdict agreement at least 95% and top-5 recall at least 85% over at least 1000 generated words, recorded per language in this ADR. A language that fails is labelled "basic spell check" in the UI; a language with no Word reference is not claimed as Word-compatible. The script's generator currently reads `crates/spell/dictionaries/<tag>.dic`, so a new language needs its dictionary at that path first.
6. **Not Hunspell-shaped.** Hebrew, Arabic, Thai, CJK and Indic scripts need a different tokeniser or the OS backend; out of scope for the engine until measured.

## Open

Suggestion ranking has no word-frequency signal and only a rough phonetic one; non-English dictionaries are unmeasured and unbundled (plan above); unsupported Hunspell directives are listed in `spell-check.md`; no performance target is set (en-US load about 90 to 140 ms, a bad-word suggestion 3 to 120 ms in release on a loaded machine).
