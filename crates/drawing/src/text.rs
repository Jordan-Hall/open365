//! Text inside a drawing object.
//!
//! The story is host-generic: Word keeps block content (`w:txbxContent`), Excel and
//! PowerPoint keep DrawingML paragraphs (`a:p`). This crate owns only the frame
//! properties (`a:bodyPr`); the host decides how its story is laid out and edited.

use crate::model::RawXml;
use crate::paint::{Color, Fill};
use crate::parse::fill_of;
use crate::xml::{Node, parse as parse_xml};

/// Vertical anchoring of text in its frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    Top,
    Middle,
    Bottom,
    Justified,
    Distributed,
}

/// Text direction (`vert`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Vert {
    #[default]
    Horizontal,
    Vertical,
    Vertical270,
    EastAsianVertical,
    MongolianVertical,
    WordArtVertical,
    Other,
}

/// Autofit behaviour of the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Autofit {
    #[default]
    None,
    /// `a:spAutoFit`: the shape grows to fit the text.
    ShapeToFitText,
    /// `a:normAutofit`: the text shrinks; scales are in percent.
    ShrinkText {
        font_scale_pct: Option<u32>,
        line_reduction_pct: Option<u32>,
    },
}

/// Frame properties from `a:bodyPr`. Insets are in EMU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyProps {
    pub left_inset: i64,
    pub top_inset: i64,
    pub right_inset: i64,
    pub bottom_inset: i64,
    /// `wrap="none"` is `false`; the default is `square` (wrap).
    pub wrap: bool,
    pub anchor: Anchor,
    pub vert: Vert,
    pub autofit: Autofit,
    /// Text rotation in 60000ths of a degree.
    pub rot: i64,
}

impl Default for BodyProps {
    /// Schema defaults (0.1 in left/right, 0.05 in top/bottom). Application defaults for
    /// new shapes are measured, not taken from here.
    fn default() -> Self {
        Self {
            left_inset: 91_440,
            top_inset: 45_720,
            right_inset: 91_440,
            bottom_inset: 45_720,
            wrap: true,
            anchor: Anchor::Top,
            vert: Vert::Horizontal,
            autofit: Autofit::None,
            rot: 0,
        }
    }
}

/// Text frame: properties plus the host's story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextBody<S = RawXml> {
    pub props: BodyProps,
    pub story: S,
}

impl<S> TextBody<S> {
    /// Same properties, another story type.
    pub fn map_story<T>(self, f: impl FnOnce(S) -> T) -> TextBody<T> {
        TextBody {
            props: self.props,
            story: f(self.story),
        }
    }
}

/// Horizontal alignment of a paragraph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// One paragraph of a DrawingML story, reduced to what a simple painter needs: the text of all runs
/// with the properties of the first run.
#[derive(Debug, Clone, PartialEq)]
pub struct Para {
    pub text: String,
    /// Font size in points; `None` for the default (11 pt in Office documents).
    pub size_pt: Option<f64>,
    pub bold: bool,
    pub italic: bool,
    /// Colour of the first run when it sets one.
    pub color: Option<Color>,
    pub align: Align,
}

fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let ent = &rest[1..end];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let code = ent
                    .strip_prefix("#x")
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32);
                match code {
                    Some(c) => out.push(c),
                    None => out.push_str(&rest[..=end]),
                }
            }
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn text_of(t: &Node, src: &str) -> String {
    let raw = &src[t.span.clone()];
    let inner = raw
        .find('>')
        .and_then(|a| raw.rfind("</").filter(|b| *b > a).map(|b| &raw[a + 1..b]))
        .unwrap_or("");
    unescape(inner)
}

/// Reads the paragraphs (`a:p`) of an Excel or PowerPoint text body (`story` is the verbatim
/// `txBody`). Line breaks (`a:br`) become newlines. A story that does not parse has no paragraphs.
#[must_use]
pub fn paragraphs(story: &RawXml) -> Vec<Para> {
    let src = story.as_str();
    let Ok(root) = parse_xml(src) else {
        return Vec::new();
    };
    root.children_named("p")
        .map(|p| {
            let mut text = String::new();
            let mut first: Option<&Node> = None;
            for c in &p.children {
                match c.local_name() {
                    "r" | "fld" => {
                        if first.is_none() {
                            first = c.child("rPr");
                        }
                        if let Some(t) = c.child("t") {
                            text.push_str(&text_of(t, src));
                        }
                    }
                    "br" => text.push('\n'),
                    _ => {}
                }
            }
            let rpr = first.or_else(|| p.child("endParaRPr"));
            Para {
                text,
                size_pt: rpr
                    .and_then(|r| r.attr_i64("sz"))
                    .map(|sz| sz as f64 / 100.0),
                bold: rpr.and_then(|r| r.attr_bool("b")).unwrap_or(false),
                italic: rpr.and_then(|r| r.attr_bool("i")).unwrap_or(false),
                color: rpr.and_then(fill_of).and_then(|f| match f {
                    Fill::Solid(c) => Some(c),
                    _ => None,
                }),
                align: match p.child("pPr").and_then(|n| n.attr("algn")) {
                    Some("ctr") => Align::Center,
                    Some("r") => Align::Right,
                    Some("just" | "dist") => Align::Justify,
                    _ => Align::Left,
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_read_text_size_and_alignment() {
        let story = RawXml(
            r#"<xdr:txBody><a:bodyPr/><a:lstStyle/><a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-GB" sz="1400" b="1"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:rPr><a:t>Q1 &amp; Q2</a:t></a:r><a:r><a:rPr lang="en-GB"/><a:t> done</a:t></a:r></a:p><a:p><a:endParaRPr lang="en-GB" sz="1100"/></a:p></xdr:txBody>"#.to_owned(),
        );
        let p = paragraphs(&story);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].text, "Q1 & Q2 done");
        assert_eq!(p[0].size_pt, Some(14.0));
        assert!(p[0].bold && !p[0].italic);
        assert_eq!(p[0].align, Align::Center);
        assert!(p[0].color.is_some());
        assert_eq!(p[1].text, "");
        assert_eq!(p[1].size_pt, Some(11.0));
    }

    #[test]
    fn map_story_keeps_props() {
        let b = TextBody {
            props: BodyProps {
                wrap: false,
                ..BodyProps::default()
            },
            story: 3u8,
        };
        let m = b.map_story(|n| n.to_string());
        assert!(!m.props.wrap);
        assert_eq!(m.story, "3");
    }
}
