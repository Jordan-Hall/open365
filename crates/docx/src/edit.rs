//! Text editing on a single paragraph. Offsets are Unicode scalar values (`char`s) into
//! [`Paragraph::text`]; tabs and line breaks count as one character each.

use std::ops::Range;

use crate::DocxError;
use crate::model::{Break, BreakKind, Inline, Paragraph, Run, RunContent};

/// What a string to insert turns into.
enum Piece {
    Text(String),
    Tab,
    Break,
}

/// XML 1.0 `Char` production, minus what `w:t` should never carry.
fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

/// Splits `text` into text, tab and break pieces. `\r\n` and `\r` become one break.
fn pieces_of(text: &str) -> Result<Vec<Piece>, DocxError> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if !is_xml_char(c) {
            return Err(DocxError::InvalidText(c));
        }
        let special = match c {
            '\t' => Some(Piece::Tab),
            '\n' => Some(Piece::Break),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                Some(Piece::Break)
            }
            _ => None,
        };
        match special {
            Some(piece) => {
                if !current.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut current)));
                }
                pieces.push(piece);
            }
            None => current.push(c),
        }
    }
    if !current.is_empty() {
        pieces.push(Piece::Text(current));
    }
    Ok(pieces)
}

/// True when `xml:space="preserve"` is needed for the text to survive: leading or
/// trailing whitespace, or consecutive whitespace.
pub fn needs_preserve(text: &str) -> bool {
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r');
    let mut previous_ws = false;
    for (i, c) in text.chars().enumerate() {
        let ws = is_ws(c);
        if ws && (i == 0 || previous_ws) {
            return true;
        }
        previous_ws = ws;
    }
    previous_ws
}

fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(byte, _)| byte)
}

fn text_item(text: String) -> RunContent {
    RunContent::Text {
        preserve_space: needs_preserve(&text),
        text,
        attrs: Vec::new(),
    }
}

impl Run {
    fn items_for(&self, pieces: Vec<Piece>) -> Vec<RunContent> {
        pieces
            .into_iter()
            .map(|piece| match piece {
                Piece::Text(t) => text_item(t),
                Piece::Tab => RunContent::Tab,
                Piece::Break => RunContent::Break(Break::new(BreakKind::TextWrapping, &self.head)),
            })
            .collect()
    }

    /// Inserts `pieces` at character offset `local` (0 ..= `char_len`) of this run.
    fn insert_pieces(&mut self, local: usize, mut pieces: Vec<Piece>) {
        // The first item with characters that ends at or after `local` (the first one
        // with characters when `local` is 0).
        let mut target = None;
        let mut acc = 0;
        for (i, item) in self.content.iter().enumerate() {
            let len = item.char_len();
            if len == 0 {
                continue;
            }
            if local == 0 || (local > acc && local <= acc + len) {
                target = Some((i, local - acc, len));
                break;
            }
            acc += len;
        }
        let Some((index, k, len)) = target else {
            let items = self.items_for(pieces);
            self.content.extend(items);
            return;
        };

        if pieces.len() == 1
            && let Some(Piece::Text(insert)) = pieces.first()
            && let RunContent::Text {
                text,
                preserve_space,
                ..
            } = &mut self.content[index]
        {
            text.insert_str(byte_index(text, k), insert);
            *preserve_space = *preserve_space || needs_preserve(text);
            return;
        }

        let insert_at = if k > 0 && k < len {
            // Only text can have more than one character; split it around the insertion.
            if let RunContent::Text {
                text,
                preserve_space,
                attrs,
            } = &mut self.content[index]
            {
                let right = text.split_off(byte_index(text, k));
                let original = *preserve_space;
                *preserve_space = original || needs_preserve(text);
                let right_item = RunContent::Text {
                    preserve_space: original || needs_preserve(&right),
                    text: right,
                    attrs: attrs.clone(),
                };
                self.content.insert(index + 1, right_item);
            }
            index + 1
        } else if k == 0 {
            index
        } else {
            index + 1
        };
        let items = self.items_for(std::mem::take(&mut pieces));
        self.content.splice(insert_at..insert_at, items);
    }

    /// Splits this run at character offset `local` (0 < `local` < `char_len`) and returns
    /// the right half, which has the same element name, attributes and properties.
    fn split_at(&mut self, local: usize) -> Run {
        let mut acc = 0;
        let mut after_last = 0;
        let mut split_index = self.content.len();
        for i in 0..self.content.len() {
            let len = self.content[i].char_len();
            if len == 0 {
                continue;
            }
            if local < acc + len {
                let k = local - acc;
                if k == 0 {
                    split_index = after_last;
                } else {
                    if let RunContent::Text {
                        text,
                        preserve_space,
                        attrs,
                    } = &mut self.content[i]
                    {
                        let right = text.split_off(byte_index(text, k));
                        let original = *preserve_space;
                        *preserve_space = original || needs_preserve(text);
                        let right_item = RunContent::Text {
                            preserve_space: original || needs_preserve(&right),
                            text: right,
                            attrs: attrs.clone(),
                        };
                        self.content.insert(i + 1, right_item);
                    }
                    split_index = i + 1;
                }
                break;
            }
            acc += len;
            after_last = i + 1;
        }
        let mut right = self.empty_clone();
        right.content = self.content.split_off(split_index);
        right
    }
}

