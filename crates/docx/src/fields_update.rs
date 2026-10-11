//! `Document::fields` and `Document::update_fields`: finding fields, evaluating them and
//! rewriting their cached results the way Word's `Fields.Update` does.

use std::cell::OnceCell;
use std::cmp::Ordering;

use crate::DocxError;
use crate::document::Document;
use crate::edit::needs_preserve;
use crate::fields::{
    Field, FieldContext, FieldKind, FieldUpdateReport, GeneralFormat, Instruction, SkipReason,
    SkippedField,
};
use crate::fields_eval::{Evaluator, Outcome, View};
use crate::fields_scan::{
    Loc, RawField, Scanner, Slot, blocks_mut, fld_char, group_at_mut, inlines_mut, new_run,
    paragraph_at, paragraph_ref, run_at_mut, slots, strip_between,
};
use crate::fields_snap::Snapshot;
use crate::model::{Block, Break, BreakKind, Inline, Paragraph, Run, RunContent};
use crate::props::RunProps;
use crate::xml::{ElementHead, RawXml};

/// The scan of the main story that one `update_fields` call keeps current: the fields, the
/// place of every paragraph and the lazily built snapshot the evaluators read. A result
/// written inside one paragraph only rescans that paragraph; anything else rescans the story.
pub(crate) struct UpdateState {
    pub raws: Vec<RawField>,
    pub slots: Vec<Slot>,
    pub snap: OnceCell<Snapshot>,
    /// Paragraph insertions and removals so far as `(first paragraph affected, delta)`, so
    /// positions in the edited story can be handed to the page provider in the numbering it
    /// was built for.
    pub shifts: Vec<(usize, isize)>,
}

impl UpdateState {
    pub(crate) fn new(doc: &Document) -> Self {
        Self {
            raws: Scanner::scan(doc.body()),
            slots: slots(doc.body()),
            snap: OnceCell::new(),
            shifts: Vec::new(),
        }
    }

    /// Rescans the whole story after an edit that began after paragraph `after`, noting how many
    /// paragraphs it added or removed.
    pub(crate) fn refresh_after(&mut self, doc: &Document, after: usize) {
        let before = self.slots.len();
        self.refresh(doc);
        let delta = self.slots.len() as isize - before as isize;
        if delta != 0 {
            self.shifts.push((after + 1, delta));
        }
    }

    /// Rescans the whole story.
    pub(crate) fn refresh(&mut self, doc: &Document) {
        self.raws = Scanner::scan(doc.body());
        self.slots = slots(doc.body());
        self.snap = OnceCell::new();
    }

    /// Call after a write that touched only paragraph `para` and left the paragraph count as
    /// it was. Rescans just that paragraph when no field reaches into it from outside.
    fn refresh_paragraph(&mut self, doc: &Document, para: usize) {
        self.snap = OnceCell::new();
        if !self.try_refresh_paragraph(doc, para) {
            self.refresh(doc);
        }
    }

    fn try_refresh_paragraph(&mut self, doc: &Document, para: usize) -> bool {
        let start = self.raws.partition_point(|f| f.begin.para < para);
        let end = self.raws.partition_point(|f| f.begin.para <= para);
        // Every field of the paragraph must close in it, and no other field may span it.
        let closes_here = self.raws[start..end].iter().all(|f| {
            f.simple
                || f.end
                    .as_ref()
                    .is_some_and(|e| e.para == para && f.parent.is_none_or(|p| p >= start))
        });
        if !closes_here {
            return false;
        }
        let spanned = self.raws[..start]
            .iter()
            .any(|f| f.end.as_ref().is_none_or(|e| e.para >= para));
        if spanned {
            return false;
        }
        let Some(paragraph) = self
            .slots
            .get(para)
            .and_then(|slot| paragraph_ref(doc.body(), slot))
        else {
            return false;
        };
        let mut fresh = Scanner::scan_one(para, paragraph);
        for f in &mut fresh {
            f.parent = f.parent.map(|p| p + start);
        }
        let old_len = end - start;
        let new_len = fresh.len();
        self.raws.splice(start..end, fresh);
        if new_len != old_len {
            for f in &mut self.raws[start + new_len..] {
                f.parent = f
                    .parent
                    .map(|p| if p >= end { p + new_len - old_len } else { p });
            }
        }
        true
    }
}

