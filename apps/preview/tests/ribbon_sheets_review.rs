//! Sheets Review tab: every wired control runs through `Shell::run_command` and changes the sheet.

#![allow(clippy::unwrap_used)]

use open365_commands::review_ops::legacy_password_hash;
use open365_preview::ribbon::Handled;
use open365_preview::sheet::SheetModel;
use open365_preview::sheet::sheet_ext::review::{PROTECTED_MESSAGE, Request, Severity};
use open365_preview::{AppKind, Preview};

mod dialogs_common;

/// The bare control opened the dialog `title`: it is modal, and Esc closes it unchanged.
fn cancels(p: &mut Preview, title: &str) {
    assert!(p.shell.dialog_open(), "{title} is open");
    assert_eq!(dialogs_common::title(p).as_deref(), Some(title));
    dialogs_common::key(p, open365_ui_core::Key::Escape);
    assert!(!p.shell.dialog_open(), "Esc closes {title}");
}

fn rig() -> Preview {
    Preview::new(AppKind::Sheets, true, 1600, 1000, 1.25)
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

fn sheet(p: &mut Preview) -> &mut SheetModel {
    p.shell.sheet_mut()
}

fn reopen(p: &mut Preview, name: &str) -> SheetModel {
    let path = std::env::temp_dir().join(name);
    sheet(p).save(&path).unwrap();
    let m = SheetModel::open(&path).unwrap();
    let _ = std::fs::remove_file(path);
    m
}

#[test]
fn legacy_hash_matches_the_known_value() {
    assert_eq!(legacy_password_hash("password"), "83AF");
}

#[test]
fn protect_sheet_refuses_edits_to_locked_cells_and_unprotects_with_the_password() {
    let mut p = rig();
    sheet(&mut p).enter(1, 1, "keep").unwrap();
    run(&mut p, "review.protect.protect-sheet=password=secret");
    assert!(sheet(&mut p).review_sheet_protection().is_some());
    assert!(sheet(&mut p).review_cell_protected(1, 1));
    let err = sheet(&mut p).review_enter(1, 1, "changed").unwrap_err();
    assert_eq!(err, PROTECTED_MESSAGE);
    assert_eq!(sheet(&mut p).source(1, 1), "keep");
    // The wrong password leaves it protected; the right one unprotects.
    run(&mut p, "review.protect.protect-sheet=unprotect=wrong");
    assert!(sheet(&mut p).review_sheet_protection().is_some());
    assert!(sheet(&mut p).status.contains("not correct"));
    run(&mut p, "review.protect.protect-sheet=unprotect=secret");
    assert!(sheet(&mut p).review_sheet_protection().is_none());
    sheet(&mut p).review_enter(1, 1, "changed").unwrap();
    assert_eq!(sheet(&mut p).source(1, 1), "changed");
    // Options: the permission list is kept in the file.
    run(
        &mut p,
        "review.protect.protect-sheet=sort;format-cells;no-select-locked",
    );
    let o = sheet(&mut p).review_sheet_protection().unwrap();
    assert!(o.sort && o.format_cells && !o.select_locked_cells && o.select_unlocked_cells);
    assert!(!o.insert_rows);
    let again = reopen(&mut p, "open365-review-protect.xlsx");
    assert_eq!(again.review_sheet_protection(), Some(o));
    assert_eq!(again.source(1, 1), "changed");
    // Undo takes the protection off again.
    assert!(sheet(&mut p).undo());
    assert!(sheet(&mut p).review_sheet_protection().is_none());
    // The bare button asks for the dialog.
    run(&mut p, "review.protect.protect-sheet");
    cancels(&mut p, "Protect Sheet");
}

#[test]
fn protect_workbook_locks_the_structure_and_toggles() {
    let mut p = rig();
    run(&mut p, "review.protect.protect-workbook=password=pw");
    assert!(sheet(&mut p).review_workbook_protected());
    let again = reopen(&mut p, "open365-review-wbprotect.xlsx");
    assert!(again.review_workbook_protected());
    run(&mut p, "review.protect.protect-workbook=unprotect=nope");
    assert!(sheet(&mut p).review_workbook_protected());
    run(&mut p, "review.protect.protect-workbook=unprotect=pw");
    assert!(!sheet(&mut p).review_workbook_protected());
    assert!(sheet(&mut p).undo());
    assert!(sheet(&mut p).review_workbook_protected());
}

#[test]
fn notes_can_be_added_edited_navigated_deleted_and_survive_a_save() {
    let mut p = rig();
    sheet(&mut p).select_cell((2, 3));
    run(&mut p, "review.notes.notes.new=Check this & that");
    sheet(&mut p).select_cell((5, 1));
    run(&mut p, "review.notes.notes=Second");
    assert_eq!(sheet(&mut p).review_notes().len(), 2);
    let n = sheet(&mut p).review_note_at(2, 3).unwrap();
    assert_eq!(n.text, "Check this & that");
    assert!(!n.author.is_empty());
    // Edit keeps the author.
    sheet(&mut p).select_cell((2, 3));
    run(&mut p, "review.notes.notes.edit=Better text");
    assert_eq!(
        sheet(&mut p).review_note_at(2, 3).unwrap().text,
        "Better text"
    );
    assert_eq!(sheet(&mut p).review_note_at(2, 3).unwrap().author, n.author);
    // Next goes to the next note in reading order, wrapping.
    run(&mut p, "review.notes.notes.next");
    assert_eq!(sheet(&mut p).active, (5, 1));
    run(&mut p, "review.notes.notes.next");
    assert_eq!(sheet(&mut p).active, (2, 3));
    run(&mut p, "review.notes.notes.previous");
    assert_eq!(sheet(&mut p).active, (5, 1));
    let again = reopen(&mut p, "open365-review-notes.xlsx");
    assert_eq!(again.review_notes().len(), 2);
    assert_eq!(again.review_note_at(2, 3).unwrap().text, "Better text");
    run(&mut p, "review.notes.notes.delete");
    assert_eq!(sheet(&mut p).review_notes().len(), 1);
    assert!(sheet(&mut p).undo());
    assert_eq!(sheet(&mut p).review_notes().len(), 2);
    sheet(&mut p).select_cell((2, 3));
    run(&mut p, "review.notes.notes.delete");
    sheet(&mut p).select_cell((5, 1));
    run(&mut p, "review.notes.notes.delete");
    assert!(sheet(&mut p).review_notes().is_empty());
    let none = reopen(&mut p, "open365-review-nonotes.xlsx");
    assert!(none.review_notes().is_empty());
}

#[test]
fn spelling_finds_misspelled_words_and_selects_the_first() {
    let mut p = rig();
    sheet(&mut p).enter(1, 1, "The quick brown fox").unwrap();
    sheet(&mut p).enter(3, 2, "recieve the the report").unwrap();
    sheet(&mut p).enter(4, 1, "=\"wrongg\"").unwrap();
    sheet(&mut p).select_cell((9, 9));
    run(&mut p, "review.proofing.spelling");
    let issues = sheet(&mut p).ext_review.spelling.clone();
    let words: Vec<(&str, bool)> = issues
        .iter()
        .map(|i| (i.word.as_str(), i.repeated))
        .collect();
    assert_eq!(words, [("recieve", false), ("the", true)]);
    assert!(issues[0].suggestions.iter().any(|s| s == "receive"));
    assert_eq!(
        sheet(&mut p).active,
        (3, 2),
        "the first mistake is selected"
    );
    assert_eq!(
        sheet(&mut p).ext_review.take_request(),
        Some(Request::Spelling)
    );
    // Change replaces the word; Ignore All drops it from the next run.
    sheet(&mut p)
        .review_spell_change(&issues[0], "receive")
        .unwrap();
    assert_eq!(sheet(&mut p).source(3, 2), "receive the the report");
    sheet(&mut p).review_spell_ignore_all("the");
    run(&mut p, "review.proofing.spelling");
    assert!(sheet(&mut p).ext_review.spelling.is_empty());
    assert!(sheet(&mut p).status.contains("good to go"));
}

#[test]
fn accessibility_check_lists_findings() {
    let mut p = rig();
    run(&mut p, "review.accessibility.check-accessibility");
    let issues = sheet(&mut p).review_accessibility();
    assert!(
        issues
            .iter()
            .any(|i| i.rule == "Default sheet name" && i.severity == Severity::Tip)
    );
    assert_eq!(
        sheet(&mut p).ext_review.take_request(),
        Some(Request::Accessibility)
    );
}

#[test]
fn workbook_statistics_counts_cells_and_formulas() {
    let mut p = rig();
    sheet(&mut p).enter(1, 1, "1").unwrap();
    sheet(&mut p).enter(1, 2, "=A1+1").unwrap();
    sheet(&mut p).enter(2, 1, "text").unwrap();
    run(&mut p, "review.proofing.workbook-statistics");
    let s = sheet(&mut p).review_statistics();
    assert_eq!((s.worksheets, s.cells_with_data, s.formulas), (1, 3, 1));
    assert!(sheet(&mut p).status.contains("Cells with data: 3"));
    assert_eq!(
        sheet(&mut p).ext_review.take_request(),
        Some(Request::Statistics)
    );
}

#[test]
fn unsupported_review_controls_are_left_unhandled() {
    let mut p = rig();
    for id in [
        "review.proofing.thesaurus",
        "review.performance.check-performance",
        "review.language.translate",
        "review.changes.show-changes",
        "review.protect.allow-edit-ranges",
        "review.protect.unshare-workbook",
        "review.ink.hide-ink",
    ] {
        assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::No, "{id}");
    }
}

