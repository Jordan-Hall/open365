//! Writer pane: the References tab. Tables of contents and figures, footnotes and endnotes,
//! captions, the index and the table of authorities. Every edit is a `Command::DocxReferences`
//! of the command layer (logged, one undo step); page numbers come from this pane's layout.
//!
//! Word asks for options in a dialog first (Insert Caption, Insert Index, Mark Entry...). The
//! pane has no dialog host yet, so each method takes Word's defaults, the values the dialog
//! offers before the person changes anything.

use open365_commands::{Command, FieldScope, RefOp};
use open365_docx::DocPos;

use super::WriterPane;

/// The built-in table of contents choices of the gallery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TocStyle {
    /// "Automatic Table 1": the title "Contents".
    Automatic1,
    /// "Automatic Table 2": the title "Table of Contents".
    Automatic2,
    /// "Custom Table of Contents...": no title, the dialog's default levels.
    Custom,
}

impl WriterPane {
    /// The first page (1-based) of every paragraph of the main story, from the current layout.
    #[must_use]
    pub fn paragraph_pages(&self) -> Vec<u32> {
        let mut pages = vec![0u32; self.models.len()];
        for page in &self.engine.layout().pages {
            for line in &page.lines {
                if let Some(slot) = pages.get_mut(line.paragraph)
                    && *slot == 0
                {
                    *slot = u32::try_from(page.number).unwrap_or(1);
                }
            }
        }
        // A paragraph without a line (an empty one in a table row end) sits on its neighbour's page.
        let mut prev = 1;
        for p in &mut pages {
            if *p == 0 {
                *p = prev;
            }
            prev = *p;
        }
        pages
    }

    /// The document as a `.docx` package, as saving would write it (for tests and tools).
    pub fn docx_bytes(&mut self) -> Option<Vec<u8>> {
        self.target.snapshot_bytes().ok()
    }

    fn refs(&mut self, op: RefOp) -> bool {
        self.run(Command::DocxReferences { op }).is_some()
    }

    /// The end of the selection, where marks and tables go.
    fn insertion_point(&self) -> DocPos {
        self.selection().1
    }

    fn doc_in_table(&self, para: usize) -> bool {
        self.target
            .docx()
            .is_some_and(|d| d.paragraph_in_table(para))
    }

    // ---- table of contents ---------------------------------------------------------------------

    /// "Automatic Table 1" of the gallery: the title "Contents" and a table built from the headings.
    pub fn insert_toc_automatic_1(&mut self) -> bool {
        self.insert_toc(TocStyle::Automatic1)
    }

    /// "Automatic Table 2": the title "Table of Contents".
    pub fn insert_toc_automatic_2(&mut self) -> bool {
        self.insert_toc(TocStyle::Automatic2)
    }

    /// "Custom Table of Contents...": no title, levels 1 to 3.
    pub fn insert_toc_custom(&mut self) -> bool {
        self.insert_toc(TocStyle::Custom)
    }

    fn insert_toc(&mut self, style: TocStyle) -> bool {
        let title = match style {
            TocStyle::Automatic1 => Some("Contents"),
            TocStyle::Automatic2 => Some("Table of Contents"),
            TocStyle::Custom => None,
        };
        let op = RefOp::InsertToc {
            at: self.caret,
            instruction: "TOC \\o \"1-3\" \\h \\z \\u".to_owned(),
            title: title.map(str::to_owned),
            pages: self.paragraph_pages(),
        };
        self.refs(op)
    }

    /// Update Table (Table of Contents group): page numbers and entries.
    pub fn update_toc(&mut self) -> bool {
        let pages = self.paragraph_pages();
        self.refs(RefOp::UpdateFields {
            scope: FieldScope::Toc,
            pages,
        })
    }

    /// Remove Table of Contents: deletes the table the caret is in, with its title.
    pub fn remove_toc(&mut self) -> bool {
        let para = self.caret.para;
        self.refs(RefOp::RemoveToc { para })
    }

    /// Add Text: gives the paragraphs of the selection the heading `level` (1 to 3), or with 0 the
    /// body style, so they appear in (or leave) the table of contents.
    pub fn toc_add_text(&mut self, level: u8) -> bool {
        let style = match level {
            0 => "Normal".to_owned(),
            n => format!("Heading{}", n.min(3)),
        };
        let (a, b) = self.selection();
        for para in a.para..=b.para {
            if self
                .run(Command::DocxSetParagraphStyle {
                    para,
                    style_id: style.clone(),
                })
                .is_none()
            {
                return false;
            }
        }
        true
    }

    // ---- footnotes and endnotes ----------------------------------------------------------------

    /// Insert Footnote / Insert Endnote: a note with its mark at the caret.
    pub fn insert_note(&mut self, endnote: bool) -> bool {
        let at = self.insertion_point();
        self.refs(RefOp::InsertNote {
            at,
            endnote,
            text: String::new(),
        })
    }

    /// As [`Self::insert_note`] with the note's text (for scripts and tests; Word types it in the
    /// note area).
    pub fn insert_note_with_text(&mut self, endnote: bool, text: &str) -> bool {
        let at = self.insertion_point();
        self.refs(RefOp::InsertNote {
            at,
            endnote,
            text: text.to_owned(),
        })
    }

