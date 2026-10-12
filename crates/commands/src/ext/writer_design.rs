//! Writer Design tab operations: themes, theme colours, fonts and effects, style sets, paragraph
//! spacing, page colour, page borders and watermarks. Each runs as one undo step.
//!
//! The galleries (themes, colour sets, font sets, style sets, spacing presets) are original
//! designs of this project; names are our own.

use open365_docx::{Document, PageBorderSide, PageBorders, StyleOverride, ThemeSpec, Watermark};
use serde::{Deserialize, Serialize};

use crate::EffectClass;
use crate::error::CommandError;

/// A theme as the command carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeData {
    pub name: String,
    /// Twelve `RRGGBB` values: dark 1, light 1, dark 2, light 2, accents 1 to 6, hyperlink,
    /// followed hyperlink.
    pub colours: Vec<String>,
    pub major_font: String,
    pub minor_font: String,
    /// 0 flat, 1 subtle, 2 moderate, 3 intense.
    pub effects: u8,
}

/// One side of a page border.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideData {
    pub style: String,
    pub width_eighths: u32,
    pub colour: String,
    pub space_pt: u32,
}

/// Page borders: top, left, bottom and right.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BordersData {
    pub top: Option<SideData>,
    pub left: Option<SideData>,
    pub bottom: Option<SideData>,
    pub right: Option<SideData>,
    pub from_text: bool,
    pub shadow: bool,
    /// `allPages`, `firstPage` or `notFirstPage`.
    pub display: String,
}

/// A text watermark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatermarkData {
    pub text: String,
    pub font: String,
    /// `None` is Auto.
    pub size_pt: Option<f64>,
    pub colour: String,
    pub semi_transparent: bool,
    pub diagonal: bool,
}

/// One style a style set redefines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleData {
    pub id: String,
    pub name: String,
    pub ppr: String,
    pub rpr: String,
}

/// What a [`crate::Command::DocxDesign`] does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DesignOp {
    /// Themes gallery: colours, fonts and effects together.
    Theme { theme: ThemeData },
    /// Colours gallery. `base` supplies the rest of a theme the document may lack.
    ThemeColours { name: String, colours: Vec<String> },
    /// Fonts gallery.
    ThemeFonts {
        name: String,
        major_font: String,
        minor_font: String,
    },
    /// Effects gallery, 0 flat to 3 intense.
    ThemeEffects { level: u8 },
    /// Style Sets gallery.
    StyleSet {
        name: String,
        styles: Vec<StyleData>,
    },
    /// Paragraph Spacing gallery: points before and after, and the line spacing as a multiple.
    ParagraphSpacing {
        before_pt: f64,
        after_pt: f64,
        line: f64,
    },
    /// Page Colour: `None` is No Colour; `theme` names the theme colour the value came from.
    PageColour {
        colour: Option<String>,
        theme: Option<String>,
    },
    /// Page Borders for the sections; `None` removes them.
    PageBorders {
        sections: Vec<usize>,
        borders: Option<BordersData>,
    },
    /// Watermark; `None` removes it.
    Watermark { mark: Option<WatermarkData> },
}

impl DesignOp {
    pub fn effect(&self) -> EffectClass {
        EffectClass::Edit
    }
}

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::Format(msg.into())
}

fn hex(value: &str) -> Result<String, CommandError> {
    let v = value.trim().trim_start_matches('#');
    if v.len() == 6 && v.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(v.to_ascii_uppercase())
    } else if v.eq_ignore_ascii_case("auto") {
        Ok("auto".to_owned())
    } else {
        Err(bad(format!("{value:?} is not an RRGGBB colour")))
    }
}

fn colours12(list: &[String]) -> Result<[String; 12], CommandError> {
    let v: Vec<String> = list.iter().map(|c| hex(c)).collect::<Result<_, _>>()?;
    <[String; 12]>::try_from(v)
        .map_err(|v| bad(format!("a colour scheme has 12 colours, not {}", v.len())))
}

fn spec_of(t: &ThemeData) -> Result<ThemeSpec, CommandError> {
    Ok(ThemeSpec {
        name: t.name.clone(),
        colours: colours12(&t.colours)?,
        major_font: t.major_font.clone(),
        minor_font: t.minor_font.clone(),
        effects: t.effects.min(3),
    })
}

