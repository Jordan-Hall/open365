//! Design tab edits: the theme (colours, fonts, effects), style sets, paragraph spacing, page
//! colour, page borders and watermarks. Each edit works on raw XML and leaves unrelated
//! children alone; parts that do not exist yet (theme, header) are created with their
//! relationship and content-type override.

use crate::DocxError;
use crate::document::Document;
use crate::ext_layout::upsert_ordered;
use crate::model::DocChild;
use crate::xml::{self, Attr, RawXml, XmlElement};

pub(crate) const REL_THEME: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
const CT_THEME: &str = "application/vnd.openxmlformats-officedocument.theme+xml";
const REL_HEADER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/header";
const CT_HEADER: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// Slot names of a colour scheme, in schema order.
pub const THEME_COLOUR_SLOTS: [&str; 12] = [
    "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6",
    "hlink", "folHlink",
];

/// A complete theme: colours (`RRGGBB`, in [`THEME_COLOUR_SLOTS`] order), the heading and body
/// font, and the effect level (0 flat, 1 subtle, 2 moderate, 3 intense).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeSpec {
    pub name: String,
    pub colours: [String; 12],
    pub major_font: String,
    pub minor_font: String,
    pub effects: u8,
}

/// One style a style set redefines: its paragraph and run properties as the inner XML of
/// `w:pPr` and `w:rPr` (empty for none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleOverride {
    pub id: String,
    /// `w:name`, used when the document lacks the style.
    pub name: String,
    pub ppr: String,
    pub rpr: String,
}

/// One side of a page border.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBorderSide {
    /// `w:val`: a line style (`single`, `double`...) or an art name.
    pub style: String,
    /// `w:sz` in eighths of a point.
    pub width_eighths: u32,
    /// `RRGGBB` or `auto`.
    pub colour: String,
    /// `w:space` in points (0 to 31).
    pub space_pt: u32,
}

/// Page borders (Borders and Shading, Page Border).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBorders {
    /// Top, left, bottom, right.
    pub sides: [Option<PageBorderSide>; 4],
    /// Measure the distance from the text instead of the page edge.
    pub from_text: bool,
    pub shadow: bool,
    /// `allPages`, `firstPage` or `notFirstPage`.
    pub display: String,
}

/// A text watermark in the headers (Design > Watermark).
#[derive(Debug, Clone, PartialEq)]
pub struct Watermark {
    pub text: String,
    pub font: String,
    /// `None` is Auto: the text is sized to the page.
    pub size_pt: Option<f64>,
    /// `RRGGBB`.
    pub colour: String,
    pub semi_transparent: bool,
    pub diagonal: bool,
}

fn esc(text: &str) -> String {
    let mut out = String::new();
    xml::escape_text(text, &mut out);
    out.replace('"', "&quot;")
}

fn theme_xml(spec: &ThemeSpec) -> String {
    let mut colours = String::new();
    for (slot, hex) in THEME_COLOUR_SLOTS.iter().zip(&spec.colours) {
        colours.push_str(&format!("<a:{slot}><a:srgbClr val=\"{hex}\"/></a:{slot}>"));
    }
    let shadow = |blur: u32, dist: u32, alpha: u32| {
        format!(
            "<a:effectStyle><a:effectLst><a:outerShdw blurRad=\"{blur}\" dist=\"{dist}\" dir=\"5400000\" rotWithShape=\"0\"><a:srgbClr val=\"000000\"><a:alpha val=\"{alpha}\"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle>"
        )
    };
    let flat = "<a:effectStyle><a:effectLst/></a:effectStyle>".to_owned();
    let effects = match spec.effects {
        0 => [flat.clone(), flat.clone(), flat],
        1 => [flat.clone(), flat, shadow(57150, 19050, 63000)],
        2 => [
            flat,
            shadow(40000, 23000, 35000),
            shadow(57150, 19050, 63000),
        ],
        _ => [
            shadow(40000, 23000, 35000),
            shadow(63500, 38100, 50000),
            shadow(101600, 63500, 60000),
        ],
    }
    .concat();
    let fill = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    let line = |w: u32| {
        format!("<a:ln w=\"{w}\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>")
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<a:theme xmlns:a=\"{NS_A}\" name=\"{name}\"><a:themeElements><a:clrScheme name=\"{name}\">{colours}</a:clrScheme><a:fontScheme name=\"{name}\"><a:majorFont><a:latin typeface=\"{major}\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"{minor}\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme><a:fmtScheme name=\"{name}\"><a:fillStyleLst>{fill}{fill}{fill}</a:fillStyleLst><a:lnStyleLst>{l1}{l2}{l3}</a:lnStyleLst><a:effectStyleLst>{effects}</a:effectStyleLst><a:bgFillStyleLst>{fill}{fill}{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>",
        name = esc(&spec.name),
        major = esc(&spec.major_font),
        minor = esc(&spec.minor_font),
        l1 = line(6350),
        l2 = line(12700),
        l3 = line(19050),
    )
}

