//! Replace one match and section margins through the command layer: one undo step each,
//! redo, save and reopen.

use std::path::PathBuf;

use open365_commands::{Command, CommandLog, EffectClass, ReplaceOne, SectionMargins, Target};
use open365_docx::{DocMatch, FindOptions};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

fn open() -> Target {
    Target::open_path(&fixture("docx/plain-paragraphs.docx")).expect("open docx")
}

fn texts(t: &Target) -> Vec<String> {
    let doc = t.docx().expect("docx");
    (0..doc.paragraph_count())
        .map(|i| doc.paragraph_text(i).expect("paragraph"))
        .collect()
}

fn reopened(t: &mut Target) -> Target {
    Target::open_docx(&t.snapshot_bytes().expect("save")).expect("reopen")
}

fn opts(find: &str) -> FindOptions {
    FindOptions {
        pattern: find.to_owned(),
        match_case: true,
        ..FindOptions::default()
    }
}

/// The first two characters of paragraph 0 and the matches for them.
fn first_match(t: &Target) -> (String, Vec<DocMatch>) {
    let word: String = texts(t)[0].chars().take(2).collect();
    let all = t
        .docx()
        .expect("docx")
        .find_all(&opts(&word))
        .expect("find");
    assert!(!all.is_empty());
    (word, all)
}

#[test]
fn replace_one_changes_only_that_match_and_is_one_undo_step() {
    let mut t = open();
    let before = texts(&t);
    let (word, all) = first_match(&t);
    let m = all[0];
    let op = ReplaceOne::new(&opts(&word), "<^&>", &m);
    let out = t.apply(Command::DocxReplaceOne { op }).expect("replace");
    assert_eq!(out.effect, EffectClass::Destructive);
    assert_eq!(out.replaced, 1);
    let after = texts(&t);
    assert_eq!(after[0], format!("<{word}>{}", &before[0][word.len()..]));
    // Every other match is untouched.
    let remaining = t.docx().unwrap().find_all(&opts(&word)).unwrap().len();
    assert_eq!(remaining, all.len());
    assert_eq!(texts(&reopened(&mut t)), after);

    t.undo().expect("undo");
    assert_eq!(texts(&t), before);
    t.redo().expect("redo");
    assert_eq!(texts(&t), after);
}

#[test]
fn replace_one_with_caret_p_splits_and_undoes() {
    let mut t = open();
    let before = texts(&t);
    let (word, all) = first_match(&t);
    let op = ReplaceOne::new(&opts(&word), "A^pB", &all[0]);
    let caret = open365_commands::search_ops::caret_after(t.docx().unwrap(), &op).expect("caret");
    t.apply(Command::DocxReplaceOne { op }).expect("replace");
    let after = texts(&t);
    assert_eq!(after.len(), before.len() + 1);
    assert_eq!(after[0], "A");
    assert!(after[1].starts_with('B'));
    assert_eq!((caret.para, caret.offset), (1, 1));

    t.undo().expect("undo");
    assert_eq!(texts(&t), before);
}

#[test]
fn replace_one_with_a_line_break_and_wildcard_groups() {
    let mut t = open();
    let (word, all) = first_match(&t);
    let op = ReplaceOne::new(&opts(&word), "x^ly", &all[0]);
    t.apply(Command::DocxReplaceOne { op }).expect("replace");
    assert!(texts(&t)[0].starts_with("x\ny"));

    let mut t = open();
    let w = FindOptions {
        pattern: "(?)(?)".to_owned(),
        wildcards: true,
        ..FindOptions::default()
    };
    let m = t.docx().unwrap().find_all(&w).unwrap()[0];
    let before = texts(&t)[0].clone();
    let chars: Vec<char> = before.chars().collect();
    t.apply(Command::DocxReplaceOne {
        op: ReplaceOne::new(&w, "\\2\\1", &m),
    })
    .expect("swap");
    let after: Vec<char> = texts(&t)[0].chars().collect();
    assert_eq!((after[0], after[1]), (chars[1], chars[0]));
}

