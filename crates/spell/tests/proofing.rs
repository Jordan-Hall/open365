//! The application-facing API: language selection, spans, repeated words, user words, caching and
//! the background worker. Uses the bundled English dictionaries and a tiny installed German one.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use open365_spell::{
    IssueKind, LangSpan, ProofRequest, ProofSession, ProofWorker, Proofing, bundled_tags,
};

fn installed() -> Proofing {
    let proofing = Proofing::new();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/library");
    proofing.install_dir(dir).expect("scans");
    proofing
}

fn flagged(proofing: &Proofing, text: &str, lang: &str) -> Vec<String> {
    proofing
        .check_paragraph(text, lang)
        .issues
        .into_iter()
        .map(|i| i.word)
        .collect()
}

#[test]
fn bundles_exactly_the_four_permissive_english_dictionaries() {
    assert_eq!(bundled_tags(), ["en-US", "en-GB", "en-CA", "en-AU"]);
}

#[test]
fn language_tag_picks_the_regional_dictionary() {
    let proofing = Proofing::new();
    assert!(flagged(&proofing, "colour", "en-US").contains(&"colour".to_owned()));
    assert!(flagged(&proofing, "colour", "en-GB").is_empty());
    assert!(flagged(&proofing, "color", "en-GB").contains(&"color".to_owned()));
    assert!(flagged(&proofing, "color", "en-US").is_empty());
    // Other regions use the nearest spelling convention; the match is reported as inexact.
    let nz = proofing.resolve("en_nz").expect("resolves");
    assert_eq!((nz.matched.as_str(), nz.exact), ("en-AU", false));
    let bare = proofing.resolve("en").expect("resolves");
    assert_eq!(bare.matched, "en-US");
    assert!(proofing.resolve("und").is_none());
}

#[test]
fn installed_dictionaries_cover_other_languages() {
    let proofing = installed();
    assert!(flagged(&proofing, "Schule", "de-DE").is_empty());
    assert_eq!(flagged(&proofing, "Schuel", "de-DE"), ["Schuel"]);
    let missing = proofing.check_paragraph("Bonjour tout", "fr-FR");
    assert!(missing.issues.is_empty());
    assert_eq!(missing.missing_languages, ["fr-FR"]);
    assert!(proofing.available_tags().iter().any(|t| t == "de-DE"));
}

#[test]
fn spans_switch_language_inside_a_paragraph_and_keep_utf16_offsets() {
    let proofing = installed();
    // "caf\u{e9} " is 5 bytes and 5 UTF-16 units; the emoji is 4 bytes and 2 UTF-16 units.
    let text = "colour \u{1F600} Schuel Schule";
    let split = text.find("Schuel").expect("present");
    let spans = [
        LangSpan::new(0..split, "en-US"),
        LangSpan::new(split..text.len(), "de-DE"),
    ];
    let report = proofing.check_spans(text, &spans);
    let words: Vec<&str> = report.issues.iter().map(|i| i.word.as_str()).collect();
    assert_eq!(words, ["colour", "Schuel"]);
    let schuel = &report.issues[1];
    assert_eq!(&text[schuel.byte_range.clone()], "Schuel");
    assert_eq!(schuel.utf16_range, 10..16);
    assert_eq!(schuel.char_range, 9..15);

    // noProof text is skipped.
    let spans = [
        LangSpan::no_proof(0..7),
        LangSpan::new(7..text.len(), "de-DE"),
    ];
    let words: Vec<String> = proofing
        .check_spans(text, &spans)
        .issues
        .into_iter()
        .map(|i| i.word)
        .collect();
    assert_eq!(words, ["Schuel"]);
}

#[test]
fn word_defaults_skip_uppercase_numbers_and_addresses() {
    let proofing = Proofing::new();
    let text = "NASA abc123 http://example.com/xq user@example.com C:\\temp\\xqz.txt 42 teh";
    assert_eq!(flagged(&proofing, text, "en-US"), ["teh"]);
}

#[test]
fn flags_repeated_words_like_word() {
    let proofing = Proofing::new();
    let report =
        proofing.check_paragraph("It is is a the The test, test. That that is fine.", "en-US");
    let repeats: Vec<(&str, IssueKind, usize)> = report
        .issues
        .iter()
        .map(|i| (i.word.as_str(), i.kind, i.byte_range.start))
        .collect();
    // `is is` and `the The` repeat; `test, test` has a comma between; `That that` is allowed.
    assert_eq!(
        repeats,
        [
            ("is", IssueKind::RepeatedWord, 6),
            ("The", IssueKind::RepeatedWord, 15)
        ]
    );
    let mut options = proofing.options();
    options.flag_repeated_words = false;
    proofing.set_options(options);
    assert!(proofing.check_paragraph("is is", "en-US").issues.is_empty());
}