fn child_named<'a>(e: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    e.children.iter().find_map(|c| match c {
        RawXml::Element(x) if x.head.local_name() == local => Some(x),
        _ => None,
    })
}

/// Replaces `themeElements/<local>` of `target` by the same element of `fresh`.
fn replace_theme_part(target: &mut XmlElement, fresh: &XmlElement, local: &str) -> bool {
    let Some(new_elements) = child_named(fresh, "themeElements") else {
        return false;
    };
    let Some(replacement) = child_named(new_elements, local).cloned() else {
        return false;
    };
    let Some(RawXml::Element(elements)) = target
        .children
        .iter_mut()
        .find(|c| matches!(c, RawXml::Element(e) if e.head.local_name() == "themeElements"))
    else {
        return false;
    };
    match elements
        .children
        .iter_mut()
        .find(|c| matches!(c, RawXml::Element(e) if e.head.local_name() == local))
    {
        Some(slot) => *slot = RawXml::Element(replacement),
        None => elements.children.push(RawXml::Element(replacement)),
    }
    true
}

fn parse_snippet(inner: &str, local: &str) -> Result<XmlElement, DocxError> {
    let text = format!("<w:{local} xmlns:w=\"{NS_W}\">{inner}</w:{local}>");
    let mut parsed = xml::parse(text.as_bytes())?;
    parsed
        .root
        .head
        .attrs
        .retain(|a| !a.name.starts_with("xmlns"));
    Ok(parsed.root)
}

fn plain_attr(name: &str, value: &str) -> Attr {
    Attr {
        name: name.to_owned(),
        ns: None,
        value: value.to_owned(),
    }
}

impl Document {
    // ---- theme -------------------------------------------------------------------------

    /// Replaces the document theme (colours, fonts and effects) by `spec`, creating the theme
    /// part when the package has none.
    ///
    /// # Errors
    /// The package cannot be edited.
    pub fn set_theme(&mut self, spec: &ThemeSpec) -> Result<(), DocxError> {
        let xml_text = theme_xml(spec);
        let (name, _) =
            self.ensure_part(REL_THEME, "word/theme/theme1.xml", CT_THEME, &xml_text)?;
        self.ext_package_mut().set_part(name, xml_text.into_bytes());
        self.ext_reload_parts();
        Ok(())
    }

    /// Changes one part of the theme (`clrScheme`, `fontScheme` or the effects in `fmtScheme`)
    /// to what `spec` has, keeping the rest of the theme as it is.
    fn set_theme_part(&mut self, spec: &ThemeSpec, local: &str) -> Result<(), DocxError> {
        let xml_text = theme_xml(spec);
        let (name, _) =
            self.ensure_part(REL_THEME, "word/theme/theme1.xml", CT_THEME, &xml_text)?;
        let fresh = xml::parse(xml_text.as_bytes())?.root;
        self.edit_part_xml(&name, |root| {
            if !replace_theme_part(root, &fresh, local) {
                *root = fresh.clone();
            }
        })
    }

