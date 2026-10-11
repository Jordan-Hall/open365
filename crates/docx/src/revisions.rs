//! Format-aware views of revised paragraphs.
//!
//! [`Document::paragraph_format_runs`] answers "what does each character look like" for the
//! markup and final views, so a front end (and the Word parity tests) can compare formatting
//! before and after a revision is accepted or rejected.

use crate::document::Document;
use crate::model::{Inline, Paragraph};
use crate::review::{TextView, run_markup_text};
use crate::styles::EffectiveRun;

/// Text with one effective run format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatRun {
    pub text: String,
    pub format: EffectiveRun,
}

fn walk(
    p: &Paragraph,
    doc: &Document,
    content: &[Inline],
    view: TextView,
    hidden: bool,
    out: &mut Vec<FormatRun>,
) {
    let Some(styles) = doc.styles() else { return };
    for inline in content {
        match inline {
            Inline::Run(r) => {
                let mut text = String::new();
                if view == TextView::Markup {
                    run_markup_text(r, &mut text);
                } else if !hidden {
                    text = r.text();
                }
                if text.is_empty() {
                    continue;
                }
                let format = styles.effective_run_props(p, r);
                match out.last_mut() {
                    Some(last) if last.format == format => last.text.push_str(&text),
                    _ => out.push(FormatRun { text, format }),
                }
            }
            Inline::Group(g) => {
                let deleted = g.head.is_w("del") || g.head.is_w("moveFrom");
                walk(p, doc, &g.content, view, hidden || deleted, out);
            }
            Inline::Raw(_) => {}
        }
    }
}

impl Document {
    /// The text of paragraph `index` in `view`, split where the effective run format
    /// (styles resolved) changes. Empty when the document has no styles part.
    pub fn paragraph_format_runs(&self, index: usize, view: TextView) -> Option<Vec<FormatRun>> {
        let p = self.paragraph(index)?;
        let mut out = Vec::new();
        walk(p, self, &p.content, view, false, &mut out);
        Some(out)
    }

    /// Display name of the style of paragraph `index` (`Normal`, `heading 1`...).
    pub fn paragraph_style_name(&self, index: usize) -> Option<String> {
        let p = self.paragraph(index)?;
        let styles = self.styles()?;
        let id = p.style_id().or_else(|| styles.default_paragraph_style())?;
        Some(
            styles
                .style(id)
                .map_or_else(|| id.to_owned(), |s| s.name.clone()),
        )
    }
}
