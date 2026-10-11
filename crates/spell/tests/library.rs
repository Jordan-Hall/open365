//! Dictionary discovery by BCP-47 tag, fallback and the user dictionary file format.

use std::path::PathBuf;

use open365_spell::{Checker, DictionaryLibrary, SpellError, UserDictionary};

fn library_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/library")
}

fn matched(library: &DictionaryLibrary, tag: &str) -> Option<(String, bool)> {
    library.resolve(tag).map(|(_, r)| (r.matched, r.exact))
}

#[test]
fn lists_flat_and_folder_layouts_and_ignores_other_files() {
    let library = DictionaryLibrary::open(library_dir()).expect("opens");
    let tags: Vec<_> = library
        .dictionaries()
        .iter()
        .map(|d| d.tag.as_str())
        .collect();
    assert_eq!(tags, ["de-DE", "en", "en-GB", "pt-BR", "sr-Latn"]);
}

#[test]
fn resolves_exact_tags_in_any_spelling() {
    let library = DictionaryLibrary::open(library_dir()).expect("opens");
    for tag in ["en-GB", "en_GB", "EN-gb", "en_gb"] {
        assert_eq!(
            matched(&library, tag),
            Some(("en-GB".to_owned(), true)),
            "{tag}"
        );
    }
    assert_eq!(
        matched(&library, "sr-Latn"),
        Some(("sr-Latn".to_owned(), true))
    );
}

#[test]
fn falls_back_from_region_to_language() {
    let library = DictionaryLibrary::open(library_dir()).expect("opens");
    assert_eq!(matched(&library, "en-AU"), Some(("en".to_owned(), false)));
    assert_eq!(matched(&library, "en-US"), Some(("en".to_owned(), false)));
    assert_eq!(
        matched(&library, "sr-Latn-RS"),
        Some(("sr-Latn".to_owned(), false))
    );
}

#[test]
fn bare_language_picks_a_home_region_variant() {
    let library = DictionaryLibrary::open(library_dir()).expect("opens");
    assert_eq!(matched(&library, "de"), Some(("de-DE".to_owned(), false)));
    assert_eq!(matched(&library, "pt"), Some(("pt-BR".to_owned(), false)));
    assert_eq!(
        matched(&library, "de-AT"),
        Some(("de-DE".to_owned(), false))
    );
}

#[test]
fn unknown_languages_are_an_error() {
    let library = DictionaryLibrary::open(library_dir()).expect("opens");
    assert!(library.resolve("xx-YY").is_none());
    assert!(library.resolve("").is_none());
    assert!(matches!(
        Checker::from_library(&library, "xx"),
        Err(SpellError::NoDictionary { .. })
    ));
    assert!(matches!(
        DictionaryLibrary::open(library_dir().join("missing")),
        Err(SpellError::Io { .. })
    ));
}

#[test]
fn checker_loads_the_resolved_dictionary() {
    let gb = Checker::open(library_dir(), "en-GB").expect("loads");
    assert!(gb.check("colour") && !gb.check("color"));
    assert_eq!(gb.resolved().map(|r| r.exact), Some(true));

    let au = Checker::open(library_dir(), "en-AU").expect("falls back");
    assert!(au.check("color") && !au.check("colour"));
    assert_eq!(
        au.resolved().map(|r| (r.matched.as_str(), r.exact)),
        Some(("en", false))
    );

    let sr = Checker::open(library_dir(), "sr-Latn").expect("folder layout");
    assert!(sr.check("škola"));
}

#[test]
fn user_dictionary_round_trips_through_text_and_files() {
    let mut user = UserDictionary::new();
    assert!(user.add("Zorblat"));
    assert!(user.add("kwyjibo"));
    assert!(!user.add("kwyjibo"), "duplicates are not re-added");
    assert!(!user.add("two words") && !user.add("") && !user.add("tab\tbed"));
    user.ignore("session");

    let text = user.to_text();
    assert_eq!(text, "Zorblat\nkwyjibo\n");
    let loaded = UserDictionary::from_text(&format!("# comment\n\u{feff}{text}\n"));
    assert_eq!(loaded.words().collect::<Vec<_>>(), ["Zorblat", "kwyjibo"]);

    let path = std::env::temp_dir().join(format!("open365-spell-user-{}.txt", std::process::id()));
    user.save(&path).expect("saves");
    let reread = UserDictionary::load(&path).expect("loads");
    std::fs::remove_file(&path).expect("cleanup");
    assert_eq!(reread.len(), 2);
    assert!(
        !reread.contains("session"),
        "ignored words are session-only"
    );
    assert!(UserDictionary::load(&path).is_err());
}

#[test]
fn user_dictionary_case_rules_follow_word() {
    let mut user = UserDictionary::new();
    user.add("kwyjibo");
    user.add("Zorblat");
    assert!(user.contains("kwyjibo") && user.contains("Kwyjibo") && user.contains("KWYJIBO"));
    assert!(user.contains("Zorblat") && user.contains("ZORBLAT"));
    assert!(!user.contains("zorblat") && !user.contains("zORBLAT"));
    assert!(user.remove("kwyjibo") && !user.contains("kwyjibo"));
}