/// The theme a document without one gets before a single part of it is replaced.
#[must_use]
pub fn base_theme() -> ThemeData {
    themes().remove(0)
}

/// Runs `op` on `doc`.
pub fn apply(doc: &mut Document, op: &DesignOp) -> Result<(), CommandError> {
    match op {
        DesignOp::Theme { theme } => doc.set_theme(&spec_of(theme)?)?,
        DesignOp::ThemeColours { name, colours } => {
            doc.set_theme_colours(name, &colours12(colours)?, &spec_of(&base_theme())?)?;
        }
        DesignOp::ThemeFonts {
            name,
            major_font,
            minor_font,
        } => doc.set_theme_fonts(name, major_font, minor_font, &spec_of(&base_theme())?)?,
        DesignOp::ThemeEffects { level } => {
            doc.set_theme_effects((*level).min(3), &spec_of(&base_theme())?)?;
        }
        DesignOp::StyleSet { styles, .. } => {
            let list: Vec<StyleOverride> = styles
                .iter()
                .map(|s| StyleOverride {
                    id: s.id.clone(),
                    name: s.name.clone(),
                    ppr: s.ppr.clone(),
                    rpr: s.rpr.clone(),
                })
                .collect();
            doc.apply_style_overrides(&list)?;
        }
        DesignOp::ParagraphSpacing {
            before_pt,
            after_pt,
            line,
        } => {
            if !(before_pt.is_finite() && after_pt.is_finite() && line.is_finite())
                || *before_pt < 0.0
                || *after_pt < 0.0
                || !(0.06..=13.2).contains(line)
            {
                return Err(bad("paragraph spacing out of range"));
            }
            doc.set_default_paragraph_spacing(
                (before_pt * 20.0).round() as u32,
                (after_pt * 20.0).round() as u32,
                (line * 240.0).round() as u32,
            )?;
        }
        DesignOp::PageColour { colour, theme } => match colour {
            Some(c) => doc.set_page_colour(Some((&hex(c)?, theme.as_deref())))?,
            None => doc.set_page_colour(None)?,
        },
        DesignOp::PageBorders { sections, borders } => {
            if sections.is_empty() {
                return Err(bad("no section to change"));
            }
            let b = borders.as_ref().map(borders_of).transpose()?;
            doc.set_page_borders(sections, b.as_ref())?;
        }
        DesignOp::Watermark { mark } => {
            let m = mark.as_ref().map(watermark_of).transpose()?;
            doc.set_watermark(m.as_ref())?;
        }
    }
    Ok(())
}

fn side_of(s: &Option<SideData>) -> Result<Option<PageBorderSide>, CommandError> {
    s.as_ref()
        .map(|s| {
            if s.style.is_empty() || !s.style.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err(bad(format!("{:?} is not a border style", s.style)));
            }
            Ok(PageBorderSide {
                style: s.style.clone(),
                width_eighths: s.width_eighths.clamp(2, 255),
                colour: hex(&s.colour)?,
                space_pt: s.space_pt.min(31),
            })
        })
        .transpose()
}

fn borders_of(b: &BordersData) -> Result<PageBorders, CommandError> {
    if !matches!(
        b.display.as_str(),
        "allPages" | "firstPage" | "notFirstPage"
    ) {
        return Err(bad(format!("{:?} is not a page border display", b.display)));
    }
    Ok(PageBorders {
        sides: [
            side_of(&b.top)?,
            side_of(&b.left)?,
            side_of(&b.bottom)?,
            side_of(&b.right)?,
        ],
        from_text: b.from_text,
        shadow: b.shadow,
        display: b.display.clone(),
    })
}

fn watermark_of(m: &WatermarkData) -> Result<Watermark, CommandError> {
    if m.text.chars().any(char::is_control) || m.text.is_empty() {
        return Err(bad("a watermark needs text on one line"));
    }
    Ok(Watermark {
        text: m.text.clone(),
        font: m.font.clone(),
        size_pt: m
            .size_pt
            .filter(|s| s.is_finite() && (1.0..=1638.0).contains(s)),
        colour: hex(&m.colour)?,
        semi_transparent: m.semi_transparent,
        diagonal: m.diagonal,
    })
}

