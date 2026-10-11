//! Ruler edits through the command layer: indents and tab stops, undo/redo, save and reopen.

use std::path::PathBuf;

use open365_commands::{Command, CommandLog, EffectClass, TabAlign, Target};

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

fn open(rel: &str) -> Target {
    Target::open_path(&fixture(rel)).expect("open docx")
}

/// (left, first line, right) in points, style resolved.
fn indents(t: &Target, para: usize) -> (f32, f32, f32) {
    let doc = t.docx().expect("docx");
    let p = doc.paragraph(para).expect("paragraph");
    let e = doc.styles().expect("styles").effective_para_props(p);
    (e.indent_left_pt, e.first_line_pt, e.indent_right_pt)
}

/// Effective tab stops as (position in points, kind).
fn tabs(t: &Target, para: usize) -> Vec<(f32, open365_docx::TabKind)> {
    let doc = t.docx().expect("docx");
    let p = doc.paragraph(para).expect("paragraph");
    doc.styles()
        .expect("styles")
        .paragraph_flow_props(p)
        .tabs
        .iter()
        .map(|s| (s.pos_pt(), s.kind))
        .collect()
}

fn reopened(t: &mut Target) -> Target {
    Target::open_docx(&t.snapshot_bytes().expect("save")).expect("reopen")
}

#[test]
fn set_indents_writes_w_ind_and_undoes() {
    let mut t = open("docx/plain-paragraphs.docx");
    let before = indents(&t, 0);
    let out = t
        .apply(Command::DocxSetIndents {
            para: 0,
            left_pt: 36.0,
            first_line_pt: -18.0,
            right_pt: 7.5,
        })
        .expect("indents");
    assert_eq!(out.effect, EffectClass::Edit);
    assert_eq!(indents(&t, 0), (36.0, -18.0, 7.5));
    assert_eq!(indents(&reopened(&mut t), 0), (36.0, -18.0, 7.5));

    t.undo().expect("undo");
    assert_eq!(indents(&t, 0), before);
    t.redo().expect("redo");
    assert_eq!(indents(&t, 0), (36.0, -18.0, 7.5));

    // A positive first-line indent replaces the hanging one.
    t.apply(Command::DocxSetIndents {
        para: 0,
        left_pt: 0.0,
        first_line_pt: 21.0,
        right_pt: 0.0,
    })
    .expect("first line");
    assert_eq!(indents(&t, 0), (0.0, 21.0, 0.0));
    let xml = String::from_utf8(t.docx().expect("docx").serialize_main()).expect("utf8");
    assert!(xml.contains("w:firstLine=\"420\""), "{xml}");
    assert!(!xml.contains("w:hanging"), "{xml}");
}

#[test]
fn bad_lengths_and_paragraphs_are_rejected_without_a_trace() {
    let mut t = open("docx/plain-paragraphs.docx");
    assert!(
        t.apply(Command::DocxSetIndents {
            para: 0,
            left_pt: f64::NAN,
            first_line_pt: 0.0,
            right_pt: 0.0,
        })
        .is_err()
    );
    assert!(
        t.apply(Command::DocxSetIndents {
            para: 9999,
            left_pt: 1.0,
            first_line_pt: 0.0,
            right_pt: 0.0,
        })
        .is_err()
    );
    assert!(!t.can_undo());
    assert!(t.applied_log().commands().is_empty());
}

#[test]
fn add_move_and_clear_a_direct_tab() {
    let mut t = open("docx/plain-paragraphs.docx");
    assert!(tabs(&t, 0).is_empty());
    t.apply(Command::DocxAddTab {
        para: 0,
        pos_pt: 288.0,
        kind: TabAlign::Right,
    })
    .expect("add");
    t.apply(Command::DocxAddTab {
        para: 0,
        pos_pt: 144.0,
        kind: TabAlign::Decimal,
    })
    .expect("add");
    let k = open365_docx::TabKind::Right;
    let d = open365_docx::TabKind::Decimal;
    assert_eq!(tabs(&t, 0), vec![(144.0, d), (288.0, k)]);

    t.apply(Command::DocxMoveTab {
        para: 0,
        from_pt: 144.0,
        to_pt: 360.0,
    })
    .expect("move");
    assert_eq!(tabs(&t, 0), vec![(288.0, k), (360.0, d)]);
    assert_eq!(tabs(&reopened(&mut t), 0), vec![(288.0, k), (360.0, d)]);

    t.apply(Command::DocxClearTab {
        para: 0,
        pos_pt: 288.0,
    })
    .expect("clear");
    assert_eq!(tabs(&t, 0), vec![(360.0, d)]);
    assert!(
        t.apply(Command::DocxClearTab {
            para: 0,
            pos_pt: 288.0,
        })
        .is_err(),
        "there is no stop left there"
    );

    for _ in 0..4 {
        t.undo().expect("undo");
    }
    assert!(tabs(&t, 0).is_empty());
    t.redo().expect("redo");
    assert_eq!(tabs(&t, 0), vec![(288.0, k)]);
}

#[test]
fn clearing_a_style_tab_writes_a_clear_entry() {
    let mut t = open("docx/layout-tabs.docx");
    let para = (0..t.docx().expect("docx").paragraph_count())
        .find(|&p| !tabs(&t, p).is_empty())
        .expect("a paragraph with custom tab stops");
    let before = tabs(&t, para);
    let (pos, _) = before[0];
    t.apply(Command::DocxClearTab {
        para,
        pos_pt: f64::from(pos),
    })
    .expect("clear");
    let after = tabs(&t, para);
    assert_eq!(after.len(), before.len() - 1);
    assert!(after.iter().all(|(p, _)| (*p - pos).abs() > 0.01));
    assert_eq!(tabs(&reopened(&mut t), para), after);
    t.undo().expect("undo");
    assert_eq!(tabs(&t, para), before);
}

#[test]
fn ruler_commands_replay_from_the_log() {
    let mut t = open("docx/plain-paragraphs.docx");
    t.apply(Command::DocxSetIndents {
        para: 1,
        left_pt: 18.0,
        first_line_pt: 0.0,
        right_pt: 0.0,
    })
    .expect("indents");
    t.apply(Command::DocxAddTab {
        para: 1,
        pos_pt: 100.0,
        kind: TabAlign::Center,
    })
    .expect("tab");
    let json = t.applied_log().to_json().expect("json");
    assert!(json.contains("docx_set_indents") || json.contains("\"command\""));
    let log = CommandLog::from_json(&json).expect("parse");
    let mut fresh = open("docx/plain-paragraphs.docx");
    for c in log.commands() {
        fresh.apply(c.clone()).expect("replay");
    }
    assert_eq!(indents(&fresh, 1), indents(&t, 1));
    assert_eq!(tabs(&fresh, 1), tabs(&t, 1));
}
