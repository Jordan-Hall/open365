//! Paragraph layout: text and runs to shaped atoms to lines (independent of the page).

use std::ops::Range;

use open365_docx::{
    Alignment, EffectiveParagraph, EffectiveRun, ElementHead, Inline, Paragraph, Run, RunContent,
    RunProps, Styles,
};
use open365_text::{
    BreakKind, FaceId, FontChain, FontLibrary, FontStyle, ShapeFeatures, break_opportunities,
    shape_run,
};

use crate::metrics::{LineMetricModel, line_height};
use crate::props::{DocExtras, ParaExtra, TabKind, TabStop};

const EPS: f64 = 1e-6;

/// Resolved formatting of a run of text.
#[derive(Debug, Clone)]
pub(crate) struct RunStyle {
    pub chain: FontChain,
    pub face: FaceId,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Option<[u8; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomClass {
    Text,
    Space,
    Tab,
    LineBreak,
    PageBreak,
    Mark,
}

#[derive(Debug, Clone)]
pub(crate) struct Atom {
    pub byte_start: usize,
    /// Natural advance in points (tabs and marks are computed at line time).
    pub adv: f64,
    pub style: usize,
    pub glyphs: Range<usize>,
    pub class: AtomClass,
    pub ch: char,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PlacedGlyphRaw {
    pub glyph_id: u32,
    pub x_offset: f64,
    pub y_offset: f64,
    pub adv: f64,
}

/// A line of a paragraph, positioned horizontally in text-area coordinates.
#[derive(Debug, Clone)]
pub(crate) struct LineBox {
    /// Story offsets (UTF-16 units) of the first and one past the last character.
    pub start: usize,
    pub end: usize,
    pub height: f64,
    /// Distance from the line top to the baseline.
    pub baseline_off: f64,
    /// X of the first character, text-area coordinates.
    pub left: f64,
    /// X of the last character's left edge.
    pub last_x: f64,
    pub atoms: Range<usize>,
    /// Atom x positions (text-area coordinates), parallel to `atoms`.
    pub xs: Vec<f64>,
    /// True when the line ends with a page break; the next line starts a page.
    pub page_break_after: bool,
}

/// A paragraph laid out into lines, ready for pagination.
#[derive(Debug)]
pub(crate) struct ParaLayout {
    pub index: usize,
    pub props: EffectiveParagraph,
    pub extra: ParaExtra,
    pub lines: Vec<LineBox>,
    pub atoms: Vec<Atom>,
    pub glyphs: Vec<PlacedGlyphRaw>,
    pub styles: Vec<RunStyle>,
    pub story_end: usize,
}

pub(crate) struct Context<'a> {
    pub fonts: &'a FontLibrary,
    pub styles: Option<&'a Styles>,
    pub extras: &'a DocExtras,
    pub model: LineMetricModel,
}

fn dummy_run() -> Run {
    Run {
        head: ElementHead {
            name: "w:r".to_owned(),
            ns: None,
            attrs: Vec::new(),
        },
        props: RunProps::default(),
        content: Vec::new(),
    }
}

fn fallback_run(er: Option<EffectiveRun>) -> EffectiveRun {
    er.unwrap_or(EffectiveRun {
        font: "Calibri".to_owned(),
        size_half_points: 22,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        color: None,
    })
}

fn style_from(ctx: &Context<'_>, er: &EffectiveRun) -> RunStyle {
    let fs = FontStyle {
        bold: er.bold,
        italic: er.italic,
    };
    let chain = ctx.fonts.chain(&er.font, fs);
    let face = chain
        .faces
        .first()
        .copied()
        .unwrap_or_else(|| FaceId::from_index(0));
    RunStyle {
        chain,
        face,
        size: f64::from(er.size_half_points) / 2.0,
        bold: er.bold,
        italic: er.italic,
        underline: er.underline,
        strike: er.strike,
        color: er.color,
    }
}

struct Collector<'a, 'b> {
    ctx: &'a Context<'b>,
    para: &'a Paragraph,
    text: String,
    spans: Vec<(Range<usize>, usize)>,
    styles: Vec<RunStyle>,
}

impl Collector<'_, '_> {
    fn push(&mut self, s: &str, style: usize) {
        if s.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(s);
        let end = self.text.len();
        match self.spans.last_mut() {
            Some((r, st)) if *st == style && r.end == start => r.end = end,
            _ => self.spans.push((start..end, style)),
        }
    }

    fn run(&mut self, run: &Run) {
        let eff = self
            .ctx
            .styles
            .map(|s| s.effective_run_props(self.para, run));
        let er = fallback_run(eff);
        let style = {
            let st = style_from(self.ctx, &er);
            self.styles.push(st);
            self.styles.len() - 1
        };
        for item in &run.content {
            match item {
                RunContent::Text { text, .. } => self.push(text, style),
                RunContent::Tab => self.push("\t", style),
                RunContent::Break(b) => match b.kind() {
                    open365_docx::BreakKind::TextWrapping => self.push("\u{b}", style),
                    open365_docx::BreakKind::Page => self.push("\u{c}", style),
                    open365_docx::BreakKind::Column => self.push("\u{e}", style),
                },
                RunContent::Raw(_) => {}
            }
        }
    }

    fn inlines(&mut self, content: &[Inline]) {
        for inline in content {
            match inline {
                Inline::Run(r) => self.run(r),
                Inline::Group(g) => self.inlines(&g.content),
                Inline::Raw(_) => {}
            }
        }
    }
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// Lay out one paragraph at `text_width` (width of the section's text area).
pub(crate) fn layout_paragraph(
    ctx: &Context<'_>,
    para: &Paragraph,
    index: usize,
    story_start: usize,
    text_width: f64,
) -> ParaLayout {
    let props = ctx
        .styles
        .map(|s| s.effective_para_props(para))
        .unwrap_or_default();
    let extra = ctx.extras.para_extra(para);

    let mut col = Collector {
        ctx,
        para,
        text: String::new(),
        spans: Vec::new(),
        styles: Vec::new(),
    };
    col.inlines(&para.content);

    // Paragraph mark style: the paragraph style's run properties plus the mark's own rPr.
    let mut mark_er = fallback_run(
        ctx.styles
            .map(|s| s.effective_run_props(para, &dummy_run())),
    );
    if let Some(hp) = extra.mark_size_half_points {
        mark_er.size_half_points = hp;
    }
    if let Some(font) = &extra.mark_font {
        mark_er.font.clone_from(font);
    }
    if let Some(b) = extra.mark_bold {
        mark_er.bold = b;
    }
    if let Some(i) = extra.mark_italic {
        mark_er.italic = i;
    }
    let mark_style = {
        col.styles.push(style_from(ctx, &mark_er));
        col.styles.len() - 1
    };
    col.push("\r", mark_style);
    let Collector {
        text,
        spans,
        styles,
        ..
    } = col;

    // Shape every span, then group glyphs into atoms by cluster.
    let mut glyphs: Vec<PlacedGlyphRaw> = Vec::new();
    let mut atoms: Vec<Atom> = Vec::new();
    for (range, style_idx) in &spans {
        let st = &styles[*style_idx];
        let seg = &text[range.clone()];
        let shaped = shape_run(ctx.fonts, &st.chain, seg, ShapeFeatures::default());
        let mut idx = 0;
        let gl = &shaped.glyphs;
        while idx < gl.len() {
            let cluster = gl[idx].cluster as usize;
            let mut j = idx + 1;
            while j < gl.len() && gl[j].cluster as usize == cluster {
                j += 1;
            }
            let byte_start = range.start + cluster;
            let ch = text[byte_start..].chars().next().unwrap_or(' ');
            let class = match ch {
                '\t' => AtomClass::Tab,
                '\u{b}' => AtomClass::LineBreak,
                '\u{c}' | '\u{e}' => AtomClass::PageBreak,
                '\r' => AtomClass::Mark,
                ' ' => AtomClass::Space,
                _ => AtomClass::Text,
            };
            let gstart = glyphs.len();
            let mut adv_total = 0.0;
            if matches!(class, AtomClass::Text | AtomClass::Space) {
                for g in &gl[idx..j] {
                    let upem = f64::from(ctx.fonts.face(g.face).units_per_em());
                    let adv = f64::from(g.x_advance) * st.size / upem;
                    glyphs.push(PlacedGlyphRaw {
                        glyph_id: g.glyph_id,
                        x_offset: f64::from(g.x_offset) * st.size / upem,
                        y_offset: f64::from(g.y_offset) * st.size / upem,
                        adv,
                    });
                    adv_total += adv;
                }
            }
            atoms.push(Atom {
                byte_start,
                adv: adv_total,
                style: *style_idx,
                glyphs: gstart..glyphs.len(),
                class,
                ch,
            });
            idx = j;
        }
    }

    // Story offsets of every atom start (UTF-16 units), via a running count.
    let mut story_of: Vec<usize> = Vec::with_capacity(atoms.len() + 1);
    {
        let mut u = story_start;
        let mut b = 0usize;
        for a in &atoms {
            if a.byte_start > b {
                u += utf16_len(&text[b..a.byte_start]);
                b = a.byte_start;
            }
            story_of.push(u);
        }
        u += utf16_len(&text[b..]);
        story_of.push(u);
    }
    let story_end = *story_of.last().unwrap_or(&story_start);

    let breaks = break_opportunities(&text);
    let mut opp: Vec<(usize, BreakKind)> = breaks.iter().map(|b| (b.offset, b.kind)).collect();
    opp.sort_by_key(|o| o.0);

    let lb = LineBuilder {
        ctx,
        atoms: &atoms,
        styles: &styles,
        props: &props,
        extra: &extra,
        text_width,
        opp: &opp,
        story_of: &story_of,
    };
    let lines = lb.build();

    ParaLayout {
        index,
        props,
        extra,
        lines,
        atoms,
        glyphs,
        styles,
        story_end,
    }
}

struct LineBuilder<'a> {
    ctx: &'a Context<'a>,
    atoms: &'a [Atom],
    styles: &'a [RunStyle],
    props: &'a EffectiveParagraph,
    extra: &'a ParaExtra,
    text_width: f64,
    opp: &'a [(usize, BreakKind)],
    story_of: &'a [usize],
}

impl LineBuilder<'_> {
    fn opportunity_at(&self, byte: usize) -> Option<BreakKind> {
        self.opp
            .binary_search_by_key(&byte, |o| o.0)
            .ok()
            .map(|i| self.opp[i].1)
    }

    fn build(&self) -> Vec<LineBox> {
        let mut lines = Vec::new();
        let mut a0 = 0usize;
        let n = self.atoms.len();
        while a0 < n {
            let first = lines.is_empty();
            let line = self.fill(a0, first);
            a0 = line.atoms.end;
            lines.push(line);
        }
        if lines.is_empty() {
            // Cannot happen: every paragraph has a mark atom. Keep layout total anyway.
            lines.push(LineBox {
                start: 0,
                end: 0,
                height: 0.0,
                baseline_off: 0.0,
                left: 0.0,
                last_x: 0.0,
                atoms: 0..0,
                xs: Vec::new(),
                page_break_after: false,
            });
        }
        lines
    }

    /// Next tab stop after `x` (text-area coordinates).
    fn next_stop(&self, x: f64) -> TabStop {
        let mut best: Option<TabStop> = None;
        for t in &self.extra.tabs {
            if t.pos > x + 0.01 && t.kind != TabKind::Bar {
                best = Some(*t);
                break;
            }
        }
        // A hanging indent makes the left indent an implicit tab stop.
        let ind = self.props.indent_left_pt as f64;
        if self.props.first_line_pt < 0.0 && ind > x + 0.01 && best.is_none_or(|b| ind < b.pos) {
            best = Some(TabStop {
                pos: ind,
                kind: TabKind::Left,
                leader: crate::props::TabLeader::None,
            });
        }
        best.unwrap_or_else(|| {
            let dt = self.ctx.extras.default_tab.max(1.0);
            let k = ((x + 0.01) / dt).floor() + 1.0;
            TabStop {
                pos: k * dt,
                kind: TabKind::Left,
                leader: crate::props::TabLeader::None,
            }
        })
    }

    /// Advance of the tab at atom `i` placed at `x`.
    fn tab_advance(&self, i: usize, x: f64, right_limit: f64) -> f64 {
        let stop = self.next_stop(x);
        let target = match stop.kind {
            TabKind::Left | TabKind::Bar => stop.pos,
            TabKind::Center | TabKind::Right | TabKind::Decimal => {
                // Width of the text that follows, up to the next tab or line end.
                let mut w = 0.0;
                let mut before_dec: Option<f64> = None;
                let mut trailing_space = 0.0;
                let mut j = i + 1;
                while j < self.atoms.len() {
                    let a = &self.atoms[j];
                    if matches!(
                        a.class,
                        AtomClass::Tab
                            | AtomClass::LineBreak
                            | AtomClass::PageBreak
                            | AtomClass::Mark
                    ) {
                        break;
                    }
                    if stop.kind == TabKind::Decimal && a.ch == '.' && before_dec.is_none() {
                        before_dec = Some(w);
                    }
                    w += a.adv;
                    if a.class == AtomClass::Space {
                        trailing_space += a.adv;
                    } else {
                        trailing_space = 0.0;
                    }
                    j += 1;
                }
                w -= trailing_space;
                let start = match stop.kind {
                    TabKind::Center => stop.pos - w / 2.0,
                    TabKind::Right => stop.pos - w,
                    _ => stop.pos - before_dec.unwrap_or(w),
                };
                start.max(x)
            }
        };
        let _ = right_limit;
        (target - x).max(0.0)
    }

    fn fill(&self, a0: usize, first: bool) -> LineBox {
        let n = self.atoms.len();
        let indent_left = f64::from(self.props.indent_left_pt);
        let indent_right = f64::from(self.props.indent_right_pt);
        let first_line = f64::from(self.props.first_line_pt);
        let left_edge = indent_left + if first { first_line } else { 0.0 };
        let right_limit = self.text_width - indent_right;

        let mut xs: Vec<f64> = Vec::new();
        let mut advs: Vec<f64> = Vec::new();
        let mut x = left_edge;
        let mut vis_end_x = left_edge;
        let mut i = a0;
        let mut cand: Option<usize> = None; // best break: atom index boundary
        let mut end = n;
        let mut forced = false;
        let mut page_break_after = false;
        while i < n {
            let a = &self.atoms[i];
            let adv = match a.class {
                AtomClass::Tab => self.tab_advance(i, x, right_limit),
                AtomClass::LineBreak | AtomClass::PageBreak | AtomClass::Mark => 0.0,
                _ => a.adv,
            };
            if a.class == AtomClass::Text && i > a0 && x + adv > right_limit + EPS {
                end = cand.unwrap_or(i);
                break;
            }
            xs.push(x);
            advs.push(adv);
            x += adv;
            if matches!(a.class, AtomClass::Text | AtomClass::Tab) {
                vis_end_x = x;
            }
            i += 1;
            match a.class {
                AtomClass::LineBreak | AtomClass::PageBreak | AtomClass::Mark => {
                    if a.class == AtomClass::PageBreak {
                        page_break_after = true;
                        // The paragraph mark right after a page break stays on this line.
                        if i < n && self.atoms[i].class == AtomClass::Mark {
                            xs.push(x);
                            advs.push(0.0);
                            i += 1;
                            page_break_after = true;
                        }
                    }
                    end = i;
                    forced = true;
                    break;
                }
                _ => {}
            }
            if i < n && self.opportunity_at(self.atoms[i].byte_start).is_some() {
                cand = Some(i);
            }
        }
        if !forced && end == n && i >= n {
            end = n;
        }
        // Truncate positions when we broke at an earlier candidate.
        let count = end - a0;
        xs.truncate(count);
        advs.truncate(count);
        if count == 0 {
            // Always make progress.
            let a = &self.atoms[a0];
            let adv = if a.class == AtomClass::Text || a.class == AtomClass::Space {
                a.adv
            } else {
                0.0
            };
            xs.push(left_edge);
            advs.push(adv);
            end = a0 + 1;
        }

        // Alignment.
        let last_atom = end - 1;
        let mut vis_end = left_edge;
        for (k, a) in self.atoms[a0..end].iter().enumerate() {
            if matches!(a.class, AtomClass::Text | AtomClass::Tab) {
                vis_end = xs[k] + advs[k];
            }
        }
        let _ = vis_end_x;
        let avail = right_limit - left_edge;
        let vis_w = vis_end - left_edge;
        let ends_paragraph =
            self.atoms[last_atom].class == AtomClass::Mark || (page_break_after && end == n);
        let ends_manual = matches!(
            self.atoms[last_atom].class,
            AtomClass::LineBreak | AtomClass::PageBreak
        );
        match self.props.alignment {
            Alignment::Left => {}
            Alignment::Center => {
                let shift = ((avail - vis_w) / 2.0).max(0.0);
                shift_all(&mut xs, shift);
            }
            Alignment::Right => {
                let shift = (avail - vis_w).max(0.0);
                shift_all(&mut xs, shift);
            }
            Alignment::Justify => {
                if !ends_paragraph && !ends_manual {
                    // Stretch the spaces between the first and last visible atoms.
                    let last_vis = self.atoms[a0..end]
                        .iter()
                        .rposition(|a| a.class != AtomClass::Space)
                        .unwrap_or(0);
                    let spaces: Vec<usize> = (0..last_vis)
                        .filter(|k| self.atoms[a0 + k].class == AtomClass::Space)
                        .collect();
                    if !spaces.is_empty() && avail > vis_w {
                        let per = (avail - vis_w) / spaces.len() as f64;
                        let mut added = 0.0;
                        for k in 0..xs.len() {
                            xs[k] += added;
                            if spaces.binary_search(&k).is_ok() {
                                advs[k] += per;
                                added += per;
                            }
                        }
                    }
                }
            }
        }

        // Height from the fonts on the line.
        let mut asc: f64 = 0.0;
        let mut desc: f64 = 0.0;
        let mut seen: Vec<usize> = Vec::new();
        for a in &self.atoms[a0..end] {
            if seen.contains(&a.style) {
                continue;
            }
            seen.push(a.style);
            let st = &self.styles[a.style];
            let m = self.ctx.fonts.face(st.face).metrics();
            let (ca, cd) = self.ctx.model.ascent_descent(m, st.size);
            asc = asc.max(ca);
            desc = desc.max(cd);
        }
        let (height, baseline_off) = line_height(asc, desc, self.props.line, self.ctx.model);

        let last_x = xs.last().copied().unwrap_or(left_edge);
        LineBox {
            start: self.story_of[a0],
            end: self.story_of[end],
            height,
            baseline_off,
            left: xs.first().copied().unwrap_or(left_edge),
            last_x,
            atoms: a0..end,
            xs,
            page_break_after,
        }
    }
}

fn shift_all(xs: &mut [f64], by: f64) {
    for x in xs {
        *x += by;
    }
}
