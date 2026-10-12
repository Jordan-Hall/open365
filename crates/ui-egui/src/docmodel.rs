//! View model of a DOCX body: one entry per paragraph with text and resolved run and
//! paragraph formatting.
//!
//! Built from the typed model in `open365_docx` (styles resolved by `Styles`) and rebuilt per
//! paragraph after an edit. Character offsets are Unicode scalar values, the same unit the
//! commands use. Page geometry and line breaks come from `open365-layout`, not from here.

use std::sync::Arc;

use open365_docx::{
    Alignment, Document, EffectiveParagraph, Inline, Paragraph, RawXml, RunContent, RunPropItem,
    Styles, VertAlign,
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
    /// The font comes from the document's theme (body or headings), not a literal name.
    pub theme_font: Option<ThemeFont>,
    /// `w:vertAlign` after the style chain: baseline, superscript or subscript.
    pub vert_align: VertAlign,
}

/// Which theme font slot a run's font is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeFont {
    Body,
    Headings,
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
            theme_font: None,
            vert_align: VertAlign::Baseline,
        }
    }
}

impl SpanStyle {
    pub fn size_pt(&self) -> f32 {
        self.size_half_points as f32 / 2.0
    }

    /// What the Font box shows: the theme font's own name with its slot, as Office does
    /// (`Calibri (Body)`), else the literal name. Rendering substitutes metric-compatible fonts
    /// separately; this never shows the substitute.
    pub fn font_label(&self) -> String {
        match self.theme_font {
            Some(ThemeFont::Body) => format!("{} (Body)", self.font),
            Some(ThemeFont::Headings) => format!("{} (Headings)", self.font),
            None => self.font.to_string(),
        }
    }
}

/// A styled stretch of a paragraph, `start..end` in characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub style: SpanStyle,
}

/// A stretch of a paragraph with its own proofing language (`w:lang`, or `w:noProof`), in characters.
/// Text outside every stretch uses the document default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LangRun {
    pub start: usize,
    pub end: usize,
    /// BCP-47 tag, or `None` for text that is not proofed.
    pub lang: Option<String>,
}

/// One paragraph as the view needs it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParaModel {
    /// Runs that set their own proofing language or switch proofing off.
    pub langs: Vec<LangRun>,
    /// Text as laid out: tabs become an em space so one character stays one character.
    pub text: String,
    pub char_len: usize,
    pub spans: Vec<Span>,
    /// Style id from `w:pStyle`, `None` for the default style.
    pub style_id: Option<String>,
    pub para: EffectiveParagraph,
    /// Formatting an empty paragraph (or a caret at its end) takes.
    pub mark: SpanStyle,
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
        let literal = run
            .props
            .fonts()
            .and_then(|f| f.head.attr_w("ascii"))
            .is_some();
        let theme = styles.theme_fonts();
        let theme_font = if literal {
            None
        } else if e.font == theme.minor {
            Some(ThemeFont::Body)
        } else if e.font == theme.major {
            Some(ThemeFont::Headings)
        } else {
            None
        };
        return SpanStyle {
            theme_font,
            font: Arc::from(e.font.as_str()),
            size_half_points: e.size_half_points,
            bold: e.bold,
            italic: e.italic,
            underline: e.underline,
            strike: e.strike,
            color: e.color,
            vert_align: styles.effective_run_layout(p, run).vert_align,
        };
    }
    let props = &run.props;
    SpanStyle {
        theme_font: None,
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
        vert_align: props
            .vert_align()
            .map_or(VertAlign::Baseline, VertAlign::parse),
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
                        RunContent::Break(_) => model.text.push('\n'),
                        RunContent::Raw(_) => {}
                    }
                }
                let end = model.text.chars().count();
                if end > start {
                    if let Some(lang) = run_lang(run) {
                        match model.langs.last_mut() {
                            Some(last) if last.end == start && last.lang == lang => last.end = end,
                            _ => model.langs.push(LangRun { start, end, lang }),
                        }
                    }
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

/// The proofing language a run sets itself: `Some(Some(tag))` for `w:lang`, `Some(None)` when `w:noProof`
/// switches proofing off, `None` when the run inherits.
fn run_lang(run: &open365_docx::Run) -> Option<Option<String>> {
    let mut lang = None;
    for item in run.props.items() {
        let (RunPropItem::Known(_, e) | RunPropItem::Raw(RawXml::Element(e))) = item else {
            continue;
        };
        if e.head.is_w("noProof")
            && e.head.attr_w("val") != Some("0")
            && e.head.attr_w("val") != Some("false")
        {
            return Some(None);
        }
        if e.head.is_w("lang")
            && let Some(v) = e.head.attr_w("val")
            && v != "x-none"
            && v != "zxx"
        {
            lang = Some(Some(v.to_owned()));
        }
    }
    lang
}

fn parse_color(s: &str) -> Option<[u8; 3]> {
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
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
        assert!(
            landscape
                .sections()
                .iter()
                .any(|s| s.page.width_twips > s.page.height_twips),
            "a landscape section"
        );
    }

    #[test]
    fn vertical_alignment_resolves_through_the_style_chain() {
        use std::io::Write;
        const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        let parts = [
            ("[Content_Types].xml", r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#.to_owned()),
            ("_rels/.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_owned()),
            ("word/_rels/document.xml.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#.to_owned()),
            ("word/styles.xml", format!(r#"<?xml version="1.0"?><w:styles xmlns:w="{W}"><w:style w:type="character" w:styleId="Sup"><w:name w:val="Sup"/><w:rPr><w:vertAlign w:val="superscript"/></w:rPr></w:style></w:styles>"#)),
            ("word/document.xml", format!(r#"<?xml version="1.0"?><w:document xmlns:w="{W}"><w:body><w:p><w:r><w:t>a</w:t></w:r><w:r><w:rPr><w:rStyle w:val="Sup"/></w:rPr><w:t>b</w:t></w:r><w:r><w:rPr><w:rStyle w:val="Sup"/><w:vertAlign w:val="baseline"/></w:rPr><w:t>c</w:t></w:r><w:r><w:rPr><w:vertAlign w:val="subscript"/></w:rPr><w:t>d</w:t></w:r></w:p></w:body></w:document>"#)),
        ];
        for (name, content) in parts {
            zip.start_file(name, opts).expect("start");
            zip.write_all(content.as_bytes()).expect("write");
        }
        let bytes = zip.finish().expect("finish").into_inner();
        let doc = Document::open(&bytes, &Limits::default()).expect("open");
        let m = &build_models(&doc)[0];
        let at = |i: usize| m.style_at(i + 1).vert_align;
        assert_eq!(at(0), VertAlign::Baseline);
        assert_eq!(at(1), VertAlign::Superscript, "from the character style");
        assert_eq!(
            at(2),
            VertAlign::Baseline,
            "direct baseline overrides the style"
        );
        assert_eq!(at(3), VertAlign::Subscript);
    }
}
