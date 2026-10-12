//! Writer pane: the Review tab. Spelling, word count, comments and tracked changes. Every edit is
//! a `Command::DocxReview` of the command layer, so it is logged and undoable.

use std::cell::RefCell;
use std::path::Path;
use std::sync::OnceLock;

use open365_commands::{Command, WriterReviewOp};
use open365_docx::{DocPos, Revision, TextView, align_paragraphs, diff_text};
use open365_spell::Proofing;

use super::WriterPane;
use crate::ribbon::writer::review::{EditRestriction, ReviewDisplay, ReviewState};

/// What Word's Word Count dialog shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WordCount {
    pub pages: usize,
    pub words: usize,
    pub characters_no_spaces: usize,
    pub characters_with_spaces: usize,
    pub paragraphs: usize,
    pub lines: usize,
}

/// The reviewer name tracked changes and comments carry (the operating system user).
#[must_use]
pub fn reviewer_name() -> String {
    ["USERNAME", "USER"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
        .unwrap_or_else(|| "Open365 User".to_owned())
}

fn proofing() -> &'static Proofing {
    static P: OnceLock<Proofing> = OnceLock::new();
    P.get_or_init(Proofing::new)
}

fn count_text(text: &str, c: &mut WordCount) {
    c.words += text.split_whitespace().count();
    c.characters_with_spaces += text.chars().filter(|ch| !matches!(ch, '\r' | '\n')).count();
    c.characters_no_spaces += text.chars().filter(|ch| !ch.is_whitespace()).count();
}

/// The span of a revision in the final-view text: start and end.
fn revision_span(r: &Revision) -> (DocPos, DocPos) {
    let start = DocPos::new(r.paragraph, r.offset);
    let end = if r.paragraph == r.last_paragraph && !r.kind.is_deletion_like() {
        DocPos::new(r.paragraph, r.offset + r.text.chars().count())
    } else {
        start
    };
    (start, end)
}

impl WriterPane {
    // ---- proofing ----------------------------------------------------------------------------

    /// The statistics of the whole document, notes included.
    #[must_use]
    pub fn word_count(&self) -> WordCount {
        let mut c = WordCount {
            pages: self.page_count(),
            paragraphs: self.models.len(),
            ..WordCount::default()
        };
        for m in &self.models {
            count_text(&m.text, &mut c);
        }
        if let Some(doc) = self.target.docx() {
            for note in doc.footnotes().iter().chain(doc.endnotes().iter()) {
                if note.kind != open365_docx::NoteKind::Normal {
                    continue;
                }
                for block in &note.blocks {
                    if let open365_docx::Block::Paragraph(p) = block {
                        count_text(&p.text(), &mut c);
                    }
                }
            }
        }
        c.lines = self
            .engine
            .layout()
            .pages
            .iter()
            .map(|p| p.lines.len())
            .sum();
        c
    }

    /// Word Count: reports the statistics in the status bar, for the selection when there is one.
    pub fn show_word_count(&mut self) {
        let total = self.word_count();
        let message = if self.has_selection() {
            let mut sel = WordCount::default();
            count_text(&self.selected_text(), &mut sel);
            format!(
                "Words: {} of {}  Characters (no spaces): {}  Characters (with spaces): {}",
                sel.words, total.words, sel.characters_no_spaces, sel.characters_with_spaces
            )
        } else {
            format!(
                "Pages: {}  Words: {}  Characters (no spaces): {}  Characters (with spaces): {}  Paragraphs: {}  Lines: {}",
                total.pages,
                total.words,
                total.characters_no_spaces,
                total.characters_with_spaces,
                total.paragraphs,
                total.lines
            )
        };
        self.status = Some(message);
    }