    /// Next Footnote (`forward`) or Previous Footnote: moves the caret to the mark of the next
    /// note, wrapping around. `endnotes` steps through endnotes instead.
    pub fn step_note(&mut self, forward: bool, endnotes: bool) -> Option<DocPos> {
        let marks: Vec<_> = self
            .target
            .docx()?
            .note_references()
            .into_iter()
            .filter(|m| m.endnote == endnotes)
            .collect();
        let at = self.caret;
        let pick = if forward {
            marks.iter().find(|m| m.pos > at)
        } else {
            marks.iter().rev().find(|m| m.pos < at)
        };
        let m = pick.or(if forward { marks.first() } else { marks.last() })?;
        let n = marks.iter().position(|x| x.id == m.id).map_or(1, |i| i + 1);
        self.set_caret(m.pos);
        self.reveal_caret();
        self.status = Some(format!(
            "{} {n} of {}",
            if endnotes { "Endnote" } else { "Footnote" },
            marks.len()
        ));
        Some(m.pos)
    }

    // ---- captions and tables of figures -------------------------------------------------------

    /// Insert Caption: "Figure 1" below the current item, or "Table 1" above a table, in the
    /// Caption style, with the caret after the number ready for the title.
    pub fn insert_caption(&mut self, label: Option<&str>) -> bool {
        let para = self.selection().1.para;
        let in_table = self.doc_in_table(para);
        let label = label.unwrap_or(if in_table { "Table" } else { "Figure" });
        let above = in_table && label == "Table";
        let op = RefOp::InsertCaption {
            para,
            label: label.to_owned(),
            text: String::new(),
            above,
            pages: self.paragraph_pages(),
        };
        if !self.refs(op) {
            return false;
        }
        if !in_table {
            let at = if above { para } else { para + 1 };
            if let Some(m) = self.models.get(at) {
                self.set_caret(DocPos::new(at, m.char_len));
            }
        }
        true
    }

    /// Insert Table of Figures for the label "Figure".
    pub fn insert_table_of_figures(&mut self, label: &str) -> bool {
        let op = RefOp::InsertTableOfFigures {
            at: self.caret,
            label: label.to_owned(),
            pages: self.paragraph_pages(),
        };
        self.refs(op)
    }

    /// Update Table (Captions group): renumbers captions, then rebuilds the tables of figures.
    pub fn update_table_of_figures(&mut self) -> bool {
        let pages = self.paragraph_pages();
        self.refs(RefOp::UpdateFields {
            scope: FieldScope::Seq,
            pages: pages.clone(),
        }) && self.refs(RefOp::UpdateFields {
            scope: FieldScope::Figures,
            pages,
        })
    }

    // ---- index ---------------------------------------------------------------------------------

    /// The selected text, else the word at the caret.
    fn refs_selected_or_word(&self) -> String {
        if self.has_selection() {
            return self.selected_text();
        }
        let at = self.caret;
        let Some(m) = self.models.get(at.para) else {
            return String::new();
        };
        let chars: Vec<char> = m.text.chars().collect();
        let mut s = at.offset.min(chars.len());
        let mut e = s;
        while s > 0 && Self::is_word(chars[s - 1]) {
            s -= 1;
        }
        while e < chars.len() && Self::is_word(chars[e]) {
            e += 1;
        }
        chars[s..e].iter().collect()
    }

    /// Mark Entry: an index entry for the selected text (or the word at the caret) after it.
    pub fn mark_index_entry(&mut self) -> bool {
        let entry = self.refs_selected_or_word();
        if entry.trim().is_empty() || entry.contains('\n') {
            self.status = Some("Select the text to mark for the index.".to_owned());
            return false;
        }
        let at = self.insertion_point();
        let at = if self.has_selection() {
            at
        } else {
            self.refs_word_end(at)
        };
        self.refs(RefOp::MarkIndexEntry {
            at,
            entry,
            cross_reference: None,
        })
    }

    fn refs_word_end(&self, at: DocPos) -> DocPos {
        let Some(m) = self.models.get(at.para) else {
            return at;
        };
        let chars: Vec<char> = m.text.chars().collect();
        let mut e = at.offset.min(chars.len());
        while e < chars.len() && Self::is_word(chars[e]) {
            e += 1;
        }
        DocPos::new(at.para, e)
    }

    /// Insert Index at the caret.
    pub fn insert_index(&mut self) -> bool {
        let op = RefOp::InsertIndex {
            at: self.caret,
            pages: self.paragraph_pages(),
        };
        self.refs(op)
    }

    /// Update Index.
    pub fn update_index(&mut self) -> bool {
        let pages = self.paragraph_pages();
        self.refs(RefOp::UpdateFields {
            scope: FieldScope::Index,
            pages,
        })
    }

    // ---- table of authorities ------------------------------------------------------------------

    /// Mark Citation: the selection as a case citation (category 1) for the table of authorities.
    pub fn mark_citation(&mut self) -> bool {
        let text = self.refs_selected_or_word();
        if text.trim().is_empty() || text.contains('\n') {
            self.status = Some("Select the text to mark as a citation.".to_owned());
            return false;
        }
        let at = self.insertion_point();
        let at = if self.has_selection() {
            at
        } else {
            self.refs_word_end(at)
        };
        self.refs(RefOp::MarkCitation {
            at,
            long: text.clone(),
            short: text,
            category: 1,
        })
    }

    /// Insert Table of Authorities (all categories) at the caret.
    pub fn insert_table_of_authorities(&mut self) -> bool {
        let op = RefOp::InsertTableOfAuthorities {
            at: self.caret,
            pages: self.paragraph_pages(),
        };
        self.refs(op)
    }

    /// Update Table (Table of Authorities group).
    pub fn update_table_of_authorities(&mut self) -> bool {
        let pages = self.paragraph_pages();
        self.refs(RefOp::UpdateFields {
            scope: FieldScope::Authorities,
            pages,
        })
    }
}