    /// Design > Colours: the 12 theme colours.
    ///
    /// # Errors
    /// The package cannot be edited.
    pub fn set_theme_colours(
        &mut self,
        name: &str,
        colours: &[String; 12],
        base: &ThemeSpec,
    ) -> Result<(), DocxError> {
        let spec = ThemeSpec {
            name: name.to_owned(),
            colours: colours.clone(),
            ..base.clone()
        };
        self.set_theme_part(&spec, "clrScheme")
    }

    /// Design > Fonts: the heading and body fonts of the theme.
    ///
    /// # Errors
    /// The package cannot be edited.
    pub fn set_theme_fonts(
        &mut self,
        name: &str,
        major: &str,
        minor: &str,
        base: &ThemeSpec,
    ) -> Result<(), DocxError> {
        let spec = ThemeSpec {
            name: name.to_owned(),
            major_font: major.to_owned(),
            minor_font: minor.to_owned(),
            ..base.clone()
        };
        self.set_theme_part(&spec, "fontScheme")
    }

    /// Design > Effects: the theme's effect styles (0 flat to 3 intense).
    ///
    /// # Errors
    /// The package cannot be edited.
    pub fn set_theme_effects(&mut self, level: u8, base: &ThemeSpec) -> Result<(), DocxError> {
        let spec = ThemeSpec {
            effects: level,
            ..base.clone()
        };
        self.set_theme_part(&spec, "fmtScheme")
    }

    // ---- styles ------------------------------------------------------------------------

    /// Redefines the paragraph and run properties of the listed styles (a style set). A style
    /// the document lacks is added, based on Normal.
    ///
    /// # Errors
    /// The document has no styles part or a definition is not valid XML.
    pub fn apply_style_overrides(&mut self, styles: &[StyleOverride]) -> Result<(), DocxError> {
        let Some(part) = self.styles().map(|s| s.part_name().clone()) else {
            return Err(DocxError::Xml("the document has no styles part".to_owned()));
        };
        let mut parsed = Vec::new();
        for s in styles {
            let ppr = if s.ppr.is_empty() {
                None
            } else {
                Some(parse_snippet(&s.ppr, "pPr")?)
            };
            let rpr = if s.rpr.is_empty() {
                None
            } else {
                Some(parse_snippet(&s.rpr, "rPr")?)
            };
            parsed.push((s, ppr, rpr));
        }
        self.edit_part_xml(&part, |root| {
            for (s, ppr, rpr) in parsed {
                apply_style(root, s, ppr, rpr);
            }
        })
    }

    /// Document-wide paragraph spacing (Design > Paragraph Spacing): space before and after in
    /// twips and the line spacing in 240ths of a line. Written to the document defaults; the
    /// spacing the Normal style itself carries is dropped so the default applies.
    ///
    /// # Errors
    /// The document has no styles part.
    pub fn set_default_paragraph_spacing(
        &mut self,
        before: u32,
        after: u32,
        line_240: u32,
    ) -> Result<(), DocxError> {
        let Some(part) = self.styles().map(|s| s.part_name().clone()) else {
            return Err(DocxError::Xml("the document has no styles part".to_owned()));
        };
        self.edit_part_xml(&part, |root| {
            let defaults = ensure_child(root, "docDefaults", 0);
            let pdef = ensure_child(defaults, "pPrDefault", 1);
            let ppr = ensure_child(pdef, "pPr", 0);
            crate::ppr::upsert_child(ppr, "spacing", |e| {
                e.head.attrs.clear();
                e.head.set_attr_w("before", &before.to_string());
                e.head.set_attr_w("after", &after.to_string());
                e.head.set_attr_w("line", &line_240.to_string());
                e.head.set_attr_w("lineRule", "auto");
            });
            // The Normal style (and the headings' line spacing) must not override the default.
            for c in &mut root.children {
                let RawXml::Element(style) = c else { continue };
                if !style.head.is_w("style") {
                    continue;
                }
                let id = style.head.attr_w("styleId").unwrap_or_default().to_owned();
                let Some(RawXml::Element(ppr)) = style
                    .children
                    .iter_mut()
                    .find(|c| matches!(c, RawXml::Element(e) if e.head.is_w("pPr")))
                else {
                    continue;
                };
                let Some(RawXml::Element(sp)) = ppr
                    .children
                    .iter_mut()
                    .find(|c| matches!(c, RawXml::Element(e) if e.head.is_w("spacing")))
                else {
                    continue;
                };
                if id == "Normal" {
                    sp.head.remove_attr_w("before");
                    sp.head.remove_attr_w("after");
                }
                sp.head.remove_attr_w("line");
                sp.head.remove_attr_w("lineRule");
                if sp.head.attrs.is_empty() {
                    ppr.children
                        .retain(|c| !matches!(c, RawXml::Element(e) if e.head.is_w("spacing")));
                }
            }
        })
    }

