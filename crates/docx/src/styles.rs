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
    FontHint, LayoutFmt, ParagraphBorders, ParagraphMark, ParagraphProps, RunLayoutProps, TabOp,
    resolve_tabs, tab_ops,
};
use crate::model::{Paragraph, Run};
use crate::numbering::{NumPr, StyleNumPr};
use crate::ppr::Alignment;
use crate::props::RunProps;
use crate::table::{CellPosition, CellProps, Shading, TableProps, TableStyleCondition};
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
    /// East Asian font family (`w:rFonts` `eastAsia`, or the theme's East Asian slot); `None`
    /// when the run names none and the theme has none.
    pub east_asia_font: Option<String>,
    /// `w:rFonts/@w:hint`.
    pub hint: FontHint,
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
    east_asia: Option<FontSpec>,
    hint: Option<FontHint>,
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
        let east_asia = props.fonts().and_then(|f| {
            match f.head.attr_w("eastAsiaTheme") {
                Some(t) if t.starts_with("major") => Some(FontSpec::Major),
                Some(t) if t.starts_with("minor") => Some(FontSpec::Minor),
                _ => None,
            }
            .or_else(|| {
                f.head
                    .attr_w("eastAsia")
                    .map(|n| FontSpec::Name(n.to_owned()))
            })
        });
        let hint = props
            .fonts()
            .and_then(|f| f.head.attr_w("hint"))
            .map(FontHint::parse);
        Self {
            font,
            east_asia,
            hint,
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
        put(&mut self.east_asia, &other.east_asia);
        put(&mut self.hint, &other.hint);
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
    /// `w:beforeAutospacing` / `w:afterAutospacing`: Word's automatic space replaces the value.
    before_auto: Option<bool>,
    after_auto: Option<bool>,
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
    borders: ParagraphBorders,
    /// `w:shd` of `w:pPr`.
    shading: Option<Shading>,
}

fn twips(value: Option<&str>) -> Option<f32> {
    value
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map(|t| t / 20.0)
}

fn spacing_flag(value: &str) -> bool {
    !matches!(value.trim(), "0" | "false" | "off")
}