impl Document {
    /// The fields of the main story in begin-marker order. Parsing never changes the document.
    #[must_use]
    pub fn fields(&self) -> Vec<Field> {
        Scanner::scan(self.body())
            .into_iter()
            .enumerate()
            .map(|(index, raw)| to_field(index, &raw))
            .collect()
    }

    /// Updates every field Open365 can evaluate, in document order and innermost first, the way
    /// Word's `Fields.Update` does. Locked fields and fields of unsupported types keep their
    /// results; so do fields whose input the context cannot give (page numbers without a
    /// layout provider). Fields whose new result equals the cached one are not touched, so an
    /// up-to-date document stays byte-identical.
    pub fn update_fields(
        &mut self,
        ctx: &FieldContext<'_>,
    ) -> Result<FieldUpdateReport, DocxError> {
        let mut report = FieldUpdateReport::default();
        let evaluator = Evaluator::new(self, ctx);
        let mut state = UpdateState::new(self);
        let mut next = 0usize;
        while next < state.raws.len() {
            let advance = self.update_at(next, &evaluator, &mut state, &mut report)?;
            next += advance.max(1);
        }
        Ok(report)
    }

    /// Updates one field (and the fields nested in its instruction) by its index in
    /// [`Document::fields`].
    pub fn update_field(
        &mut self,
        index: usize,
        ctx: &FieldContext<'_>,
    ) -> Result<FieldUpdateReport, DocxError> {
        let mut report = FieldUpdateReport::default();
        let evaluator = Evaluator::new(self, ctx);
        let mut state = UpdateState::new(self);
        if index >= state.raws.len() {
            return Ok(report);
        }
        self.update_at(index, &evaluator, &mut state, &mut report)?;
        Ok(report)
    }

    /// Processes field `index`: first the fields in its instruction, then the field itself.
    /// Returns how many indices the caller can advance past.
    fn update_at(
        &mut self,
        index: usize,
        evaluator: &Evaluator<'_>,
        state: &mut UpdateState,
        report: &mut FieldUpdateReport,
    ) -> Result<usize, DocxError> {
        // Fields nested in the instruction first.
        let mut k = 0usize;
        loop {
            let kids = instruction_children(&state.raws, index);
            let Some(&kid) = kids.get(k) else { break };
            self.update_at(kid, evaluator, state, report)?;
            k += 1;
        }
        let Some(raw) = state.raws.get(index).cloned() else {
            return Ok(1);
        };
        let instruction = Instruction::parse(&raw.instr);
        let name = instruction.name.clone();
        let skip = |report: &mut FieldUpdateReport, reason: SkipReason| {
            report.skipped.push(SkippedField {
                index,
                name: name.clone(),
                reason,
            });
            Ok(1)
        };
        if raw.locked {
            return skip(report, SkipReason::Locked);
        }
        if instruction.kind == FieldKind::Other {
            return skip(report, SkipReason::UnsupportedType);
        }
        if !raw.simple && raw.end.is_none() {
            return skip(report, SkipReason::UnsupportedType);
        }
        let outcome = {
            let view = View::new(
                self,
                &state.raws,
                &state.snap,
                &evaluator.styles,
                &state.shifts,
            );
            evaluator.evaluate(&view, index, &instruction)
        };
        match outcome {
            Outcome::Skip(reason) => skip(report, reason),
            Outcome::Text { text, merge } => {
                let policy = if merge { Policy::Merge } else { Policy::Code };
                if text != raw.result_text || raw.separate.is_none() && !raw.simple {
                    self.write_result(&state.slots, &state.raws, index, &text, policy)?;
                    let one_paragraph = raw.end.as_ref().is_none_or(|e| e.para == raw.begin.para)
                        && raw
                            .separate
                            .as_ref()
                            .is_none_or(|s| s.para == raw.begin.para);
                    if one_paragraph && !text.contains('\r') {
                        state.refresh_paragraph(self, raw.begin.para);
                    } else {
                        state.refresh(self);
                    }
                }
                report.updated += 1;
                Ok(1 + descendants(&state.raws, index))
            }
            Outcome::Keep => {
                report.updated += 1;
                Ok(1)
            }
            Outcome::Toc(toc) => {
                let current = raw.result_text.trim_end_matches('\r');
                let fresh = toc.result_text();
                if raw.separate.is_none() || raw.simple || fresh.trim_end_matches('\r') != current {
                    crate::fields_toc::write_toc(self, index, &toc)?;
                    state.refresh_after(self, raw.begin.para);
                }
                report.updated += 1;
                Ok(1 + descendants(&state.raws, index))
            }
        }
    }

