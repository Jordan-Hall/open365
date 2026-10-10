//! Styles (ECMA-376 Part 1 §17.7): `word/styles.xml` and the theme fonts, resolved into the
//! properties a run or paragraph really has. Resolution order for a run is document
//! defaults, then the paragraph style chain (`w:basedOn`), then the character style chain,
//! then direct formatting. Toggle properties (bold, italic) are overridden, not XOR-ed.
//!
//! The styles tree is kept as generic XML; unknown properties survive and the part is
//! only written back after an edit (see [`Styles::ensure_builtin`]).

use std::collections::HashMap;

use open365_opc::PartName;

use crate::DocxError;
use crate::layout_props::{
    LayoutFmt, ParagraphMark, ParagraphProps, RunLayoutProps, TabOp, resolve_tabs, tab_ops,
};
use crate::model::{Paragraph, Run};
use crate::ppr::Alignment;
use crate::props::RunProps;
use crate::xml::{self, ParsedXml, RawXml, XmlElement};

const WORD_NS: &str = crate::xml::W_NS;
/// Font used when nothing in the document names one (the format's own default).
const FALLBACK_FONT: &str = "Times New Roman";
const FALLBACK_SIZE_HALF_POINTS: u32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleKind {
    Paragraph,
    Character,
    Table,
    Numbering,
}

/// Name and relations of one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleInfo {
    pub id: String,
    /// Display name (`w:name`), for example `heading 1`.
    pub name: String,
    pub kind: StyleKind,
    pub based_on: Option<String>,
    pub next: Option<String>,
    pub is_default: bool,
}

/// Line spacing (`w:spacing` `w:line` and `w:lineRule`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineSpacing {
    /// Multiple of the single line height (1.0 is single).
    Multiple(f32),
    /// At least this many points.
    AtLeast(f32),
    /// Exactly this many points.
    Exactly(f32),
}

/// Run formatting after style resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveRun {
    /// Latin font family name (theme fonts resolved).
    pub font: String,
    pub size_half_points: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// `None` is automatic colour.
    pub color: Option<[u8; 3]>,
}

/// Paragraph formatting after style resolution. Lengths are in points.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveParagraph {
    pub alignment: Alignment,
    pub space_before_pt: f32,
    pub space_after_pt: f32,
    pub line: LineSpacing,
    pub indent_left_pt: f32,
    pub indent_right_pt: f32,
    /// First-line indent; negative for a hanging indent.
    pub first_line_pt: f32,
    pub keep_next: bool,
    pub keep_lines: bool,
}

