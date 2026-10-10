use std::path::PathBuf;

use open365_commands::{CellEntry, Command, CommandError, CommandLog, EffectClass, Target, apply};
use open365_xlsx::CellValue;

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("open365-commands-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir.join(name)
}

fn plain() -> Target {
    Target::open_path(&fixture("docx/plain-paragraphs.docx")).expect("open docx")
}

fn basic() -> Target {
    Target::open_path(&fixture("xlsx/formulas-basic.xlsx")).expect("open xlsx")
}

fn number(t: &Target, row: u32, col: u32) -> f64 {
    match t.xlsx().and_then(|w| w.cell(0, row, col)).map(|c| &c.value) {
        Some(CellValue::Number(n)) => *n,
        other => panic!("expected number at {row},{col}, got {other:?}"),
    }
}

fn text(t: &Target, row: u32, col: u32) -> String {
    t.xlsx()
        .and_then(|w| w.cell(0, row, col))
        .and_then(|c| c.value.as_text().map(str::to_owned))
        .unwrap_or_default()
}

fn set_number(sheet: usize, row: u32, col: u32, n: f64) -> Command {
    Command::XlsxSetCellValue {
        sheet,
        row,
        col,
        value: CellEntry::Number(n),
    }
}

#[test]
fn docx_insert_text_changes_the_paragraph() {
    let mut t = plain();
    let out = apply(
        &mut t,
        Command::DocxInsertText {
            para: 0,
            offset: 5,
            text: "XY".into(),
        },
    )
    .expect("insert");
    assert_eq!(out.effect, EffectClass::Edit);
    assert_eq!(
        t.docx().and_then(|d| d.paragraph_text(0)).as_deref(),
        Some("FirstXY paragraph of plain text.")
    );
    assert!(t.is_dirty());
}

#[test]
fn docx_set_bold_marks_the_range() {
    let mut t = plain();
    apply(
        &mut t,
        Command::DocxSetBold {
            para: 1,
            start: 0,
            end: 6,
            bold: true,
        },
    )
    .expect("bold");
    let doc = t.docx().expect("docx");
    let p = doc.paragraph(1).expect("paragraph");
    let bold_runs: Vec<String> = p
        .content
        .iter()
        .filter_map(|i| match i {
            open365_docx::Inline::Run(r) if r.props.bold() == Some(true) => Some(r.text()),
            _ => None,
        })
        .collect();
    assert_eq!(bold_runs, vec!["Second".to_owned()]);
}

#[test]
fn docx_errors_leave_the_document_and_log_untouched() {
    let mut t = plain();
    let err = apply(
        &mut t,
        Command::DocxInsertText {
            para: 99,
            offset: 0,
            text: "x".into(),
        },
    )
    .expect_err("out of range");
    assert!(matches!(err, CommandError::Docx(_)));
    assert!(t.audit_log().commands().is_empty());
    assert!(!t.can_undo());
    assert!(!t.is_dirty());
}

#[test]
fn spreadsheet_command_on_a_document_is_refused() {
    let mut t = plain();
    let err = apply(&mut t, set_number(0, 1, 1, 1.0)).expect_err("wrong target");
    assert!(matches!(err, CommandError::WrongTarget { .. }));
}

#[test]
fn docx_undo_and_redo() {
    let mut t = plain();
    let original = t.docx().and_then(|d| d.paragraph_text(2));
    let ins = |text: &str| Command::DocxInsertText {
        para: 2,
        offset: 0,
        text: text.into(),
    };
    apply(&mut t, ins("A ")).expect("a");
    apply(&mut t, ins("B ")).expect("b");
    let rev = t.revision();
    t.undo().expect("undo b");
    assert!(t.revision() > rev);
    assert_eq!(
        t.docx().and_then(|d| d.paragraph_text(2)).as_deref(),
        Some("A Third paragraph.")
    );
    t.undo().expect("undo a");
    assert_eq!(t.docx().and_then(|d| d.paragraph_text(2)), original);
    assert!(!t.is_dirty());
    assert!(matches!(t.undo(), Err(CommandError::NothingToUndo)));
    t.redo().expect("redo a");
    t.redo().expect("redo b");
    assert_eq!(
        t.docx().and_then(|d| d.paragraph_text(2)).as_deref(),
        Some("B A Third paragraph.")
    );
    assert!(matches!(t.redo(), Err(CommandError::NothingToRedo)));
}

