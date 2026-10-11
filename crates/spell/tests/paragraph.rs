//! Paragraph tokenizer behaviour: offsets, skipped token classes and options.

use open365_spell::{Checker, TokenizerOptions, find_words};

fn checker() -> Checker {
    Checker::from_strs(
        "SET UTF-8\nWORDCHARS '\nTRY esianrtolcdugmphbyfvkwz\n",
        "9\nthe\nquick\nbrown\nfox\njumps\ndon't\nnaïve\nwell\nknown\n",
    )
    .expect("fixture parses")
}

fn flagged(text: &str, options: &TokenizerOptions) -> Vec<String> {
    checker()
        .find_misspellings(text, options)
        .into_iter()
        .map(|m| m.word)
        .collect()
}

#[test]
fn reports_char_and_utf16_offsets() {
    let text = "the 😀 quik fox"; // the emoji is 1 char but 2 UTF-16 units and 4 bytes
    let found = checker().find_misspellings(text, &TokenizerOptions::default());
    assert_eq!(found.len(), 1);
    let m = &found[0];
    assert_eq!(m.word, "quik");
    assert_eq!(m.char_range, 6..10);
    assert_eq!(m.utf16_range, 7..11);
    assert_eq!(m.byte_range, 9..13);
    assert_eq!(&text[m.byte_range.clone()], "quik");
}

#[test]
fn offsets_count_combining_marks_and_accents() {
    // `e` + combining acute: two chars, two UTF-16 units; the word still starts after them.
    let text = "cafe\u{301} bror";
    let found = checker().find_misspellings(text, &TokenizerOptions::default());
    let words: Vec<_> = found.iter().map(|m| m.word.as_str()).collect();
    assert_eq!(words, ["cafe\u{301}", "bror"]);
    assert_eq!(found[0].char_range, 0..5);
    assert_eq!(found[1].char_range, 6..10);
    assert_eq!(found[1].utf16_range, 6..10);
}

#[test]
fn skips_urls_emails_paths_numbers_digits_and_caps() {
    let text = "see https://exmple.org/pth, mail bob@exmple.org (www.exmple.org) \
                C:\\tmp\\fiel.txt /usr/lib/zzz 3.14 mp3 NASA kwyjibo";
    assert_eq!(
        flagged(text, &TokenizerOptions::default()),
        ["see", "mail", "kwyjibo"]
    );
}

#[test]
fn options_turn_each_skip_off() {
    let base = TokenizerOptions::default();
    let digits = TokenizerOptions {
        ignore_words_with_digits: false,
        ..base.clone()
    };
    assert_eq!(flagged("mp3 fox", &digits), ["mp3"]);
    let caps = TokenizerOptions {
        ignore_uppercase: false,
        ..base.clone()
    };
    assert_eq!(flagged("NASA fox", &caps), ["NASA"]);
    let urls = TokenizerOptions {
        ignore_internet_addresses: false,
        ..base.clone()
    };
    assert!(flagged("http://exmple.org", &urls).contains(&"exmple".to_owned()));
    let min = TokenizerOptions {
        min_word_chars: 5,
        ..base
    };
    assert_eq!(flagged("xx kwyjibo", &min), ["kwyjibo"]);
}

#[test]
fn apostrophes_and_hyphens_stay_inside_words() {
    let options = TokenizerOptions::default();
    assert!(
        flagged("don't don\u{2019}t well-known 'quick' fox's", &options)
            .iter()
            .all(|w| w == "fox's")
    );
    assert_eq!(flagged("well-knwon", &options), ["well-knwon"]);
}

#[test]
fn sentence_punctuation_without_spaces_splits_words() {
    let options = TokenizerOptions::default();
    assert!(flagged("the.quick,brown;fox:the", &options).is_empty());
    assert_eq!(flagged("quik.brwn", &options), ["quik", "brwn"]);
}

#[test]
fn skips_cjk_and_punctuation_only_text() {
    assert!(
        flagged(
            "日本語のテキスト 12345 — ... !",
            &TokenizerOptions::default()
        )
        .is_empty()
    );
}

#[test]
fn find_words_is_independent_of_any_dictionary() {
    let words = find_words("Hello, wörld! x", &TokenizerOptions::default());
    let texts: Vec<_> = words.iter().map(|w| w.text).collect();
    assert_eq!(texts, ["Hello", "wörld", "x"]);
    assert_eq!(words[1].char_range, 7..12);
    assert_eq!(words[1].byte_range, 7..13);
}

#[test]
fn user_words_and_ignores_are_honoured_in_paragraphs() {
    let mut checker = checker();
    let text = "the kwyjibo and kwyjibo again";
    let options = TokenizerOptions::default();
    assert_eq!(checker.find_misspellings(text, &options).len(), 4);
    checker.ignore_word("kwyjibo");
    checker.ignore_word("and");
    checker.ignore_word("again");
    assert!(checker.find_misspellings(text, &options).is_empty());
}
