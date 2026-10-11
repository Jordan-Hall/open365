//! Custom table styles: `tableStyles` and `dxfs` of the style sheet.

use std::collections::HashMap;

use open365_xlsx::{Rgb, XmlElement};

use super::{EdgeDef, ElementDef, StyleColor, StyleDef, element};

/// The custom table styles of a workbook (`xl/styles.xml` `tableStyles`), by name.
#[derive(Debug, Clone, Default)]
pub struct CustomStyles {
    styles: HashMap<String, StyleDef>,
}

/// `tableStyleElement/@type` in Excel's element order.
const ELEMENT_TYPES: [&str; element::COUNT] = [
    "wholeTable",
    "headerRow",
    "totalRow",
    "firstColumn",
    "lastColumn",
    "firstRowStripe",
    "secondRowStripe",
    "firstColumnStripe",
    "secondColumnStripe",
    "firstHeaderCell",
    "lastHeaderCell",
    "firstTotalCell",
    "lastTotalCell",
    "firstSubtotalColumn",
    "secondSubtotalColumn",
    "thirdSubtotalColumn",
    "firstSubtotalRow",
    "secondSubtotalRow",
    "thirdSubtotalRow",
    "blankRow",
    "firstColumnSubheading",
    "secondColumnSubheading",
    "thirdColumnSubheading",
    "firstRowSubheading",
    "secondRowSubheading",
    "thirdRowSubheading",
    "pageFieldLabels",
    "pageFieldValues",
];

fn dxf_color(e: &XmlElement) -> Option<StyleColor> {
    if let Some(rgb) = e.head.attr("rgb") {
        let v = u32::from_str_radix(rgb.trim(), 16).ok()?;
        return Some(StyleColor::Rgb(Rgb::new(
            (v >> 16) as u8,
            (v >> 8) as u8,
            v as u8,
        )));
    }
    let theme = e.head.attr("theme")?.trim().parse().ok()?;
    let tint = e
        .head
        .attr("tint")
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0.0);
    Some(StyleColor::Theme(theme, tint))
}

/// (`XlLineStyle`, `XlBorderWeight`) Excel reports for a dxf border style.
fn border_style(style: &str) -> (i32, i32) {
    match style {
        "medium" => (1, -4138),
        "thick" => (1, 4),
        "hair" => (1, 1),
        "dotted" => (-4118, 2),
        "dashed" => (-4115, 2),
        "double" => (-4119, 4),
        _ => (1, 2),
    }
}

fn flag_element(e: &XmlElement) -> bool {
    !matches!(e.head.attr("val").map(str::trim), Some("0" | "false"))
}

fn dxf_element(dxf: &XmlElement, stripe: usize) -> ElementDef {
    let mut def = ElementDef {
        stripe,
        ..ElementDef::default()
    };
    if let Some(font) = dxf.child_main("font") {
        if let Some(b) = font.child_main("b") {
            def.bold = Some(flag_element(b));
        }
        if let Some(i) = font.child_main("i") {
            def.italic = Some(flag_element(i));
        }
        def.font = font.child_main("color").and_then(dxf_color);
    }
    if let Some(pf) = dxf
        .child_main("fill")
        .and_then(|f| f.child_main("patternFill"))
        && pf.head.attr("patternType") != Some("none")
    {
        def.fill = pf
            .child_main("bgColor")
            .and_then(dxf_color)
            .or_else(|| pf.child_main("fgColor").and_then(dxf_color));
    }
    if let Some(border) = dxf.child_main("border") {
        for (k, name) in ["left", "top", "bottom", "right", "vertical", "horizontal"]
            .iter()
            .enumerate()
        {
            let Some(b) = border.child_main(name) else {
                continue;
            };
            let Some(style) = b.head.attr("style").filter(|s| *s != "none") else {
                continue;
            };
            let (line, weight) = border_style(style);
            let color = b
                .child_main("color")
                .and_then(dxf_color)
                .unwrap_or(StyleColor::Rgb(Rgb::BLACK));
            def.borders[k] = Some(EdgeDef {
                line,
                weight,
                color,
            });
        }
    }
    def
}

impl CustomStyles {
    /// Reads `tableStyles` (pivot styles only) and the `dxfs` they refer to from a style sheet.
    pub fn parse(styles_xml: &[u8]) -> Self {
        let mut out = Self::default();
        let Ok(xml) = open365_xlsx::parse_xml(styles_xml) else {
            return out;
        };
        let dxfs: Vec<&XmlElement> = xml
            .root
            .child_main("dxfs")
            .map(|d| d.elements().filter(|e| e.head.is_main("dxf")).collect())
            .unwrap_or_default();
        let Some(ts) = xml.root.child_main("tableStyles") else {
            return out;
        };
        for st in ts.elements().filter(|e| e.head.is_main("tableStyle")) {
            let Some(name) = st.head.attr("name") else {
                continue;
            };
            if st
                .head
                .attr("pivot")
                .is_some_and(|p| matches!(p.trim(), "0" | "false"))
            {
                continue;
            }
            let mut def: StyleDef = vec![None; element::COUNT];
            for el in st
                .elements()
                .filter(|e| e.head.is_main("tableStyleElement"))
            {
                let Some(idx) = el
                    .head
                    .attr("type")
                    .and_then(|t| ELEMENT_TYPES.iter().position(|n| *n == t))
                else {
                    continue;
                };
                let Some(dxf) = el
                    .head
                    .attr("dxfId")
                    .and_then(|d| d.trim().parse::<usize>().ok())
                    .and_then(|d| dxfs.get(d))
                else {
                    continue;
                };
                let stripe = el
                    .head
                    .attr("size")
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(1);
                def[idx] = Some(dxf_element(dxf, stripe));
            }
            out.styles.insert(name.to_owned(), def);
        }
        out
    }

    /// The custom styles of an opened workbook (`xl/styles.xml`).
    pub fn from_workbook(wb: &open365_xlsx::Workbook) -> Self {
        wb.package()
            .part("xl/styles.xml")
            .map(|p| Self::parse(p.data()))
            .unwrap_or_default()
    }

    pub(super) fn get(&self, name: &str) -> Option<&StyleDef> {
        self.styles.get(name)
    }

    /// Names of the custom pivot styles.
    pub fn names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.styles.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }
}
