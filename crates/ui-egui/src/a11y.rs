//! Our own AccessKit subtrees for the custom views (ADR-004).
//!
//! egui already exposes its widgets. The paged document and the grid are custom canvases, so
//! they publish paragraphs, text runs and cells themselves. Parenting uses child [`Ui`]s with
//! `accessibility_parent`, because egui only attaches a node to a parent that was registered
//! through its widget or `Ui` stack.

use egui::epaint::text::CharIndex;
use egui::text::CCursor;
use egui::{Context, Galley, Id, Pos2, Rect, Sense, Ui, UiBuilder, accesskit};

/// AccessKit stores `word_starts` as `u8`, so one text run holds at most this many characters.
pub const MAX_CHARS_PER_RUN: usize = 255;

/// One published text run, kept so screen-reader selection requests map back to the document.
#[derive(Debug, Clone)]
pub struct RunEntry {
    pub node: accesskit::NodeId,
    pub para: usize,
    /// Offset of the run's first character within the paragraph.
    pub start: usize,
}

pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Id of the run holding `row`/`chunk` of a paragraph.
pub fn run_id(doc: Id, para: usize, row: usize, chunk: usize) -> Id {
    doc.with(("run", para, row, chunk))
}

pub fn paragraph_id(doc: Id, para: usize) -> Id {
    doc.with(("para", para))
}

/// Whether the context is building an accessibility tree this frame.
pub fn is_enabled(ctx: &Context, probe: Id) -> bool {
    ctx.accesskit_node_builder(probe, |_| ()).is_some()
}

/// Converts a caret offset in a paragraph to a (run node, character index) position.
pub fn text_position(
    doc: Id,
    para: usize,
    galley: &Galley,
    offset: usize,
) -> accesskit::TextPosition {
    let layout = galley.layout_from_cursor(CCursor::new(CharIndex(offset)));
    let column = layout.column.0;
    // A caret exactly on a chunk boundary belongs to the end of the previous chunk.
    let chunk = if column > 0 && column.is_multiple_of(MAX_CHARS_PER_RUN) {
        column / MAX_CHARS_PER_RUN - 1
    } else {
        column / MAX_CHARS_PER_RUN
    };
    accesskit::TextPosition {
        node: run_id(doc, para, layout.row, chunk).accesskit_id(),
        character_index: column - chunk * MAX_CHARS_PER_RUN,
    }
}

