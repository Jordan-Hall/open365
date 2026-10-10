//! Properties layout needs that `open365-docx` does not resolve yet, read from the raw XML
//! that the docx model exposes: tab stops, widow control, page-break-before, contextual
//! spacing, the paragraph mark's own run properties, section geometry and the default tab
//! interval. Style inheritance (`w:basedOn`) is resolved here the same way docx does it.

use std::collections::HashMap;

use open365_docx::{Document, Paragraph, RawXml, XmlElement, parse_xml};

/// Kind of a tab stop (`w:tab` `w:val`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    Left,
    Center,
    Right,
    Decimal,
    /// Bar tabs draw a vertical rule and never move text.
    Bar,
}

/// Leader drawn in the gap a tab leaves (`w:leader`). Layout only records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabLeader {
    None,
    Dot,
    Hyphen,
    Underscore,
    Other,
}

/// A custom tab stop, in points from the left edge of the text area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabStop {
    pub pos: f64,
    pub kind: TabKind,
    pub leader: TabLeader,
}

/// Page geometry of one section (`w:sectPr`), in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SectionProps {
    pub page_width: f64,
    pub page_height: f64,
    pub margin_top: f64,
    pub margin_right: f64,
    pub margin_bottom: f64,
    pub margin_left: f64,
    pub gutter: f64,
}

impl Default for SectionProps {
    /// US Letter with one inch margins, the format's customary default.
    fn default() -> Self {
        Self {
            page_width: 612.0,
            page_height: 792.0,
            margin_top: 72.0,
            margin_right: 72.0,
            margin_bottom: 72.0,
            margin_left: 72.0,
            gutter: 0.0,
        }
    }
}

impl SectionProps {
    pub(crate) fn from_element(sect: &XmlElement) -> Self {
        let mut out = Self::default();
        for child in sect.children.iter().filter_map(RawXml::as_element) {
            if child.head.is_w("pgSz") {
                let landscape = child.head.attr_w("orient") == Some("landscape");
                let w = twips(child.head.attr_w("w"));
                let h = twips(child.head.attr_w("h"));
                if let (Some(w), Some(h)) = (w, h) {
                    // Word stores the physical size and also marks the orientation; both agree.
                    let _ = landscape;
                    out.page_width = w;
                    out.page_height = h;
                }
            } else if child.head.is_w("pgMar") {
                let get =
                    |name: &str, current: f64| twips(child.head.attr_w(name)).unwrap_or(current);
                out.margin_top = get("top", out.margin_top);
                out.margin_right = get("right", out.margin_right);
                out.margin_bottom = get("bottom", out.margin_bottom);
                out.margin_left = get("left", out.margin_left);
                out.gutter = get("gutter", out.gutter);
            }
        }
        out
    }

    /// Width of the text area.
    pub fn text_width(&self) -> f64 {
        self.page_width - self.margin_left - self.margin_right - self.gutter
    }
}

fn twips(value: Option<&str>) -> Option<f64> {
    value
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|t| t / 20.0)
}

fn child<'a>(e: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    e.children
        .iter()
        .filter_map(RawXml::as_element)
        .find(|c| c.head.is_w(local))
}

fn on_off(e: &XmlElement) -> bool {
    !matches!(e.head.attr_w("val"), Some("0" | "false" | "off"))
}

/// Paragraph properties from outside the typed docx resolver.
#[derive(Debug, Clone, PartialEq)]
pub struct ParaExtra {
    pub widow_control: bool,
    pub page_break_before: bool,
    pub contextual_spacing: bool,
    /// Effective custom tab stops, sorted by position.
    pub tabs: Vec<TabStop>,
    /// Paragraph style id, resolved to the default style when unset.
    pub style_id: Option<String>,
    /// `w:pPr/w:rPr` of the paragraph mark: font size in half points, font name.
    pub mark_size_half_points: Option<u32>,
    pub mark_font: Option<String>,
    pub mark_bold: Option<bool>,
    pub mark_italic: Option<bool>,
}

impl Default for ParaExtra {
    fn default() -> Self {
        Self {
            widow_control: true,
            page_break_before: false,
            contextual_spacing: false,
            tabs: Vec::new(),
            style_id: None,
            mark_size_half_points: None,
            mark_font: None,
            mark_bold: None,
            mark_italic: None,
        }
    }
}

#[derive(Debug, Default)]
struct StyleEntry {
    based_on: Option<String>,
    ppr: Option<XmlElement>,
}

/// Raw style data and document settings, indexed once per document.
#[derive(Debug, Default)]
pub struct DocExtras {
    styles: HashMap<String, StyleEntry>,
    defaults_ppr: Option<XmlElement>,
    default_style: Option<String>,
    /// `w:defaultTabStop` in points (36 when settings do not say).
    pub default_tab: f64,
}

fn part_bytes<'a>(doc: &'a Document, suffix: &str) -> Option<&'a [u8]> {
    doc.package()
        .parts()
        .find(|p| p.name().as_str().ends_with(suffix))
        .map(|p| p.data())
}