impl Default for EffectiveParagraph {
    fn default() -> Self {
        Self {
            alignment: Alignment::Left,
            space_before_pt: 0.0,
            space_after_pt: 0.0,
            line: LineSpacing::Multiple(1.0),
            indent_left_pt: 0.0,
            indent_right_pt: 0.0,
            first_line_pt: 0.0,
            keep_next: false,
            keep_lines: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FontSpec {
    Name(String),
    Major,
    Minor,
}

/// Run properties as written in one place; `None` means "not set here".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RunFmt {
    font: Option<FontSpec>,
    size: Option<u32>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    color: Option<Option<[u8; 3]>>,
    layout: LayoutFmt,
}

impl RunFmt {
    fn from_props(props: &RunProps) -> Self {
        let font = props.fonts().and_then(|f| {
            let theme = |v: Option<&str>| match v {
                Some(t) if t.starts_with("major") => Some(FontSpec::Major),
                Some(t) if t.starts_with("minor") => Some(FontSpec::Minor),
                _ => None,
            };
            theme(f.head.attr_w("asciiTheme"))
                .or_else(|| f.head.attr_w("ascii").map(|n| FontSpec::Name(n.to_owned())))
                .or_else(|| theme(f.head.attr_w("hAnsiTheme")))
                .or_else(|| f.head.attr_w("hAnsi").map(|n| FontSpec::Name(n.to_owned())))
        });
        Self {
            font,
            size: props.size_half_points(),
            bold: props.bold(),
            italic: props.italic(),
            underline: props.underline().map(|u| u != "none"),
            strike: props.strike(),
            color: props.color().map(parse_color),
            layout: LayoutFmt::from_props(props),
        }
    }

    fn overlay(&mut self, other: &Self) {
        fn put<T: Clone>(slot: &mut Option<T>, value: &Option<T>) {
            if value.is_some() {
                slot.clone_from(value);
            }
        }
        put(&mut self.font, &other.font);
        put(&mut self.size, &other.size);
        put(&mut self.bold, &other.bold);
        put(&mut self.italic, &other.italic);
        put(&mut self.underline, &other.underline);
        put(&mut self.strike, &other.strike);
        put(&mut self.color, &other.color);
        self.layout.overlay(&other.layout);
    }
}

fn parse_color(value: &str) -> Option<[u8; 3]> {
    if value.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(value, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Paragraph properties as written in one place.
#[derive(Debug, Clone, Default, PartialEq)]
struct ParaFmt {
    alignment: Option<Alignment>,
    before: Option<f32>,
    after: Option<f32>,
    line: Option<LineSpacing>,
    left: Option<f32>,
    right: Option<f32>,
    first_line: Option<f32>,
    keep_next: Option<bool>,
    keep_lines: Option<bool>,
    widow_control: Option<bool>,
    page_break_before: Option<bool>,
    contextual_spacing: Option<bool>,
    suppress_auto_hyphens: Option<bool>,
    tab_ops: Vec<TabOp>,
}

fn twips(value: Option<&str>) -> Option<f32> {
    value
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map(|t| t / 20.0)
}

fn on_off(element: &XmlElement) -> bool {
    !matches!(element.head.attr_w("val"), Some("0" | "false" | "off"))
}

impl ParaFmt {
    fn from_ppr(ppr: &XmlElement) -> Self {
        let mut fmt = Self::default();
        for child in &ppr.children {
            let RawXml::Element(e) = child else { continue };
            if !e.head.ns.as_deref().is_some_and(|n| xml::is_w_ns(Some(n))) {
                continue;
            }
            match e.head.local_name() {
                "jc" => fmt.alignment = e.head.attr_w("val").and_then(Alignment::from_val),
                "keepNext" => fmt.keep_next = Some(on_off(e)),
                "keepLines" => fmt.keep_lines = Some(on_off(e)),
                "widowControl" => fmt.widow_control = Some(on_off(e)),
                "pageBreakBefore" => fmt.page_break_before = Some(on_off(e)),
                "contextualSpacing" => fmt.contextual_spacing = Some(on_off(e)),
                "suppressAutoHyphens" => fmt.suppress_auto_hyphens = Some(on_off(e)),
                "tabs" => fmt.tab_ops.extend(tab_ops(e)),
                "spacing" => {
                    fmt.before = twips(e.head.attr_w("before")).or(fmt.before);
                    fmt.after = twips(e.head.attr_w("after")).or(fmt.after);
                    if let Some(line) = e
                        .head
                        .attr_w("line")
                        .and_then(|v| v.trim().parse::<f32>().ok())
                    {
                        fmt.line = Some(match e.head.attr_w("lineRule") {
                            Some("atLeast") => LineSpacing::AtLeast(line / 20.0),
                            Some("exact") => LineSpacing::Exactly(line / 20.0),
                            _ => LineSpacing::Multiple(line / 240.0),
                        });
                    }
                }
                "ind" => {
                    let attr = |a: &str, b: &str| {
                        twips(e.head.attr_w(a)).or_else(|| twips(e.head.attr_w(b)))
                    };
                    fmt.left = attr("left", "start").or(fmt.left);
                    fmt.right = attr("right", "end").or(fmt.right);
                    if let Some(hanging) = twips(e.head.attr_w("hanging")) {
                        fmt.first_line = Some(-hanging);
                    } else if let Some(first) = twips(e.head.attr_w("firstLine")) {
                        fmt.first_line = Some(first);
                    }
                }
                _ => {}
            }
        }
        fmt
    }

    fn overlay(&mut self, other: &Self) {
        fn put<T: Copy>(slot: &mut Option<T>, value: Option<T>) {
            if value.is_some() {
                *slot = value;
            }
        }
        put(&mut self.alignment, other.alignment);
        put(&mut self.before, other.before);
        put(&mut self.after, other.after);
        put(&mut self.line, other.line);
        put(&mut self.left, other.left);
        put(&mut self.right, other.right);
        put(&mut self.first_line, other.first_line);
        put(&mut self.keep_next, other.keep_next);
        put(&mut self.keep_lines, other.keep_lines);
        put(&mut self.widow_control, other.widow_control);
        put(&mut self.page_break_before, other.page_break_before);
        put(&mut self.contextual_spacing, other.contextual_spacing);
        put(&mut self.suppress_auto_hyphens, other.suppress_auto_hyphens);
        self.tab_ops.extend(other.tab_ops.iter().copied());
    }
}

#[derive(Debug, Clone, PartialEq)]
struct StyleDef {
    info: StyleInfo,
    run: RunFmt,
    para: ParaFmt,
}

/// Theme fonts (`a:fontScheme` of the theme part): the Latin typeface of each slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeFonts {
    pub major: String,
    pub minor: String,
}

impl Default for ThemeFonts {
    /// The Office theme, used when the document has no theme part.
    fn default() -> Self {
        Self {
            major: "Calibri Light".to_owned(),
            minor: "Calibri".to_owned(),
        }
    }
}

fn local_child<'a>(el: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    el.children.iter().find_map(|c| match c {
        RawXml::Element(e) if e.head.local_name() == local => Some(e),
        _ => None,
    })
}

