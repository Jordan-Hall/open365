//! Writer ribbon: the Review tab (`review.*`): spelling, word count, comments and tracked
//! changes. The work is in the pane (`doc/writer/ext_review.rs`) and the command layer.
//!
//! A control can carry a menu choice after its id (`review.tracking.accept.accept-all`); the
//! plain id and `<id>.run` run the control's default action (the main part of a split button).
//!
//! The Markup group keeps its state in the pane's `ReviewState`; Display for Review takes the
//! combo value (`review.markup.display-for-review=No Markup`). Compare / Combine take the path
//! of the revised document (`review.compare.compare.compare=C:\a.docx`); without a path they do
//! nothing until the file dialog supplies one.
//!
//! Not wired (no data or subsystem): Thesaurus, Translate, Block Authors, Read Aloud, Check
//! Accessibility, Language and Hide Ink.

use crate::ribbon::{Handled, RibbonCtx};

/// Display for Review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewDisplay {
    SimpleMarkup,
    #[default]
    AllMarkup,
    NoMarkup,
    Original,
}

impl ReviewDisplay {
    /// The label Word shows in the combo box.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::SimpleMarkup => "Simple Markup",
            Self::AllMarkup => "All Markup",
            Self::NoMarkup => "No Markup",
            Self::Original => "Original",
        }
    }

    /// From a combo value or a menu choice (`simple-markup`).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().replace(' ', "-").as_str() {
            "simple-markup" => Some(Self::SimpleMarkup),
            "all-markup" => Some(Self::AllMarkup),
            "no-markup" => Some(Self::NoMarkup),
            "original" => Some(Self::Original),
            _ => None,
        }
    }
}

/// What Restrict Editing allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditRestriction {
    TrackedChanges,
    Comments,
    FormFilling,
    ReadOnly,
}

/// The Review tab view state (Word keeps it per window, not in the file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewState {
    pub display: ReviewDisplay,
    pub show_comments: bool,
    pub show_insertions_deletions: bool,
    pub show_formatting: bool,
    pub show_ink: bool,
    pub reviewing_pane: bool,
    /// Reviewers whose markup is shown; empty means all.
    pub reviewers: Vec<String>,
    pub restriction: Option<EditRestriction>,
}

impl Default for ReviewState {
    fn default() -> Self {
        Self {
            display: ReviewDisplay::AllMarkup,
            show_comments: true,
            show_insertions_deletions: true,
            show_formatting: true,
            show_ink: true,
            reviewing_pane: false,
            reviewers: Vec::new(),
            restriction: None,
        }
    }
}

/// The control ids this module handles (ids as in the spec, without `=` or `#` suffixes).
pub const IDS: &[&str] = &[
    "review.proofing.proofread",
    "review.proofing.spelling-and-grammar",
    "review.proofing.document-count",
    "review.comments.new-comment",
    "review.comments.delete",
    "review.comments.previous",
    "review.comments.next",
    "review.comments.show-comments",
    "review.markup.filter-all-markup",
    "review.markup.display-for-review",
    "review.markup.show-markup",
    "review.markup.reviewing-pane",
    "review.compare.compare",
    "review.protect.protect",
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
    let value = id.split_once('=').map_or("", |(_, v)| v);
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
        ("review.comments.show-comments", "" | "toggle") => {
            p.edit_review_state(|s| s.show_comments = !s.show_comments);
        }
        ("review.markup.display-for-review", "") => {
            if let Some(d) = ReviewDisplay::parse(value) {
                p.set_review_display(d);
            }
        }
        ("review.markup.display-for-review", c) => {
            if let Some(d) = ReviewDisplay::parse(c) {
                p.set_review_display(d);
            }
        }
        ("review.markup.show-markup", "comments") => {
            p.edit_review_state(|s| s.show_comments = !s.show_comments);
        }
        ("review.markup.show-markup", "insertions-and-deletions") => {
            p.edit_review_state(|s| s.show_insertions_deletions = !s.show_insertions_deletions);
        }
        ("review.markup.show-markup", "formatting") => {
            p.edit_review_state(|s| s.show_formatting = !s.show_formatting);
        }
        ("review.markup.show-markup", "ink") => {
            p.edit_review_state(|s| s.show_ink = !s.show_ink);
        }
        ("review.markup.filter-all-markup" | "review.markup.show-markup", "") => {}
        ("review.markup.filter-all-markup", "all-reviewers") => {
            p.edit_review_state(|s| s.reviewers.clear());
        }
        ("review.markup.filter-all-markup", name) if !name.is_empty() => {
            let name = name.to_owned();
            p.edit_review_state(|s| {
                if let Some(i) = s.reviewers.iter().position(|r| *r == name) {
                    s.reviewers.remove(i);
                } else {
                    s.reviewers.push(name);
                }
            });
        }
        ("review.markup.reviewing-pane", "" | "toggle" | "vertical" | "horizontal") => {
            p.edit_review_state(|s| s.reviewing_pane = !s.reviewing_pane);
        }
        ("review.compare.compare", "compare" | "combine") => {
            if !value.is_empty()
                && let Err(e) = p.compare_with_file(std::path::Path::new(value))
            {
                p.status = Some(e);
            }
        }
        ("review.compare.compare", "") => {}
        ("review.protect.protect", "" | "restrict-editing" | "tracked-changes") => {
            p.set_edit_restriction(Some(EditRestriction::TrackedChanges));
        }
        ("review.protect.protect", "comments") => {
            p.set_edit_restriction(Some(EditRestriction::Comments));
        }
        ("review.protect.protect", "filling-in-forms") => {
            p.set_edit_restriction(Some(EditRestriction::FormFilling));
        }
        ("review.protect.protect", "no-changes") => {
            p.set_edit_restriction(Some(EditRestriction::ReadOnly));
        }
        ("review.protect.protect", "stop-protection") => p.set_edit_restriction(None),
        _ => return Handled::No,
    }
    *ctx.refocus_doc = true;
    Handled::Yes
}