    // ---- page background ---------------------------------------------------------------

    /// Page colour (`w:background`); `None` is No Colour. The colour shows on screen only
    /// when the settings ask for it (`w:displayBackgroundShape`), so that is set with it.
    ///
    /// # Errors
    /// The settings part cannot be edited.
    pub fn set_page_colour(
        &mut self,
        colour: Option<(&str, Option<&str>)>,
    ) -> Result<(), DocxError> {
        let root = self.ext_root_mut();
        let head = root.head.sibling("background");
        root.children.retain(
            |c| !matches!(c, DocChild::Raw(RawXml::Element(e)) if e.head.is_w("background")),
        );
        if let Some((hex, theme)) = colour {
            let mut el = head.into_empty();
            el.head.set_attr_w("color", hex);
            if let Some(t) = theme {
                el.head.set_attr_w("themeColor", t);
            }
            root.children.insert(0, DocChild::Raw(RawXml::Element(el)));
        }
        self.edited();
        self.edit_settings(|s| {
            if colour.is_some() {
                upsert_ordered(s, &SETTINGS_BG, "displayBackgroundShape", |_| {});
            } else {
                s.children.retain(
                    |c| !matches!(c, RawXml::Element(e) if e.head.is_w("displayBackgroundShape")),
                );
            }
        })
    }

    /// The twelve theme colours (`RRGGBB`, in [`THEME_COLOUR_SLOTS`] order), when the document
    /// has a theme with a colour scheme.
    pub fn theme_colours(&self) -> Option<Vec<String>> {
        let main = self.main_part();
        let rels_name = open365_opc::rels_part_name(Some(main));
        let rels =
            open365_opc::Relationships::parse(self.package().part(rels_name.as_str())?.data())
                .ok()?;
        let rel = rels.iter().find(|r| r.rel_type == REL_THEME)?;
        let name = open365_opc::resolve_target(Some(main), &rel.target).ok()?;
        let parsed = xml::parse(self.package().part(name.as_str())?.data()).ok()?;
        let scheme = child_named(child_named(&parsed.root, "themeElements")?, "clrScheme")?;
        THEME_COLOUR_SLOTS
            .iter()
            .map(|slot| {
                let slot_el = child_named(scheme, slot)?;
                let c = slot_el.children.iter().find_map(RawXml::as_element)?;
                let attr = |n: &str| c.head.attrs.iter().find(|a| a.name == n);
                attr("lastClr")
                    .or_else(|| attr("val"))
                    .map(|a| a.value.clone())
            })
            .collect()
    }

    /// The page colour (`RRGGBB`), when the document has one.
    pub fn page_colour(&self) -> Option<String> {
        self.ext_root().children.iter().find_map(|c| match c {
            DocChild::Raw(RawXml::Element(e)) if e.head.is_w("background") => {
                e.head.attr_w("color").map(str::to_owned)
            }
            _ => None,
        })
    }