impl ThemeFonts {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let parsed = xml::parse(bytes).ok()?;
        let elements = local_child(&parsed.root, "themeElements")?;
        let scheme = local_child(elements, "fontScheme")?;
        let latin = |slot: &str| -> Option<String> {
            let latin = local_child(local_child(scheme, slot)?, "latin")?;
            latin
                .head
                .attrs
                .iter()
                .find(|a| a.name == "typeface")
                .map(|a| a.value.clone())
                .filter(|t| !t.is_empty())
        };
        let default = Self::default();
        Some(Self {
            major: latin("majorFont").unwrap_or(default.major),
            minor: latin("minorFont").unwrap_or(default.minor),
        })
    }
}

/// The parsed styles part plus theme fonts.
#[derive(Debug, Clone)]
pub struct Styles {
    part: PartName,
    parsed: ParsedXml,
    defaults_run: RunFmt,
    defaults_para: ParaFmt,
    index: HashMap<String, StyleDef>,
    order: Vec<String>,
    default_paragraph: Option<String>,
    theme: ThemeFonts,
    dirty: bool,
}

fn child_val(e: &XmlElement, local: &str) -> Option<String> {
    child_el(e, local).and_then(|x| x.head.attr_w("val").map(str::to_owned))
}

fn child_el<'a>(e: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    e.children.iter().find_map(|c| match c {
        RawXml::Element(x) if x.head.is_w(local) => Some(x),
        _ => None,
    })
}

fn parse_style(e: &XmlElement) -> Option<StyleDef> {
    let id = e.head.attr_w("styleId")?.to_owned();
    let kind = match e.head.attr_w("type").unwrap_or("paragraph") {
        "character" => StyleKind::Character,
        "table" => StyleKind::Table,
        "numbering" => StyleKind::Numbering,
        _ => StyleKind::Paragraph,
    };
    let is_default = matches!(e.head.attr_w("default"), Some("1" | "true" | "on"));
    Some(StyleDef {
        info: StyleInfo {
            name: child_val(e, "name").unwrap_or_else(|| id.clone()),
            id,
            kind,
            based_on: child_val(e, "basedOn"),
            next: child_val(e, "next"),
            is_default,
        },
        run: child_el(e, "rPr")
            .map(|r| RunFmt::from_props(&RunProps::from_element(r.clone())))
            .unwrap_or_default(),
        para: child_el(e, "pPr")
            .map(ParaFmt::from_ppr)
            .unwrap_or_default(),
    })
}