/// Publishes a paragraph node with one `TextRun` per laid-out row (split into chunks of at most
/// 255 characters). `origin` is where the galley's (0, 0) sits on screen. The last row carries a
/// trailing `\n` so assistive technology sees the paragraph end. Returns the published runs.
pub fn publish_paragraph(
    parent_ui: &mut Ui,
    doc: Id,
    para: usize,
    galley: &Galley,
    origin: Pos2,
    label: Option<&str>,
) -> Vec<RunEntry> {
    let para_id = paragraph_id(doc, para);
    let bounds = Rect::from_min_size(origin, galley.size());
    parent_ui.interact(bounds, para_id, Sense::hover());
    parent_ui.ctx().accesskit_node_builder(para_id, |b| {
        b.set_role(accesskit::Role::Paragraph);
        b.set_bounds(to_ak_rect(bounds));
        if let Some(label) = label {
            b.set_label(label.to_owned());
        }
    });

    let run_ui = parent_ui.new_child(
        UiBuilder::new()
            .id_salt(("a11y-runs", para))
            .max_rect(bounds)
            .accessibility_parent(para_id),
    );

    let mut entries = Vec::new();
    let row_count = galley.rows.len().max(1);
    let mut char_start = 0usize;
    for row_index in 0..row_count {
        let (glyphs, row_pos, row_rect, ends_with_newline) = match galley.rows.get(row_index) {
            Some(row) => (
                row.glyphs.as_slice(),
                row.pos,
                row.rect_without_leading_space(),
                row.ends_with_newline,
            ),
            None => (
                &[][..],
                Pos2::ZERO,
                Rect::from_min_size(Pos2::ZERO, egui::vec2(0.0, 16.0)),
                false,
            ),
        };
        let is_last = row_index + 1 == row_count;
        let mut value = String::new();
        let mut lengths: Vec<u8> = Vec::new();
        let mut positions: Vec<f32> = Vec::new();
        let mut widths: Vec<f32> = Vec::new();
        let mut word_starts: Vec<usize> = Vec::new();
        // A wrapped continuation row starts mid-word at the row boundary.
        let mut was_at_word_end = row_index > 0
            && galley
                .rows
                .get(row_index - 1)
                .is_some_and(|r| !r.ends_with_newline);
        for glyph in glyphs {
            let word = is_word_char(glyph.chr);
            if word && was_at_word_end {
                word_starts.push(lengths.len());
            }
            was_at_word_end = !word;
            let before = value.len();
            value.push(glyph.chr);
            lengths.push((value.len() - before) as u8);
            positions.push(glyph.pos.x - row_pos.x);
            widths.push(glyph.advance_width);
        }
        let row_width = row_rect.width();
        if ends_with_newline || is_last {
            value.push('\n');
            lengths.push(1);
            positions.push(row_width);
            widths.push(0.0);
        }

        let total = lengths.len();
        let chunks = total.div_ceil(MAX_CHARS_PER_RUN).max(1);
        let mut byte_start = 0usize;
        for chunk in 0..chunks {
            let from = chunk * MAX_CHARS_PER_RUN;
            let to = (from + MAX_CHARS_PER_RUN).min(total);
            let bytes: usize = lengths[from..to].iter().map(|l| usize::from(*l)).sum();
            let text = value[byte_start..byte_start + bytes].to_owned();
            byte_start += bytes;
            let id = run_id(doc, para, row_index, chunk);
            let x0 = positions.get(from).copied().unwrap_or(0.0);
            let x1 = if to > from {
                positions[to - 1] + widths[to - 1]
            } else {
                x0
            };
            let rect = Rect::from_min_max(
                Pos2::new(origin.x + row_rect.min.x + x0, origin.y + row_rect.min.y),
                Pos2::new(origin.x + row_rect.min.x + x1, origin.y + row_rect.max.y),
            );
            run_ui.interact(rect, id, Sense::hover());
            let starts: Vec<u8> = word_starts
                .iter()
                .filter(|w| **w >= from && **w < to)
                .map(|w| (*w - from) as u8)
                .collect();
            let chunk_positions: Vec<f32> = positions
                .get(from..to)
                .unwrap_or_default()
                .iter()
                .map(|p| p - x0)
                .collect();
            let chunk_widths = widths.get(from..to).unwrap_or_default().to_vec();
            let chunk_lengths = lengths[from..to].to_vec();
            run_ui.ctx().accesskit_node_builder(id, |b| {
                b.set_role(accesskit::Role::TextRun);
                b.set_text_direction(accesskit::TextDirection::LeftToRight);
                b.set_bounds(to_ak_rect(rect));
                b.set_value(text);
                b.set_character_lengths(chunk_lengths);
                b.set_character_positions(chunk_positions);
                b.set_character_widths(chunk_widths);
                b.set_word_starts(starts);
                if chunk > 0 {
                    b.set_previous_on_line(run_id(doc, para, row_index, chunk - 1).accesskit_id());
                }
                if chunk + 1 < chunks {
                    b.set_next_on_line(run_id(doc, para, row_index, chunk + 1).accesskit_id());
                }
            });
            entries.push(RunEntry {
                node: id.accesskit_id(),
                para,
                start: char_start + from,
            });
        }
        // Layout characters only: the paragraph-end newline added above is not part of them.
        char_start += glyphs.len() + usize::from(ends_with_newline);
    }
    entries
}

pub fn to_ak_rect(r: Rect) -> accesskit::Rect {
    accesskit::Rect {
        x0: f64::from(r.min.x),
        y0: f64::from(r.min.y),
        x1: f64::from(r.max.x),
        y1: f64::from(r.max.y),
    }
}