    /// Page borders for `sections` (`w:pgBorders`); `None` removes them.
    ///
    /// # Errors
    /// A section does not exist.
    pub fn set_page_borders(
        &mut self,
        sections: &[usize],
        borders: Option<&PageBorders>,
    ) -> Result<(), DocxError> {
        for &section in sections {
            self.edit_section(section, |sect| {
                sect.children
                    .retain(|c| !matches!(c, RawXml::Element(e) if e.head.is_w("pgBorders")));
                let Some(b) = borders else { return };
                crate::ext_layout::upsert_sect(sect, "pgBorders", |pg| {
                    pg.head
                        .set_attr_w("offsetFrom", if b.from_text { "text" } else { "page" });
                    if b.display != "allPages" {
                        pg.head.set_attr_w("display", &b.display);
                    }
                    pg.children.clear();
                    for (name, side) in ["top", "left", "bottom", "right"].iter().zip(&b.sides) {
                        let Some(side) = side else { continue };
                        let mut el = pg.sibling(name);
                        el.head.set_attr_w("val", &side.style);
                        el.head.set_attr_w("sz", &side.width_eighths.to_string());
                        el.head
                            .set_attr_w("space", &side.space_pt.min(31).to_string());
                        el.head.set_attr_w("color", &side.colour);
                        if b.shadow {
                            el.head.set_attr_w("shadow", "1");
                        }
                        pg.children.push(RawXml::Element(el));
                    }
                });
            })?;
        }
        Ok(())
    }

    // ---- watermark ---------------------------------------------------------------------

    /// Puts a text watermark in every header of every section (first-page and even-page headers
    /// only where the section uses them), replacing an earlier watermark; `None` removes them.
    ///
    /// # Errors
    /// The package cannot be edited.
    pub fn set_watermark(&mut self, mark: Option<&Watermark>) -> Result<(), DocxError> {
        // Remove what is there first, in every header part.
        let headers: Vec<_> = self
            .package()
            .parts()
            .map(|p| p.name().clone())
            .filter(|n| {
                let s = n.as_str();
                s.starts_with("word/header") && s.ends_with(".xml")
            })
            .collect();
        for name in headers {
            self.edit_part_xml(&name, strip_watermarks)?;
        }
        let Some(mark) = mark else { return Ok(()) };
        self.ensure_root_ns("r", NS_R);
        let even = self.settings().even_and_odd_headers;
        let sections = self.sections();
        for (index, section) in sections.iter().enumerate() {
            let mut kinds = vec!["default"];
            if section.title_pg {
                kinds.push("first");
            }
            if even {
                kinds.push("even");
            }
            for kind in kinds {
                let existing = self.header_part_of(index, kind);
                let name = match existing {
                    Some(name) => name,
                    None => self.add_header(index, kind)?,
                };
                let shape_id = format!("PowerPlusWaterMarkObject{}", index * 3 + 1);
                self.edit_part_xml(&name, |root| put_watermark(root, mark, &shape_id))?;
            }
        }
        Ok(())
    }

    /// The text of the watermark in the first header that has one.
    pub fn watermark_text(&self) -> Option<String> {
        self.package().parts().find_map(|p| {
            let s = p.name().as_str();
            if !(s.starts_with("word/header") && s.ends_with(".xml")) {
                return None;
            }
            let text = std::str::from_utf8(p.data()).ok()?;
            let at = text.find("PowerPlusWaterMarkObject")?;
            let rest = &text[at..];
            let start = rest.find("string=\"")? + 8;
            let end = rest[start..].find('"')?;
            Some(rest[start..start + end].to_owned())
        })
    }

    fn ensure_root_ns(&mut self, prefix: &str, uri: &str) {
        let root = self.ext_root_mut();
        let name = format!("xmlns:{prefix}");
        if !root.head.attrs.iter().any(|a| a.name == name) {
            root.head.attrs.push(plain_attr(&name, uri));
        }
    }