// ---- galleries ---------------------------------------------------------------------------

/// A colour scheme: name and twelve colours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColourSet {
    pub id: &'static str,
    pub name: &'static str,
    pub colours: [&'static str; 12],
}

/// The Colours gallery.
#[must_use]
pub fn colour_sets() -> &'static [ColourSet] {
    &[
        ColourSet {
            id: "open365",
            name: "Open365",
            colours: [
                "000000", "FFFFFF", "44546A", "E7E6E6", "4472C4", "ED7D31", "A5A5A5", "FFC000",
                "5B9BD5", "70AD47", "0563C1", "954F72",
            ],
        },
        ColourSet {
            id: "slate",
            name: "Slate",
            colours: [
                "000000", "FFFFFF", "2F3E4E", "E3E8EE", "3B6EA8", "E07A5F", "81B29A", "F2CC8F",
                "6C757D", "8E5572", "1D5FA8", "7D4E7D",
            ],
        },
        ColourSet {
            id: "forest",
            name: "Forest",
            colours: [
                "000000", "FFFFFF", "2D4A33", "E9EFE6", "3E7C4E", "A3B18A", "6B8F71", "DAA520",
                "588157", "8C5A3C", "2A6F4D", "7A5C61",
            ],
        },
        ColourSet {
            id: "sunrise",
            name: "Sunrise",
            colours: [
                "000000", "FFFFFF", "5A3E36", "F6EEE6", "E4572E", "F3A712", "A8C686", "669BBC",
                "29335C", "9B5DE5", "C2410C", "7C3AED",
            ],
        },
        ColourSet {
            id: "ocean",
            name: "Ocean",
            colours: [
                "000000", "FFFFFF", "0B3954", "E6F0F5", "087E8B", "3E8EDE", "BFD7EA", "FF5A5F",
                "C81D25", "5E7CE2", "0066CC", "6A4C93",
            ],
        },
        ColourSet {
            id: "violet",
            name: "Violet",
            colours: [
                "000000", "FFFFFF", "3A2E5C", "ECE8F4", "7E57C2", "EC407A", "26A69A", "FFA726",
                "5C6BC0", "9CCC65", "5E35B1", "8E24AA",
            ],
        },
        ColourSet {
            id: "mono",
            name: "Greyscale",
            colours: [
                "000000", "FFFFFF", "3F3F3F", "EDEDED", "595959", "7F7F7F", "A6A6A6", "BFBFBF",
                "404040", "262626", "404040", "7F7F7F",
            ],
        },
        ColourSet {
            id: "terracotta",
            name: "Terracotta",
            colours: [
                "000000", "FFFFFF", "4A2F27", "F5EBE0", "C1440E", "D4A373", "8D6B52", "E9C46A",
                "6B705C", "A44A3F", "9C3D10", "7F5539",
            ],
        },
    ]
}

/// A heading and body font pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontSet {
    pub id: &'static str,
    pub name: &'static str,
    pub major: &'static str,
    pub minor: &'static str,
}

/// The Fonts gallery.
#[must_use]
pub fn font_sets() -> &'static [FontSet] {
    &[
        FontSet {
            id: "open365",
            name: "Open365",
            major: "Calibri Light",
            minor: "Calibri",
        },
        FontSet {
            id: "classic",
            name: "Classic",
            major: "Times New Roman",
            minor: "Times New Roman",
        },
        FontSet {
            id: "modern",
            name: "Modern",
            major: "Arial",
            minor: "Arial",
        },
        FontSet {
            id: "editorial",
            name: "Editorial",
            major: "Georgia",
            minor: "Georgia",
        },
        FontSet {
            id: "headline",
            name: "Headline",
            major: "Arial Black",
            minor: "Verdana",
        },
        FontSet {
            id: "serif-sans",
            name: "Serif and Sans",
            major: "Cambria",
            minor: "Calibri",
        },
        FontSet {
            id: "typewriter",
            name: "Typewriter",
            major: "Courier New",
            minor: "Courier New",
        },
        FontSet {
            id: "humanist",
            name: "Humanist",
            major: "Trebuchet MS",
            minor: "Trebuchet MS",
        },
    ]
}