#[test]
fn suggestions_are_capped_and_use_the_user_dictionary() {
    let proofing = Proofing::new();
    let suggestions = proofing.suggest("recieve", "en-US");
    assert!(suggestions.len() <= 5);
    assert_eq!(suggestions.first().map(String::as_str), Some("receive"));
    assert!(proofing.suggest("receive", "en-US").is_empty());
    assert!(proofing.suggest("anything", "xx-YY").is_empty());

    assert!(proofing.add_to_dictionary("Open365"));
    assert!(
        proofing
            .suggest("Open356", "en-US")
            .contains(&"Open365".to_owned())
    );
}

#[test]
fn add_to_dictionary_and_ignore_all_apply_to_every_language_and_bump_the_generation() {
    let proofing = installed();
    let before = proofing.generation();
    assert_eq!(flagged(&proofing, "Zorblat", "en-AU"), ["Zorblat"]);
    assert!(proofing.ignore_all("Zorblat"));
    assert!(proofing.generation() > before);
    assert!(flagged(&proofing, "Zorblat and ZORBLAT", "en-AU").is_empty());
    assert!(flagged(&proofing, "Zorblat", "de-DE").is_empty());
    assert!(!proofing.ignore_all("Zorblat"), "already ignored");

    assert!(proofing.add_to_dictionary("frobnicate"));
    assert!(flagged(&proofing, "Frobnicate and FROBNICATE", "en-US").is_empty());
    assert!(proofing.remove_from_dictionary("frobnicate"));
    assert_eq!(flagged(&proofing, "frobnicate", "en-US"), ["frobnicate"]);
    proofing.reset_ignored();
    assert_eq!(flagged(&proofing, "Zorblat", "en-AU"), ["Zorblat"]);
}

#[test]
fn user_dictionary_round_trips_through_a_file() {
    let proofing = Proofing::new();
    assert!(proofing.add_to_dictionary("Open365"));
    assert!(proofing.ignore_all("Qzzx"));
    let dir = std::env::temp_dir().join(format!("open365-spell-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("custom.dic");
    proofing.save_user_dictionary(&path).expect("saves");
    let other = Proofing::new();
    other.load_user_dictionary(&path).expect("loads");
    assert!(other.check_word("Open365", "en-US"));
    assert!(
        !other.check_word("Qzzx", "en-US"),
        "Ignore All is not persisted"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn session_returns_the_same_result_until_something_changes() {
    let mut session = ProofSession::new(Proofing::new());
    let first = session.check(1, "A mispelled word.", "en-US");
    assert_eq!(first.issues.len(), 1);
    let again = session.check(1, "A mispelled word.", "en-US");
    assert!(Arc::ptr_eq(&first, &again));

    // An edit elsewhere in the paragraph leaves the verdict, and the shared result, identical only
    // when the ranges are identical; here they move, so the result is new.
    let edited = session.check(1, "A mispelled word again.", "en-US");
    assert_eq!(edited.issues[0].word, "mispelled");
    let fixed = session.check(1, "A misspelled word.", "en-US");
    assert!(fixed.issues.is_empty());

    session.proofing().ignore_all("mispelled");
    let after = session.check(1, "A mispelled word.", "en-US");
    assert!(
        after.issues.is_empty(),
        "generation change invalidates the cache"
    );
    session.forget(1);
    assert!(session.is_empty());
}

#[test]
fn worker_checks_off_thread_and_coalesces_stale_requests() {
    let notified = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&notified);
    let worker = ProofWorker::spawn_with_notify(Proofing::new(), move || {
        counter.fetch_add(1, Ordering::SeqCst);
    })
    .expect("spawns");
    let request = |id, revision, text: &str| ProofRequest {
        id,
        revision,
        text: text.to_owned(),
        spans: vec![LangSpan::new(0..text.len(), "en-US")],
    };
    assert!(worker.submit(request(7, 1, "teh")));
    let first = worker
        .recv_timeout(Duration::from_secs(60))
        .expect("answered");
    assert_eq!((first.id, first.revision), (7, 1));
    assert_eq!(first.report.issues[0].word, "teh");

    for revision in 2..=40 {
        assert!(worker.submit(request(7, revision, "the cat sat")));
    }
    assert!(worker.submit(request(8, 1, "dogg")));
    let mut seen = Vec::new();
    while let Some(response) = worker.recv_timeout(Duration::from_secs(5)) {
        seen.push((response.id, response.revision, response.report.issues.len()));
        if response.id == 8 {
            break;
        }
    }
    // The newest revision of paragraph 7 is always answered; older queued ones may be skipped.
    assert!(seen.contains(&(7, 40, 0)), "{seen:?}");
    assert!(seen.contains(&(8, 1, 1)), "{seen:?}");
    assert!(notified.load(Ordering::SeqCst) >= 3);
    drop(worker);
}

#[test]
fn proofing_is_shareable_between_threads() {
    let proofing = Proofing::new();
    proofing.preload("en-GB").expect("loads");
    std::thread::scope(|scope| {
        for i in 0..4 {
            let proofing = proofing.clone();
            scope.spawn(move || {
                let text = format!("colour{} favourite", ["a", "b", "c", "d"][i]);
                assert_eq!(proofing.check_paragraph(&text, "en-GB").issues.len(), 1);
            });
        }
    });
}