    /// Replaces the result of field `index` of `raws` with `text`.
    pub(crate) fn write_result(
        &mut self,
        all_slots: &[Slot],
        raws: &[RawField],
        index: usize,
        text: &str,
        policy: Policy,
    ) -> Result<(), DocxError> {
        let raw = &raws[index];
        let run_head = raw
            .run_head
            .clone()
            .or_else(|| self.first_run_head(raw.begin.para))
            .unwrap_or_else(default_run_head);
        let segments = result_segments(raw, text, policy);
        if raw.simple {
            return self.write_simple(all_slots, raw, &run_head, &segments);
        }
        let (Some(sep), Some(end)) = (raw.separate.clone(), raw.end.clone()) else {
            return self.write_missing_separator(all_slots, raw, &run_head, &segments);
        };
        self.write_complex(all_slots, &sep, &end, &run_head, &segments)
    }

    /// Replaces what lies between the separator `sep` and the end marker `end`.
    fn write_complex(
        &mut self,
        all_slots: &[Slot],
        sep: &Loc,
        end: &Loc,
        run_head: &ElementHead,
        segments: &[(String, RunProps)],
    ) -> Result<(), DocxError> {
        let segments = segments.to_vec();
        let (sep, end, run_head) = (sep.clone(), end.clone(), run_head.clone());
        let (Some(sep_slot), Some(end_slot)) = (
            all_slots.get(sep.para).cloned(),
            all_slots.get(end.para).cloned(),
        ) else {
            return Err(DocxError::Xml(
                "field marker outside the paragraph list".to_owned(),
            ));
        };
        if sep_slot.steps != end_slot.steps || sep_slot.pos > end_slot.pos {
            return Err(DocxError::Xml(
                "field spans different containers".to_owned(),
            ));
        }
        let steps = sep_slot.steps.clone();
        // Paragraph texts of the new result: pieces between `\r`.
        let pieces: Vec<Vec<(String, RunProps)>> = split_paragraphs(&segments);
        {
            let body = self.body_mut();
            let blocks = blocks_mut(body, &steps)
                .ok_or_else(|| DocxError::Xml("bad field container".to_owned()))?;
            merge_region(blocks, &sep, &end, &sep_slot, &end_slot)?;
            // Insert the new result after the separator run.
            let Block::Paragraph(p) = &mut blocks[sep_slot.pos] else {
                return Err(DocxError::Xml("marker paragraph missing".to_owned()));
            };
            insert_after_marker(
                p,
                &sep,
                &run_head,
                &pieces,
                sep_slot.pos,
                blocks_len_hint(0),
            )?;
            // `insert_after_marker` handled the first piece; the others become paragraphs.
            if pieces.len() > 1 {
                split_into_paragraphs(blocks, sep_slot.pos, &sep, &run_head, &pieces)?;
            }
        }
        self.edited();
        Ok(())
    }

    fn first_run_head(&self, _para: usize) -> Option<ElementHead> {
        None
    }

    fn write_simple(
        &mut self,
        all: &[Slot],
        raw: &RawField,
        run_head: &ElementHead,
        segments: &[(String, RunProps)],
    ) -> Result<(), DocxError> {
        let para = raw.begin.para;
        let slot = all
            .get(para)
            .cloned()
            .ok_or_else(|| DocxError::Xml("field paragraph missing".to_owned()))?;
        if segments.iter().any(|(t, _)| t.contains('\r')) {
            return self.write_simple_split(all, &slot, raw, run_head, segments);
        }
        let paragraph = paragraph_at(self.body_mut(), &slot)
            .ok_or_else(|| DocxError::Xml("field paragraph missing".to_owned()))?;
        let group = group_at_mut(&mut paragraph.content, &raw.begin.path)
            .ok_or_else(|| DocxError::Xml("fldSimple missing".to_owned()))?;
        let joined = join_with_breaks(segments);
        group.content = joined
            .into_iter()
            .map(|(text, props)| {
                Inline::Run(new_run(run_head, props, run_content(&text, run_head)))
            })
            .collect();
        self.edited();
        Ok(())
    }