    /// Spelling and Grammar (F7): selects the next word not in the dictionary after the caret,
    /// wrapping at the end of the document. Returns the word, or `None` when the check is done.
    pub fn spelling_next(&mut self) -> Option<String> {
        let service = proofing();
        let from = self.selection().1;
        let n = self.models.len();
        for step in 0..=n {
            let para = (from.para + step) % n.max(1);
            let Some(model) = self.models.get(para) else {
                break;
            };
            let report = service.check_paragraph(&model.text, "en-US");
            let hit = report.issues.into_iter().find(|issue| {
                // The first paragraph is read from the caret on; on the wrap, up to the caret.
                if step == 0 {
                    issue.char_range.start >= from.offset
                } else if step == n {
                    issue.char_range.start < from.offset
                } else {
                    true
                }
            });
            if let Some(issue) = hit {
                self.select(
                    DocPos::new(para, issue.char_range.start),
                    DocPos::new(para, issue.char_range.end),
                );
                self.reveal_caret();
                self.status = Some(format!("Not in dictionary: {}", issue.word));
                return Some(issue.word);
            }
        }
        self.status = Some("The spelling and grammar check is complete.".to_owned());
        None
    }

    // ---- comments ----------------------------------------------------------------------------

    /// The range a new comment anchors to: the selection, else the word at the caret.
    fn comment_range(&self) -> (DocPos, DocPos) {
        if self.has_selection() {
            return self.selection();
        }
        let at = self.caret;
        let Some(model) = self.models.get(at.para) else {
            return (at, at);
        };
        let chars: Vec<char> = model.text.chars().collect();
        let mut s = at.offset.min(chars.len());
        let mut e = s;
        while s > 0 && Self::is_word(chars[s - 1]) {
            s -= 1;
        }
        while e < chars.len() && Self::is_word(chars[e]) {
            e += 1;
        }
        (DocPos::new(at.para, s), DocPos::new(at.para, e))
    }

    /// New Comment: a comment by the reviewer on the selection (or the word at the caret).
    pub fn comment_new(&mut self, text: &str) -> Option<u32> {
        let (start, end) = self.comment_range();
        let before = self.target.docx().map_or(0, |d| d.comments().len());
        self.run(Command::DocxReview {
            op: WriterReviewOp::AddComment {
                start,
                end,
                author: reviewer_name(),
                text: text.to_owned(),
            },
        })?;
        let doc = self.target.docx()?;
        let all = doc.comments();
        (all.len() > before)
            .then(|| all.iter().map(|c| c.id).max())
            .flatten()
    }

    /// The comments anchored in the main story, by position.
    fn anchored_comments(&self) -> Vec<(u32, DocPos, DocPos)> {
        let Some(doc) = self.target.docx() else {
            return Vec::new();
        };
        let mut v: Vec<(u32, DocPos, DocPos)> = doc
            .comments()
            .into_iter()
            .filter(|c| c.parent.is_none())
            .filter_map(|c| c.anchor.map(|a| (c.id, a.start, a.end)))
            .collect();
        v.sort_by_key(|(id, s, _)| (*s, *id));
        v
    }

    /// Whether the caret or selection touches a comment (Delete is enabled then).
    #[must_use]
    pub fn comment_at_caret(&self) -> Option<u32> {
        let (a, b) = self.selection();
        self.anchored_comments()
            .into_iter()
            .filter(|(_, s, e)| *s <= b && a <= *e)
            .min_by_key(|(_, s, e)| e.para.abs_diff(s.para) * 100_000 + e.offset.abs_diff(s.offset))
            .map(|(id, _, _)| id)
    }

    /// Delete: the comment the caret is in.
    pub fn comment_delete(&mut self) -> bool {
        let Some(id) = self.comment_at_caret() else {
            self.status = Some("Select a comment to delete.".to_owned());
            return false;
        };
        self.run(Command::DocxReview {
            op: WriterReviewOp::DeleteComment { id },
        })
        .is_some()
    }

    /// Delete All Comments in Document.
    pub fn comment_delete_all(&mut self) -> bool {
        self.run(Command::DocxReview {
            op: WriterReviewOp::DeleteAllComments,
        })
        .is_some()
    }

    /// Next or Previous comment: selects its range and reports it in the status bar.
    pub fn comment_step(&mut self, forward: bool) -> Option<u32> {
        let all = self.anchored_comments();
        let (a, b) = self.selection();
        let pick = if forward {
            all.iter()
                .find(|(_, s, _)| *s > b || (*s >= b && !self.has_selection() && *s > a))
        } else {
            all.iter().rev().find(|(_, s, _)| *s < a)
        };
        let wrap = if forward { all.first() } else { all.last() };
        let (id, s, e) = *pick.or(wrap)?;
        self.select(s, e);
        self.reveal_caret();
        if let Some(c) = self.target.docx().and_then(|d| d.comment(id)) {
            self.status = Some(format!("Comment by {}: {}", c.author, c.text));
        }
        Some(id)
    }