    /// The header part section `section` names for `kind` (`default`, `first`, `even`).
    fn header_part_of(&self, section: usize, kind: &str) -> Option<open365_opc::PartName> {
        let s = self.sections();
        let s = s.get(section)?;
        let want = match kind {
            "first" => crate::section::HeaderFooterType::First,
            "even" => crate::section::HeaderFooterType::Even,
            _ => crate::section::HeaderFooterType::Default,
        };
        let r = s
            .header_footer_refs
            .iter()
            .find(|r| r.kind == want && !r.footer)?;
        let rels_name = open365_opc::rels_part_name(Some(self.main_part()));
        let rels =
            open365_opc::Relationships::parse(self.package().part(rels_name.as_str())?.data())
                .ok()?;
        let target = rels.get(&r.rel_id)?.target.clone();
        open365_opc::resolve_target(Some(self.main_part()), &target).ok()
    }

    fn add_header(
        &mut self,
        section: usize,
        kind: &str,
    ) -> Result<open365_opc::PartName, DocxError> {
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:hdr xmlns:w=\"{NS_W}\" xmlns:r=\"{NS_R}\"><w:p/></w:hdr>"
        );
        let (name, id) =
            self.add_numbered_part(REL_HEADER, "header", CT_HEADER, body.into_bytes())?;
        let rid_attr = Attr {
            name: "r:id".to_owned(),
            ns: Some(NS_R.into()),
            value: id,
        };
        self.edit_section(section, |sect| {
            let mut el = sect.sibling("headerReference");
            el.head.set_attr_w("type", kind);
            el.head.attrs.push(rid_attr);
            let at = sect
                .children
                .iter()
                .position(|c| matches!(c, RawXml::Element(e) if !e.head.is_w("headerReference")))
                .unwrap_or(sect.children.len());
            sect.children.insert(at, RawXml::Element(el));
        })?;
        Ok(name)
    }
}

const SETTINGS_BG: [&str; 7] = [
    "view",
    "zoom",
    "removePersonalInformation",
    "removeDateAndTime",
    "doNotDisplayPageBoundaries",
    "displayBackgroundShape",
    "printPostScriptOverText",
];

/// Finds `w:local` among `parent`'s children or adds it (before the first child whose local
/// name is in `before`, or `at` positions from the start when none).
fn ensure_child<'a>(parent: &'a mut XmlElement, local: &str, at: usize) -> &'a mut XmlElement {
    let found = parent
        .children
        .iter()
        .position(|c| matches!(c, RawXml::Element(e) if e.head.is_w(local)));
    let i = match found {
        Some(i) => i,
        None => {
            let el = parent.sibling(local);
            let at = at.min(parent.children.len());
            parent.children.insert(at, RawXml::Element(el));
            at
        }
    };
    match &mut parent.children[i] {
        RawXml::Element(e) => e,
        _ => unreachable!("an element was just found or inserted"),
    }
}

/// Sets `pPr` and `rPr` of style `s` in a `w:styles` root.
fn apply_style(
    root: &mut XmlElement,
    s: &StyleOverride,
    ppr: Option<XmlElement>,
    rpr: Option<XmlElement>,
) {
    let found = root.children.iter().position(
        |c| matches!(c, RawXml::Element(e) if e.head.is_w("style") && e.head.attr_w("styleId") == Some(s.id.as_str())),
    );
    let i = match found {
        Some(i) => i,
        None => {
            let mut style = root.sibling("style");
            style.head.set_attr_w("type", "paragraph");
            style.head.set_attr_w("styleId", &s.id);
            let mut name = style.sibling("name");
            name.head.set_attr_w("val", &s.name);
            style.children.push(RawXml::Element(name));
            if s.id != "Normal" {
                let mut based = style.sibling("basedOn");
                based.head.set_attr_w("val", "Normal");
                style.children.push(RawXml::Element(based));
                let mut next = style.sibling("next");
                next.head.set_attr_w("val", "Normal");
                style.children.push(RawXml::Element(next));
                style
                    .children
                    .push(RawXml::Element(style.sibling("qFormat")));
            }
            root.children.push(RawXml::Element(style));
            root.children.len() - 1
        }
    };
    let RawXml::Element(style) = &mut root.children[i] else {
        return;
    };
    for (local, new, later) in [
        (
            "pPr",
            ppr,
            &["rPr", "tblPr", "trPr", "tcPr", "tblStylePr"][..],
        ),
        ("rPr", rpr, &["tblPr", "trPr", "tcPr", "tblStylePr"][..]),
    ] {
        style
            .children
            .retain(|c| !matches!(c, RawXml::Element(e) if e.head.is_w(local)));
        let Some(mut new) = new else { continue };
        new.head = style.head.sibling(local);
        let at = style
            .children
            .iter()
            .position(|c| matches!(c, RawXml::Element(e) if later.contains(&e.head.local_name())))
            .unwrap_or(style.children.len());
        style.children.insert(at, RawXml::Element(new));
    }
}

