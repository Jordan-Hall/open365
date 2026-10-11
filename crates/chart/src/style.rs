//! The two optional parts that sit next to a chart part: `xl/charts/styleN.xml`
//! (`cs:chartStyle`, the style gallery entry) and `xl/charts/colorsN.xml` (`cs:colorStyle`,
//! the colour set). Both are kept as lossless trees; the typed fields are the parts a caller
//! needs to choose series colours.

use open365_xlsx::{ParsedXml, XmlElement, parse_xml};

use crate::error::ChartError;
use crate::model::Rgb;

const STYLE_NS: &str = "http://schemas.microsoft.com/office/drawing/2012/chartStyle";
const DML_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

fn is_style_ns(e: &XmlElement) -> bool {
    e.head.ns.as_deref() == Some(STYLE_NS)
}

/// One colour of a colour style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleColor {
    /// `a:srgbClr`.
    Rgb(Rgb),
    /// `a:schemeClr`, for example `accent1`. Resolves against the theme.
    Scheme(String),
}

/// `cs:colorStyle`.
#[derive(Debug, Clone)]
pub struct ColorStyle {
    /// `meth`: `cycle` repeats the colours, `withinLinear` and `acrossLinear` and
    /// `withinLinearReversed` and `acrossLinearReversed` derive a gradient.
    pub method: String,
    pub id: u32,
    pub colors: Vec<StyleColor>,
    /// Number of `cs:variation` entries (brightness steps used after the colours run out).
    pub variations: usize,
    xml: ParsedXml,
}

impl ColorStyle {
    pub fn parse(bytes: &[u8]) -> Result<Self, ChartError> {
        let xml = parse_xml(bytes).map_err(|e| ChartError::Xml(e.to_string()))?;
        let root = &xml.root;
        if !is_style_ns(root) || root.head.local_name() != "colorStyle" {
            return Err(ChartError::NotAChart);
        }
        let colors = root
            .elements()
            .filter(|c| c.head.ns.as_deref() == Some(DML_NS))
            .filter_map(|c| match c.head.local_name() {
                "srgbClr" => {
                    let h = c.head.attr("val")?;
                    let n = u32::from_str_radix(h.trim(), 16).ok()?;
                    Some(StyleColor::Rgb(Rgb(
                        (n >> 16) as u8,
                        (n >> 8) as u8,
                        n as u8,
                    )))
                }
                "schemeClr" => Some(StyleColor::Scheme(c.head.attr("val")?.to_owned())),
                _ => None,
            })
            .collect();
        Ok(Self {
            method: root.head.attr("meth").unwrap_or("cycle").to_owned(),
            id: root
                .head
                .attr("id")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            colors,
            variations: root
                .elements()
                .filter(|c| is_style_ns(c) && c.head.local_name() == "variation")
                .count(),
            xml,
        })
    }

    /// The colour of series `index`: the colour list cycles; the `n`-th lap gets a lighter or
    /// darker variation when the part has any. Scheme colours need the theme, so the caller
    /// resolves them with `scheme`.
    pub fn series_color(&self, index: usize, scheme: impl Fn(&str) -> Option<Rgb>) -> Option<Rgb> {
        if self.colors.is_empty() {
            return None;
        }
        let base = match &self.colors[index % self.colors.len()] {
            StyleColor::Rgb(c) => *c,
            StyleColor::Scheme(name) => scheme(name)?,
        };
        let lap = index / self.colors.len();
        // Excel darkens the second lap and lightens the third; the exact lumMod/lumOff steps
        // live in the `cs:variation` children and stay in the tree.
        let f = match lap {
            0 => 1.0,
            1 => 0.6,
            _ => 1.4,
        };
        let g = |v: u8| (f64::from(v) * f).round().clamp(0.0, 255.0) as u8;
        Some(Rgb(g(base.0), g(base.1), g(base.2)))
    }

    pub fn xml(&self) -> &ParsedXml {
        &self.xml
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.xml.to_bytes()
    }
}

/// `cs:chartStyle`.
#[derive(Debug, Clone)]
pub struct ChartStyle {
    /// The gallery style number (`id`, for example 201 for the default column style).
    pub id: u32,
    /// Names of the element classes the style defines (`axisTitle`, `dataPoint`, ...).
    pub entries: Vec<String>,
    xml: ParsedXml,
}

impl ChartStyle {
    pub fn parse(bytes: &[u8]) -> Result<Self, ChartError> {
        let xml = parse_xml(bytes).map_err(|e| ChartError::Xml(e.to_string()))?;
        let root = &xml.root;
        if !is_style_ns(root) || root.head.local_name() != "chartStyle" {
            return Err(ChartError::NotAChart);
        }
        let entries = root
            .elements()
            .filter(|c| is_style_ns(c))
            .map(|c| c.head.local_name().to_owned())
            .collect();
        Ok(Self {
            id: root
                .head
                .attr("id")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            entries,
            xml,
        })
    }

    pub fn xml(&self) -> &ParsedXml {
        &self.xml
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.xml.to_bytes()
    }
}