    // ---- tracked changes ----------------------------------------------------------------------

    /// Whether edits are recorded as tracked changes.
    #[must_use]
    pub fn track_changes_on(&self) -> bool {
        self.target
            .docx()
            .is_some_and(|d| d.track_changes().is_some())
    }

    /// Track Changes: turns recording on or off for this document.
    pub fn set_track_changes(&mut self, on: bool) -> bool {
        self.run(Command::DocxReview {
            op: WriterReviewOp::TrackChanges {
                on,
                author: reviewer_name(),
            },
        })
        .is_some()
    }

    /// The tracked changes of the document.
    #[must_use]
    pub fn revisions(&self) -> Vec<Revision> {
        self.target
            .docx()
            .map(|d| d.revisions())
            .unwrap_or_default()
    }

    /// Revisions the selection covers, or the one at the caret, or failing that the next one.
    fn revisions_here(&self) -> Vec<usize> {
        let all = self.revisions();
        let (a, b) = self.selection();
        let touching: Vec<usize> = all
            .iter()
            .filter(|r| {
                let (s, e) = revision_span(r);
                if self.has_selection() {
                    (s < b && a < e) || (s >= a && s <= b)
                } else {
                    s <= a && a <= e
                }
            })
            .map(|r| r.index)
            .collect();
        if !touching.is_empty() {
            return touching;
        }
        all.iter()
            .find(|r| revision_span(r).0 > b)
            .map(|r| vec![r.index])
            .unwrap_or_default()
    }

    /// Selects the first change after `from` (wrapping); false when there is none.
    fn goto_revision(&mut self, from: DocPos, forward: bool) -> bool {
        let all = self.revisions();
        let pick = if forward {
            all.iter().find(|r| revision_span(r).0 > from)
        } else {
            all.iter().rev().find(|r| revision_span(r).0 < from)
        };
        let Some(r) = pick.or(if forward { all.first() } else { all.last() }) else {
            return false;
        };
        let (s, e) = revision_span(r);
        self.select(s, e);
        self.reveal_caret();
        self.status = Some(format!(
            "{}: {} {}",
            r.kind.word_type_name(),
            r.author,
            r.text
        ));
        true
    }

    /// Accept or Reject: resolves the changes in the selection (or at the caret, or the next one)
    /// and, with `advance`, moves to the following change as Word does.
    pub fn resolve_here(&mut self, accept: bool, advance: bool) -> bool {
        let indices = self.revisions_here();
        if indices.is_empty() {
            self.status = Some("There are no tracked changes to resolve.".to_owned());
            return false;
        }
        let from = self.selection().0;
        let op = if accept {
            WriterReviewOp::AcceptRevisions { indices }
        } else {
            WriterReviewOp::RejectRevisions { indices }
        };
        if self.run(Command::DocxReview { op }).is_none() {
            return false;
        }
        if advance {
            let at = self.clamp(from);
            self.set_caret(at);
            let behind = if at.offset > 0 {
                DocPos::new(at.para, at.offset - 1)
            } else {
                at
            };
            self.goto_revision(behind, true);
        }
        true
    }

    /// Accept All Changes / Reject All Changes.
    pub fn resolve_all(&mut self, accept: bool) -> bool {
        let op = if accept {
            WriterReviewOp::AcceptAll
        } else {
            WriterReviewOp::RejectAll
        };
        self.run(Command::DocxReview { op }).is_some()
    }

    /// Next Change / Previous Change.
    pub fn revision_step(&mut self, forward: bool) -> bool {
        let from = if forward {
            self.selection().1
        } else {
            self.selection().0
        };
        let from = if forward && self.has_selection() {
            DocPos::new(from.para, from.offset.saturating_sub(1))
        } else {
            from
        };
        self.goto_revision(from, forward)
    }

    /// The text of paragraph `para` with deletions shown (the markup view).
    #[must_use]
    pub fn markup_text(&self, para: usize) -> Option<String> {
        self.target
            .docx()?
            .paragraph_text_view(para, TextView::Markup)
    }
}

thread_local! {
    static REVIEW: RefCell<ReviewState> = RefCell::new(ReviewState::default());
}

