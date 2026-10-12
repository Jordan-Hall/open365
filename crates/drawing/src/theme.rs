//! The parts of an Office theme that drawings need: the colour scheme and the format scheme
//! (the fill and line styles that `fillRef` / `lnRef` index).
//!
//! Both are read from a theme part (`a:theme`) written by Office; nothing here is a guess.
//! Without a theme part the Office 2013 to 2022 values are used (the same colours as the
//! default theme of a workbook).

use crate::error::ParseError;
use crate::paint::{Color, ColorBase, Fill, Line, Rgb, SchemeColor, Theme};
use crate::parse::{fill_of, line};
use crate::xml::{self, Node};

/// Colours and format styles of a theme.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeData {
    /// `dk1 lt1 dk2 lt2 accent1..accent6 hlink folHlink`.
    pub colors: [Rgb; 12],
    /// `a:fillStyleLst` in order (`fillRef idx` 1 is the first).
    pub fills: Vec<Fill>,
    /// `a:lnStyleLst` in order (`lnRef idx` 1 is the first).
    pub lines: Vec<Line>,
}

impl Default for ThemeData {
    fn default() -> Self {
        let c = |v: u32| Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
        let ph = || {
            Fill::Solid(Color {
                base: ColorBase::Scheme(SchemeColor::Phclr),
                mods: Vec::new(),
            })
        };
        let ln = |w: i64| Line {
            width: Some(w),
            fill: Some(ph()),
            ..Line::default()
        };
        Self {
            colors: [
                c(0x00_0000),
                c(0xFF_FFFF),
                c(0x44_546A),
                c(0xE7_E6E6),
                c(0x44_72C4),
                c(0xED_7D31),
                c(0xA5_A5A5),
                c(0xFF_C000),
                c(0x5B_9BD5),
                c(0x70_AD47),
                c(0x05_63C1),
                c(0x95_4F72),
            ],
            fills: vec![ph(), ph(), ph()],
            lines: vec![ln(6350), ln(12700), ln(19050)],
        }
    }
}

impl Theme for ThemeData {
    fn scheme_color(&self, c: SchemeColor) -> Rgb {
        let i = match c {
            SchemeColor::Dk1 => 0,
            SchemeColor::Lt1 => 1,
            SchemeColor::Dk2 => 2,
            SchemeColor::Lt2 => 3,
            SchemeColor::Accent1 => 4,
            SchemeColor::Accent2 => 5,
            SchemeColor::Accent3 => 6,
            SchemeColor::Accent4 => 7,
            SchemeColor::Accent5 => 8,
            SchemeColor::Accent6 => 9,
            SchemeColor::Hlink => 10,
            SchemeColor::FolHlink | SchemeColor::Phclr => 11,
        };
        self.colors[i]
    }
}

const SLOTS: [&str; 12] = [
    "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6",
    "hlink", "folHlink",
];

impl ThemeData {
    /// Reads a theme part. Missing pieces keep the default theme's values.
    pub fn parse(src: &str) -> Result<Self, ParseError> {
        let root = xml::parse(src)?;
        let mut theme = Self::default();
        if let Some(scheme) = root.find("clrScheme") {
            for el in &scheme.children {
                let Some(slot) = SLOTS.iter().position(|n| *n == el.local_name()) else {
                    continue;
                };
                let hex = el.children.first().and_then(|c| match c.local_name() {
                    "srgbClr" => c.attr("val"),
                    "sysClr" => c.attr("lastClr"),
                    _ => None,
                });
                if let Some(v) = hex.and_then(|h| u32::from_str_radix(h, 16).ok()) {
                    theme.colors[slot] = Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
                }
            }
        }
        if let Some(fmt) = root.find("fmtScheme") {
            let fills = fmt.child("fillStyleLst").map(fill_list);
            if let Some(f) = fills.filter(|f| !f.is_empty()) {
                theme.fills = f;
            }
            let lines: Vec<Line> = fmt
                .child("lnStyleLst")
                .map(|l| l.children_named("ln").map(line).collect())
                .unwrap_or_default();
            if !lines.is_empty() {
                theme.lines = lines;
            }
        }
        Ok(theme)
    }
}

/// Each child of a fill style list is one fill (`solidFill`, `gradFill`, ...).
fn fill_list(list: &Node) -> Vec<Fill> {
    list.children
        .iter()
        .map(|c| {
            let wrapper = Node {
                name: "wrap".to_owned(),
                attrs: Vec::new(),
                children: vec![c.clone()],
                span: 0..0,
            };
            fill_of(&wrapper).unwrap_or(Fill::None)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEME: &str = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements>
<a:clrScheme name="t"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
<a:dk2><a:srgbClr val="112233"/></a:dk2><a:accent1><a:srgbClr val="FF0000"/></a:accent1></a:clrScheme>
<a:fmtScheme name="f"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:noFill/></a:fillStyleLst>
<a:lnStyleLst><a:ln w="9999"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst></a:fmtScheme>
</a:themeElements></a:theme>"#;

    #[test]
    fn reads_colours_and_format_scheme() {
        let t = ThemeData::parse(THEME).unwrap();
        assert_eq!(t.colors[2], Rgb(0x11, 0x22, 0x33));
        assert_eq!(t.colors[4], Rgb(255, 0, 0));
        // Not in the file: keeps the default.
        assert_eq!(t.colors[5], Rgb(0xED, 0x7D, 0x31));
        assert_eq!(t.fills.len(), 2);
        assert_eq!(t.fills[1], Fill::None);
        assert_eq!(t.lines.len(), 1);
        assert_eq!(t.lines[0].width, Some(9999));
    }
}
