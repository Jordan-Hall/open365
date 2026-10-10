//! View model of a DOCX body: one entry per paragraph with text, resolved run formatting and
//! paragraph formatting, plus page setup.
//!
//! Built from the typed model in `open365_docx` (styles resolved by `Styles`) and rebuilt per
//! paragraph after an edit. Character offsets are Unicode scalar values, the same unit the
//! commands use.

use std::sync::Arc;

use open365_docx::{
    Alignment, Block, BreakKind, Document, EffectiveParagraph, Inline, Paragraph, RawXml,
    RunContent, Styles,
};

/// Font used when a document has no styles part and the run names no font.
const BARE_FONT: &str = "Calibri";
const BARE_SIZE_HALF_POINTS: u32 = 22;

/// Resolved character formatting of a span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanStyle {
    pub font: Arc<str>,
    /// Font size in half-points.
    pub size_half_points: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// `RRGGBB`; `None` is automatic (black).
    pub color: Option<[u8; 3]>,
}

impl Default for SpanStyle {
    fn default() -> Self {
        Self {
            font: Arc::from(BARE_FONT),
            size_half_points: BARE_SIZE_HALF_POINTS,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            color: None,
        }
    }
}

impl SpanStyle {
    pub fn size_pt(&self) -> f32 {
        self.size_half_points as f32 / 2.0
    }
}

/// A styled stretch of a paragraph, `start..end` in characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: SpanStyle,
}

/// One paragraph as the view needs it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParaModel {
    /// Text as laid out: tabs become an em space so one character stays one character.
    pub text: String,
    pub char_len: usize,
    pub spans: Vec<Span>,
    /// Style id from `w:pStyle`, `None` for the default style.
    pub style_id: Option<String>,
    pub para: EffectiveParagraph,
    /// Formatting an empty paragraph (or a caret at its end) takes.
    pub mark: SpanStyle,
    /// A page break inside the paragraph: the next paragraph starts a new page.
    pub page_break: bool,
}

impl ParaModel {
    pub fn from_paragraph(p: &Paragraph, styles: Option<&Styles>) -> Self {
        let mut model = Self {
            style_id: p.style_id().map(str::to_owned),
            para: styles.map_or_else(
                || EffectiveParagraph {
                    alignment: p.direct_alignment().unwrap_or(Alignment::Left),
                    space_after_pt: 8.0,
                    ..EffectiveParagraph::default()
                },
                |s| s.effective_para_props(p),
            ),
            ..Self::default()
        };
        collect(&p.content, p, styles, &mut model);
        model.char_len = model.text.chars().count();
        model.mark = model
            .spans
            .last()
            .map(|s| s.style.clone())
            .unwrap_or_else(|| mark_style(p, styles));
        model
    }

    /// The style of the character at `offset` (the one before it at the end of a paragraph).
    pub fn style_at(&self, offset: usize) -> &SpanStyle {
        let at = offset
            .saturating_sub(1)
            .min(self.char_len.saturating_sub(1));
        self.spans
            .iter()
            .find(|s| s.start <= at && at < s.end)
            .map_or(&self.mark, |s| &s.style)
    }

    /// Whether `test` holds for every character in `start..end` (false for an empty range).
    pub fn all(&self, start: usize, end: usize, test: impl Fn(&SpanStyle) -> bool) -> bool {
        if start >= end {
            return false;
        }
        let mut covered = start;
        for span in &self.spans {
            if span.end <= covered {
                continue;
            }
            if span.start > covered || !test(&span.style) {
                return false;
            }
            covered = span.end;
            if covered >= end {
                return true;
            }
        }
        false
    }
}

fn span_style(p: &Paragraph, run: &open365_docx::Run, styles: Option<&Styles>) -> SpanStyle {
    if let Some(styles) = styles {
        let e = styles.effective_run_props(p, run);
        return SpanStyle {
            font: Arc::from(e.font.as_str()),
            size_half_points: e.size_half_points,
            bold: e.bold,
            italic: e.italic,
            underline: e.underline,
            strike: e.strike,
            color: e.color,
        };
    }
    let props = &run.props;
    SpanStyle {
        font: props
            .fonts()
            .and_then(|f| f.head.attr_w("ascii"))
            .map_or_else(|| Arc::from(BARE_FONT), Arc::from),
        size_half_points: props.size_half_points().unwrap_or(BARE_SIZE_HALF_POINTS),
        bold: props.bold().unwrap_or(false),
        italic: props.italic().unwrap_or(false),
        underline: props.underline().is_some_and(|u| u != "none"),
        strike: props.strike().unwrap_or(false),
        color: props.color().and_then(parse_color),
    }
}

/// Formatting for an empty paragraph: the paragraph-mark properties on top of the style.
fn mark_style(p: &Paragraph, styles: Option<&Styles>) -> SpanStyle {
    let probe = open365_docx::Run {
        head: p.head.sibling("r"),
        props: p.mark_run_props(),
        content: Vec::new(),
    };
    span_style(p, &probe, styles)
}