/// A Themes gallery entry: a colour set, a font set and an effect level.
#[must_use]
pub fn themes() -> Vec<ThemeData> {
    let mk = |c: usize, f: usize, e: u8, name: &str| {
        let (c, f) = (&colour_sets()[c], &font_sets()[f]);
        ThemeData {
            name: name.to_owned(),
            colours: c.colours.iter().map(|s| (*s).to_owned()).collect(),
            major_font: f.major.to_owned(),
            minor_font: f.minor.to_owned(),
            effects: e,
        }
    };
    vec![
        mk(0, 0, 1, "Open365"),
        mk(1, 2, 0, "Harbour"),
        mk(2, 3, 1, "Meadow"),
        mk(3, 7, 2, "Sunrise"),
        mk(4, 5, 1, "Tidewater"),
        mk(5, 2, 2, "Amethyst"),
        mk(6, 1, 0, "Newsprint"),
        mk(7, 3, 3, "Adobe"),
    ]
}

/// A Paragraph Spacing gallery entry: points before, points after, line multiple.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpacingPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub before: f64,
    pub after: f64,
    pub line: f64,
}

/// The Paragraph Spacing gallery.
#[must_use]
pub fn spacing_presets() -> &'static [SpacingPreset] {
    &[
        SpacingPreset {
            id: "no-paragraph-space",
            label: "No Paragraph Space",
            before: 0.0,
            after: 0.0,
            line: 1.0,
        },
        SpacingPreset {
            id: "compact",
            label: "Compact",
            before: 0.0,
            after: 4.0,
            line: 1.0,
        },
        SpacingPreset {
            id: "tight",
            label: "Tight",
            before: 0.0,
            after: 6.0,
            line: 1.0,
        },
        SpacingPreset {
            id: "open",
            label: "Open",
            before: 0.0,
            after: 10.0,
            line: 1.15,
        },
        SpacingPreset {
            id: "relaxed",
            label: "Relaxed",
            before: 0.0,
            after: 6.0,
            line: 1.5,
        },
        SpacingPreset {
            id: "double",
            label: "Double",
            before: 0.0,
            after: 8.0,
            line: 2.0,
        },
    ]
}

/// The Effects gallery names, by level.
#[must_use]
pub fn effect_names() -> [&'static str; 4] {
    ["Flat", "Subtle", "Moderate", "Intense"]
}

/// How a style set looks.
struct SetLook {
    id: &'static str,
    /// Sizes in half points: title, heading 1 to 3.
    sizes: [u32; 4],
    /// Heading colour: a theme colour name, or `None` for automatic (black).
    colour: Option<&'static str>,
    bold: bool,
    caps: bool,
    centred: bool,
    /// Rule under Title and Heading 1.
    rule: bool,
    /// Body paragraph spacing: after in twips, line in 240ths.
    after: u32,
    line: u32,
    /// Heading fonts: the theme's heading font (else the body font).
    major: bool,
}

const LOOKS: [SetLook; 6] = [
    SetLook {
        id: "default",
        sizes: [56, 32, 26, 24],
        colour: Some("accent1"),
        bold: false,
        caps: false,
        centred: false,
        rule: false,
        after: 160,
        line: 259,
        major: true,
    },
    SetLook {
        id: "classic",
        sizes: [48, 32, 28, 24],
        colour: None,
        bold: true,
        caps: false,
        centred: false,
        rule: false,
        after: 120,
        line: 240,
        major: false,
    },
    SetLook {
        id: "centred",
        sizes: [52, 32, 26, 24],
        colour: Some("text2"),
        bold: false,
        caps: true,
        centred: true,
        rule: false,
        after: 160,
        line: 259,
        major: true,
    },
    SetLook {
        id: "lines",
        sizes: [52, 30, 26, 24],
        colour: Some("accent1"),
        bold: true,
        caps: false,
        centred: false,
        rule: true,
        after: 160,
        line: 259,
        major: true,
    },
    SetLook {
        id: "minimal",
        sizes: [40, 28, 24, 22],
        colour: None,
        bold: true,
        caps: false,
        centred: false,
        rule: false,
        after: 0,
        line: 240,
        major: false,
    },
    SetLook {
        id: "relaxed",
        sizes: [60, 36, 28, 24],
        colour: Some("accent2"),
        bold: false,
        caps: false,
        centred: false,
        rule: false,
        after: 200,
        line: 300,
        major: true,
    },
];