#[test]
fn threaded_comments_add_reply_navigate_delete_and_survive_a_save() {
    let mut p = rig();
    sheet(&mut p).select_cell((3, 2));
    run(&mut p, "review.comments.new-comment=First thought & more");
    sheet(&mut p).select_cell((1, 1));
    run(&mut p, "review.comments.new-comment=Other cell");
    sheet(&mut p).select_cell((3, 2));
    run(&mut p, "review.comments.new-comment=A reply");
    let all = sheet(&mut p).review_comments();
    assert_eq!(all.len(), 2);
    let t = sheet(&mut p).review_comment_at(3, 2).unwrap();
    assert_eq!(t.items.len(), 2);
    assert_eq!(t.items[0].text, "First thought & more");
    assert_eq!(t.items[1].text, "A reply");
    assert!(!t.items[0].author.is_empty() && t.items[0].time.contains('T'));
    run(&mut p, "review.comments.previous-comment");
    assert_eq!(sheet(&mut p).active, (1, 1));
    run(&mut p, "review.comments.next-comment");
    assert_eq!(sheet(&mut p).active, (3, 2));
    run(&mut p, "review.comments.show-comments");
    assert!(sheet(&mut p).ext_review.show_comments);
    let again = reopen(&mut p, "open365-review-threads.xlsx");
    assert_eq!(again.review_comments(), all_with_replies(&all, &t));
    // A note and a thread live side by side.
    sheet(&mut p).select_cell((5, 5));
    run(&mut p, "review.notes.notes=just a note");
    assert_eq!(sheet(&mut p).review_notes().len(), 1);
    assert_eq!(sheet(&mut p).review_comments().len(), 2);
    sheet(&mut p).select_cell((3, 2));
    run(&mut p, "review.comments.delete");
    assert!(sheet(&mut p).review_comment_at(3, 2).is_none());
    sheet(&mut p).select_cell((5, 5));
    run(&mut p, "review.comments.delete");
    assert!(sheet(&mut p).review_notes().is_empty());
    assert!(sheet(&mut p).undo());
    assert_eq!(sheet(&mut p).review_notes().len(), 1);
}

fn all_with_replies(
    all: &[open365_commands::review_ops::Thread],
    t: &open365_commands::review_ops::Thread,
) -> Vec<open365_commands::review_ops::Thread> {
    all.iter()
        .map(|x| {
            if (x.row, x.col) == (t.row, t.col) {
                t.clone()
            } else {
                x.clone()
            }
        })
        .collect()
}