/// Word's automatic paragraph space (`w:beforeAutospacing`, `w:afterAutospacing`), in points.
pub const AUTO_SPACING_PT: f32 = 14.0;

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
                "pBdr" => fmt.borders = ParagraphBorders::from_element(e),
                "shd" => fmt.shading = Some(Shading::from_element(e)),
                "spacing" => {
                    fmt.before = twips(e.head.attr_w("before")).or(fmt.before);
                    fmt.after = twips(e.head.attr_w("after")).or(fmt.after);
                    // A written space value without the flag replaces inherited automatic space.
                    if e.head.attr_w("before").is_some() {
                        fmt.before_auto = Some(false);
                    }
                    if e.head.attr_w("after").is_some() {
                        fmt.after_auto = Some(false);
                    }
                    if let Some(v) = e.head.attr_w("beforeAutospacing") {
                        fmt.before_auto = Some(spacing_flag(v));
                    }
                    if let Some(v) = e.head.attr_w("afterAutospacing") {
                        fmt.after_auto = Some(spacing_flag(v));
                    }
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
        put(&mut self.before_auto, other.before_auto);
        put(&mut self.after_auto, other.after_auto);
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
        self.borders.overlay(&other.borders);
        if other.shading.is_some() {
            self.shading.clone_from(&other.shading);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct StyleDef {
    info: StyleInfo,
    run: RunFmt,
    para: ParaFmt,
    /// `w:pPr/w:numPr` as written in this style.
    num_pr: NumPr,
    table: Option<TableStyleDef>,
}

/// Table-specific parts of a table style.
#[derive(Debug, Clone, PartialEq)]
struct TableStyleDef {
    /// `w:tblPr`.
    props: TableProps,
    /// `w:tcPr` of the style itself.
    cell: CellProps,
    conditional: Vec<ConditionalDef>,
}

/// One `w:tblStylePr`.
#[derive(Debug, Clone, PartialEq)]
struct ConditionalDef {
    condition: TableStyleCondition,
    props: TableProps,
    cell: CellProps,
    para: ParaFmt,
    run: RunFmt,
}

/// What a table style adds to one cell: its cell properties and the paragraph and run
/// formatting that sits between the document defaults and the paragraph style.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableCellFormat {
    /// Borders, shading, margins and vertical alignment the style gives the cell.
    pub cell: CellProps,
    para: ParaFmt,
    run: RunFmt,
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
    /// The `a:ea` typefaces of the major and minor font slots; an empty typeface is `None`.
    fn parse_east_asia(bytes: &[u8]) -> (Option<String>, Option<String>) {
        let Some(parsed) = xml::parse(bytes).ok() else {
            return (None, None);
        };
        let scheme =
            local_child(&parsed.root, "themeElements").and_then(|e| local_child(e, "fontScheme"));
        let ea = |slot: &str| -> Option<String> {
            let ea = local_child(local_child(scheme?, slot)?, "ea")?;
            ea.head
                .attrs
                .iter()
                .find(|a| a.name == "typeface")
                .map(|a| a.value.clone())
                .filter(|t| !t.is_empty())
        };
        (ea("majorFont"), ea("minorFont"))
    }

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
    /// East Asian typeface of the theme's major and minor font slots (empty slots are `None`).
    theme_ea: (Option<String>, Option<String>),
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
        num_pr: child_el(e, "pPr")
            .and_then(|p| child_el(p, "numPr"))
            .map(NumPr::from_element)
            .unwrap_or_default(),
        table: (kind == StyleKind::Table).then(|| TableStyleDef {
            props: child_el(e, "tblPr")
                .map(TableProps::from_element)
                .unwrap_or_default(),
            cell: child_el(e, "tcPr")
                .map(CellProps::from_element)
                .unwrap_or_default(),
            conditional: e
                .children
                .iter()
                .filter_map(RawXml::as_element)
                .filter(|c| c.head.is_w("tblStylePr"))
                .filter_map(|c| {
                    Some(ConditionalDef {
                        condition: TableStyleCondition::parse(c.head.attr_w("type")?)?,
                        props: child_el(c, "tblPr")
                            .map(TableProps::from_element)
                            .unwrap_or_default(),
                        cell: child_el(c, "tcPr")
                            .map(CellProps::from_element)
                            .unwrap_or_default(),
                        para: child_el(c, "pPr")
                            .map(ParaFmt::from_ppr)
                            .unwrap_or_default(),
                        run: child_el(c, "rPr")
                            .map(|r| RunFmt::from_props(&RunProps::from_element(r.clone())))
                            .unwrap_or_default(),
                    })
                })
                .collect(),
        }),
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
            theme_ea: theme.map(ThemeFonts::parse_east_asia).unwrap_or_default(),
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

    fn resolved_run_fmt(
        &self,
        para: &Paragraph,
        direct: &RunProps,
        table: Option<&TableCellFormat>,
    ) -> RunFmt {
        let mut fmt = self.defaults_run.clone();
        if let Some(t) = table {
            fmt.overlay(&t.run);
        }
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
            east_asia_font: match fmt.east_asia.as_ref() {
                Some(FontSpec::Name(n)) => Some(n.clone()),
                Some(FontSpec::Major) => self.theme_ea.0.clone(),
                Some(FontSpec::Minor) => self.theme_ea.1.clone(),
                None => None,
            },
            hint: fmt.hint.unwrap_or_default(),
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
        self.effective_run_props_in(para, run, None)
    }

    /// [`Styles::effective_run_props`] for a paragraph inside a table cell: the table
    /// style's run formatting sits between the document defaults and the paragraph style.
    pub fn effective_run_props_in(
        &self,
        para: &Paragraph,
        run: &Run,
        table: Option<&TableCellFormat>,
    ) -> EffectiveRun {
        self.effective_from(&self.resolved_run_fmt(para, &run.props, table))
    }

    /// Kerning, ligatures, position, character spacing and scale of `run` inside `para`
    /// after defaults, styles and direct formatting (ECMA-376 Part 1 §17.3.2).
    pub fn effective_run_layout(&self, para: &Paragraph, run: &Run) -> RunLayoutProps {
        self.effective_run_layout_in(para, run, None)
    }

    /// [`Styles::effective_run_layout`] inside a table cell.
    pub fn effective_run_layout_in(
        &self,
        para: &Paragraph,
        run: &Run,
        table: Option<&TableCellFormat>,
    ) -> RunLayoutProps {
        self.resolved_run_fmt(para, &run.props, table)
            .layout
            .finish()
    }

    /// Run properties of the paragraph mark (`w:pPr/w:rPr`, §17.3.1.29): defaults, the
    /// paragraph style's run properties, then the mark's own direct formatting.
    pub fn paragraph_mark_props(&self, para: &Paragraph) -> ParagraphMark {
        self.paragraph_mark_props_in(para, None)
    }

    /// [`Styles::paragraph_mark_props`] inside a table cell.
    pub fn paragraph_mark_props_in(
        &self,
        para: &Paragraph,
        table: Option<&TableCellFormat>,
    ) -> ParagraphMark {
        let direct = para
            .props
            .as_ref()
            .and_then(|p| child_el(p, "rPr"))
            .map(|r| RunProps::from_element(r.clone()))
            .unwrap_or_default();
        let fmt = self.resolved_run_fmt(para, &direct, table);
        ParagraphMark {
            run: self.effective_from(&fmt),
            layout: fmt.layout.finish(),
        }
    }

    /// Widow control, keep-with-next, keep-lines, page-break-before, contextual spacing,
    /// auto-hyphenation suppression and tab stops of `para` after style resolution
    /// (ECMA-376 Part 1 §17.3.1).
    pub fn paragraph_flow_props(&self, para: &Paragraph) -> ParagraphProps {
        self.paragraph_flow_props_in(para, None)
    }

    /// [`Styles::paragraph_flow_props`] inside a table cell.
    pub fn paragraph_flow_props_in(
        &self,
        para: &Paragraph,
        table: Option<&TableCellFormat>,
    ) -> ParagraphProps {
        let fmt = self.resolved_para_fmt(para, table);
        ParagraphProps {
            widow_control: fmt.widow_control.unwrap_or(true),
            keep_next: fmt.keep_next.unwrap_or(false),
            keep_lines: fmt.keep_lines.unwrap_or(false),
            page_break_before: fmt.page_break_before.unwrap_or(false),
            contextual_spacing: fmt.contextual_spacing.unwrap_or(false),
            suppress_auto_hyphens: fmt.suppress_auto_hyphens.unwrap_or(false),
            before_auto: fmt.before_auto.unwrap_or(false),
            after_auto: fmt.after_auto.unwrap_or(false),
            shading: fmt.shading.clone(),
            tabs: resolve_tabs(&fmt.tab_ops),
            borders: fmt.borders.visible(),
        }
    }

    fn resolved_para_fmt(&self, para: &Paragraph, table: Option<&TableCellFormat>) -> ParaFmt {
        let mut fmt = self.defaults_para.clone();
        if let Some(t) = table {
            fmt.overlay(&t.para);
        }
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
        self.effective_para_props_in(para, None)
    }

    /// [`Styles::effective_para_props`] inside a table cell.
    pub fn effective_para_props_in(
        &self,
        para: &Paragraph,
        table: Option<&TableCellFormat>,
    ) -> EffectiveParagraph {
        let fmt = self.resolved_para_fmt(para, table);
        EffectiveParagraph {
            alignment: fmt.alignment.unwrap_or(Alignment::Left),
            space_before_pt: if fmt.before_auto == Some(true) {
                AUTO_SPACING_PT
            } else {
                fmt.before.unwrap_or(0.0)
            },
            space_after_pt: if fmt.after_auto == Some(true) {
                AUTO_SPACING_PT
            } else {
                fmt.after.unwrap_or(0.0)
            },
            line: fmt.line.unwrap_or(LineSpacing::Multiple(1.0)),
            indent_left_pt: fmt.left.unwrap_or(0.0),
            indent_right_pt: fmt.right.unwrap_or(0.0),
            first_line_pt: fmt.first_line.unwrap_or(0.0),
            keep_next: fmt.keep_next.unwrap_or(false),
            keep_lines: fmt.keep_lines.unwrap_or(false),
        }
    }

    /// Id of the default table style (`w:type="table" w:default="1"`).
    pub fn default_table_style(&self) -> Option<&str> {
        self.order
            .iter()
            .filter_map(|id| self.index.get(id))
            .find(|d| d.info.kind == StyleKind::Table && d.info.is_default)
            .map(|d| d.info.id.as_str())
    }

    /// The table's effective properties: its own `w:tblPr` over its table style (and the
    /// styles that one is based on), over the default table style.
    pub fn table_props(&self, direct: &TableProps) -> TableProps {
        let mut out = direct.clone();
        let style = direct
            .style_id
            .as_deref()
            .or_else(|| self.default_table_style());
        if let Some(id) = style {
            for def in self.chain(id) {
                if let Some(t) = &def.table {
                    out = out.over(t.props.clone());
                }
            }
        }
        out
    }

    /// What the table style of `table` (already resolved with [`Styles::table_props`])
    /// gives the cell at `pos`: whole-table formatting of the style chain, then each
    /// condition that applies, base styles first.
    pub fn table_cell_format(&self, table: &TableProps, pos: CellPosition) -> TableCellFormat {
        let mut out = TableCellFormat::default();
        let Some(id) = table
            .style_id
            .as_deref()
            .or_else(|| self.default_table_style())
        else {
            return out;
        };
        let look = table.look.unwrap_or_default();
        let band = (
            table.row_band_size.unwrap_or(1),
            table.col_band_size.unwrap_or(1),
        );
        let conditions = TableStyleCondition::applying(look, band, pos);
        for def in self.chain(id).into_iter().rev() {
            let Some(t) = &def.table else { continue };
            // The style's own pPr/rPr/tcPr are its whole-table formatting.
            out.para.overlay(&def.para);
            out.run.overlay(&def.run);
            out.cell = merge_cell(&t.cell, &out.cell);
            for cond in &conditions {
                for c in t.conditional.iter().filter(|c| c.condition == *cond) {
                    out.para.overlay(&c.para);
                    out.run.overlay(&c.run);
                    let mut cell = c.cell.clone();
                    cell.margins = cell.margins.over(c.props.cell_margins);
                    out.cell = merge_cell(&cell, &out.cell);
                }
            }
        }
        out
    }

    /// The list a paragraph style assigns through `w:numPr`, looking through `w:basedOn`:
    /// the first style of the chain with a `w:numId` wins and supplies the level too.
    /// `None` for an unknown or non-paragraph style, or when no style in the chain numbers.
    pub fn style_num_pr(&self, id: &str) -> Option<StyleNumPr> {
        if self.index.get(id)?.info.kind != StyleKind::Paragraph {
            return None;
        }
        self.chain(id).into_iter().find_map(|def| {
            Some(StyleNumPr {
                num_id: def.num_pr.num_id?,
                ilvl: def.num_pr.ilvl,
                holder: def.info.id.clone(),
            })
        })
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

/// `top` over `lower`, field by field.
fn merge_cell(top: &CellProps, lower: &CellProps) -> CellProps {
    CellProps {
        width: top.width.or(lower.width),
        grid_span: top.grid_span,
        v_merge: top.v_merge,
        margins: top.margins.over(lower.margins),
        v_align: top.v_align.or(lower.v_align),
        borders: top.borders.clone().over(lower.borders.clone()),
        shading: top.shading.clone().or_else(|| lower.shading.clone()),
        no_wrap: top.no_wrap || lower.no_wrap,
    }
}
