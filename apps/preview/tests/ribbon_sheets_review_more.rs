//! Review tab: Allow Edit Ranges and Notes > Convert to Comments.

#![allow(clippy::unwrap_used)]

use open365_preview::ribbon::Handled;
use open365_preview::sheet::SheetModel;
use open365_preview::{AppKind, Preview};

fn rig() -> Preview {
    Preview::new(AppKind::Sheets, true, 1600, 1000, 1.25)
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

fn reopen(p: &mut Preview, name: &str) -> SheetModel {
    let path = std::env::temp_dir().join(name);
    p.shell.sheet_mut().save(&path).unwrap();
    let m = SheetModel::open(&path).unwrap();
    let _ = std::fs::remove_file(path);
    m
}

#[test]
fn allow_edit_ranges_add_modify_delete_undo_and_survive_a_save() {
    let mut p = rig();
    run(
        &mut p,
        "review.protect.allow-edit-ranges=add:Inputs|$B$2:C4|",
    );
    run(&mut p, "review.protect.allow-edit-ranges=add:Secret|E1|pw1");
    let list = p.shell.sheet().review_edit_ranges();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].title, "Inputs");
    assert_eq!(list[0].refers_to, "B2:C4");
    assert_eq!(list[0].password_hash, None);
    assert_eq!(
        list[1].password_hash.as_deref(),
        Some(open365_commands::review_ops::legacy_password_hash("pw1").as_str())
    );
    assert_eq!(p.shell.sheet().review_next_range_title(), "Range1");
    // Cells of a range without a password can be edited on a protected sheet.
    assert!(p.shell.sheet().review_in_open_edit_range(3, 3));
    assert!(!p.shell.sheet().review_in_open_edit_range(1, 5));

    let again = reopen(&mut p, "open365-edit-ranges.xlsx");
    assert_eq!(again.review_edit_ranges(), list);

    run(
        &mut p,
        "review.protect.allow-edit-ranges=modify:Inputs|Data|A1:A9|",
    );
    let list = p.shell.sheet().review_edit_ranges();
    assert_eq!(
        (list[0].title.as_str(), list[0].refers_to.as_str()),
        ("Data", "A1:A9")
    );
    run(&mut p, "review.protect.allow-edit-ranges=delete:Secret");
    assert_eq!(p.shell.sheet().review_edit_ranges().len(), 1);
    assert!(p.shell.sheet_mut().undo());
    assert_eq!(p.shell.sheet().review_edit_ranges().len(), 2);

    // A bad reference and a duplicate title are refused, with nothing changed.
    run(
        &mut p,
        "review.protect.allow-edit-ranges=add:Bad|not a range|",
    );
    assert_eq!(p.shell.sheet().status, "The reference is not valid.");
    run(&mut p, "review.protect.allow-edit-ranges=add:data|A1|");
    assert_eq!(p.shell.sheet().review_edit_ranges().len(), 2);
}

#[test]
fn allow_edit_ranges_are_locked_while_the_sheet_is_protected() {
    let mut p = rig();
    run(&mut p, "review.protect.allow-edit-ranges=add:Inputs|A1:B2|");
    run(&mut p, "review.protect.protect-sheet=password=");
    assert!(p.shell.sheet().review_sheet_protection().is_some());
    run(&mut p, "review.protect.allow-edit-ranges=add:More|D1|");
    assert!(p.shell.sheet().status.contains("protected"));
    assert_eq!(p.shell.sheet().review_edit_ranges().len(), 1);
}

#[test]
fn notes_convert_to_comments_in_one_undo_step() {
    let mut p = rig();
    p.shell.sheet_mut().select_cell((2, 2));
    run(&mut p, "review.notes.notes=first note");
    p.shell.sheet_mut().select_cell((6, 1));
    run(&mut p, "review.notes.notes=second note");
    assert_eq!(p.shell.sheet().review_notes().len(), 2);
    run(&mut p, "review.notes.notes.convert-to-comments");
    assert!(
        p.shell.sheet().review_notes().is_empty(),
        "{}",
        p.shell.sheet().status
    );
    let threads = p.shell.sheet().review_comments();
    assert_eq!(threads.len(), 2);
    assert_eq!((threads[0].row, threads[0].col), (2, 2));
    assert_eq!(threads[0].items[0].text, "first note");
    assert_eq!(threads[1].items[0].text, "second note");
    let again = reopen(&mut p, "open365-convert-notes.xlsx");
    assert_eq!(again.review_comments().len(), 2);
    assert!(again.review_notes().is_empty());
    assert!(p.shell.sheet_mut().undo());
    assert_eq!(p.shell.sheet().review_notes().len(), 2);
    assert!(p.shell.sheet().review_comments().is_empty());
    // With no notes left to convert, the command says so.
    run(&mut p, "review.notes.notes.convert-to-comments");
    p.shell.sheet_mut().status.clear();
    run(&mut p, "review.notes.notes.convert-to-comments");
    assert!(
        p.shell
            .sheet()
            .status
            .contains("There are no notes in this workbook."),
        "{}",
        p.shell.sheet().status
    );
}

#[test]
fn shared_workbook_and_legacy_track_changes_stay_unhandled() {
    let mut p = rig();
    for id in [
        "review.protect.unshare-workbook",
        "review.changes.show-changes",
    ] {
        assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::No, "{id}");
    }
}
