//! Home tab support for the Writer pane: clipboard variants, font steps, colours, change case,
//! lists, indents, spacing, shading, borders, sort and the format painter.
//!
//! The pane struct has no field for the last-used colours or the painter, so they live in a
//! per-thread table (one Writer window per thread), as the View tab state does.

use std::cell::RefCell;

use open365_commands::{BorderKind, CaseMode, Command, ListKind, WriterHomeOp, char_format_xml};
use open365_docx::{DocPos, RunPropsPatch};

use super::WriterPane;

/// The word sizes Grow Font and Shrink Font step through, in half points.
const SIZES: [u32; 17] = [
    16, 18, 20, 21, 22, 24, 28, 32, 36, 40, 44, 48, 52, 56, 72, 96, 144,
];

/// Indent step of Increase Indent and Decrease Indent, points (half an inch).
const INDENT_STEP_PT: f64 = 36.0;

/// Home tab state of one window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeState {
    /// Last colour the Font Colour button applied (`RRGGBB`); red until another is picked.
    pub font_colour: String,
    /// Last highlight (`None` is No Color); yellow until another is picked.
    pub highlight: Option<String>,
    /// Last paragraph shading (`RRGGBB`).
    pub shading: String,
    /// Last border of the Borders button; the bottom border at first.
    pub border: BorderKind,
    /// Format Painter is armed: the formatting it carries (`None` is plain text).
    pub painter: Option<Option<String>>,
    /// Show All (formatting marks) is on.
    pub show_all: bool,
}

impl Default for HomeState {
    fn default() -> Self {
        Self {
            font_colour: "FF0000".to_owned(),
            highlight: Some("yellow".to_owned()),
            shading: "D9D9D9".to_owned(),
            border: BorderKind::Bottom,
            painter: None,
            show_all: false,
        }
    }
}

thread_local! {
    static STATE: RefCell<HomeState> = RefCell::new(HomeState::default());
}

fn half_points(size: u32, grow: bool) -> u32 {
    if grow {
        if let Some(next) = SIZES.iter().find(|s| **s > size) {
            return *next;
        }
        // Above 72 pt the steps are 10 pt.
        (size / 20 + 1) * 20
    } else {
        if let Some(prev) = SIZES.iter().rev().find(|s| **s < size)
            && size <= 144
        {
            return *prev;
        }
        if size > 144 {
            return ((size - 1) / 20 * 20).max(144);
        }
        // Below 8 pt the steps are 1 pt.
        size.saturating_sub(2).max(2)
    }
}

impl WriterPane {
    /// The Home tab state.
    #[must_use]
    pub fn home_state(&self) -> HomeState {
        STATE.with(|s| s.borrow().clone())
    }

    /// Changes the Home tab state.
    pub fn edit_home_state(&mut self, f: impl FnOnce(&mut HomeState)) {
        STATE.with(|s| f(&mut s.borrow_mut()));
    }

    /// The selection, or the word at the caret when nothing is selected.
    fn selection_or_word(&self) -> (DocPos, DocPos) {
        if self.has_selection() {
            self.selection()
        } else {
            self.word_at(self.caret)
        }
    }

    /// Paragraph numbers of the selection.
    fn selected_paragraphs(&self) -> (usize, usize) {
        let (a, b) = self.selection();
        (a.para, b.para)
    }

    /// Runs a Home operation and keeps the selection.
    fn home_op(&mut self, op: WriterHomeOp) -> bool {
        let (anchor, caret) = (self.anchor, self.caret);
        let ok = self.run(Command::DocxHome { op }).is_some();
        if ok {
            self.select(anchor, caret);
        }
        ok
    }

    /// Paste Special variants: this clipboard holds text only, so Paste Values, Paste
    /// Formatting, Paste Link and Paste Special all paste plain text.
    pub fn paste_plain(&mut self) {
        self.paste();
    }

    /// Grow Font and Shrink Font: the next or previous size of the Font Size list.
    pub fn grow_font(&mut self, grow: bool) {
        if self.models.is_empty() {
            return;
        }
        let current = self.models[self.caret.para]
            .style_at(self.caret.offset)
            .size_half_points;
        let size = half_points(self.pending.size_half_points.unwrap_or(current), grow);
        self.format(super::Format::Size(size));
    }

