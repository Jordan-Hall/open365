//! View model of a DOCX body: one entry per paragraph with text, run styling and page setup.
//!
//! Built from the typed model in `open365_docx` and rebuilt per paragraph after an edit.
//! Character offsets are Unicode scalar values, the same unit the commands use.

use open365_docx::{Block, BreakKind, Document, Inline, Paragraph, RawXml, RunContent};

/// Character formatting of a span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// Font size in half-points, as written.
    pub size_half_points: Option<u32>,
    /// `RRGGBB`.
    pub color: Option<[u8; 3]>,
}

/// A styled stretch of a paragraph, `start..end` in characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: SpanStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParaKind {
    #[default]
    Normal,
    Title,
    Heading(u8),
}

/// One paragraph as the view needs it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParaModel {
    /// Text as laid out: tabs become an em space so one character stays one character.
    pub text: String,
    pub char_len: usize,
    pub spans: Vec<Span>,
    pub kind: ParaKind,
    /// A page break inside the paragraph: the next paragraph starts a new page.
    pub page_break: bool,
}

impl ParaModel {
    pub fn from_paragraph(p: &Paragraph) -> Self {
        let mut model = Self {
            kind: kind_of(p),
            ..Self::default()
        };
        collect(&p.content, &mut model);
        model.char_len = model.text.chars().count();
        model
    }

    /// Whether every character in `start..end` is bold (an empty range is not bold).
    pub fn is_bold(&self, start: usize, end: usize) -> bool {
        if start >= end {
            return false;
        }
        let mut covered = start;
        for span in &self.spans {
            if span.end <= covered {
                continue;
            }
            if span.start > covered || !span.style.bold {
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

fn kind_of(p: &Paragraph) -> ParaKind {
    let style = p.props.as_ref().and_then(|props| {
        props.children.iter().find_map(|c| match c {
            RawXml::Element(e) if e.head.is_w("pStyle") => e.head.attr_w("val"),
            _ => None,
        })
    });
    let Some(style) = style else {
        return ParaKind::Normal;
    };
    let lower = style.to_ascii_lowercase();
    if lower == "title" {
        return ParaKind::Title;
    }
    if let Some(level) = lower.strip_prefix("heading")
        && let Ok(level) = level.trim().parse::<u8>()
    {
        return ParaKind::Heading(level.clamp(1, 6));
    }
    ParaKind::Normal
}

fn collect(content: &[Inline], model: &mut ParaModel) {
    for inline in content {
        match inline {
            Inline::Run(run) => {
                let props = &run.props;
                let style = SpanStyle {
                    bold: props.bold().unwrap_or(false),
                    italic: props.italic().unwrap_or(false),
                    underline: props.underline().is_some_and(|u| u != "none"),
                    strike: props.strike().unwrap_or(false),
                    size_half_points: props.size_half_points(),
                    color: props.color().and_then(parse_color),
                };
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
            Inline::Group(g) => collect(&g.content, model),
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
    doc.body()
        .paragraphs()
        .into_iter()
        .map(ParaModel::from_paragraph)
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
    fn headings_and_page_setup() {
        let doc = Document::open(&fixture("docx/styles-headings.docx"), &Limits::default())
            .expect("open");
        let models = build_models(&doc);
        assert!(
            models
                .iter()
                .any(|m| matches!(m.kind, ParaKind::Heading(_)))
        );
        let landscape =
            Document::open(&fixture("docx/sections-landscape.docx"), &Limits::default())
                .expect("open");
        let setup = PageSetup::from_document(&landscape);
        assert!(setup.width > 0.0 && setup.height > 0.0);
    }
}