/// The Style Sets gallery: id and label.
#[must_use]
pub fn style_set_names() -> [(&'static str, &'static str); 6] {
    [
        ("default", "Default"),
        ("classic", "Classic"),
        ("centred", "Centred"),
        ("lines", "Lines"),
        ("minimal", "Minimal"),
        ("relaxed", "Relaxed"),
    ]
}

/// The styles of style set `id`, `None` for an unknown set.
#[must_use]
pub fn style_set(id: &str) -> Option<Vec<StyleData>> {
    let look = LOOKS.iter().find(|l| l.id == id)?;
    let font = if look.major {
        r#"<w:rFonts w:asciiTheme="majorHAnsi" w:hAnsiTheme="majorHAnsi" w:eastAsiaTheme="majorEastAsia" w:cstheme="majorBidi"/>"#
    } else {
        r#"<w:rFonts w:asciiTheme="minorHAnsi" w:hAnsiTheme="minorHAnsi" w:eastAsiaTheme="minorEastAsia" w:cstheme="minorBidi"/>"#
    };
    let colour = look
        .colour
        .map(|c| format!(r#"<w:color w:val="auto" w:themeColor="{c}"/>"#))
        .unwrap_or_default();
    let weight = if look.bold { "<w:b/><w:bCs/>" } else { "" };
    let caps = if look.caps { "<w:caps/>" } else { "" };
    let jc = if look.centred {
        r#"<w:jc w:val="center"/>"#
    } else {
        ""
    };
    let rule = |on: bool| {
        if on {
            r#"<w:pBdr><w:bottom w:val="single" w:sz="8" w:space="4" w:color="auto" w:themeColor="accent1"/></w:pBdr>"#
        } else {
            ""
        }
    };
    let head = |id: &str,
                name: &str,
                size: u32,
                before: u32,
                after: u32,
                outline: Option<u32>,
                ruled: bool| {
        let outline = outline
            .map(|o| format!(r#"<w:outlineLvl w:val="{o}"/>"#))
            .unwrap_or_default();
        StyleData {
            id: id.to_owned(),
            name: name.to_owned(),
            ppr: format!(
                r#"<w:keepNext/><w:keepLines/>{}<w:spacing w:before="{before}" w:after="{after}" w:line="240" w:lineRule="auto"/>{jc}{outline}"#,
                rule(ruled)
            ),
            rpr: format!(
                r#"{font}{weight}{caps}{colour}<w:sz w:val="{size}"/><w:szCs w:val="{size}"/>"#
            ),
        }
    };
    let [t, h1, h2, h3] = look.sizes;
    Some(vec![
        StyleData {
            id: "Normal".to_owned(),
            name: "Normal".to_owned(),
            ppr: format!(
                r#"<w:spacing w:after="{}" w:line="{}" w:lineRule="auto"/>"#,
                look.after, look.line
            ),
            rpr: String::new(),
        },
        head("Title", "Title", t, 0, 120, None, look.rule),
        head("Heading1", "heading 1", h1, 360, 80, Some(0), look.rule),
        head("Heading2", "heading 2", h2, 160, 80, Some(1), false),
        head("Heading3", "heading 3", h3, 160, 80, Some(2), false),
    ])
}

/// Page colours of the Page Colour menu: Standard Colours.
#[must_use]
pub fn standard_colours() -> Vec<(&'static str, &'static str)> {
    vec![
        ("dark-red", "C00000"),
        ("red", "FF0000"),
        ("orange", "FFC000"),
        ("yellow", "FFFF00"),
        ("light-green", "92D050"),
        ("green", "00B050"),
        ("light-blue", "00B0F0"),
        ("blue", "0070C0"),
        ("dark-blue", "002060"),
        ("purple", "7030A0"),
    ]
}
