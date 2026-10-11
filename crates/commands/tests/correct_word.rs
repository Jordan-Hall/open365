//! Right-click spelling correction through the command layer: replace the flagged word, fail closed on stale
//! text, one undo step.

use std::path::PathBuf;

use open365_commands::{Command, CommandError, EffectClass, Target};

fn open() -> Target {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/docx/plain-paragraphs.docx");
    Target::open_path(&path).expect("open docx")
}

fn text(t: &Target) -> String {
    t.docx()
        .expect("docx")
        .paragraph_text(0)
        .expect("paragraph")
}

#[test]
fn the_flagged_word_is_replaced_and_undone_in_one_step() {
    let mut t = open();
    let original = text(&t);
    let word: String = original.chars().take(3).collect();
    let cmd = Command::DocxCorrectWord {
        para: 0,
        start: 0,
        end: 3,
        expected: word.clone(),
        replacement: "Xyzzy".to_owned(),
    };
    assert_eq!(cmd.effect(None), EffectClass::Destructive);
    assert_eq!(cmd.name(), "docx_correct_word");
    t.apply(cmd).expect("correct");
    assert!(text(&t).starts_with("Xyzzy"), "{}", text(&t));
    t.undo().expect("undo");
    assert_eq!(text(&t), original);
}

#[test]
fn a_stale_word_changes_nothing() {
    let mut t = open();
    let original = text(&t);
    for (start, end, expected, replacement) in [
        (0, 3, "zzz", "Xyzzy"),
        (0, 0, "", "Xyzzy"),
        (0, 3, "", "Xyzzy"),
        (500, 503, "abc", "Xyzzy"),
    ] {
        let err = t
            .apply(Command::DocxCorrectWord {
                para: 0,
                start,
                end,
                expected: expected.to_owned(),
                replacement: replacement.to_owned(),
            })
            .expect_err("stale");
        assert!(matches!(err, CommandError::Proofing(_)), "{err}");
    }
    let err = t
        .apply(Command::DocxCorrectWord {
            para: 0,
            start: 0,
            end: 3,
            expected: original.chars().take(3).collect(),
            replacement: "a\nb".to_owned(),
        })
        .expect_err("line break");
    assert!(matches!(err, CommandError::Proofing(_)));
    assert_eq!(text(&t), original);
}