impl Styles {
    /// Parses a styles part. `theme` is the theme part, when the document has one.
    pub(crate) fn parse(
        part: PartName,
        xml_bytes: &[u8],
        theme: Option<&[u8]>,
    ) -> Result<Self, DocxError> {
        let parsed = xml::parse(xml_bytes)?;
        if !parsed.root.head.is_w("styles") {
            return Err(DocxError::NotWordprocessing(
                "styles part root is not w:styles".to_owned(),
            ));
        }
        let mut styles = Self {
            part,
            defaults_run: RunFmt::default(),
            defaults_para: ParaFmt::default(),
            index: HashMap::new(),
            order: Vec::new(),
            default_paragraph: None,
            theme: theme.and_then(ThemeFonts::parse).unwrap_or_default(),
            dirty: false,
            parsed,
        };
        let root = styles.parsed.root.clone();
        if let Some(defaults) = child_el(&root, "docDefaults") {
            if let Some(r) = child_el(defaults, "rPrDefault").and_then(|d| child_el(d, "rPr")) {
                styles.defaults_run = RunFmt::from_props(&RunProps::from_element(r.clone()));
            }
            if let Some(p) = child_el(defaults, "pPrDefault").and_then(|d| child_el(d, "pPr")) {
                styles.defaults_para = ParaFmt::from_ppr(p);
            }
        }
        for child in &root.children {
            if let RawXml::Element(e) = child
                && e.head.is_w("style")
            {
                styles.add_to_index(e);
            }
        }
        Ok(styles)
    }

    fn add_to_index(&mut self, e: &XmlElement) {
        let Some(def) = parse_style(e) else { return };
        if def.info.is_default && def.info.kind == StyleKind::Paragraph {
            self.default_paragraph = Some(def.info.id.clone());
        }
        if !self.index.contains_key(&def.info.id) {
            self.order.push(def.info.id.clone());
        }
        self.index.insert(def.info.id.clone(), def);
    }

    pub fn theme_fonts(&self) -> &ThemeFonts {
        &self.theme
    }

    pub fn has_style(&self, id: &str) -> bool {
        self.index.contains_key(id)
    }

    pub fn style(&self, id: &str) -> Option<&StyleInfo> {
        self.index.get(id).map(|d| &d.info)
    }

    /// All styles in file order.
    pub fn styles(&self) -> impl Iterator<Item = &StyleInfo> {
        self.order
            .iter()
            .filter_map(|id| self.index.get(id).map(|d| &d.info))
    }

    /// Id of the style used by paragraphs without `w:pStyle` (`w:default="1"`).
    pub fn default_paragraph_style(&self) -> Option<&str> {
        self.default_paragraph.as_deref()
    }

