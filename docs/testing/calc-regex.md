# REGEXTEST, REGEXEXTRACT, REGEXREPLACE

Excel runs PCRE2 (10.4x: `\w` includes Mn and Pc, `{,n}` is a quantifier, lookbehind up to 255 characters) in UTF and
UCP mode with the ANYCRLF newline convention. Open365 has its own backtracking engine (`functions/textpcre.rs`,
`textpcrevm.rs`, `textpcreprops.rs`) rather than a crate: no permissive crate has PCRE2 semantics (lookaround, back
references, atomic groups, recursion). The only dependency added is `unicode-general-category` (Apache-2.0).

Measured against the oracle (corpus groups `regextest`, `regexextract`, `regexreplace`; all pass): syntax and error
cases, the replacement escapes (`$1`, `${n:-d}`, `$<name>`, `$&`, `` $` ``, `$'`, `$_`, `\U \L \u`), return modes, the
global match loop of `pcre2_substitute` (empty matches), and the limits: `^(a+)+$` answers up to 18 `a` and is
`#VALUE!` from 19 (`STEP_LIMIT`); 10,000 characters of `^(a|b)*$` match, 100,000 abort. A pattern needing a literal
that the subject lacks fails at once (PCRE2's required code unit).

## Known differences from PCRE2 (to burn down)

- Recursion `(?1)` and atomic groups are atomic; PCRE2 10.30+ can backtrack into a recursion.
- `(*COMMIT)`, `(*PRUNE)`, `(*SKIP)`, `(*THEN)` are accepted and do nothing; `(*ACCEPT)` is simplified.
- Script properties (`\p{Greek}`, ...) use range tables for 13 common scripts; other scripts are `#VALUE!`.
- Caseless matching uses simple case folding from the Rust standard library, not CaseFolding.txt.
- `\X` is a base character plus following marks, not the full grapheme cluster rules.
- The step limit counts backtracking steps, not PCRE2 `match()` calls, and is calibrated on three patterns only.

## Other functions in this batch

ASC and DBCS return the text unchanged (no double-byte editing language on the oracle machine); PHONETIC is always
`#N/A`; JIS is `#NAME?` in the oracle Excel and is not registered. BAHTTEXT and ENCODEURL are exact on the corpus.
TEXT: `aaa`/`aaaa` are the `ddd`/`dddd` names of the locale; `b` is the Gregorian year under a `[$-LCID]` prefix;
`[DBNum1-3]` numerals under 411/412/404/804 (tens notation for unpadded date parts); Japanese eras under `[$-411]`.
Left: `[DBNum1]General` (Chinese number words) and `[$-F800]`/`[$-F400]` (Windows regional settings).
