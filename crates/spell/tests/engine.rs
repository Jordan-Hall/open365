//! Affix, case, compound, conversion and suggestion behaviour against hand-made fixtures.

use std::path::PathBuf;

use open365_spell::{Checker, Dictionary, SuggestOptions};

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn load(stem: &str) -> Dictionary {
    Dictionary::from_files(&data(&format!("{stem}.aff")), &data(&format!("{stem}.dic")))
        .expect("fixture parses")
}

fn suggest(dict: &Dictionary, word: &str) -> Vec<String> {
    dict.suggest(word, &SuggestOptions::default())
}

#[test]
fn plain_words_and_case_handling() {
    let d = load("basic");
    for word in [
        "the", "The", "THE", "quick", "Quick", "QUICK", "Paris", "PARIS",
    ] {
        assert!(d.spell(word), "{word} should be accepted");
    }
    // Proper nouns do not match their lower-case form; mixed case never matches.
    for word in ["paris", "tHe", "qUick", "thee", ""] {
        assert_eq!(d.spell(word), word.is_empty(), "{word:?}");
    }
}

#[test]
fn suffixes_prefixes_and_cross_products() {
    let d = load("basic");
    for word in [
        "dogs",
        "foxes",
        "jumped",
        "jumping",
        "jumps",
        "receives",
        "received",
        "receiving",
        "happies",
        "unhappy",
        "unhappies",
        "unlock",
        "unlocked",
        "unlocking",
        "unlocks",
        "Londons",
        "LONDONS",
    ] {
        assert!(d.spell(word), "{word} should be accepted");
    }
    for word in ["dogss", "happied", "unwalk", "jumpped", "receiveed", "foxs"] {
        assert!(!d.spell(word), "{word} should be rejected");
    }
}

#[test]
fn continuation_flags_allow_two_suffixes() {
    let d = load("basic");
    assert!(d.spell("walker"));
    assert!(d.spell("walkers"), "walk + R + S via the continuation flag");
    assert!(!d.spell("walkerd"), "D is not a continuation of R");
    assert!(!d.spell("walkerss"));
}

#[test]
fn special_flags() {
    let d = load("basic");
    // NEEDAFFIX: `foo` is only valid with an affix.
    assert!(!d.spell("foo"));
    assert!(d.spell("foos"));
    // KEEPCASE: only the exact spelling.
    assert!(d.spell("nasa"));
    assert!(!d.spell("Nasa"));
    assert!(!d.spell("NASA"));
    // FORBIDDENWORD beats the affixed form that would otherwise be valid.
    assert!(d.spell("color"));
    assert!(!d.spell("colors"));
    // NOSUGGEST words are valid but never offered.
    assert!(d.spell("damn"));
    assert!(!suggest(&d, "damm").contains(&"damn".to_owned()));
}

#[test]
fn iconv_normalises_typographic_apostrophes() {
    let d = load("basic");
    assert!(d.spell("don't"));
    assert!(d.spell("don\u{2019}t"));
    assert!(!d.spell("dont"));
}

#[test]
fn break_table_accepts_hyphenated_words() {
    let d = load("basic");
    assert!(d.spell("quick-brown"));
    assert!(d.spell("Quick-Brown"));
    assert!(d.spell("-quick"));
    assert!(d.spell("quick-"));
    assert!(!d.spell("quick-brwn"));
    assert!(d.spell("1,234.50"));
    assert!(d.spell("ice cream"));
}

#[test]
fn suggestions_fix_transpositions_and_keep_case() {
    let d = load("basic");
    assert_eq!(suggest(&d, "teh").first().map(String::as_str), Some("the"));
    assert_eq!(suggest(&d, "Teh").first().map(String::as_str), Some("The"));
    assert_eq!(suggest(&d, "TEH").first().map(String::as_str), Some("THE"));
    assert_eq!(
        suggest(&d, "recieve").first().map(String::as_str),
        Some("receive")
    );
    assert!(
        suggest(&d, "the").is_empty(),
        "a correct word needs no suggestion"
    );
}

#[test]
fn suggestions_use_rep_map_split_and_distance() {
    let d = load("basic");
    // REP shun -> tion is the only route: the word is three edits from `nation`.
    assert!(suggest(&d, "nashun").contains(&"nation".to_owned()));
    // MAP e -> é; `é` is not in TRY.
    assert!(suggest(&d, "cafe").contains(&"café".to_owned()));
    // Splitting into two valid words.
    assert!(suggest(&d, "alot").contains(&"a lot".to_owned()));
    // Two edits (swap + extra letter) need the distance fallback.
    assert!(suggest(&d, "qiuckk").contains(&"quick".to_owned()));
    // Keyboard neighbour (`o` next to `p`) and a missing letter.
    assert!(suggest(&d, "phpne").contains(&"phone".to_owned()));
    assert!(suggest(&d, "defintely").contains(&"definitely".to_owned()));
    // Proper noun capitalisation.
    assert_eq!(
        suggest(&d, "paris").first().map(String::as_str),
        Some("Paris")
    );
}

#[test]
fn oconv_is_applied_to_suggestions() {
    let d = load("basic");
    let found = suggest(&d, "dont");
    assert!(found.contains(&"don\u{2019}t".to_owned()), "{found:?}");
}

