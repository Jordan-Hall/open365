//! Writer Review tab controls run through `Shell::run_command`.

#![allow(clippy::unwrap_used)]

use open365_docx::DocPos;
use open365_preview::doc::WriterPane;
use open365_preview::ribbon::Handled;
use open365_preview::{AppKind, Preview};

fn rig(text: &str) -> Preview {
    let mut p = Preview::new(AppKind::Writer, false, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text(text);
    p.shell.set_writer(&mut p.ui, pane);
    p
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

fn pane(p: &mut Preview) -> &mut WriterPane {
    p.shell.writer_mut().unwrap()
}

#[test]
fn word_count_reports_in_the_status_bar() {
    let mut p = rig("one two three");
    run(&mut p, "review.proofing.document-count");
    let status = pane(&mut p).status.clone().unwrap();
    assert!(status.contains("Words: 3"), "{status}");
    assert!(status.contains("Characters (no spaces): 11"), "{status}");
    assert!(status.contains("Characters (with spaces): 13"), "{status}");
    // With a selection the status bar counts the selection.
    pane(&mut p).select(DocPos::new(0, 0), DocPos::new(0, 7));
    run(&mut p, "review.proofing.document-count");
    let status = pane(&mut p).status.clone().unwrap();
    assert!(status.contains("Words: 2 of 3"), "{status}");
}

#[test]
fn spelling_selects_the_next_unknown_word() {
    let mut p = rig("The quick brwon fox");
    pane(&mut p).set_caret(DocPos::new(0, 0));
    run(&mut p, "review.proofing.spelling-and-grammar");
    assert_eq!(pane(&mut p).selected_text(), "brwon");
    assert_eq!(
        pane(&mut p).status.as_deref(),
        Some("Not in dictionary: brwon")
    );
    // Nothing further: the check wraps and finds the same word again, so fix it first.
    pane(&mut p).insert_text("brown");
    run(&mut p, "review.proofing.proofread");
    assert_eq!(
        pane(&mut p).status.as_deref(),
        Some("The spelling and grammar check is complete.")
    );
}

#[test]
fn track_changes_record_and_resolve() {
    let mut p = rig("Hello world");
    assert!(!pane(&mut p).track_changes_on());
    run(&mut p, "review.tracking.track-changes");
    assert!(pane(&mut p).track_changes_on());
    pane(&mut p).set_caret(DocPos::new(0, 5));
    pane(&mut p).insert_text(" big");
    assert_eq!(pane(&mut p).revisions().len(), 1);
    assert_eq!(pane(&mut p).text(), "Hello big world");
    // Reject takes the insertion out again.
    pane(&mut p).set_caret(DocPos::new(0, 6));
    run(&mut p, "review.tracking.reject");
    assert!(pane(&mut p).revisions().is_empty());
    assert_eq!(pane(&mut p).text(), "Hello world");
    // Accept All keeps a new insertion.
    pane(&mut p).insert_text("!");
    assert_eq!(pane(&mut p).revisions().len(), 1);
    run(&mut p, "review.tracking.accept.accept-all");
    assert!(pane(&mut p).revisions().is_empty());
    assert!(pane(&mut p).text().contains('!'));
    // Turning it off again.
    run(&mut p, "review.tracking.track-changes");
    assert!(!pane(&mut p).track_changes_on());
}

#[test]
fn next_and_previous_change_move_the_selection() {
    let mut p = rig("alpha beta gamma");
    run(&mut p, "review.tracking.track-changes");
    pane(&mut p).set_caret(DocPos::new(0, 5));
    pane(&mut p).insert_text("X");
    pane(&mut p).set_caret(DocPos::new(0, 17));
    pane(&mut p).insert_text("Y");
    assert_eq!(pane(&mut p).revisions().len(), 2);
    pane(&mut p).set_caret(DocPos::new(0, 0));
    run(&mut p, "review.tracking.next");
    assert_eq!(pane(&mut p).selected_text(), "X");
    run(&mut p, "review.tracking.next");
    assert_eq!(pane(&mut p).selected_text(), "Y");
    run(&mut p, "review.tracking.previous");
    assert_eq!(pane(&mut p).selected_text(), "X");
    // Accept and move to next: X is accepted, Y becomes the selection.
    run(&mut p, "review.tracking.accept");
    assert_eq!(pane(&mut p).revisions().len(), 1);
    assert_eq!(pane(&mut p).selected_text(), "Y");
}

#[test]
fn comments_add_navigate_and_delete() {
    let mut p = rig("alpha beta gamma");
    pane(&mut p).select(DocPos::new(0, 6), DocPos::new(0, 10));
    run(&mut p, "review.comments.new-comment");
    pane(&mut p).select(DocPos::new(0, 11), DocPos::new(0, 16));
    run(&mut p, "review.comments.new-comment");
    let comments = |p: &mut Preview| {
        pane(p).docx_bytes().map(|b| {
            open365_docx::Document::open(&b, &open365_opc::Limits::default())
                .unwrap()
                .comments()
        })
    };
    assert_eq!(comments(&mut p).unwrap().len(), 2);
    pane(&mut p).set_caret(DocPos::new(0, 0));
    run(&mut p, "review.comments.next");
    assert_eq!(pane(&mut p).selected_text(), "beta");
    run(&mut p, "review.comments.next");
    assert_eq!(pane(&mut p).selected_text(), "gamma");
    run(&mut p, "review.comments.previous");
    assert_eq!(pane(&mut p).selected_text(), "beta");
    run(&mut p, "review.comments.delete");
    let left = comments(&mut p).unwrap();
    assert_eq!(left.len(), 1);
    run(&mut p, "review.comments.delete.delete-all-comments");
    assert!(comments(&mut p).unwrap().is_empty());
    assert!(pane(&mut p).undo());
    assert_eq!(comments(&mut p).unwrap().len(), 1);
}

#[test]
fn controls_answer_without_a_document() {
    let mut p = Preview::new(AppKind::Writer, false, 1600, 1000, 1.25);
    for id in open365_preview::ribbon::writer::review::IDS {
        assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
    }
}

#[test]
fn every_control_runs_on_a_document() {
    let mut p = rig("hello world");
    for id in open365_preview::ribbon::writer::review::IDS {
        for i in [(*id).to_owned(), format!("{id}=12")] {
            assert_eq!(p.shell.run_command(&mut p.ui, &i), Handled::Yes, "{i}");
        }
    }
}

#[test]
fn markup_display_and_filters_keep_state() {
    let mut p = rig("Hello");
    run(&mut p, "review.markup.display-for-review=No Markup");
    assert_eq!(pane(&mut p).review_state().display.label(), "No Markup");
    run(&mut p, "review.markup.display-for-review.original");
    assert_eq!(pane(&mut p).review_state().display.label(), "Original");
    run(&mut p, "review.comments.show-comments");
    assert!(!pane(&mut p).review_state().show_comments);
    run(&mut p, "review.markup.show-markup.formatting");
    assert!(!pane(&mut p).review_state().show_formatting);
    run(&mut p, "review.markup.filter-all-markup.Ann");
    assert_eq!(
        pane(&mut p).review_state().reviewers,
        vec!["Ann".to_owned()]
    );
    run(&mut p, "review.markup.filter-all-markup.all-reviewers");
    assert!(pane(&mut p).review_state().reviewers.is_empty());
    run(&mut p, "review.markup.reviewing-pane");
    assert!(pane(&mut p).review_state().reviewing_pane);
}

#[test]
fn restrict_editing_tracked_changes_turns_tracking_on() {
    let mut p = rig("Hello");
    run(&mut p, "review.protect.protect.tracked-changes");
    assert!(pane(&mut p).track_changes_on());
    run(&mut p, "review.protect.protect.no-changes");
    assert!(pane(&mut p).editing_blocked());
    run(&mut p, "review.protect.protect.stop-protection");
    assert!(!pane(&mut p).editing_blocked());
}

#[test]
fn compare_records_differences_as_tracked_changes() {
    let mut p = rig("The quick fox");
    let n = pane(&mut p).compare_with_text("The slow fox");
    assert_eq!(n, 1);
    assert!(!pane(&mut p).revisions().is_empty());
    assert_eq!(pane(&mut p).text(), "The slow fox");
    run(&mut p, "review.tracking.reject.reject-all");
    assert_eq!(pane(&mut p).text(), "The quick fox");
}