fn is_watermark_run(r: &XmlElement) -> bool {
    fn has_mark(e: &XmlElement) -> bool {
        e.head
            .attrs
            .iter()
            .any(|a| a.local_name() == "id" && a.value.starts_with("PowerPlusWaterMarkObject"))
            || e.children
                .iter()
                .filter_map(RawXml::as_element)
                .any(has_mark)
    }
    r.head.is_w("r") && has_mark(r)
}

fn strip_watermarks(e: &mut XmlElement) {
    e.children
        .retain(|c| !matches!(c, RawXml::Element(x) if is_watermark_run(x)));
    for c in &mut e.children {
        if let RawXml::Element(x) = c {
            strip_watermarks(x);
        }
    }
}

fn put_watermark(root: &mut XmlElement, mark: &Watermark, shape_id: &str) {
    for (prefix, uri) in [
        ("xmlns:v", "urn:schemas-microsoft-com:vml"),
        ("xmlns:o", "urn:schemas-microsoft-com:office:office"),
        ("xmlns:r", NS_R),
    ] {
        if !root.head.attrs.iter().any(|a| a.name == prefix) {
            root.head.attrs.push(plain_attr(prefix, uri));
        }
    }
    // Auto size fits the text across the page: width 468 pt (6.5 in) shared by the characters.
    let chars = mark.text.chars().count().max(1) as f64;
    let (width, height, size) = match mark.size_pt {
        Some(sz) => (sz * chars * 0.6, sz * 1.2, sz),
        None => {
            let w = 468.0;
            let h = (w / chars * 1.6).clamp(20.0, 234.0);
            (w, h, h)
        }
    };
    let rotation = if mark.diagonal { "rotation:315;" } else { "" };
    let opacity = if mark.semi_transparent {
        "<v:fill opacity=\".5\"/>"
    } else {
        ""
    };
    let text = format!(
        "<w:r xmlns:w=\"{NS_W}\" xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\"><w:rPr><w:noProof/></w:rPr><w:pict><v:shape id=\"{shape_id}\" o:spid=\"_x0000_s2049\" type=\"#_x0000_t136\" style=\"position:absolute;margin-left:0;margin-top:0;width:{width:.1}pt;height:{height:.1}pt;{rotation}z-index:-251658752;mso-position-horizontal:center;mso-position-horizontal-relative:margin;mso-position-vertical:center;mso-position-vertical-relative:margin\" o:allowincell=\"f\" fillcolor=\"#{colour}\" stroked=\"f\">{opacity}<v:textpath style=\"font-family:&quot;{font}&quot;;font-size:{size:.0}pt\" string=\"{string}\"/></v:shape></w:pict></w:r>",
        colour = mark.colour,
        font = esc(&mark.font),
        string = esc(&mark.text),
    );
    let Ok(parsed) = xml::parse(text.as_bytes()) else {
        return;
    };
    let mut run = parsed.root;
    run.head.attrs.retain(|a| !a.name.starts_with("xmlns"));
    // The first paragraph of the header, created when the header has none.
    let p = ensure_child(root, "p", 0);
    let at = p
        .children
        .iter()
        .position(|c| !matches!(c, RawXml::Element(e) if e.head.is_w("pPr")))
        .unwrap_or(p.children.len());
    p.children.insert(at, RawXml::Element(run));
}