    /// Font Colour with `RRGGBB` or `auto`.
    pub fn set_font_colour(&mut self, colour: &str) {
        let hex = colour.trim().trim_start_matches('#');
        let auto = hex.eq_ignore_ascii_case("auto");
        if !(auto || hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())) {
            self.status = Some(format!("{colour} is not a colour"));
            return;
        }
        let value = if auto {
            "auto".to_owned()
        } else {
            hex.to_ascii_uppercase()
        };
        if !auto {
            let v = value.clone();
            self.edit_home_state(|s| s.font_colour = v);
        }
        let patch = RunPropsPatch {
            color: Some(value),
            ..RunPropsPatch::default()
        };
        if self.has_selection() {
            let (a, b) = self.selection();
            self.run(Command::DocxSetRunProps {
                start: a,
                end: b,
                patch,
            });
        } else {
            self.pending.color = patch.color;
        }
    }

    /// Text Highlight Colour: a name, `RRGGBB` or `none`.
    pub fn set_highlight(&mut self, colour: &str) {
        let c = colour.trim();
        let color = if c.eq_ignore_ascii_case("none") {
            None
        } else {
            Some(c.to_owned())
        };
        let kept = color.clone();
        self.edit_home_state(|s| s.highlight = kept);
        if !self.has_selection() {
            self.status = Some("Select text to highlight".to_owned());
            return;
        }
        let (start, end) = self.selection();
        self.home_op(WriterHomeOp::Highlight { start, end, color });
    }

    /// Underline with a style (`single`, `double`, `thick`, `dotted`, `dash`, `wave`...).
    pub fn set_underline_style(&mut self, style: &str) {
        if !self.has_selection() {
            self.status = Some("Select text to underline".to_owned());
            return;
        }
        let (start, end) = self.selection();
        self.home_op(WriterHomeOp::Underline {
            start,
            end,
            style: style.to_owned(),
        });
    }

    /// Change Case on the selection (or the word at the caret).
    pub fn change_case(&mut self, mode: CaseMode) {
        let (start, end) = self.selection_or_word();
        if start == end {
            return;
        }
        self.home_op(WriterHomeOp::ChangeCase { start, end, mode });
    }

    /// Clear All Formatting on the selection or the paragraph at the caret.
    pub fn clear_formatting(&mut self) {
        let (start, end) = self.selection();
        self.pending = RunPropsPatch::default();
        self.home_op(WriterHomeOp::ClearFormatting { start, end });
    }

    /// Bullets, Numbering and Multilevel List: sets or removes the list.
    pub fn toggle_list(&mut self, kind: ListKind) {
        let (first, last) = self.selected_paragraphs();
        self.home_op(WriterHomeOp::List { first, last, kind });
    }

    /// Increase Indent and Decrease Indent: half an inch on the left indent.
    pub fn change_indent(&mut self, increase: bool) {
        let (first, last) = self.selected_paragraphs();
        for para in first..=last {
            let Some(m) = self.models.get(para) else {
                continue;
            };
            let e = &m.para;
            let left = f64::from(e.indent_left_pt);
            let left = if increase {
                (left / INDENT_STEP_PT).floor() * INDENT_STEP_PT + INDENT_STEP_PT
            } else {
                ((left / INDENT_STEP_PT).ceil() * INDENT_STEP_PT - INDENT_STEP_PT).max(0.0)
            };
            self.run(Command::DocxSetIndents {
                para,
                left_pt: left,
                first_line_pt: f64::from(e.first_line_pt),
                right_pt: f64::from(e.indent_right_pt),
            });
        }
    }

    /// Line spacing as a multiple (1.0, 1.15, 1.5, 2.0...).
    pub fn set_line_spacing(&mut self, multiple: f64) {
        let (first, last) = self.selected_paragraphs();
        self.home_op(WriterHomeOp::Spacing {
            first,
            last,
            line: Some(multiple),
            before: None,
            after: None,
        });
    }

    /// Space before or after the paragraphs, points (Add Space Before is 12, Remove is 0).
    pub fn set_paragraph_space(&mut self, before: Option<f64>, after: Option<f64>) {
        let (first, last) = self.selected_paragraphs();
        self.home_op(WriterHomeOp::Spacing {
            first,
            last,
            line: None,
            before,
            after,
        });
    }

    /// Paragraph shading `RRGGBB`, or `none`.
    pub fn set_shading(&mut self, colour: &str) {
        let (first, last) = self.selected_paragraphs();
        let c = colour.trim();
        let fill = if c.eq_ignore_ascii_case("none") || c.eq_ignore_ascii_case("auto") {
            None
        } else {
            let hex = c.trim_start_matches('#').to_ascii_uppercase();
            self.edit_home_state(|s| s.shading = hex.clone());
            Some(hex)
        };
        self.home_op(WriterHomeOp::Shading { first, last, fill });
    }

    /// Borders menu.
    pub fn set_border(&mut self, border: BorderKind) {
        let (first, last) = self.selected_paragraphs();
        self.edit_home_state(|s| s.border = border);
        self.home_op(WriterHomeOp::Border {
            first,
            last,
            border,
        });
    }

    /// Sort Text ascending or descending over the selected paragraphs.
    pub fn sort_paragraphs(&mut self, descending: bool) {
        let (first, last) = self.selected_paragraphs();
        if first == last {
            self.status = Some("Select the paragraphs to sort".to_owned());
            return;
        }
        let (anchor, caret) = (self.anchor, self.caret);
        if self
            .run(Command::DocxHome {
                op: WriterHomeOp::Sort {
                    first,
                    last,
                    descending,
                },
            })
            .is_some()
        {
            self.select(anchor, caret);
        }
    }

    /// Format Painter: the first call picks up the formatting at the caret; the next call (or
    /// [`WriterPane::paint_format`]) puts it on the selection.
    pub fn format_painter(&mut self) {
        let armed = self.home_state().painter.is_some();
        if armed {
            self.paint_format();
            return;
        }
        let Some(doc) = self.target.docx() else {
            return;
        };
        let rpr = char_format_xml(doc, self.caret);
        self.edit_home_state(|s| s.painter = Some(rpr));
        self.status = Some("Format Painter: select the text to format".to_owned());
    }

    /// Applies the picked-up formatting to the selection and disarms the painter.
    pub fn paint_format(&mut self) {
        let Some(rpr) = self.home_state().painter else {
            return;
        };
        if !self.has_selection() {
            self.status = Some("Format Painter: select the text to format".to_owned());
            return;
        }
        let (start, end) = self.selection();
        self.edit_home_state(|s| s.painter = None);
        self.home_op(WriterHomeOp::FormatPainter { start, end, rpr });
    }

    /// Show All: formatting marks on or off.
    pub fn toggle_show_all(&mut self) {
        self.edit_home_state(|s| s.show_all = !s.show_all);
    }
}

#[cfg(test)]
mod tests {
    use super::half_points;

    #[test]
    fn grow_and_shrink_follow_the_size_list() {
        assert_eq!(half_points(22, true), 24, "11 pt grows to 12 pt");
        assert_eq!(half_points(24, false), 22);
        assert_eq!(half_points(144, true), 160);
        assert_eq!(half_points(16, false), 14, "below 8 pt the step is 1 pt");
    }
}
