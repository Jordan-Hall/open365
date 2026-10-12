//! Writer Review tab: comments and tracked changes. Thin wrappers over the document's own
//! comment and revision API so each action is one logged, undoable command.

use open365_docx::{DocPos, Document, NewComment, TrackChanges};
use serde::{Deserialize, Serialize};

use crate::CommandError;
use crate::command::EffectClass;

/// One Review-tab edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterReviewOp {
    /// Turns recording of edits as tracked changes on or off.
    TrackChanges {
        on: bool,
        author: String,
    },
    /// New comment on the range `start..end` (a collapsed range anchors at a point).
    AddComment {
        start: DocPos,
        end: DocPos,
        author: String,
        text: String,
    },
    DeleteComment {
        id: u32,
    },
    DeleteAllComments,
    /// Accepts the revisions at these indices of `Document::revisions`.
    AcceptRevisions {
        indices: Vec<usize>,
    },
    RejectRevisions {
        indices: Vec<usize>,
    },
    AcceptAll,
    RejectAll,
}

impl WriterReviewOp {
    #[must_use]
    pub fn effect(&self) -> EffectClass {
        match self {
            Self::TrackChanges { .. } | Self::AddComment { .. } => EffectClass::Edit,
            Self::DeleteComment { .. }
            | Self::DeleteAllComments
            | Self::AcceptRevisions { .. }
            | Self::RejectRevisions { .. }
            | Self::AcceptAll
            | Self::RejectAll => EffectClass::Destructive,
        }
    }
}

/// Runs `op` on `doc`.
pub fn apply(doc: &mut Document, op: &WriterReviewOp) -> Result<(), CommandError> {
    match op {
        WriterReviewOp::TrackChanges { on, author } => {
            doc.set_track_changes(on.then(|| TrackChanges::new(author)));
        }
        WriterReviewOp::AddComment {
            start,
            end,
            author,
            text,
        } => {
            doc.add_comment(*start, *end, &NewComment::new(author, text))?;
        }
        WriterReviewOp::DeleteComment { id } => doc.delete_comment(*id)?,
        WriterReviewOp::DeleteAllComments => {
            let roots: Vec<u32> = doc
                .comments()
                .into_iter()
                .filter(|c| c.parent.is_none())
                .map(|c| c.id)
                .collect();
            for id in roots {
                doc.delete_comment(id)?;
            }
        }
        WriterReviewOp::AcceptRevisions { indices } => {
            doc.accept_revisions(indices)?;
        }
        WriterReviewOp::RejectRevisions { indices } => {
            doc.reject_revisions(indices)?;
        }
        WriterReviewOp::AcceptAll => {
            doc.accept_all_revisions();
        }
        WriterReviewOp::RejectAll => {
            doc.reject_all_revisions();
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{Command, Target};

    fn target() -> Target {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/docx/plain-paragraphs.docx"),
        )
        .unwrap();
        Target::open_docx(&bytes).unwrap()
    }

    fn review(t: &mut Target, op: WriterReviewOp) {
        t.apply(Command::DocxReview { op }).unwrap();
    }

    #[test]
    fn tracked_edit_is_accepted_and_rejected() {
        let mut t = target();
        let original = t.docx().unwrap().paragraph_text(0).unwrap();
        review(
            &mut t,
            WriterReviewOp::TrackChanges {
                on: true,
                author: "Ada".to_owned(),
            },
        );
        t.apply(Command::DocxInsertText {
            para: 0,
            offset: 0,
            text: "NEW ".to_owned(),
        })
        .unwrap();
        let revs = t.docx().unwrap().revisions();
        assert_eq!(revs.len(), 1);
        assert_eq!(revs[0].author, "Ada");
        review(&mut t, WriterReviewOp::RejectAll);
        assert!(t.docx().unwrap().revisions().is_empty());
        assert_eq!(t.docx().unwrap().paragraph_text(0).unwrap(), original);
        t.undo().unwrap();
        assert_eq!(t.docx().unwrap().revisions().len(), 1);
        review(&mut t, WriterReviewOp::AcceptRevisions { indices: vec![0] });
        assert!(t.docx().unwrap().revisions().is_empty());
        assert!(
            t.docx()
                .unwrap()
                .paragraph_text(0)
                .unwrap()
                .starts_with("NEW ")
        );
    }

    #[test]
    fn comments_are_added_and_deleted() {
        let mut t = target();
        review(
            &mut t,
            WriterReviewOp::AddComment {
                start: DocPos::new(0, 0),
                end: DocPos::new(0, 3),
                author: "Ada Lovelace".to_owned(),
                text: String::new(),
            },
        );
        let all = t.docx().unwrap().comments();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].initials, "AL");
        review(&mut t, WriterReviewOp::DeleteComment { id: all[0].id });
        assert!(t.docx().unwrap().comments().is_empty());
        t.undo().unwrap();
        assert_eq!(t.docx().unwrap().comments().len(), 1);
    }
}
