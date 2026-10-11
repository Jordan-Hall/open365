//! Our own AccessKit subtrees for the custom views (ADR-004).
//!
//! egui already exposes its widgets. The paged document and the grid are custom canvases, so
//! they publish paragraphs, text runs and cells themselves. Parenting uses child [`Ui`]s with
//! `accessibility_parent`, because egui only attaches a node to a parent that was registered
//! through its widget or `Ui` stack.

use egui::{Context, Id, Rect, Sense, Ui, UiBuilder, accesskit};
use open365_layout::ParaRole;

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

/// One laid-out line of a paragraph as the screen reader sees it. Positions and widths are in
/// screen points, positions relative to the left edge of `rect`.
#[derive(Debug, Clone)]
pub struct LineData {
    pub rect: Rect,
    /// Offset of the line's first character within the paragraph.
    pub start: usize,
    pub chars: Vec<char>,
    /// Left edge of every character.
    pub positions: Vec<f32>,
    pub widths: Vec<f32>,
    /// The line is a wrapped continuation (its first word starts at the line start).
    pub continuation: bool,
    /// The last line of the paragraph carries the paragraph end as a trailing newline.
    pub last: bool,
}

/// Converts a caret offset to a (run node, character index) position. `line` is the index of
/// the line within the paragraph and `column` the character index within that line.
pub fn text_position(doc: Id, para: usize, line: usize, column: usize) -> accesskit::TextPosition {
    // A caret exactly on a chunk boundary belongs to the end of the previous chunk.
    let chunk = if column > 0 && column.is_multiple_of(MAX_CHARS_PER_RUN) {
        column / MAX_CHARS_PER_RUN - 1
    } else {
        column / MAX_CHARS_PER_RUN
    };
    accesskit::TextPosition {
        node: run_id(doc, para, line, chunk).accesskit_id(),
        character_index: column - chunk * MAX_CHARS_PER_RUN,
    }
}

/// Publishes an image node over `rect` with the picture's alternative text.
pub fn publish_image(parent_ui: &mut Ui, doc: Id, index: usize, rect: Rect, alt: Option<&str>) {
    let id = doc.with(("image", index));
    parent_ui.interact(rect, id, Sense::hover());
    parent_ui.ctx().accesskit_node_builder(id, |b| {
        b.set_role(accesskit::Role::Image);
        b.set_bounds(to_ak_rect(rect));
        if let Some(alt) = alt {
            b.set_label(alt.to_owned());
        }
    });
}

/// Publishes a paragraph node with one `TextRun` per laid-out line (split into chunks of at
/// most 255 characters). The last line carries a trailing newline so assistive technology sees the
/// paragraph end. Returns the published runs.
pub fn publish_paragraph(
    parent_ui: &mut Ui,
    doc: Id,
    para: usize,
    lines: &[LineData],
    label: Option<&str>,
    role: ParaRole,
) -> Vec<RunEntry> {
    let para_id = paragraph_id(doc, para);
    let bounds = lines
        .iter()
        .map(|l| l.rect)
        .reduce(|a, b| a.union(b))
        .unwrap_or(Rect::NOTHING);
    parent_ui.interact(bounds, para_id, Sense::hover());
    parent_ui.ctx().accesskit_node_builder(para_id, |b| {
        match role {
            ParaRole::Plain => b.set_role(accesskit::Role::Paragraph),
            ParaRole::Heading(level) => {
                b.set_role(accesskit::Role::Heading);
                b.set_level(usize::from(level));
            }
            ParaRole::ListItem => b.set_role(accesskit::Role::ListItem),
        }
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
    for (row_index, line) in lines.iter().enumerate() {
        let mut value = String::new();
        let mut lengths: Vec<u8> = Vec::new();
        let mut positions: Vec<f32> = line.positions.clone();
        let mut widths: Vec<f32> = line.widths.clone();
        let mut word_starts: Vec<usize> = Vec::new();
        let mut was_at_word_end = line.continuation;
        for ch in &line.chars {
            let word = is_word_char(*ch);
            if word && was_at_word_end {
                word_starts.push(lengths.len());
            }
            was_at_word_end = !word;
            let before = value.len();
            value.push(*ch);
            lengths.push((value.len() - before) as u8);
        }
        if line.last {
            value.push('\n');
            lengths.push(1);
            positions.push(line.rect.width());
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
                egui::pos2(line.rect.min.x + x0, line.rect.min.y),
                egui::pos2(line.rect.min.x + x1, line.rect.max.y),
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
                start: line.start + from,
            });
        }
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