#[test]
fn docx_undo_of_bold_removes_the_bold_run() {
    let mut t = plain();
    apply(
        &mut t,
        Command::DocxSetBold {
            para: 0,
            start: 0,
            end: 5,
            bold: true,
        },
    )
    .expect("bold");
    t.undo().expect("undo");
    let p = t.docx().and_then(|d| d.paragraph(0)).expect("paragraph");
    assert!(p.content.iter().all(|i| match i {
        open365_docx::Inline::Run(r) => r.props.bold() != Some(true),
        _ => true,
    }));
}

#[test]
fn a_new_edit_clears_redo() {
    let mut t = plain();
    let ins = |text: &str| Command::DocxInsertText {
        para: 0,
        offset: 0,
        text: text.into(),
    };
    apply(&mut t, ins("a")).expect("a");
    t.undo().expect("undo");
    assert!(t.can_redo());
    apply(&mut t, ins("b")).expect("b");
    assert!(!t.can_redo());
}

#[test]
fn xlsx_value_edit_recalculates_dependents() {
    let mut t = basic();
    assert_eq!(number(&t, 1, 4), 15.0);
    let out = apply(&mut t, set_number(0, 2, 1, -10.0)).expect("set");
    // 1 + -10 + 3 + 4 + 5
    assert_eq!(number(&t, 1, 4), 3.0);
    assert_eq!(text(&t, 2, 4), "small");
    assert!(out.changed_cells.iter().any(|c| c.row == 1 && c.col == 4));
    assert!(out.changed_cells.iter().any(|c| c.row == 2 && c.col == 4));
}

#[test]
fn xlsx_text_value_and_clear() {
    let mut t = basic();
    apply(
        &mut t,
        Command::XlsxSetCellValue {
            sheet: 0,
            row: 1,
            col: 2,
            value: CellEntry::Text("renamed".into()),
        },
    )
    .expect("text");
    assert_eq!(text(&t, 1, 2), "renamed");
    // CONCAT depends on B1.
    assert_eq!(text(&t, 5, 4), "renamed-item2");
    apply(
        &mut t,
        Command::XlsxSetCellValue {
            sheet: 0,
            row: 1,
            col: 2,
            value: CellEntry::Empty,
        },
    )
    .expect("clear");
    assert!(matches!(
        t.xlsx().and_then(|w| w.cell(0, 1, 2)).map(|c| &c.value),
        Some(CellValue::Empty) | None
    ));
}

#[test]
fn xlsx_formula_edit_computes_a_cached_value() {
    let mut t = basic();
    apply(
        &mut t,
        Command::XlsxSetCellFormula {
            sheet: 0,
            row: 8,
            col: 1,
            formula: "=SUM(A1:A5)*2".into(),
        },
    )
    .expect("formula");
    assert_eq!(number(&t, 8, 1), 30.0);
    assert_eq!(
        t.xlsx().and_then(|w| w.formula_text(0, 8, 1)).as_deref(),
        Some("SUM(A1:A5)*2")
    );
    // Editing a precedent updates the new formula's cached value too.
    apply(&mut t, set_number(0, 1, 1, 11.0)).expect("a1");
    assert_eq!(number(&t, 8, 1), 50.0);
}

#[test]
fn xlsx_bad_formula_is_rejected_without_change() {
    let mut t = basic();
    let err = apply(
        &mut t,
        Command::XlsxSetCellFormula {
            sheet: 0,
            row: 8,
            col: 1,
            formula: "=SUM(".into(),
        },
    )
    .expect_err("bad formula");
    assert!(matches!(err, CommandError::Formula(_)));
    assert!(t.xlsx().and_then(|w| w.cell(0, 8, 1)).is_none());
    assert!(!t.can_undo());
}

#[test]
fn xlsx_cross_sheet_recalc() {
    let mut t = Target::open_path(&fixture("xlsx/cross-sheet.xlsx")).expect("open");
    apply(&mut t, set_number(0, 1, 1, 100.0)).expect("set");
    let wb = t.xlsx().expect("xlsx");
    let n = |s: usize| match wb.cell(s, 1, 1).map(|c| &c.value) {
        Some(CellValue::Number(n)) => *n,
        other => panic!("{other:?}"),
    };
    assert_eq!(n(1), 200.0); // Two!A1 = One!A1*2
    assert_eq!(n(2), 300.0); // Three!A1 = Two!A1 + One!A1
}

