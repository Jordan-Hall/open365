//! Writer Find and Replace: replace one match (the Replace button of the dialog and the
//! pane), with Word's special codes and wildcard groups in the replacement.
//!
//! The match is the range `find_all` gave the pane. It is checked against the current text
//! first, so a stale range is refused and nothing changes. The edit is one undo step.

use open365_docx::{DocMatch, DocPos, Document, FindOptions};
use serde::{Deserialize, Serialize};

use crate::EffectClass;
use crate::error::CommandError;

/// Replaces the match `para:start` to `end_para:end` of `find` with `replacement`.
///
/// `^&`, `^p`, `^t`, `^l`, `^m`, `^n`, `^~`, `^-`, `^s`, `^c` and `^^` work in `replacement`
/// (`^c` is `clipboard`); with `wildcards`, `\1` to `\9` are the groups of `find`. `^p`
/// splits the paragraph. `end_para` is `para` when left out (a match inside one paragraph).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaceOne {
    pub find: String,
    pub replacement: String,
    #[serde(default)]
    pub match_case: bool,
    #[serde(default)]
    pub whole_word: bool,
    #[serde(default)]
    pub wildcards: bool,
    pub para: usize,
    pub start: usize,
    #[serde(default)]
    pub end_para: Option<usize>,
    pub end: usize,
    /// The clipboard text for `^c`.
    #[serde(default)]
    pub clipboard: Option<String>,
}

impl ReplaceOne {
    /// A replacement of a match found by [`Document::find_all`].
    #[must_use]
    pub fn new(opts: &FindOptions, replacement: &str, m: &DocMatch) -> Self {
        Self {
            find: opts.pattern.clone(),
            replacement: replacement.to_owned(),
            match_case: opts.match_case,
            whole_word: opts.whole_word,
            wildcards: opts.wildcards,
            para: m.para,
            start: m.start,
            end_para: (m.end_para != m.para).then_some(m.end_para),
            end: m.end,
            clipboard: None,
        }
    }

    #[must_use]
    pub fn options(&self) -> FindOptions {
        FindOptions {
            pattern: self.find.clone(),
            match_case: self.match_case,
            whole_word: self.whole_word,
            wildcards: self.wildcards,
        }
    }

    #[must_use]
    pub fn matched(&self) -> DocMatch {
        DocMatch {
            para: self.para,
            start: self.start,
            end: self.end,
            end_para: self.end_para.unwrap_or(self.para),
        }
    }

    /// The first and last paragraph the edit can touch.
    #[must_use]
    pub fn paragraphs(&self) -> (usize, usize) {
        (self.para, self.end_para.unwrap_or(self.para).max(self.para))
    }

    /// The effect class: it replaces existing text.
    #[must_use]
    pub fn effect(&self) -> EffectClass {
        EffectClass::Destructive
    }
}

/// Runs the replacement on `doc`.
///
/// # Errors
/// The pattern or replacement is invalid, the match is stale, or the document refuses the
/// edit; the document is unchanged then.
pub fn apply(doc: &mut Document, op: &ReplaceOne) -> Result<(), CommandError> {
    doc.replace_match(
        &op.options(),
        &op.replacement,
        op.clipboard.as_deref(),
        &op.matched(),
    )?;
    Ok(())
}

/// Where the caret goes after [`apply`]: just after the new text. Call it before `apply`.
///
/// # Errors
/// As [`apply`].
pub fn caret_after(doc: &Document, op: &ReplaceOne) -> Result<DocPos, CommandError> {
    let text = doc.replacement_text(
        &op.options(),
        &op.replacement,
        op.clipboard.as_deref(),
        &op.matched(),
    )?;
    let is_mark = |c: char| c == '\r' || c == '\u{3}';
    let marks = text.chars().filter(|c| is_mark(*c)).count();
    if marks == 0 {
        return Ok(DocPos::new(op.para, op.start + text.chars().count()));
    }
    let tail = text.chars().rev().take_while(|c| !is_mark(*c)).count();
    Ok(DocPos::new(op.para + marks, tail))
}
