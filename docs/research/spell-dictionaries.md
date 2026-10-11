# Spell-check engines and dictionary sources

Researched 2026-10-11 for `crates/spell`. "Fetched" means read from the primary page on that date; "known" means stated from prior knowledge and not re-verified today. Licence notes are engineering triage, not legal advice; confirm with counsel before shipping any dictionary.

## 1. Engines (Rust)

| Option | Licence | Hunspell `.aff`/`.dic` | Maturity (fetched 2026-10-11) | Verdict |
|---|---|---|---|---|
| [spellbook](https://github.com/helix-editor/spellbook) 0.4.2 | **MPL-2.0** | Check, suggest, prefix/suffix, `COMPOUNDRULE`; "almost all" of the Nuspell test corpus passes; no phonetic suggestions; weak on complex compounding directives | README says "alpha"; crates.io: ~197k downloads, updated 2026-06-03; used by Helix | Best existing crate. Rejected: MPL-2.0 is not on the `deny.toml` allow list, and the brief prefers a permissive licence |
| [zspell](https://github.com/pluots/zspell) 0.5.5 | **Apache-2.0** (GitHub `LICENSE`; crates.io labels it "non-standard" because test data is MPL-licensed) | Check, forbidden words, stemming; **no compound words**; suggestions work-in-progress | ~139k downloads, last release 2024-06-13 | Permissive but incomplete (no compounding breaks Germanic and Hungarian-style locales) |
| [hunspell-rs](https://github.com/drahnr/hunspell-rs) 0.4.0 (+ `hunspell-sys`) | Apache-2.0 for the binding; wraps C++ Hunspell (MPL-1.1 / GPL-2.0 / LGPL-2.1 tri-licence, known) | Complete (it is Hunspell) | Last release 2022-09-18 | Rejected: `unsafe` FFI (forbidden by workspace lints), C++ build dependency, dependency licence needs review |
| Nuspell (C++, LGPL-3.0, known) | LGPL-3.0 | Complete, reference for behaviour | Active | Rejected: C++/LGPL; useful only as a behavioural reference |
| Native OS proofing (Windows `ISpellChecker`, macOS `NSSpellChecker`) | OS | Not Hunspell | Not evaluated | Possible per-platform backend later; does not remove the need for a portable engine (Linux, server, WASM, tests) |

**Decision:** implement a Hunspell-compatible engine clean-room in `crates/spell`, from the public format description (`man 5 hunspell`, Hunspell/Nuspell documentation) and the project's own hand-made fixtures. No Hunspell, Nuspell or spellbook source was read or copied. Detailed rationale and gaps are in `docs/architecture/spell-check.md`.

What would change the decision: if spellbook relicensed to a permissive licence, or the project accepted MPL-2.0 file-level copyleft for dependencies (record in the decision log), adopting it and contributing missing directives would be cheaper than owning the engine.

## 2. Dictionary sources

The [LibreOffice dictionaries repository](https://github.com/LibreOffice/dictionaries) (fetched) holds about 70 locale folders and is the practical upstream for every Office locale. Per-locale licences are in each folder and differ. The table below uses the mirror [wooorm/dictionaries](https://github.com/wooorm/dictionaries) (fetched; 92 dictionaries, SPDX per dictionary, each `license` file kept with the data) because it lists SPDX expressions in one place. Verify against the upstream folder before shipping.

### Class A: permissive, bundling is plausible with notices (MIT-compatible)

| Tags | SPDX (as listed) | Obligations |
|---|---|---|
| en, en-AU, en-CA, en-GB | MIT AND BSD | `license` file fetched for en-GB: SCOWL/VarCon (permissive, keep notices), Ispell-derived affix file (BSD: keep notice and disclaimer, mark modifications, no endorsement), WordNet 1.6 (keep notice), UKACD (display notice prominently, include text verbatim when redistributed), ENABLE/Moby/12dicts public domain |
| ru, lt | BSD-3-Clause | Keep copyright and disclaimer |
| nl | BSD-3-Clause OR CC-BY-3.0 | Choose BSD-3-Clause |
| tr, ka | MIT | Keep notice |
| fa, ie, tk, tlh | Apache-2.0 | Keep notice and NOTICE file |

### Class B: weak copyleft, usable as unmodified, separately distributed data files; not MIT

| Tags | SPDX (as listed) | Note |
|---|---|---|
| fr | MPL-2.0 | File-level copyleft |
| pl | GPL-3.0 OR LGPL-3.0 OR MPL-2.0 | Choose MPL-2.0 |
| pt | LGPL-3.0 OR MPL-2.0 | Choose MPL-2.0 |
| bg, br, da, el, fo, hu, hy, hyw, ko, pt-PT, ro, sk, sr, sr-Latn | include MPL-1.1 as an option | MPL-1.1 file-level copyleft |
| es and all es-* variants | GPL-3.0 OR LGPL-3.0 OR MPL-1.1 | MPL-1.1 option |
| ca, ca-valencia, sl, hr | GPL/LGPL combinations (hr: LGPL-2.1 OR SISSL) | LGPL option |
| cy, et, ltg, lv, ne, sv, sv-FI, en-ZA | LGPL-2.1 or LGPL-3.0 only | LGPL |
| is | CC-BY-SA-3.0 | Share-alike |
| mn | LPPL-1.3c | Free but with renaming conditions |
| lb | EUPL-1.1 | Copyleft |

### Class C: strong copyleft, never bundle in an MIT build

GPL only: cs, eo, eu, fur, fy, ga, gd, gl, it, la, mk, nb, nn, nds, oc, rw, uk, vi, ia, el-polyton, de/de-AT/de-CH (GPL-2.0 OR GPL-3.0). AGPL-3.0: he.

### What this means for Open365

1. **The crate bundles no dictionary.** `crates/spell` is MIT; a dictionary is data under its own licence and must never be silently folded into the binary.
2. **Bundle later only Class A**, behind a single `THIRD-PARTY-NOTICES` entry per dictionary that reproduces the licence text (en-GB's UKACD clause requires verbatim text). Use the exact files, unmodified; record the upstream commit.
3. **Classes B and C are delivered as optional "language packs"** that the user installs from the upstream source (or supplies in a folder). The installer must show the licence, keep the pack outside the MIT binary, and never be an agent-triggered silent download. Class C data stays out of any Open365-hosted bundle unless counsel says otherwise.
4. **Runtime network:** the crate performs none. Any downloader belongs to the application layer with explicit user consent and SSRF-safe fetching, and needs its own review.
5. Treat all dictionaries as untrusted input (size, line count, pathological affix files); the loader parses text only and bounds compound search.
6. Hunspell dictionaries for a few locales (Arabic, Hebrew, Thai, CJK, Indic) are linguistically weak or absent as word lists; the OS backend or a different algorithm is the realistic route there. Not evaluated.

## 3. Format references

- Hunspell `hunspell(5)` man page (affix file and dictionary file format): https://github.com/hunspell/hunspell/blob/master/man/hunspell.5 (page returned HTTP 503 on 2026-10-11, so it was not re-read today; the crate implements the publicly documented directive semantics and is validated only against our own hand-made fixtures in `crates/spell/tests/data`, not against Hunspell's or Nuspell's test corpora, which are MPL/LGPL and were not copied).
- Nuspell wiki on the format and test corpus: https://github.com/nuspell/nuspell/wiki
- LibreOffice dictionary guidance: https://wiki.documentfoundation.org/Development/Dictionaries