#[test]
fn xlsx_undo_redo_restores_cells_and_dependents() {
    let mut t = basic();
    apply(&mut t, set_number(0, 1, 1, 100.0)).expect("set");
    assert_eq!(number(&t, 1, 4), 114.0);
    apply(
        &mut t,
        Command::XlsxSetCellFormula {
            sheet: 0,
            row: 1,
            col: 1,
            formula: "=2+2".into(),
        },
    )
    .expect("formula over constant");
    assert_eq!(number(&t, 1, 1), 4.0);
    assert_eq!(number(&t, 1, 4), 18.0);

    t.undo().expect("undo formula");
    assert_eq!(number(&t, 1, 1), 100.0);
    assert!(t.xlsx().and_then(|w| w.formula_text(0, 1, 1)).is_none());
    assert_eq!(number(&t, 1, 4), 114.0);

    t.undo().expect("undo value");
    assert_eq!(number(&t, 1, 1), 1.0);
    assert_eq!(number(&t, 1, 4), 15.0);
    assert!(!t.is_dirty());

    t.redo().expect("redo value");
    t.redo().expect("redo formula");
    assert_eq!(number(&t, 1, 1), 4.0);
    assert_eq!(number(&t, 1, 4), 18.0);
}

#[test]
fn xlsx_undo_of_a_formula_overwrite_restores_the_formula() {
    let mut t = basic();
    apply(&mut t, set_number(0, 1, 4, 7.0)).expect("overwrite formula");
    assert!(t.xlsx().and_then(|w| w.formula_text(0, 1, 4)).is_none());
    t.undo().expect("undo");
    assert_eq!(
        t.xlsx().and_then(|w| w.formula_text(0, 1, 4)).as_deref(),
        Some("SUM(A1:A5)")
    );
    assert_eq!(number(&t, 1, 4), 15.0);
}

#[test]
fn enter_text_picks_formula_or_constant() {
    assert!(matches!(
        Command::xlsx_enter(0, 1, 1, "=A1+1"),
        Command::XlsxSetCellFormula { .. }
    ));
    assert_eq!(
        Command::xlsx_enter(0, 1, 1, " 3.5 "),
        set_number(0, 1, 1, 3.5)
    );
    assert_eq!(CellEntry::from_user_text("TRUE"), CellEntry::Bool(true));
    assert_eq!(CellEntry::from_user_text("  "), CellEntry::Empty);
    assert_eq!(
        CellEntry::from_user_text("abc"),
        CellEntry::Text("abc".into())
    );
}

#[test]
fn save_effect_depends_on_the_path() {
    let path = fixture("docx/plain-paragraphs.docx");
    let t = plain();
    let same = Command::Save { path: path.clone() };
    let other = Command::Save {
        path: scratch("elsewhere.docx"),
    };
    assert_eq!(same.effect(t.path()), EffectClass::Edit);
    assert_eq!(other.effect(t.path()), EffectClass::Export);
    assert_eq!(same.effect(None), EffectClass::Export);
    assert_eq!(
        Command::DocxInsertText {
            para: 0,
            offset: 0,
            text: String::new()
        }
        .effect(None),
        EffectClass::Edit
    );
}

#[test]
fn save_writes_a_reopenable_docx_and_clears_dirty() {
    let mut t = plain();
    apply(
        &mut t,
        Command::DocxInsertText {
            para: 0,
            offset: 0,
            text: ">> ".into(),
        },
    )
    .expect("edit");
    let path = scratch("saved.docx");
    let out = apply(&mut t, Command::Save { path: path.clone() }).expect("save");
    assert_eq!(out.effect, EffectClass::Export);
    assert_eq!(out.saved_to.as_deref(), Some(path.as_path()));
    assert!(out.bytes_written > 0);
    assert!(!t.is_dirty());
    assert_eq!(t.path(), Some(path.as_path()));

    let reopened = Target::open_path(&path).expect("reopen");
    assert_eq!(
        reopened.docx().and_then(|d| d.paragraph_text(0)).as_deref(),
        Some(">> First paragraph of plain text.")
    );
    // Saving the same path again is an overwrite, i.e. an Edit.
    let again = apply(&mut t, Command::Save { path }).expect("save again");
    assert_eq!(again.effect, EffectClass::Edit);
    // Undo after save still works and marks the document dirty.
    t.undo().expect("undo after save");
    assert!(t.is_dirty());
}

