# Spell check (`crates/spell`)

Status: first slice, 2026-10-11. Not yet wired into docx, layout or the UI.

## Decision record (copy into `decision-log.md`)

- **Context:** Word and Excel proofing for every locale. Dictionaries exist in Hunspell format for nearly all locales. See `docs/research/spell-dictionaries.md`.
- **Alternatives:** adopt spellbook (MPL-2.0, most complete Rust engine, alpha); adopt zspell (Apache-2.0, no compounding); bind C++ Hunspell (unsafe FFI, tri-licence); native OS checkers only.
- **Decision:** own clean-room Hunspell-compatible engine, no bundled dictionaries, dictionary directory chosen by the caller.
- **Consequences:** we own affix/compound/suggestion correctness and must test it; MIT with only `encoding_rs` ((Apache-2.0 OR MIT) AND BSD-3-Clause) and `unicode-segmentation` (MIT OR Apache-2.0) as dependencies; `cargo deny check licenses` passes with no new allow entry.
- **Confidence:** medium. The engine passes our fixtures; it has not been run against a full-size dictionary or a conformance corpus.
- **Evidence that would change it:** a full-size LibreOffice dictionary showing wrong accept/reject rates against Hunspell, suggestion quality well below Hunspell, load time or memory beyond target, or a decision to accept MPL-2.0 dependencies (then adopt spellbook).

## API

```rust
let checker = Checker::open(dir, "en-GB")?;          // en-GB -> en fallback, no network
checker.check("colour");                              // bool
checker.suggest("colur");                             // Vec<String>, best first, cased like the input
checker.find_misspellings(paragraph, &TokenizerOptions::default());
// Misspelling { word, byte_range, char_range, utf16_range }
checker.add_word("Open365");                          // persistent user word
checker.ignore_word("Zorblat");                       // "Ignore All", session only
UserDictionary::load(path) / .save(path) / from_text / to_text
```

- `Checker` = shared `Arc<Dictionary>` + per-user `UserDictionary`. `Dictionary` is immutable and `Send + Sync`.
- `DictionaryLibrary` scans directories for `en_GB.aff/.dic` pairs or `en-GB/index.aff/.dic` folders. Tags are case-insensitive and accept `_` or `-`. Resolution: exact tag, each shorter prefix (`zh-Hant-TW` -> `zh-Hant` -> `zh`), and at each step a sibling variant (`de-DE` -> `de-DE-frami`; bare `en` -> `en-US`, `pt` -> `pt-BR`, using a small home-region table). `ResolvedDictionary.exact` tells the UI when it fell back.
- `find_words` is the tokenizer alone (no dictionary), returning `Word { text, byte_range, char_range, utf16_range }`. UTF-16 offsets are the unit the OOXML/Word model uses.
- Tokenizer defaults mirror Word's options: skip ALL-CAPS words, words with digits, numbers, URLs, email addresses and file paths; each can be switched off, plus `min_word_chars` and `extra_word_chars`. Hyphen, apostrophe (straight and U+2019) and dictionary `WORDCHARS` join word parts; `.` and `:` split words unless the dictionary lists them. Han, Hiragana and Katakana are not spell-checked.

## Engine behaviour

Check path: `ICONV` + `IGNORE` -> numbers accepted -> case forms (as typed, then Title/lower for Title, Title/lower plus elided-article form for ALL-CAPS; mixed case is exact only) -> per form: exact `FORBIDDENWORD` entry rejects; otherwise dictionary stem, one prefix, up to two suffixes (continuation flags), cross-product, `CIRCUMFIX`, `NEEDAFFIX`, `ONLYINCOMPOUND`, `KEEPCASE` (typed spelling only), then compounds -> `BREAK` table (default `-`, `^-`, `-$`; `BREAK 0` disables).

Compounds: `COMPOUNDFLAG/BEGIN/MIDDLE/END`, `COMPOUNDRULE` with `*` and `?`, `COMPOUNDMIN`, `COMPOUNDWORDMAX`, `COMPOUNDPERMITFLAG`, `COMPOUNDFORBIDFLAG`, `CHECKCOMPOUNDDUP/CASE/TRIPLE/PATTERN`, `FORCEUCASE`. Search is depth-first with a fixed step budget (20 000) so a hostile dictionary or word cannot run away.

Suggestions: case variants, `REP`, `MAP`, adjacent and long swaps, `KEY` neighbours, wrong-case letter, extra letter, missing letter (`TRY`, or the most frequent dictionary characters when `TRY` is absent), moved letter, wrong letter, doubled pair, split into two words (`NOSPLITSUGS` honoured). If fewer than three result, an edit-distance fallback over dictionary stems adds up to `MAXNGRAMSUGS` (default 4) within distance `clamp(len/3, 1, 3)`. `NOSUGGEST` and forbidden words are never suggested; compounds are limited by `MAXCPDSUGS`. Output is re-cased to match the input (`TEH` -> `THE`), passed through `OCONV`, de-duplicated and capped (default 10). `Checker::suggest` puts user-dictionary words within one edit first. Order is deterministic.

Encodings: UTF-8 and any single-byte `SET` that `encoding_rs` knows (ISO-8859-x, windows-125x, KOI8-R/U, TIS-620 via windows-874). No `SET` means UTF-8 if valid, otherwise Latin-1/windows-1252. `FLAG` char, UTF-8, long and num; `AF` aliases.

## Not supported yet

`COMPLEXPREFIXES`, `SIMPLIFIEDTRIPLE`, `CHECKCOMPOUNDREP`, `COMPOUNDSYLLABLE/SYLLABLENUM`, `COMPOUNDMORESUFFIXES`, `COMPOUNDROOT`, `PHONE` (phonetic suggestions), `MAXDIFF/ONLYMAXDIFF/SUGSWITHDOTS`, `CHECKSHARPS` (German all-caps `ß` -> `SS`), `WARN`, `SUBSTANDARD`, morphological output (`AM`, stemming, analysis), trailing-dot abbreviation lookup, Unicode normalisation (NFC/NFD) of input, and a second prefix level. The distance fallback only proposes stems, not inflected forms. Locales that depend on these (Hebrew/Arabic `COMPLEXPREFIXES`, Hungarian and Finnish compounding details, German all-caps) may over-flag until added.

## Tests

`cargo test -p open365-spell`: 36 tests over hand-made fixtures in `crates/spell/tests/data` (affixes, two-level suffixes, cross products, special flags, `ICONV/OCONV`, `REP/MAP`, compounds and rules, encodings, flag modes, library fallback) plus tokenizer offsets (emoji, combining marks), skip rules and the user dictionary round trip. No dictionary is downloaded or bundled. There is no performance claim: it has not been measured on a full-size dictionary (to do before relying on it: load time, memory and `suggest` latency on `en_GB`, `de_DE`, `hu_HU`, `fi_FI`).

## Integration plan (later tasks)

1. docx: map `w:lang` / `w:proofErr` / `w:noProof` to a tag and feed paragraph text to `find_misspellings`; store ranges in UTF-16 to match the document model; honour per-run `noProof`.
2. ui-egui: squiggle painting from `utf16_range`, context menu from `suggest`, "Add to Dictionary" and "Ignore All" via `Checker`; run checking off the UI thread in idle slices.
3. commands/mcp: expose `spellcheck` and `suggest` as permission-scoped read operations; adding user words is a user-scoped write.
4. Dictionary packs: application-level installer per `docs/research/spell-dictionaries.md` section 2 (licence display, consent, no silent downloads).
5. calc: Excel's spell check works per cell text; reuse the same API.