    /// A result with paragraph marks cannot live in `w:fldSimple`: turn the field into a
    /// complex field (begin, instruction, separator, end), then write it there.
    fn write_simple_split(
        &mut self,
        all: &[Slot],
        slot: &Slot,
        raw: &RawField,
        run_head: &ElementHead,
        segments: &[(String, RunProps)],
    ) -> Result<(), DocxError> {
        let paragraph = paragraph_at(self.body_mut(), slot)
            .ok_or_else(|| DocxError::Xml("field paragraph missing".to_owned()))?;
        let (list, at) = inlines_mut(&mut paragraph.content, &raw.begin.path)
            .ok_or_else(|| DocxError::Xml("bad fldSimple path".to_owned()))?;
        let Some(Inline::Group(group)) = list.get(at) else {
            return Err(DocxError::Xml("fldSimple missing".to_owned()));
        };
        let props = group
            .content
            .iter()
            .find_map(|i| match i {
                Inline::Run(r) => Some(r.props.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let mut instr_el = run_head.sibling("instrText").into_empty();
        instr_el.head.attrs.push(crate::xml::Attr {
            name: "xml:space".to_owned(),
            ns: Some(std::sync::Arc::from("http://www.w3.org/XML/1998/namespace")),
            value: "preserve".to_owned(),
        });
        instr_el.children.push(RawXml::Text(raw.instr.clone()));
        let mut begin = fld_char(run_head, "begin");
        if let RunContent::Raw(RawXml::Element(e)) = &mut begin {
            if raw.locked {
                e.head.set_attr_w("fldLock", "true");
            }
            if raw.dirty {
                e.head.set_attr_w("dirty", "true");
            }
        }
        let runs = vec![
            Inline::Run(new_run(run_head, props.clone(), vec![begin])),
            Inline::Run(new_run(
                run_head,
                props.clone(),
                vec![RunContent::Raw(RawXml::Element(instr_el))],
            )),
            Inline::Run(new_run(
                run_head,
                props.clone(),
                vec![fld_char(run_head, "separate")],
            )),
            Inline::Run(new_run(run_head, props, vec![fld_char(run_head, "end")])),
        ];
        list.splice(at..=at, runs);
        // Find the field again: the converted field is the complex one at this path.
        let fresh = {
            let paragraph = paragraph_ref(self.body(), slot)
                .ok_or_else(|| DocxError::Xml("field paragraph missing".to_owned()))?;
            Scanner::scan_one(raw.begin.para, paragraph)
        };
        let converted = fresh
            .iter()
            .find(|f| !f.simple && f.begin.path.first() == raw.begin.path.first())
            .ok_or_else(|| DocxError::Xml("converted field missing".to_owned()))?;
        let (Some(sep), Some(end)) = (converted.separate.clone(), converted.end.clone()) else {
            return Err(DocxError::Xml("converted field incomplete".to_owned()));
        };
        self.write_complex(all, &sep, &end, run_head, segments)
    }

    fn write_missing_separator(
        &mut self,
        all: &[Slot],
        raw: &RawField,
        run_head: &ElementHead,
        segments: &[(String, RunProps)],
    ) -> Result<(), DocxError> {
        let Some(end) = raw.end.clone() else {
            return Err(DocxError::Xml("field without end".to_owned()));
        };
        let slot = all
            .get(end.para)
            .cloned()
            .ok_or_else(|| DocxError::Xml("field paragraph missing".to_owned()))?;
        let paragraph = paragraph_at(self.body_mut(), &slot)
            .ok_or_else(|| DocxError::Xml("field paragraph missing".to_owned()))?;
        let (list, at) = inlines_mut(&mut paragraph.content, &end.path)
            .ok_or_else(|| DocxError::Xml("bad end path".to_owned()))?;
        // Split the end run so that the new content lands in front of the end marker.
        let mut insert_at = at;
        if let Some(Inline::Run(run)) = list.get_mut(at)
            && end.item > 0
        {
            let tail = run.content.split_off(end.item);
            let mut rest = run.empty_clone();
            rest.content = tail;
            list.insert(at + 1, Inline::Run(rest));
            insert_at = at + 1;
        }
        let mut fresh: Vec<Inline> = vec![Inline::Run(new_run(
            run_head,
            RunProps::default(),
            vec![fld_char(run_head, "separate")],
        ))];
        for (text, props) in join_with_breaks(segments) {
            fresh.push(Inline::Run(new_run(
                run_head,
                props,
                run_content(&text, run_head),
            )));
        }
        for (k, inline) in fresh.into_iter().enumerate() {
            list.insert(insert_at + k, inline);
        }
        self.edited();
        Ok(())
    }
}

/// Removes the old result between the separator `sep` and the end marker `end`, merging the
/// end paragraph into the separator paragraph (which is at `sep_slot.pos` afterwards).
pub(crate) fn merge_region(
    blocks: &mut Vec<Block>,
    sep: &Loc,
    end: &Loc,
    sep_slot: &Slot,
    end_slot: &Slot,
) -> Result<(), DocxError> {
    // Drop whole blocks between the two marker paragraphs.
    if end_slot.pos > sep_slot.pos + 1 {
        blocks.drain(sep_slot.pos + 1..end_slot.pos);
    }
    let end_pos = if end_slot.pos > sep_slot.pos {
        sep_slot.pos + 1
    } else {
        sep_slot.pos
    };
    // Strip inside the paragraphs.
    let sep_ref = (sep.path.as_slice(), sep.item);
    let end_ref = (end.path.as_slice(), end.item);
    let mut inside = false;
    if end_pos == sep_slot.pos {
        let Block::Paragraph(p) = &mut blocks[sep_slot.pos] else {
            return Err(DocxError::Xml("marker paragraph missing".to_owned()));
        };
        strip_between(
            &mut p.content,
            &mut Vec::new(),
            Some(sep_ref),
            Some(end_ref),
            &mut inside,
        );
    } else {
        if let Block::Paragraph(p) = &mut blocks[sep_slot.pos] {
            strip_between(
                &mut p.content,
                &mut Vec::new(),
                Some(sep_ref),
                None,
                &mut inside,
            );
        }
        if let Block::Paragraph(p) = &mut blocks[end_pos] {
            strip_between(
                &mut p.content,
                &mut Vec::new(),
                None,
                Some(end_ref),
                &mut inside,
            );
        }
        // Merge the end paragraph into the separator paragraph.
        let Block::Paragraph(tail) = blocks.remove(end_pos) else {
            return Err(DocxError::Xml("marker paragraph missing".to_owned()));
        };
        if let Block::Paragraph(p) = &mut blocks[sep_slot.pos] {
            p.content.extend(tail.content);
        }
    }
    Ok(())
}

fn blocks_len_hint(n: usize) -> usize {
    n
}

fn default_run_head() -> ElementHead {
    ElementHead {
        name: "w:r".to_owned(),
        ns: Some(std::sync::Arc::from(crate::xml::W_NS)),
        attrs: Vec::new(),
    }
}

/// Which formatting a new result gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Policy {
    /// Word's default: the formatting of the field code's first character.
    Code,
    /// `\* MERGEFORMAT`: the formatting of the previous result.
    Merge,
}

fn to_field(index: usize, raw: &RawField) -> Field {
    Field {
        index,
        instruction: Instruction::parse(&raw.instr),
        depth: raw.depth,
        parent: raw.parent,
        simple: raw.simple,
        paragraph: raw.begin.para,
        result_text: raw.result_text.trim_end_matches('\r').to_owned(),
        locked: raw.locked,
        dirty: raw.dirty,
    }
}

fn loc_cmp(a: &Loc, b: &Loc) -> Ordering {
    (a.para, &a.path, a.item).cmp(&(b.para, &b.path, b.item))
}

/// Indices of the fields that are directly nested in the instruction of field `index`.
fn instruction_children(raws: &[RawField], index: usize) -> Vec<usize> {
    let Some(parent) = raws.get(index) else {
        return Vec::new();
    };
    raws.iter()
        .enumerate()
        .filter(|(_, f)| f.parent == Some(index))
        .filter(|(_, f)| match &parent.separate {
            Some(sep) => loc_cmp(&f.begin, sep) == Ordering::Less,
            None => true,
        })
        .map(|(i, _)| i)
        .collect()
}

/// Number of fields nested anywhere inside field `index`.
fn descendants(raws: &[RawField], index: usize) -> usize {
    let mut count = 0usize;
    for (i, f) in raws.iter().enumerate().skip(index + 1) {
        let mut cur = f.parent;
        let mut inside = false;
        while let Some(p) = cur {
            if p == index {
                inside = true;
                break;
            }
            if p < index {
                break;
            }
            cur = raws.get(p).and_then(|r| r.parent);
        }
        if inside {
            count += 1;
        } else if i > index + count {
            break;
        }
    }
    count
}

// ---------------------------------------------------------------------------
// Building result runs
// ---------------------------------------------------------------------------

/// Splits the new result into `(text, props)` runs.
fn result_segments(raw: &RawField, text: &str, policy: Policy) -> Vec<(String, RunProps)> {
    let first_result = raw.result_runs.first().map(|(p, _)| p.clone());
    match policy {
        Policy::Code => {
            let props = if raw.simple {
                first_result.unwrap_or_default()
            } else {
                raw.code_props.clone().or(first_result).unwrap_or_default()
            };
            vec![(text.to_owned(), props)]
        }
        Policy::Merge => merge_segments(raw, text),
    }
}

/// MERGEFORMAT: the n-th word of the new result takes the formatting of the n-th word of the
/// old one (the last old word's formatting for any extra words).
fn merge_segments(raw: &RawField, text: &str) -> Vec<(String, RunProps)> {
    let default_props = raw
        .result_runs
        .first()
        .map(|(p, _)| p.clone())
        .or_else(|| raw.code_props.clone())
        .unwrap_or_default();
    let all_same = raw.result_runs.windows(2).all(|w| w[0].0 == w[1].0);
    if raw.result_runs.is_empty() || all_same {
        return vec![(text.to_owned(), default_props)];
    }
    // Old word formatting by word index.
    let mut old_words: Vec<RunProps> = Vec::new();
    let mut in_word = false;
    for (props, run_text) in &raw.result_runs {
        for c in run_text.chars() {
            if c.is_whitespace() {
                in_word = false;
            } else if !in_word {
                in_word = true;
                old_words.push(props.clone());
            }
        }
    }
    if old_words.is_empty() {
        return vec![(text.to_owned(), default_props)];
    }
    let mut out: Vec<(String, RunProps)> = Vec::new();
    let mut word_index = 0usize;
    let mut in_word = false;
    let mut current_props = old_words[0].clone();
    for c in text.chars() {
        if c.is_whitespace() {
            in_word = false;
        } else if !in_word {
            in_word = true;
            current_props = old_words
                .get(word_index)
                .unwrap_or(&old_words[old_words.len() - 1])
                .clone();
            word_index += 1;
        }
        match out.last_mut() {
            Some((s, p)) if *p == current_props => s.push(c),
            _ => out.push((c.to_string(), current_props.clone())),
        }
    }
    out
}

/// Groups segments into paragraphs at `\r`.
fn split_paragraphs(segments: &[(String, RunProps)]) -> Vec<Vec<(String, RunProps)>> {
    let mut out: Vec<Vec<(String, RunProps)>> = vec![Vec::new()];
    for (text, props) in segments {
        let mut parts = text.split('\r');
        if let Some(first) = parts.next()
            && !first.is_empty()
            && let Some(last) = out.last_mut()
        {
            last.push((first.to_owned(), props.clone()));
        }
        for part in parts {
            out.push(Vec::new());
            if !part.is_empty()
                && let Some(last) = out.last_mut()
            {
                last.push((part.to_owned(), props.clone()));
            }
        }
    }
    out
}

/// Joins paragraph pieces with line breaks, for places that cannot split a paragraph.
fn join_with_breaks(segments: &[(String, RunProps)]) -> Vec<(String, RunProps)> {
    segments
        .iter()
        .map(|(t, p)| (t.replace('\r', "\u{b}"), p.clone()))
        .collect()
}

/// Run content for `text`: `\t` is a tab, `\u{b}` / `\n` a line break.
pub(crate) fn run_content(text: &str, owner: &ElementHead) -> Vec<RunContent> {
    let mut out = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, out: &mut Vec<RunContent>| {
        if !current.is_empty() {
            let text = std::mem::take(current);
            out.push(RunContent::Text {
                preserve_space: needs_preserve(&text),
                text,
                attrs: Vec::new(),
            });
        }
    };
    for c in text.chars() {
        match c {
            '\t' => {
                flush(&mut current, &mut out);
                out.push(RunContent::Tab);
            }
            '\u{b}' | '\n' => {
                flush(&mut current, &mut out);
                out.push(RunContent::Break(Break::new(
                    BreakKind::TextWrapping,
                    owner,
                )));
            }
            '\u{c}' => {
                flush(&mut current, &mut out);
                out.push(RunContent::Break(Break::new(BreakKind::Page, owner)));
            }
            '\u{e}' => {
                flush(&mut current, &mut out);
                out.push(RunContent::Break(Break::new(BreakKind::Column, owner)));
            }
            c if c.is_control() => {}
            c => current.push(c),
        }
    }
    flush(&mut current, &mut out);
    out
}

fn run_of(owner: &ElementHead, text: &str, props: RunProps) -> Inline {
    Inline::Run(new_run(owner, props, run_content(text, owner)))
}

/// Inserts the first piece of the new result after the separator marker of `p`, splitting the
/// separator's run when other items follow the marker in it.
fn insert_after_marker(
    p: &mut Paragraph,
    sep: &Loc,
    run_head: &ElementHead,
    pieces: &[Vec<(String, RunProps)>],
    _pos: usize,
    _hint: usize,
) -> Result<(), DocxError> {
    let (list, at) = inlines_mut(&mut p.content, &sep.path)
        .ok_or_else(|| DocxError::Xml("bad separator path".to_owned()))?;
    if let Some(Inline::Run(run)) = list.get_mut(at)
        && run.content.len() > sep.item + 1
    {
        let tail = run.content.split_off(sep.item + 1);
        let mut rest = run.empty_clone();
        rest.content = tail;
        list.insert(at + 1, Inline::Run(rest));
    }
    let first = pieces.first().cloned().unwrap_or_default();
    for (k, (text, props)) in first.into_iter().enumerate() {
        list.insert(at + 1 + k, run_of(run_head, &text, props));
    }
    Ok(())
}

/// Splits the paragraph at `pos` after the first piece into one paragraph per further piece.
/// Only fields whose separator is a direct child of the paragraph can be split.
fn split_into_paragraphs(
    blocks: &mut Vec<Block>,
    pos: usize,
    sep: &Loc,
    run_head: &ElementHead,
    pieces: &[Vec<(String, RunProps)>],
) -> Result<(), DocxError> {
    let Block::Paragraph(p) = &mut blocks[pos] else {
        return Err(DocxError::Xml("marker paragraph missing".to_owned()));
    };
    if sep.path.len() != 1 {
        // Nested marker: keep one paragraph, with line breaks instead.
        return Ok(());
    }
    let first_len = pieces[0].len();
    let split_at = sep.path[0] + 1 + first_len;
    let tail = p.content.split_off(split_at.min(p.content.len()));
    let mut template = Paragraph {
        head: p.head.clone(),
        props: p.props.clone(),
        content: Vec::new(),
    };
    template.head.attrs.clear();
    if let Some(props) = &mut template.props {
        props.children.retain(|c| match c {
            RawXml::Element(e) => !e.head.is_w("sectPr"),
            _ => true,
        });
    }
    let mut fresh: Vec<Block> = Vec::new();
    for (k, piece) in pieces.iter().enumerate().skip(1) {
        let mut para = template.clone();
        for (text, props) in piece {
            para.content.push(run_of(run_head, text, props.clone()));
        }
        if k + 1 == pieces.len() {
            para.content.extend(tail.clone());
        }
        fresh.push(Block::Paragraph(para));
    }
    for (k, b) in fresh.into_iter().enumerate() {
        blocks.insert(pos + 1 + k, b);
    }
    Ok(())
}

/// Reads the page of a run's formatting for tests and tools.
#[allow(dead_code)]
fn unused(_: &Run, _: &Slot, _: GeneralFormat) {
    let _ = run_at_mut;
}