#[test]
fn save_writes_a_reopenable_xlsx_with_recalculated_values() {
    let mut t = basic();
    apply(&mut t, set_number(0, 1, 1, 10.0)).expect("edit");
    let path = scratch("saved.xlsx");
    apply(&mut t, Command::Save { path: path.clone() }).expect("save");
    let reopened = Target::open_path(&path).expect("reopen");
    assert_eq!(number(&reopened, 1, 1), 10.0);
    assert_eq!(number(&reopened, 1, 4), 24.0);
}

#[test]
fn command_log_round_trips_through_json_and_replays() {
    let mut t = basic();
    let commands = vec![
        set_number(0, 1, 1, 10.0),
        Command::XlsxSetCellValue {
            sheet: 0,
            row: 1,
            col: 2,
            value: CellEntry::Text("hi".into()),
        },
        Command::XlsxSetCellFormula {
            sheet: 0,
            row: 9,
            col: 1,
            formula: "=A1*3".into(),
        },
        Command::DocxInsertText {
            para: 0,
            offset: 1,
            text: "\u{e9}".into(),
        },
        Command::DocxSetBold {
            para: 0,
            start: 0,
            end: 2,
            bold: false,
        },
        Command::Save {
            path: PathBuf::from("out.xlsx"),
        },
    ];
    for c in &commands[..3] {
        apply(&mut t, c.clone()).expect("apply");
    }
    assert_eq!(t.audit_log().commands(), &commands[..3]);

    let json = CommandLog(commands.clone()).to_json().expect("serialise");
    assert!(json.contains("\"command\": \"xlsx_set_cell_formula\""));
    let back = CommandLog::from_json(&json).expect("parse");
    assert_eq!(back.commands(), commands.as_slice());
    assert!(CommandLog::from_json("{not json").is_err());

    // Replaying the serialised log on a fresh copy reproduces the document.
    let mut fresh = basic();
    for c in &back.commands()[..3] {
        apply(&mut fresh, c.clone()).expect("replay");
    }
    assert_eq!(number(&fresh, 9, 1), 30.0);
    assert_eq!(number(&fresh, 1, 4), number(&t, 1, 4));
    assert_eq!(t.applied_log(), CommandLog(commands[..3].to_vec()));
}

// ---- DOCX editing commands -------------------------------------------------------------

mod docx_edits {
    use super::*;
    use open365_commands::{Alignment, DocPos, RunPropsPatch};

    fn main_xml(t: &Target) -> String {
        String::from_utf8(t.docx().expect("docx").serialize_main()).expect("utf8")
    }

    fn pos(para: usize, offset: usize) -> DocPos {
        DocPos::new(para, offset)
    }

    fn paragraph(t: &Target, i: usize) -> String {
        t.docx()
            .and_then(|d| d.paragraph_text(i))
            .unwrap_or_default()
    }

    #[test]
    fn effect_classes() {
        let del = Command::DocxDeleteRange {
            start: pos(0, 0),
            end: pos(0, 3),
        };
        assert_eq!(del.effect(None), EffectClass::Destructive);
        let empty = Command::DocxDeleteRange {
            start: pos(0, 2),
            end: pos(0, 2),
        };
        assert_eq!(empty.effect(None), EffectClass::Edit);
        let replace = Command::DocxReplaceRange {
            start: pos(0, 0),
            end: pos(1, 1),
            text: "x".into(),
        };
        assert_eq!(replace.effect(None), EffectClass::Destructive);
        for edit in [
            Command::DocxSplitParagraph { pos: pos(0, 1) },
            Command::DocxSetRunProps {
                start: pos(0, 0),
                end: pos(0, 1),
                patch: RunPropsPatch::bold(true),
            },
            Command::DocxSetParagraphStyle {
                para: 0,
                style_id: "Heading1".into(),
            },
            Command::DocxSetAlignment {
                para: 0,
                alignment: Alignment::Center,
            },
        ] {
            assert_eq!(edit.effect(None), EffectClass::Edit, "{}", edit.name());
        }
    }

    #[test]
    fn new_commands_round_trip_through_json() {
        let log = CommandLog(vec![
            Command::DocxDeleteRange {
                start: pos(0, 1),
                end: pos(2, 0),
            },
            Command::DocxSetRunProps {
                start: pos(0, 0),
                end: pos(0, 4),
                patch: RunPropsPatch {
                    italic: Some(true),
                    color: Some("00FF00".into()),
                    ..RunPropsPatch::default()
                },
            },
            Command::DocxSetAlignment {
                para: 1,
                alignment: Alignment::Justify,
            },
            Command::DocxReplaceRange {
                start: pos(0, 0),
                end: pos(0, 2),
                text: "a\nb".into(),
            },
        ]);
        let json = log.to_json().expect("json");
        assert!(json.contains("docx_delete_range") && json.contains("\"para\""));
        assert_eq!(CommandLog::from_json(&json).expect("parse"), log);
    }