#[test]
fn replace_one_stale_match_and_bad_codes_change_nothing() {
    let mut t = open();
    let (word, all) = first_match(&t);
    let mut op = ReplaceOne::new(&opts(&word), "z", &all[0]);
    op.start += 1;
    assert!(t.apply(Command::DocxReplaceOne { op: op.clone() }).is_err());
    op.start -= 1;
    op.replacement = "^q".to_owned();
    assert!(t.apply(Command::DocxReplaceOne { op: op.clone() }).is_err());
    op.replacement = "\\1".to_owned();
    op.wildcards = true;
    assert!(t.apply(Command::DocxReplaceOne { op }).is_err());
    assert!(!t.can_undo());
    assert!(t.applied_log().commands().is_empty());
}

#[test]
fn replace_one_replays_from_the_log() {
    let mut t = open();
    let (word, all) = first_match(&t);
    t.apply(Command::DocxReplaceOne {
        op: ReplaceOne::new(&opts(&word), "Q", &all[0]),
    })
    .expect("replace");
    let json = t.applied_log().to_json().expect("json");
    assert!(json.contains("docx_replace_one"));
    let log: CommandLog = CommandLog::from_json(&json).expect("parse");
    let mut again = open();
    for c in log.commands() {
        again.apply(c.clone()).expect("replay");
    }
    assert_eq!(texts(&again), texts(&t));
}

fn margins(t: &Target) -> (i32, i32, i32, i32, i32, i32, i32) {
    let m = t.docx().expect("docx target").sections()[0].margins;
    (
        m.top_twips,
        m.bottom_twips,
        m.left_twips,
        m.right_twips,
        m.gutter_twips,
        m.header_twips,
        m.footer_twips,
    )
}

#[test]
fn section_margins_are_one_undo_step_and_survive_save() {
    let mut t = open();
    let before = margins(&t);
    let out = t
        .apply(Command::DocxSectionMargins {
            op: SectionMargins {
                section: 0,
                top_pt: Some(36.0),
                bottom_pt: Some(40.0),
                left_pt: Some(54.0),
                right_pt: Some(50.0),
                gutter_pt: Some(10.0),
                header_pt: Some(20.0),
                footer_pt: Some(22.0),
            },
        })
        .expect("margins");
    assert_eq!(out.effect, EffectClass::Edit);
    assert_eq!(margins(&t), (720, 800, 1080, 1000, 200, 400, 440));
    assert_eq!(
        margins(&reopened(&mut t)),
        (720, 800, 1080, 1000, 200, 400, 440)
    );

    t.undo().expect("undo");
    assert_eq!(margins(&t), before);
    t.redo().expect("redo");
    assert_eq!(margins(&t).2, 1080);

    // A ruler drag moves one side and leaves the rest.
    t.apply(Command::DocxSectionMargins {
        op: SectionMargins {
            section: 0,
            left_pt: Some(72.0),
            ..SectionMargins::default()
        },
    })
    .expect("drag");
    assert_eq!(margins(&t), (720, 800, 1440, 1000, 200, 400, 440));
}

#[test]
fn bad_margins_are_rejected_without_a_trace() {
    let mut t = open();
    for op in [
        SectionMargins {
            left_pt: Some(f64::NAN),
            ..SectionMargins::default()
        },
        SectionMargins {
            left_pt: Some(-1.0),
            ..SectionMargins::default()
        },
        SectionMargins {
            left_pt: Some(500.0),
            right_pt: Some(500.0),
            ..SectionMargins::default()
        },
        SectionMargins {
            section: 99,
            ..SectionMargins::default()
        },
    ] {
        assert!(t.apply(Command::DocxSectionMargins { op }).is_err());
    }
    assert!(!t.can_undo());
    assert!(t.applied_log().commands().is_empty());
}