fn collect(content: &[Inline], p: &Paragraph, styles: Option<&Styles>, model: &mut ParaModel) {
    for inline in content {
        match inline {
            Inline::Run(run) => {
                let style = span_style(p, run, styles);
                let start = model.text.chars().count();
                for item in &run.content {
                    match item {
                        RunContent::Text { text, .. } => model.text.push_str(text),
                        RunContent::Tab => model.text.push('\u{2003}'),
                        RunContent::Break(b) => {
                            model.text.push('\n');
                            if b.kind() == BreakKind::Page {
                                model.page_break = true;
                            }
                        }
                        RunContent::Raw(_) => {}
                    }
                }
                let end = model.text.chars().count();
                if end > start {
                    match model.spans.last_mut() {
                        Some(last) if last.end == start && last.style == style => last.end = end,
                        _ => model.spans.push(Span { start, end, style }),
                    }
                }
            }
            Inline::Group(g) => collect(&g.content, p, styles, model),
            Inline::Raw(_) => {}
        }
    }
}

fn parse_color(s: &str) -> Option<[u8; 3]> {
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Page size and margins in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSetup {
    pub width: f32,
    pub height: f32,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
}

impl PageSetup {
    pub const LETTER: Self = Self {
        width: 612.0,
        height: 792.0,
        margin_top: 72.0,
        margin_bottom: 72.0,
        margin_left: 72.0,
        margin_right: 72.0,
    };

    /// Reads the body-level `w:sectPr` (the last section); Letter with one-inch margins when
    /// there is none.
    pub fn from_document(doc: &Document) -> Self {
        let mut setup = Self::LETTER;
        let sect = doc.body().blocks.iter().rev().find_map(|b| match b {
            Block::Raw(RawXml::Element(e)) if e.head.is_w("sectPr") => Some(e),
            _ => None,
        });
        let Some(sect) = sect else { return setup };
        let twips = |v: Option<&str>| v.and_then(|s| s.parse::<f32>().ok()).map(|t| t / 20.0);
        for child in &sect.children {
            let RawXml::Element(e) = child else { continue };
            if e.head.is_w("pgSz") {
                if let (Some(w), Some(h)) = (twips(e.head.attr_w("w")), twips(e.head.attr_w("h")))
                    && w > 0.0
                    && h > 0.0
                {
                    setup.width = w;
                    setup.height = h;
                }
            } else if e.head.is_w("pgMar") {
                let m = |name: &str, current: f32| twips(e.head.attr_w(name)).unwrap_or(current);
                setup.margin_top = m("top", setup.margin_top);
                setup.margin_bottom = m("bottom", setup.margin_bottom);
                setup.margin_left = m("left", setup.margin_left);
                setup.margin_right = m("right", setup.margin_right);
            }
        }
        // Keep a usable text area whatever the file says.
        if setup.width - setup.margin_left - setup.margin_right < 72.0
            || setup.height - setup.margin_top - setup.margin_bottom < 72.0
        {
            return Self::LETTER;
        }
        setup
    }

    pub fn body_width(&self) -> f32 {
        self.width - self.margin_left - self.margin_right
    }

    pub fn body_height(&self) -> f32 {
        self.height - self.margin_top - self.margin_bottom
    }
}

/// Builds the model of every paragraph (tables flattened, as the command layer indexes them).
pub fn build_models(doc: &Document) -> Vec<ParaModel> {
    let styles = doc.styles();
    doc.body()
        .paragraphs()
        .into_iter()
        .map(|p| ParaModel::from_paragraph(p, styles))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use open365_opc::Limits;

    fn fixture(rel: &str) -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(rel);
        std::fs::read(p).expect("fixture")
    }

    #[test]
    fn models_match_command_layer_text() {
        let doc = Document::open(&fixture("docx/plain-paragraphs.docx"), &Limits::default())
            .expect("open");
        let models = build_models(&doc);
        assert_eq!(models.len(), doc.paragraph_count());
        for (i, m) in models.iter().enumerate() {
            let text = doc
                .paragraph_text(i)
                .unwrap_or_default()
                .replace('\t', "\u{2003}");
            assert_eq!(m.text, text);
            assert_eq!(m.char_len, text.chars().count());
        }
    }

    #[test]
    fn headings_resolve_to_larger_bold_text_and_page_setup_reads() {
        let doc = Document::open(&fixture("docx/styles-headings.docx"), &Limits::default())
            .expect("open");
        let models = build_models(&doc);
        let heading = models
            .iter()
            .find(|m| m.style_id.as_deref() == Some("Heading1"))
            .expect("a heading");
        let body = models
            .iter()
            .find(|m| m.style_id.is_none() && m.char_len > 0)
            .expect("body text");
        assert!(heading.spans[0].style.size_half_points > body.spans[0].style.size_half_points);
        assert!(heading.para.space_before_pt > 0.0 || heading.para.keep_next);
        let landscape =
            Document::open(&fixture("docx/sections-landscape.docx"), &Limits::default())
                .expect("open");
        let setup = PageSetup::from_document(&landscape);
        assert!(setup.width > setup.height, "landscape: {setup:?}");
    }
}
