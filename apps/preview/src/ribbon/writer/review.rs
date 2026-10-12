//! Writer ribbon: the Review tab (`review.*`): spelling, word count, comments and tracked
//! changes. The work is in the pane (`doc/writer/ext_review.rs`) and the command layer.
//!
//! A control can carry a menu choice after its id (`review.tracking.accept.accept-all`); the
//! plain id and `<id>.run` run the control's default action (the main part of a split button).
//!
//! Not wired: Thesaurus, Read Aloud, Check Accessibility, Translate, Language, Show Comments,
//! the Markup group (view filters), Compare, Protect and Hide Ink.

use crate::ribbon::{Handled, RibbonCtx};

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "review.proofing.proofread",
    "review.proofing.spelling-and-grammar",
    "review.proofing.document-count",
    "review.comments.new-comment",
    "review.comments.delete",
    "review.comments.previous",
    "review.comments.next",
    "review.tracking.track-changes",
    "review.tracking.accept",
    "review.tracking.reject",
    "review.tracking.previous",
    "review.tracking.next",
];

fn split(id: &str) -> Option<(&'static str, &str)> {
    let id = crate::ribbon::base(id);
    IDS.iter().find_map(|c| {
        if id == *c {
            Some((*c, ""))
        } else {
            id.strip_prefix(*c)
                .and_then(|r| r.strip_prefix('.'))
                .map(|v| (*c, v))
        }
    })
}

/// Runs command `id`; [`Handled::No`] when this module does not know it.
pub fn handle(ctx: &mut RibbonCtx<'_>, id: &str) -> Handled {
    let Some((control, choice)) = split(id) else {
        return Handled::No;
    };
    let choice = if choice == "run" { "" } else { choice };
    let Some(p) = ctx.writer.as_deref_mut() else {
        return Handled::Yes;
    };
    match (control, choice) {
        ("review.proofing.proofread" | "review.proofing.spelling-and-grammar", "") => {
            p.spelling_next();
        }
        ("review.proofing.document-count", "") => p.show_word_count(),
        ("review.comments.new-comment", "") => {
            p.comment_new("");
        }
        ("review.comments.delete", "") => {
            p.comment_delete();
        }
        ("review.comments.delete", "delete-all-comments" | "delete-all-shown") => {
            p.comment_delete_all();
        }
        ("review.comments.previous", "") => {
            p.comment_step(false);
        }
        ("review.comments.next", "") => {
            p.comment_step(true);
        }
        ("review.tracking.track-changes", "" | "toggle") => {
            let on = !p.track_changes_on();
            p.set_track_changes(on);
        }
        ("review.tracking.track-changes", "just-me" | "for-everyone") => {
            p.set_track_changes(true);
        }
        ("review.tracking.accept", "" | "accept-and-move") => {
            p.resolve_here(true, true);
        }
        ("review.tracking.accept", "accept-this") => {
            p.resolve_here(true, false);
        }
        ("review.tracking.accept", "accept-all" | "accept-all-shown") => {
            p.resolve_all(true);
        }
        ("review.tracking.accept", "accept-all-stop-tracking") => {
            p.resolve_all(true);
            p.set_track_changes(false);
        }
        ("review.tracking.reject", "" | "reject-and-move") => {
            p.resolve_here(false, true);
        }
        ("review.tracking.reject", "reject-this") => {
            p.resolve_here(false, false);
        }
        ("review.tracking.reject", "reject-all" | "reject-all-shown") => {
            p.resolve_all(false);
        }
        ("review.tracking.reject", "reject-all-stop-tracking") => {
            p.resolve_all(false);
            p.set_track_changes(false);
        }
        ("review.tracking.previous", "") => {
            p.revision_step(false);
        }
        ("review.tracking.next", "") => {
            p.revision_step(true);
        }
        _ => return Handled::No,
    }
    *ctx.refocus_doc = true;
    Handled::Yes
}