    #[test]
    fn delete_split_replace_and_format_work_and_undo_in_reverse() {
        let mut t = plain();
        let original = main_xml(&t);
        let count = t.docx().expect("docx").paragraph_count();
        let mut states = vec![original.clone()];
        let commands = [
            Command::DocxDeleteRange {
                start: pos(0, 5),
                end: pos(1, 3),
            },
            Command::DocxSplitParagraph { pos: pos(0, 2) },
            Command::DocxSetRunProps {
                start: pos(0, 0),
                end: pos(2, 2),
                patch: RunPropsPatch {
                    italic: Some(true),
                    underline: Some(true),
                    ..RunPropsPatch::default()
                },
            },
            Command::DocxSetParagraphStyle {
                para: 1,
                style_id: "Heading1".into(),
            },
            Command::DocxSetAlignment {
                para: 1,
                alignment: Alignment::Right,
            },
            Command::DocxReplaceRange {
                start: pos(0, 1),
                end: pos(1, 1),
                text: "Z\nQ".into(),
            },
            Command::DocxInsertText {
                para: 0,
                offset: 0,
                text: "!".into(),
            },
        ];
        for command in commands {
            apply(&mut t, command).expect("apply");
            states.push(main_xml(&t));
        }
        // Undo one by one: every intermediate state comes back exactly.
        for expected in states.iter().rev().skip(1) {
            t.undo().expect("undo");
            assert_eq!(&main_xml(&t), expected);
        }
        assert!(!t.can_undo());
        assert!(!t.is_dirty());
        assert_eq!(t.docx().expect("docx").paragraph_count(), count);
        // Redo everything, then undo it all again.
        for expected in states.iter().skip(1) {
            t.redo().expect("redo");
            assert_eq!(&main_xml(&t), expected);
        }
        while t.can_undo() {
            t.undo().expect("undo");
        }
        assert_eq!(main_xml(&t), original);
    }

    #[test]
    fn deleting_across_paragraphs_merges_them() {
        let mut t = plain();
        let first = paragraph(&t, 0);
        let second = paragraph(&t, 1);
        let out = apply(
            &mut t,
            Command::DocxDeleteRange {
                start: pos(0, 5),
                end: pos(1, 6),
            },
        )
        .expect("delete");
        assert_eq!(out.effect, EffectClass::Destructive);
        let expected: String = first
            .chars()
            .take(5)
            .chain(second.chars().skip(6))
            .collect();
        assert_eq!(paragraph(&t, 0), expected);
        assert_eq!(t.applied_log().commands().len(), 1);
    }

    #[test]
    fn a_refused_edit_changes_nothing_and_is_not_logged() {
        let mut t = plain();
        let err = apply(
            &mut t,
            Command::DocxDeleteRange {
                start: pos(0, 0),
                end: pos(99, 0),
            },
        )
        .expect_err("out of range");
        assert!(matches!(err, CommandError::Docx(_)));
        let err = apply(
            &mut t,
            Command::DocxSetParagraphStyle {
                para: 0,
                style_id: "NoSuchStyle".into(),
            },
        )
        .expect_err("unknown style");
        assert!(matches!(err, CommandError::Docx(_)));
        assert!(!t.can_undo() && !t.is_dirty());
        assert!(t.audit_log().commands().is_empty());
    }

    #[test]
    fn removed_drawings_are_reported_in_the_outcome_and_come_back_on_undo() {
        let mut t = Target::open_path(&fixture("docx/inline-image.docx")).expect("open");
        let original = main_xml(&t);
        assert!(original.contains("<w:drawing"));
        let count = t.docx().expect("docx").paragraph_count();
        let last = count - 1;
        let len = t
            .docx()
            .and_then(|d| d.paragraph(last))
            .expect("p")
            .char_len();
        // Deleting the whole document removes the drawing.
        let out = apply(
            &mut t,
            Command::DocxDeleteRange {
                start: pos(0, 0),
                end: pos(last, len),
            },
        )
        .expect("delete");
        assert!(!main_xml(&t).contains("<w:drawing"));
        assert!(
            out.notes.iter().any(|n| n.contains("drawing")),
            "{:?}",
            out.notes
        );
        t.undo().expect("undo");
        assert_eq!(main_xml(&t), original);
    }
}