/// Finds the run covering `offset`; pushes the index path to it. Returns the offset
/// within that run.
fn locate(
    content: &[Inline],
    offset: usize,
    acc: &mut usize,
    path: &mut Vec<usize>,
) -> Option<usize> {
    for (i, inline) in content.iter().enumerate() {
        match inline {
            Inline::Run(run) => {
                let len = run.char_len();
                if len > 0 && (offset == 0 || (offset > *acc && offset <= *acc + len)) {
                    path.push(i);
                    return Some(offset - *acc);
                }
                *acc += len;
            }
            Inline::Group(group) => {
                path.push(i);
                if let Some(local) = locate(&group.content, offset, acc, path) {
                    return Some(local);
                }
                path.pop();
            }
            Inline::Raw(_) => {}
        }
    }
    None
}

fn run_at_mut<'a>(content: &'a mut [Inline], path: &[usize]) -> Option<&'a mut Run> {
    let (first, rest) = path.split_first()?;
    match content.get_mut(*first)? {
        Inline::Run(run) if rest.is_empty() => Some(run),
        Inline::Group(group) => run_at_mut(&mut group.content, rest),
        _ => None,
    }
}

/// Ensures a run boundary at `offset`. Returns true once the boundary exists.
fn split_runs_at(content: &mut Vec<Inline>, offset: usize, acc: &mut usize) -> bool {
    let mut i = 0;
    while i < content.len() {
        match &mut content[i] {
            Inline::Run(run) => {
                let len = run.char_len();
                if len > 0 && offset > *acc && offset < *acc + len {
                    let right = run.split_at(offset - *acc);
                    content.insert(i + 1, Inline::Run(right));
                    return true;
                }
                *acc += len;
            }
            Inline::Group(group) => {
                if split_runs_at(&mut group.content, offset, acc) {
                    return true;
                }
            }
            Inline::Raw(_) => {}
        }
        i += 1;
    }
    false
}

fn apply_bold(content: &mut [Inline], range: &Range<usize>, value: bool, acc: &mut usize) {
    for inline in content {
        match inline {
            Inline::Run(run) => {
                let len = run.char_len();
                let (start, end) = (*acc, *acc + len);
                let inside = if len > 0 {
                    start >= range.start && end <= range.end
                } else {
                    start > range.start && start < range.end
                };
                if inside {
                    run.set_bold(Some(value));
                }
                *acc = end;
            }
            Inline::Group(group) => apply_bold(&mut group.content, range, value, acc),
            Inline::Raw(_) => {}
        }
    }
}

impl Paragraph {
    /// Inserts `text` at character `offset`, into the run covering that offset so that it
    /// inherits the run's properties. Tabs and line breaks in `text` become `w:tab` and
    /// `w:br`. An empty paragraph (no characters) gets a new run at the end.
    pub(crate) fn insert_text(&mut self, offset: usize, text: &str) -> Result<(), DocxError> {
        let len = self.char_len();
        if offset > len {
            return Err(DocxError::RangeOutOfBounds {
                start: offset,
                end: offset,
                len,
            });
        }
        let pieces = pieces_of(text)?;
        if pieces.is_empty() {
            return Ok(());
        }
        if len == 0 {
            let mut run = self.new_run();
            run.content = run.items_for(pieces);
            self.content.push(Inline::Run(run));
            return Ok(());
        }
        let mut path = Vec::new();
        let mut acc = 0;
        let local = locate(&self.content, offset, &mut acc, &mut path).ok_or(
            DocxError::RangeOutOfBounds {
                start: offset,
                end: offset,
                len,
            },
        )?;
        let run = run_at_mut(&mut self.content, &path).ok_or(DocxError::RangeOutOfBounds {
            start: offset,
            end: offset,
            len,
        })?;
        run.insert_pieces(local, pieces);
        Ok(())
    }

    /// Sets bold on the characters in `range`, splitting runs at the boundaries.
    pub(crate) fn set_bold(&mut self, range: Range<usize>, value: bool) -> Result<(), DocxError> {
        let len = self.char_len();
        if range.start > range.end || range.end > len {
            return Err(DocxError::RangeOutOfBounds {
                start: range.start,
                end: range.end,
                len,
            });
        }
        if range.start == range.end {
            return Ok(());
        }
        for boundary in [range.start, range.end] {
            let mut acc = 0;
            split_runs_at(&mut self.content, boundary, &mut acc);
        }
        let mut acc = 0;
        apply_bold(&mut self.content, &range, value, &mut acc);
        Ok(())
    }
}
