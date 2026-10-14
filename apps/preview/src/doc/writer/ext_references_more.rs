//! Writer pane: the References tab's citations, bibliography and cross-references.
//!
//! Like Word's, the list of sources (the "master list") and the chosen style belong to the
//! person, not to one document, so they live here for the whole process. The "current list" of
//! a document is the set of sources its `CITATION` fields cite.

use std::sync::{Mutex, MutexGuard, PoisonError};

use open365_commands::Command;
use open365_commands::{EntrySeg, MoreOp};
use open365_docx::{
    BibStyle, DocPos, Source, bibliography_entry, citation_instruction, citation_text,
    parse_citation, sorted_sources,
};

use super::WriterPane;

static MASTER: Mutex<Vec<Source>> = Mutex::new(Vec::new());
static STYLE: Mutex<BibStyle> = Mutex::new(BibStyle::Apa);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The length of a caption's `Label N` prefix, when `text` starts with one.
fn caption_prefix(text: &str, label: &str) -> Option<usize> {
    let rest = text.strip_prefix(label)?.strip_prefix(' ')?;
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    (digits > 0).then(|| label.chars().count() + 1 + digits)
}

impl WriterPane {
    // ---- sources and style --------------------------------------------------------------------

    /// The master list of sources.
    #[must_use]
    pub fn bib_sources(&self) -> Vec<Source> {
        lock(&MASTER).clone()
    }

    /// Adds a source to the master list; one with the same tag is replaced.
    pub fn add_source(&mut self, source: Source) {
        let mut list = lock(&MASTER);
        match list.iter_mut().find(|s| s.tag == source.tag) {
            Some(slot) => *slot = source,
            None => list.push(source),
        }
    }

    /// Removes a source (Manage Sources, Delete).
    pub fn remove_source(&mut self, tag: &str) {
        lock(&MASTER).retain(|s| s.tag != tag);
    }

    /// The style of the Style box.
    #[must_use]
    pub fn bib_style(&self) -> BibStyle {
        *lock(&STYLE)
    }

    pub fn set_bib_style(&mut self, style: BibStyle) {
        *lock(&STYLE) = style;
        self.status = Some(format!("Style: {}", style.label()));
    }

    /// The sources the document cites, in master-list order.
    #[must_use]
    pub fn cited_sources(&self) -> Vec<Source> {
        let Some(doc) = self.target.docx() else {
            return Vec::new();
        };
        let tags: Vec<String> = doc
            .fields()
            .iter()
            .filter_map(|f| parse_citation(&f.instruction.text).map(|(tag, _)| tag))
            .collect();
        lock(&MASTER)
            .iter()
            .filter(|s| tags.contains(&s.tag))
            .cloned()
            .collect()
    }

    fn more(&mut self, op: MoreOp) -> bool {
        self.run(Command::DocxReferencesMore { op }).is_some()
    }

    // ---- citations and bibliography -----------------------------------------------------------

    /// Insert Citation: a `CITATION` field for the source `tag` at the end of the selection,
    /// showing the in-text citation of the current style (`pages` is the optional page range).
    pub fn insert_citation(&mut self, tag: &str, pages: &str) -> bool {
        let Some(source) = lock(&MASTER).iter().find(|s| s.tag == tag).cloned() else {
            self.status = Some(format!("There is no source named {tag}"));
            return false;
        };
        let result = citation_text(&source, self.bib_style(), pages);
        let at = self.selection().1;
        let len = result.chars().count();
        if !self.more(MoreOp::InsertCitation {
            at,
            instruction: citation_instruction(tag, pages),
            result,
        }) {
            return false;
        }
        self.set_caret(DocPos::new(at.para, at.offset + len));
        self.reveal_caret();
        true
    }

    /// Insert Citation with nothing chosen: the first source of the master list, by name.
    pub fn insert_first_citation(&mut self) -> bool {
        let list = sorted_sources(&self.bib_sources());
        match list.first() {
            Some(s) => self.insert_citation(&s.tag.clone(), ""),
            None => {
                self.status = Some("Add a source first (Manage Sources)".to_owned());
                false
            }
        }
    }

    /// Bibliography gallery: the cited sources in the current style under `heading` (`None`
    /// for "Insert Bibliography", which has no heading), at the caret.
    pub fn insert_bibliography(&mut self, heading: Option<&str>) -> bool {
        let style = self.bib_style();
        let cited = sorted_sources(&self.cited_sources());
        if cited.is_empty() {
            self.status = Some("The document cites no source".to_owned());
            return false;
        }
        let entries = cited
            .iter()
            .map(|s| {
                bibliography_entry(s, style)
                    .into_iter()
                    .map(|g| EntrySeg {
                        text: g.text,
                        italic: g.italic,
                    })
                    .collect()
            })
            .collect();
        let at = self.selection().1;
        self.more(MoreOp::InsertBibliography {
            at,
            heading: heading.map(str::to_owned),
            entries,
        })
    }

    // ---- cross-references ---------------------------------------------------------------------

    /// The paragraphs that are captions of `label`.
    #[must_use]
    pub fn caption_paragraphs(&self, label: &str) -> Vec<usize> {
        self.models
            .iter()
            .enumerate()
            .filter(|(_, m)| caption_prefix(&m.text, label).is_some())
            .map(|(i, _)| i)
            .collect()
    }

    /// Cross-reference to the caption of `label` nearest above the caret (else the first),
    /// inserted at the caret as a `REF` (or `PAGEREF`) field to a hidden bookmark.
    ///
    /// `kind` is `"number"` (only label and number), `"entire"` (entire caption) or `"page"`.
    pub fn insert_caption_cross_reference(&mut self, label: &str, kind: &str) -> bool {
        let caps = self.caption_paragraphs(label);
        let at = self.selection().1;
        let Some(&para) = caps
            .iter()
            .rev()
            .find(|&&p| p <= at.para)
            .or_else(|| caps.first())
        else {
            self.status = Some(format!("There are no {label} captions to refer to"));
            return false;
        };
        let text = self.models[para].text.clone();
        let total = self.models[para].char_len;
        let prefix = caption_prefix(&text, label).unwrap_or(total);
        let (end, field, switches, result) = match kind {
            "entire" => (total, "REF", "\\h", text.clone()),
            "page" => (
                prefix,
                "PAGEREF",
                "\\h",
                self.paragraph_pages()
                    .get(para)
                    .copied()
                    .unwrap_or(1)
                    .to_string(),
            ),
            _ => (
                prefix,
                "REF",
                "\\h",
                text.chars().take(prefix).collect::<String>(),
            ),
        };
        let code = usize::from(kind == "entire");
        let len = result.chars().count();
        let ok = self.more(MoreOp::CrossReference {
            at,
            target_start: DocPos::new(para, 0),
            target_end: DocPos::new(para, end),
            bookmark: format!("_Ref{}", 400_000_000 + para * 10 + code),
            field: field.to_owned(),
            switches: switches.to_owned(),
            result,
        });
        if ok {
            self.set_caret(DocPos::new(at.para, at.offset + len));
            self.reveal_caret();
        }
        ok
    }
}