impl DocExtras {
    pub fn new(doc: &Document) -> Self {
        let mut out = Self {
            default_tab: 36.0,
            ..Self::default()
        };
        if let Some(settings) = part_bytes(doc, "word/settings.xml").and_then(|b| parse_xml(b).ok())
            && let Some(tab) = child(&settings.root, "defaultTabStop")
            && let Some(v) = twips(tab.head.attr_w("val"))
            && v > 0.0
        {
            out.default_tab = v;
        }
        let Some(styles_part) = doc.styles().map(|s| s.part_name().as_str().to_owned()) else {
            return out;
        };
        let Some(bytes) = doc.package().part(&styles_part).map(|p| p.data()) else {
            return out;
        };
        let Ok(parsed) = parse_xml(bytes) else {
            return out;
        };
        out.defaults_ppr = child(&parsed.root, "docDefaults")
            .and_then(|d| child(d, "pPrDefault"))
            .and_then(|d| child(d, "pPr"))
            .cloned();
        for style in parsed
            .root
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .filter(|e| e.head.is_w("style"))
        {
            let Some(id) = style.head.attr_w("styleId") else {
                continue;
            };
            if style.head.attr_w("default") == Some("1")
                && style.head.attr_w("type").unwrap_or("paragraph") == "paragraph"
            {
                out.default_style = Some(id.to_owned());
            }
            out.styles.insert(
                id.to_owned(),
                StyleEntry {
                    based_on: child(style, "basedOn")
                        .and_then(|b| b.head.attr_w("val"))
                        .map(str::to_owned),
                    ppr: child(style, "pPr").cloned(),
                },
            );
        }
        out
    }

    /// Style chain, most general first.
    fn chain(&self, id: &str) -> Vec<&StyleEntry> {
        let mut out = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        let mut current = Some(id);
        while let Some(c) = current {
            if seen.contains(&c) || seen.len() > 32 {
                break;
            }
            let Some(entry) = self.styles.get(c) else {
                break;
            };
            seen.push(c);
            out.push(entry);
            current = entry.based_on.as_deref();
        }
        out.reverse();
        out
    }

    /// Resolve the extra paragraph properties of `para`.
    pub fn para_extra(&self, para: &Paragraph) -> ParaExtra {
        let mut extra = ParaExtra::default();
        let style = para
            .style_id()
            .map(str::to_owned)
            .or_else(|| self.default_style.clone());
        let mut levels: Vec<&XmlElement> = Vec::new();
        if let Some(d) = &self.defaults_ppr {
            levels.push(d);
        }
        if let Some(id) = &style {
            levels.extend(self.chain(id).into_iter().filter_map(|s| s.ppr.as_ref()));
        }
        if let Some(direct) = &para.props {
            levels.push(direct);
        }
        let mut tabs: Vec<TabStop> = Vec::new();
        for ppr in levels {
            for e in ppr
                .children
                .iter()
                .filter_map(RawXml::as_element)
                .filter(|e| e.head.is_w(e.head.local_name()))
            {
                match e.head.local_name() {
                    "widowControl" => {
                        extra.widow_control = on_off(e);
                    }
                    "pageBreakBefore" => {
                        extra.page_break_before = on_off(e);
                    }
                    "contextualSpacing" => {
                        extra.contextual_spacing = on_off(e);
                    }
                    "tabs" => {
                        for t in e.children.iter().filter_map(RawXml::as_element) {
                            if !t.head.is_w("tab") {
                                continue;
                            }
                            let Some(pos) = twips(t.head.attr_w("pos")) else {
                                continue;
                            };
                            let val = t.head.attr_w("val").unwrap_or("left");
                            tabs.retain(|s| (s.pos - pos).abs() > 0.01);
                            let kind = match val {
                                "clear" => continue,
                                "center" => TabKind::Center,
                                "right" | "end" => TabKind::Right,
                                "decimal" => TabKind::Decimal,
                                "bar" => TabKind::Bar,
                                _ => TabKind::Left,
                            };
                            let leader = match t.head.attr_w("leader") {
                                None | Some("none") => TabLeader::None,
                                Some("dot" | "middleDot") => TabLeader::Dot,
                                Some("hyphen") => TabLeader::Hyphen,
                                Some("underscore") => TabLeader::Underscore,
                                Some(_) => TabLeader::Other,
                            };
                            tabs.push(TabStop { pos, kind, leader });
                        }
                    }
                    _ => {}
                }
            }
        }
        tabs.sort_by(|a, b| a.pos.total_cmp(&b.pos));
        extra.tabs = tabs;
        extra.style_id = style;
        if let Some(rpr) = para.props.as_ref().and_then(|p| child(p, "rPr")) {
            for e in rpr.children.iter().filter_map(RawXml::as_element) {
                if e.head.is_w("sz") {
                    extra.mark_size_half_points = e.head.attr_w("val").and_then(|v| v.parse().ok());
                } else if e.head.is_w("rFonts") {
                    extra.mark_font = e
                        .head
                        .attr_w("ascii")
                        .or_else(|| e.head.attr_w("hAnsi"))
                        .map(str::to_owned);
                } else if e.head.is_w("b") {
                    extra.mark_bold = Some(on_off(e));
                } else if e.head.is_w("i") {
                    extra.mark_italic = Some(on_off(e));
                }
            }
        }
        extra
    }
}

/// Section properties found in a paragraph's `w:pPr`, when it ends a section.
pub(crate) fn paragraph_section(para: &Paragraph) -> Option<SectionProps> {
    let ppr = para.props.as_ref()?;
    child(ppr, "sectPr").map(SectionProps::from_element)
}