#[test]
fn suggestions_are_bounded_and_deterministic() {
    let d = load("basic");
    let options = SuggestOptions { max_suggestions: 2 };
    let first = d.suggest("hxllo", &options);
    assert!(first.len() <= 2);
    assert_eq!(first, d.suggest("hxllo", &options));
    assert!(first.contains(&"hello".to_owned()));
}

#[test]
fn compounds_by_flag() {
    let d = load("compound");
    for word in ["foot", "football", "footballs", "ballfoot", "sunflower"] {
        assert!(d.spell(word), "{word}");
    }
    assert!(!d.spell("footfoot"), "CHECKCOMPOUNDDUP");
    assert!(
        !d.spell("flowerssun"),
        "suffix S is not allowed inside a compound"
    );
    assert!(!d.spell("ballsfoot"), "S has no permit flag");
    assert!(
        d.spell("walkingball"),
        "COMPOUNDPERMITFLAG lets the -ing suffix sit inside"
    );
    assert!(!d.spell("room"), "ONLYINCOMPOUND");
    assert!(d.spell("bedroom"));
    assert!(
        d.spell("roombed"),
        "room may appear anywhere inside a compound"
    );
}

#[test]
fn compound_checks_triples_and_minimum_length() {
    let d = load("compound");
    assert!(!d.spell("basssound"), "CHECKCOMPOUNDTRIPLE");
    assert!(d.spell("soundbass"));
    assert!(!d.spell("footba"), "second part shorter than COMPOUNDMIN");
}

#[test]
fn compound_rules_with_quantifiers() {
    let d = load("compound");
    assert!(d.spell("redblueish"));
    assert!(d.spell("redish"));
    assert!(!d.spell("redblue"), "rule x*y needs a y part");
    assert!(!d.spell("ishred"));
    assert!(d.spell("bluish"), "plain dictionary word");
}

#[test]
fn legacy_encodings_are_decoded() {
    let aff = b"SET ISO8859-1\nTRY e\n".to_vec();
    let mut dic = b"1\ncaf".to_vec();
    dic.push(0xE9);
    dic.push(b'\n');
    let d = Dictionary::parse(&aff, &dic).expect("latin-1 parses");
    assert!(d.spell("café"));
    assert!(!d.spell("cafe"));

    // windows-1251: привет
    let aff = b"SET microsoft-cp1251\n".to_vec();
    let dic = [
        b"1\n".as_slice(),
        &[0xEF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2],
        b"\n",
    ]
    .concat();
    let d = Dictionary::parse(&aff, &dic).expect("cp1251 parses");
    assert!(d.spell("привет"));
    assert!(d.spell("Привет"));

    assert!(Dictionary::parse(b"SET ARMSCII-8\n", b"0\n").is_err());
}

#[test]
fn flag_modes_and_aliases() {
    let long = Dictionary::from_strs("FLAG long\nSFX Ab Y 1\nSFX Ab 0 s .\n", "2\ncat/Ab\ndog\n")
        .expect("long flags");
    assert!(long.spell("cats"));
    assert!(!long.spell("dogs"));

    let num = Dictionary::from_strs(
        "FLAG num\nSFX 101 Y 1\nSFX 101 0 s .\nSFX 7 Y 1\nSFX 7 0 ed .\n",
        "1\nwalk/101,7\n",
    )
    .expect("numeric flags");
    assert!(num.spell("walks") && num.spell("walked"));

    let alias = Dictionary::from_strs(
        "AF 2\nAF S\nAF D\nSFX S Y 1\nSFX S 0 s .\nSFX D Y 1\nSFX D 0 ed .\n",
        "2\nwalk/1\njump/2\n",
    )
    .expect("aliases");
    assert!(alias.spell("walks") && !alias.spell("walked"));
    assert!(alias.spell("jumped") && !alias.spell("jumps"));
}

#[test]
fn ignore_characters_and_morphological_fields_are_dropped() {
    let d = Dictionary::from_strs("IGNORE \u{AD}\n", "2\nco\u{AD}op\tpo:noun\nfoo po:verb\n")
        .expect("parses");
    assert!(d.spell("coop"));
    assert!(d.spell("co\u{AD}op"));
    assert!(d.spell("foo"));
}

#[test]
fn turkic_case_mapping_follows_the_language() {
    let d = Dictionary::from_strs("LANG tr_TR\nSET UTF-8\n", "1\nışık\n").expect("parses");
    assert!(d.spell("ışık"));
    assert!(d.spell("IŞIK"), "dotless I lower-cases to ı");
    let plain = Dictionary::from_strs("SET UTF-8\n", "1\nışık\n").expect("parses");
    assert!(!plain.spell("IŞIK"));
}

#[test]
fn checker_wraps_dictionary_and_user_words() {
    let mut checker = Checker::new(load("basic"));
    assert!(!checker.check("Zorblat"));
    assert!(checker.add_word("Zorblat"));
    assert!(checker.check("Zorblat"));
    assert!(checker.check("ZORBLAT"));
    assert!(!checker.check("zorblat"));
    assert!(checker.suggest("Zorblta").contains(&"Zorblat".to_owned()));
}