    pub fn part_name(&self) -> &PartName {
        &self.part
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn mark_clean(&mut self) {
        self.dirty = false;
    }

    pub(crate) fn serialize(&self) -> Vec<u8> {
        self.parsed.to_bytes()
    }

    /// The style `id` followed by its `w:basedOn` ancestors. Stops at cycles.
    fn chain(&self, id: &str) -> Vec<&StyleDef> {
        let mut out: Vec<&StyleDef> = Vec::new();
        let mut current = Some(id);
        while let Some(c) = current {
            let Some(def) = self.index.get(c) else { break };
            if out.iter().any(|d| d.info.id == def.info.id) || out.len() >= 32 {
                break;
            }
            out.push(def);
            current = def.info.based_on.as_deref();
        }
        out
    }

    fn resolve_font(&self, spec: Option<&FontSpec>) -> String {
        match spec {
            Some(FontSpec::Name(n)) => n.clone(),
            Some(FontSpec::Major) => self.theme.major.clone(),
            Some(FontSpec::Minor) => self.theme.minor.clone(),
            None => FALLBACK_FONT.to_owned(),
        }
    }

    fn paragraph_style_id<'a>(&'a self, para: &'a Paragraph) -> Option<&'a str> {
        para.style_id().or(self.default_paragraph.as_deref())
    }

    fn resolved_run_fmt(&self, para: &Paragraph, direct: &RunProps) -> RunFmt {
        let mut fmt = self.defaults_run.clone();
        if let Some(id) = self.paragraph_style_id(para) {
            for def in self.chain(id).into_iter().rev() {
                fmt.overlay(&def.run);
            }
        }
        if let Some(id) = direct.style_id() {
            for def in self.chain(id).into_iter().rev() {
                fmt.overlay(&def.run);
            }
        }
        fmt.overlay(&RunFmt::from_props(direct));
        fmt
    }

    fn effective_from(&self, fmt: &RunFmt) -> EffectiveRun {
        EffectiveRun {
            font: self.resolve_font(fmt.font.as_ref()),
            size_half_points: fmt.size.unwrap_or(FALLBACK_SIZE_HALF_POINTS),
            bold: fmt.bold.unwrap_or(false),
            italic: fmt.italic.unwrap_or(false),
            underline: fmt.underline.unwrap_or(false),
            strike: fmt.strike.unwrap_or(false),
            color: fmt.color.flatten(),
        }
    }

    /// Formatting of `run` inside `para` after defaults, styles and direct formatting.
    pub fn effective_run_props(&self, para: &Paragraph, run: &Run) -> EffectiveRun {
        self.effective_from(&self.resolved_run_fmt(para, &run.props))
    }

    /// Kerning, ligatures, position, character spacing and scale of `run` inside `para`
    /// after defaults, styles and direct formatting (ECMA-376 Part 1 §17.3.2).
    pub fn effective_run_layout(&self, para: &Paragraph, run: &Run) -> RunLayoutProps {
        self.resolved_run_fmt(para, &run.props).layout.finish()
    }

    /// Run properties of the paragraph mark (`w:pPr/w:rPr`, §17.3.1.29): defaults, the
    /// paragraph style's run properties, then the mark's own direct formatting.
    pub fn paragraph_mark_props(&self, para: &Paragraph) -> ParagraphMark {
        let direct = para
            .props
            .as_ref()
            .and_then(|p| child_el(p, "rPr"))
            .map(|r| RunProps::from_element(r.clone()))
            .unwrap_or_default();
        let fmt = self.resolved_run_fmt(para, &direct);
        ParagraphMark {
            run: self.effective_from(&fmt),
            layout: fmt.layout.finish(),
        }
    }

    /// Widow control, keep-with-next, keep-lines, page-break-before, contextual spacing,
    /// auto-hyphenation suppression and tab stops of `para` after style resolution
    /// (ECMA-376 Part 1 §17.3.1).
    pub fn paragraph_flow_props(&self, para: &Paragraph) -> ParagraphProps {
        let fmt = self.resolved_para_fmt(para);
        ParagraphProps {
            widow_control: fmt.widow_control.unwrap_or(true),
            keep_next: fmt.keep_next.unwrap_or(false),
            keep_lines: fmt.keep_lines.unwrap_or(false),
            page_break_before: fmt.page_break_before.unwrap_or(false),
            contextual_spacing: fmt.contextual_spacing.unwrap_or(false),
            suppress_auto_hyphens: fmt.suppress_auto_hyphens.unwrap_or(false),
            tabs: resolve_tabs(&fmt.tab_ops),
        }
    }

    fn resolved_para_fmt(&self, para: &Paragraph) -> ParaFmt {
        let mut fmt = self.defaults_para.clone();
        if let Some(id) = self.paragraph_style_id(para) {
            for def in self.chain(id).into_iter().rev() {
                fmt.overlay(&def.para);
            }
        }
        if let Some(direct) = &para.props {
            fmt.overlay(&ParaFmt::from_ppr(direct));
        }
        fmt
    }

    /// Formatting of the paragraph itself after defaults, style chain and direct formatting.
    pub fn effective_para_props(&self, para: &Paragraph) -> EffectiveParagraph {
        let fmt = self.resolved_para_fmt(para);
        EffectiveParagraph {
            alignment: fmt.alignment.unwrap_or(Alignment::Left),
            space_before_pt: fmt.before.unwrap_or(0.0),
            space_after_pt: fmt.after.unwrap_or(0.0),
            line: fmt.line.unwrap_or(LineSpacing::Multiple(1.0)),
            indent_left_pt: fmt.left.unwrap_or(0.0),
            indent_right_pt: fmt.right.unwrap_or(0.0),
            first_line_pt: fmt.first_line.unwrap_or(0.0),
            keep_next: fmt.keep_next.unwrap_or(false),
            keep_lines: fmt.keep_lines.unwrap_or(false),
        }
    }

    /// Ensures a definition exists for one of the built-in styles the editor offers
    /// (`Normal`, `Heading1`-`Heading3`, `Title`). Returns true when the style exists
    /// afterwards; false for any other missing id. Adds the definition to the part.
    pub fn ensure_builtin(&mut self, id: &str) -> bool {
        if self.has_style(id) {
            return true;
        }
        let Some(template) = builtin_style(id) else {
            return false;
        };
        let declared = format!(
            "<w:style xmlns:w=\"{WORD_NS}\"{}",
            template.trim_start_matches("<w:style")
        );
        let Ok(parsed) = xml::parse(declared.as_bytes()) else {
            return false;
        };
        let mut element = parsed.root;
        // The root usually declares the same prefix already.
        if self.parsed.root.head.name.starts_with("w:") {
            element.head.attrs.retain(|a| a.name != "xmlns:w");
        }
        self.add_to_index(&element);
        self.parsed.root.children.push(RawXml::Element(element));
        self.dirty = true;
        self.has_style(id)
    }
}

/// Definitions of the built-in styles, written from the public format description.
fn builtin_style(id: &str) -> Option<&'static str> {
    Some(match id {
        "Normal" => {
            r#"<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>"#
        }
        "Heading1" => {
            r#"<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="240" w:after="0"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:rFonts w:asciiTheme="majorHAnsi" w:eastAsiaTheme="majorEastAsia" w:hAnsiTheme="majorHAnsi" w:cstheme="majorBidi"/><w:color w:val="2F5496"/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr></w:style>"#
        }
        "Heading2" => {
            r#"<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:unhideWhenUsed/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="40" w:after="0"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:rFonts w:asciiTheme="majorHAnsi" w:eastAsiaTheme="majorEastAsia" w:hAnsiTheme="majorHAnsi" w:cstheme="majorBidi"/><w:color w:val="2F5496"/><w:sz w:val="26"/><w:szCs w:val="26"/></w:rPr></w:style>"#
        }
        "Heading3" => {
            r#"<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:unhideWhenUsed/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="40" w:after="0"/><w:outlineLvl w:val="2"/></w:pPr><w:rPr><w:rFonts w:asciiTheme="majorHAnsi" w:eastAsiaTheme="majorEastAsia" w:hAnsiTheme="majorHAnsi" w:cstheme="majorBidi"/><w:color w:val="1F3763"/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style>"#
        }
        "Title" => {
            r#"<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="10"/><w:qFormat/><w:pPr><w:contextualSpacing/></w:pPr><w:rPr><w:rFonts w:asciiTheme="majorHAnsi" w:eastAsiaTheme="majorEastAsia" w:hAnsiTheme="majorHAnsi" w:cstheme="majorBidi"/><w:kern w:val="28"/><w:sz w:val="56"/><w:szCs w:val="56"/></w:rPr></w:style>"#
        }
        _ => return None,
    })
}
