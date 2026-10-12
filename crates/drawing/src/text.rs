//! Text inside a drawing object.
//!
//! The story is host-generic: Word keeps block content (`w:txbxContent`), Excel and
//! PowerPoint keep DrawingML paragraphs (`a:p`). This crate owns only the frame
//! properties (`a:bodyPr`); the host decides how its story is laid out and edited.

use crate::model::RawXml;

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

#[cfg(test)]
mod tests {
    use super::*;

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