impl WriterPane {
    /// The Review tab view state.
    #[must_use]
    pub fn review_state(&self) -> ReviewState {
        REVIEW.with(|s| s.borrow().clone())
    }

    /// Changes the Review tab view state.
    pub fn edit_review_state(&mut self, f: impl FnOnce(&mut ReviewState)) {
        REVIEW.with(|s| f(&mut s.borrow_mut()));
    }

    /// Display for Review.
    pub fn set_review_display(&mut self, d: ReviewDisplay) {
        self.edit_review_state(|s| s.display = d);
        self.status = Some(d.label().to_owned());
    }

    /// Restrict Editing: `None` stops protection. Tracked-changes protection also turns
    /// tracking on, as Word does.
    pub fn set_edit_restriction(&mut self, r: Option<EditRestriction>) {
        self.edit_review_state(|s| s.restriction = r);
        if r == Some(EditRestriction::TrackedChanges) {
            self.set_track_changes(true);
        }
        self.status = Some(match r {
            Some(_) => "Document protection is on.".to_owned(),
            None => "Document protection is off.".to_owned(),
        });
    }

    /// Whether Restrict Editing blocks typing and deleting.
    #[must_use]
    pub fn editing_blocked(&self) -> bool {
        matches!(
            self.review_state().restriction,
            Some(EditRestriction::ReadOnly | EditRestriction::FormFilling)
        )
    }

    /// Compare / Combine: the differences of the document at `path` (the revised copy) against
    /// this one (the original) become tracked changes in this document.
    ///
    /// # Errors
    /// The revised document cannot be opened.
    pub fn compare_with_file(&mut self, path: &Path) -> Result<usize, String> {
        let revised = Self::open(path)?.text();
        Ok(self.compare_with_text(&revised))
    }

    /// Applies the differences to `revised` (paragraphs separated by line feeds) as tracked
    /// changes; the number of changed paragraphs.
    pub fn compare_with_text(&mut self, revised: &str) -> usize {
        let was_tracking = self.track_changes_on();
        let orig: Vec<String> = self.text().split('\n').map(str::to_owned).collect();
        let rev: Vec<String> = revised.split('\n').map(str::to_owned).collect();
        self.set_track_changes(true);
        let mut changed = 0;
        for (a, b) in align_paragraphs(&orig, &rev).into_iter().rev() {
            match (a, b) {
                (Some(i), Some(j)) if orig[i] != rev[j] => {
                    changed += 1;
                    for e in diff_text(&orig[i], &rev[j]).into_iter().rev() {
                        if e.delete > 0 {
                            self.select(DocPos::new(i, e.at), DocPos::new(i, e.at + e.delete));
                            self.delete_selection();
                        }
                        if !e.insert.is_empty() {
                            self.set_caret(DocPos::new(i, e.at));
                            self.insert_text(&e.insert);
                        }
                    }
                }
                (Some(i), None) => {
                    changed += 1;
                    let len = orig[i].chars().count();
                    if i + 1 < orig.len() {
                        self.select(DocPos::new(i, 0), DocPos::new(i + 1, 0));
                    } else if i > 0 {
                        let prev = orig[i - 1].chars().count();
                        self.select(DocPos::new(i - 1, prev), DocPos::new(i, len));
                    } else {
                        self.select(DocPos::new(0, 0), DocPos::new(0, len));
                    }
                    self.delete_selection();
                }
                (None, Some(j)) => {
                    changed += 1;
                    // Original paragraphs that precede this one in the alignment.
                    let before = align_paragraphs(&orig, &rev)
                        .iter()
                        .take_while(|(_, y)| *y != Some(j))
                        .filter(|(x, _)| x.is_some())
                        .count();
                    if before < orig.len() {
                        self.set_caret(DocPos::new(before, 0));
                        self.paste_text(&format!(
                            "{}
",
                            rev[j]
                        ));
                    } else {
                        let last = orig.len() - 1;
                        self.set_caret(DocPos::new(last, orig[last].chars().count()));
                        self.paste_text(&format!(
                            "
{}",
                            rev[j]
                        ));
                    }
                }
                _ => {}
            }
        }
        if !was_tracking {
            self.set_track_changes(false);
        }
        self.status = Some(format!("{changed} paragraphs differ."));
        changed
    }
}
