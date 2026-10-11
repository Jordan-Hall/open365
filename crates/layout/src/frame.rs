//! Paragraph frames (`w:framePr`) and drop caps: a paragraph, or a run of consecutive paragraphs
//! with the same frame properties, that leaves the flow and is laid out in a box of its own
//! beside which the text wraps. Rules measured with `fixtures/docx/layout-frame-*`
//! (`docs/architecture/word-layout.md`).

use open365_docx::{Paragraph, RawXml};

use crate::table::{PosAnchor, PosSpec};

/// How the text around a frame treats it (`w:wrap`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameWrap {
    /// `around`, `auto`, `tight`, `through`: text runs beside the frame.
    Around,
    /// `none`, `notBeside`: text stays above and below.
    Clear,
}

/// The properties of `w:framePr` that layout uses, positions in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FramePr {
    /// `w:w`: the width of the text box; without it the text's natural width.
    pub width: Option<f64>,
    /// `w:h` and whether `w:hRule` is `exact`.
    pub height: Option<f64>,
    pub exact: bool,
    pub h_space: f64,
    pub v_space: f64,
    pub wrap: FrameWrap,
    pub horz: PosAnchor,
    pub vert: PosAnchor,
    pub x: f64,
    pub y: f64,
    pub x_spec: Option<PosSpec>,
    pub y_spec: Option<PosSpec>,
    /// `w:dropCap` is `drop` or `margin`; `true` for `margin` (the cap hangs in the margin).
    pub drop_cap: Option<bool>,
}

/// A paragraph's frame and the width its text box was laid out at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FrameSpec {
    pub pr: FramePr,
    /// Width of the text box (the paragraph was laid out at this width).
    pub box_w: f64,
    /// How far the first paragraph's borders reach beyond the text box: left, top, right,
    /// bottom. A side with a `w:pBdr` element, whatever its style (`none` included), takes its
    /// distance plus its width, and left and right a further 1.5 pt; a side without one nothing
    /// (`layout-frame-nobdr`, `-bdr`, `-dropcap`).
    pub pad: [f64; 4],
}

/// The reach of the border sides of `para` (see [`FrameSpec::pad`]); `visible` are its
/// resolved visible borders, the raw `w:pBdr` adds the sides that are set to `none`.
pub(crate) fn border_pads(para: &Paragraph, visible: &open365_docx::ParagraphBorders) -> [f64; 4] {
    use open365_docx::Border;
    let reach = |b: &Border, extra: f64| crate::para::border_extent(b) + b.space + extra;
    let mut pad = [
        visible.left.as_ref().map_or(0.0, |b| reach(b, 1.5)),
        visible.top.as_ref().map_or(0.0, |b| reach(b, 0.0)),
        visible.right.as_ref().map_or(0.0, |b| reach(b, 1.5)),
        visible.bottom.as_ref().map_or(0.0, |b| reach(b, 0.0)),
    ];
    let raw = para.props.as_ref().and_then(|ppr| {
        ppr.children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_w("pBdr"))
    });
    for e in raw
        .iter()
        .flat_map(|r| r.children.iter())
        .filter_map(RawXml::as_element)
    {
        let (slot, extra) = match e.head.local_name() {
            "left" => (0, 1.5),
            "top" => (1, 0.0),
            "right" => (2, 1.5),
            "bottom" => (3, 0.0),
            _ => continue,
        };
        let num = |name: &str| {
            e.head
                .attr_w(name)
                .and_then(|v| v.trim().parse::<f64>().ok())
                .unwrap_or(0.0)
        };
        let b = Border {
            style: e.head.attr_w("val").unwrap_or("nil").to_owned(),
            width: num("sz") / 8.0,
            space: num("space"),
            color: None,
        };
        pad[slot] = reach(&b, extra);
    }
    pad
}

impl FramePr {
    /// Reads `w:pPr/w:framePr` of `para`. `None` for a paragraph without a frame and for an
    /// inline one (`w:yAlign="inline"`, which stays in the flow).
    pub(crate) fn read(para: &Paragraph) -> Option<Self> {
        let ppr = para.props.as_ref()?;
        let e = ppr
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_w("framePr"))?;
        let twips = |name: &str| {
            e.head
                .attr_w(name)
                .and_then(|v| v.trim().parse::<i64>().ok())
        };
        let drop_cap = match e.head.attr_w("dropCap") {
            Some("drop") => Some(false),
            Some("margin") => Some(true),
            _ => None,
        };
        let y_spec = e.head.attr_w("yAlign").and_then(|v| match v {
            "top" | "inside" => Some(PosSpec::Start),
            "center" => Some(PosSpec::Center),
            "bottom" | "outside" => Some(PosSpec::End),
            "inline" => Some(PosSpec::Inline),
            _ => None,
        });
        if y_spec == Some(PosSpec::Inline) {
            return None;
        }
        let x_spec = e.head.attr_w("xAlign").and_then(|v| match v {
            "left" | "inside" => Some(PosSpec::Start),
            "center" => Some(PosSpec::Center),
            "right" | "outside" => Some(PosSpec::End),
            _ => None,
        });
        let anchor = |name: &str| match e.head.attr_w(name) {
            Some("margin") => PosAnchor::Margin,
            Some("page") => PosAnchor::Page,
            _ => PosAnchor::Text,
        };
        // Word writes a position one twip too large and reads it back without that twip, as for
        // floating tables (layout-frame-basic: x 1 is 0, layout-frame-page: 2001 is 100 pt).
        let pos = |name: &str| twips(name).map_or(0.0, |t| (t - (t & 1)) as f64 / 20.0);
        let pt = |name: &str| twips(name).map_or(0.0, |t| t as f64 / 20.0);
        Some(Self {
            width: twips("w").filter(|w| *w > 0).map(|w| w as f64 / 20.0),
            height: twips("h").filter(|h| *h > 0).map(|h| h as f64 / 20.0),
            exact: e.head.attr_w("hRule") == Some("exact"),
            h_space: pt("hSpace"),
            v_space: pt("vSpace"),
            wrap: match e.head.attr_w("wrap") {
                Some("none" | "notBeside") => FrameWrap::Clear,
                _ => FrameWrap::Around,
            },
            horz: anchor("hAnchor"),
            vert: anchor("vAnchor"),
            x: pos("x"),
            y: pos("y"),
            x_spec,
            y_spec,
            drop_cap,
        })
    }
}
