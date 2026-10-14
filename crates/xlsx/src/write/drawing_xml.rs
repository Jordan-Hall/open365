//! The XML Excel writes for a new picture, shape, text box or line, and for the pieces an edit
//! replaces (fill, outline, paragraphs). The element shapes were read back from Excel 16 with
//! `tools/office/drawing-ops-oracle.ps1` (`fixtures/reference/xlsx/drawing-insert/insert.json`); the
//! creation GUID extension Excel adds to `cNvPr` is left out.

use super::super::{R_NS, esc};
use crate::XlsxError;
use crate::drawing::AnchorMarker;

pub(super) const XDR_NS: &str =
    "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
pub(super) const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const A14_NS: &str = "http://schemas.microsoft.com/office/drawing/2010/main";
const ASVG_NS: &str = "http://schemas.microsoft.com/office/drawing/2016/SVG/main";

fn bad(message: impl Into<String>) -> XlsxError {
    XlsxError::UnsupportedEdit(message.into())
}

/// The element name with the drawing's prefix (`xdr:` or none).
pub(super) struct Names<'a>(pub &'a str);

impl Names<'_> {
    pub(super) fn p(&self, local: &str) -> String {
        if self.0.is_empty() {
            local.to_owned()
        } else {
            format!("{}:{local}", self.0)
        }
    }

    fn xmlns(&self) -> String {
        if self.0.is_empty() {
            format!("xmlns=\"{XDR_NS}\" xmlns:a=\"{A_NS}\"")
        } else {
            format!("xmlns:{}=\"{XDR_NS}\" xmlns:a=\"{A_NS}\"", self.0)
        }
    }
}

fn marker_xml(n: &Names<'_>, tag: &str, m: &AnchorMarker) -> String {
    format!(
        "<{t}><{c}>{}</{c}><{co}>{}</{co}><{r}>{}</{r}><{ro}>{}</{ro}></{t}>",
        m.col,
        m.col_off,
        m.row,
        m.row_off,
        t = n.p(tag),
        c = n.p("col"),
        co = n.p("colOff"),
        r = n.p("row"),
        ro = n.p("rowOff"),
    )
}

/// Which object a new anchor carries.
pub(super) enum Body<'a> {
    Picture {
        id: u32,
        name: &'a str,
        embed: &'a str,
        /// Relationship id of the SVG part when the picture is an SVG.
        svg: Option<&'a str>,
        /// Width and height in EMU.
        ext: (i64, i64),
    },
    Shape {
        id: u32,
        name: &'a str,
        preset: &'a str,
        text_box: bool,
        connector: bool,
        ext: (i64, i64),
    },
}

/// A complete `twoCellAnchor` for a new object at absolute position `off` (EMU, from the sheet
/// origin) between the markers.
pub(super) fn anchor_xml(
    n: &Names<'_>,
    from: &AnchorMarker,
    to: &AnchorMarker,
    off: (i64, i64),
    body: &Body<'_>,
) -> String {
    let (edit_as, inner) = match body {
        Body::Picture {
            id,
            name,
            embed,
            svg,
            ext,
        } => (
            " editAs=\"oneCell\"",
            picture_xml(n, *id, name, embed, *svg, off, *ext),
        ),
        Body::Shape {
            id,
            name,
            preset,
            text_box,
            connector,
            ext,
        } => (
            "",
            shape_xml(n, *id, name, preset, *text_box, *connector, off, *ext),
        ),
    };
    format!(
        "<{a} {ns}{edit_as}>{}{}{inner}<{cd}/></{a}>",
        marker_xml(n, "from", from),
        marker_xml(n, "to", to),
        a = n.p("twoCellAnchor"),
        cd = n.p("clientData"),
        ns = n.xmlns(),
    )
}

fn xfrm(off: (i64, i64), ext: (i64, i64)) -> String {
    format!(
        "<a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        off.0, off.1, ext.0, ext.1
    )
}

fn picture_xml(
    n: &Names<'_>,
    id: u32,
    name: &str,
    embed: &str,
    svg: Option<&str>,
    off: (i64, i64),
    ext: (i64, i64),
) -> String {
    let blip_ext = match svg {
        Some(rid) => format!(
            "<a:extLst><a:ext uri=\"{{96DAC541-7B7A-43D3-8B79-37D633B846F1}}\"><asvg:svgBlip xmlns:asvg=\"{ASVG_NS}\" r:embed=\"{rid}\"/></a:ext></a:extLst>"
        ),
        None => format!(
            "<a:extLst><a:ext uri=\"{{28A0092B-C50C-407E-A947-70E740481C1C}}\"><a14:useLocalDpi xmlns:a14=\"{A14_NS}\" val=\"0\"/></a:ext></a:extLst>"
        ),
    };
    format!(
        "<{pic}><{nv}><{cnv} id=\"{id}\" name=\"{name}\"/><{cp}><a:picLocks noChangeAspect=\"1\"/></{cp}></{nv}><{bf}><a:blip xmlns:r=\"{R_NS}\" r:embed=\"{embed}\">{blip_ext}</a:blip><a:stretch><a:fillRect/></a:stretch></{bf}><{sp}>{xf}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></{sp}></{pic}>",
        pic = n.p("pic"),
        nv = n.p("nvPicPr"),
        cnv = n.p("cNvPr"),
        cp = n.p("cNvPicPr"),
        bf = n.p("blipFill"),
        sp = n.p("spPr"),
        xf = xfrm(off, ext),
        name = esc(name),
    )
}

/// The `p:style` Excel gives a filled shape: accent 1 fill, accent 1 shade outline, light 1 text.
const SHAPE_STYLE: &str = "<a:lnRef idx=\"2\"><a:schemeClr val=\"accent1\"><a:shade val=\"15000\"/></a:schemeClr></a:lnRef><a:fillRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:fillRef><a:effectRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"lt1\"/></a:fontRef>";
/// The style of a text box: no fill or line references, dark 1 text.
const TEXT_BOX_STYLE: &str = "<a:lnRef idx=\"0\"><a:scrgbClr r=\"0\" g=\"0\" b=\"0\"/></a:lnRef><a:fillRef idx=\"0\"><a:scrgbClr r=\"0\" g=\"0\" b=\"0\"/></a:fillRef><a:effectRef idx=\"0\"><a:scrgbClr r=\"0\" g=\"0\" b=\"0\"/></a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"dk1\"/></a:fontRef>";
/// The style of a straight connector.
const LINE_STYLE: &str = "<a:lnRef idx=\"2\"><a:schemeClr val=\"accent1\"/></a:lnRef><a:fillRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:fillRef><a:effectRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:effectRef><a:fontRef idx=\"minor\"><a:schemeClr val=\"tx1\"/></a:fontRef>";

#[allow(clippy::too_many_arguments)]
fn shape_xml(
    n: &Names<'_>,
    id: u32,
    name: &str,
    preset: &str,
    text_box: bool,
    connector: bool,
    off: (i64, i64),
    ext: (i64, i64),
) -> String {
    let geom = format!(
        "<a:prstGeom prst=\"{}\"><a:avLst/></a:prstGeom>",
        esc(preset)
    );
    if connector {
        return format!(
            "<{c} macro=\"\"><{nv}><{cnv} id=\"{id}\" name=\"{name}\"/><{cc}/></{nv}><{sp}>{xf}{geom}</{sp}><{st}>{LINE_STYLE}</{st}></{c}>",
            c = n.p("cxnSp"),
            nv = n.p("nvCxnSpPr"),
            cnv = n.p("cNvPr"),
            cc = n.p("cNvCxnSpPr"),
            sp = n.p("spPr"),
            st = n.p("style"),
            xf = xfrm(off, ext),
            name = esc(name),
        );
    }
    let cnv_sp = if text_box {
        format!("<{} txBox=\"1\"/>", n.p("cNvSpPr"))
    } else {
        format!("<{}/>", n.p("cNvSpPr"))
    };
    let (props, style, body_pr) = if text_box {
        (
            "<a:solidFill><a:schemeClr val=\"lt1\"/></a:solidFill><a:ln w=\"9525\" cmpd=\"sng\"><a:solidFill><a:schemeClr val=\"lt1\"><a:shade val=\"50000\"/></a:schemeClr></a:solidFill></a:ln>",
            TEXT_BOX_STYLE,
            "<a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\" vert=\"horz\" rtlCol=\"0\" anchor=\"t\"/><a:lstStyle/><a:p><a:endParaRPr lang=\"en-GB\" sz=\"1100\"/></a:p>",
        )
    } else {
        (
            "",
            SHAPE_STYLE,
            "<a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\" rtlCol=\"0\" anchor=\"t\"/><a:lstStyle/><a:p><a:pPr algn=\"l\"/><a:endParaRPr lang=\"en-GB\" sz=\"1100\"/></a:p>",
        )
    };
    format!(
        "<{s} macro=\"\" textlink=\"\"><{nv}><{cnv} id=\"{id}\" name=\"{name}\"/>{cnv_sp}</{nv}><{sp}>{xf}{geom}{props}</{sp}><{st}>{style}</{st}><{tx}>{body_pr}</{tx}></{s}>",
        s = n.p("sp"),
        nv = n.p("nvSpPr"),
        cnv = n.p("cNvPr"),
        sp = n.p("spPr"),
        st = n.p("style"),
        tx = n.p("txBody"),
        xf = xfrm(off, ext),
        name = esc(name),
    )
}

/// A DrawingML colour as XML from a short description: `RRGGBB` (hex), or a scheme colour name
/// (`accent1`, `tx1`, `bg1`, `dk1`, `lt1` ...) optionally followed by `:modifier=value` pairs separated by
/// commas (`accent1:lumMod=60000,lumOff=40000`).
pub(super) fn color_xml(spec: &str) -> Result<String, XlsxError> {
    let spec = spec.trim().trim_start_matches('#');
    let (head, mods) = spec.split_once(':').unwrap_or((spec, ""));
    let mut inner = String::new();
    for m in mods.split(',').filter(|m| !m.is_empty()) {
        let (k, v) = m
            .split_once('=')
            .ok_or_else(|| bad("a colour modifier is name=value"))?;
        if !k.chars().all(|c| c.is_ascii_alphabetic()) || v.parse::<i64>().is_err() {
            return Err(bad(format!("the colour modifier {m} is not valid")));
        }
        inner.push_str(&format!("<a:{k} val=\"{v}\"/>"));
    }
    if head.len() == 6 && head.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(format!(
            "<a:srgbClr val=\"{}\">{inner}</a:srgbClr>",
            head.to_ascii_uppercase()
        ));
    }
    const SCHEME: [&str; 16] = [
        "bg1", "tx1", "bg2", "tx2", "accent1", "accent2", "accent3", "accent4", "accent5",
        "accent6", "hlink", "folHlink", "dk1", "lt1", "dk2", "lt2",
    ];
    if SCHEME.contains(&head) {
        return Ok(format!("<a:schemeClr val=\"{head}\">{inner}</a:schemeClr>"));
    }
    Err(bad(format!("{head} is not a colour")))
}

/// `a:solidFill` of a colour description, or `a:noFill` for `none`.
pub(super) fn fill_xml(spec: &str) -> Result<String, XlsxError> {
    if spec.trim().eq_ignore_ascii_case("none") {
        return Ok("<a:noFill/>".to_owned());
    }
    Ok(format!("<a:solidFill>{}</a:solidFill>", color_xml(spec)?))
}

/// The `a:prstDash` value for a dash name (the names of the Excel dash gallery).
pub(super) fn dash_value(name: &str) -> Result<&'static str, XlsxError> {
    Ok(match name {
        "solid" => "solid",
        "round-dot" => "sysDot",
        "square-dot" => "sysDash",
        "dash" => "dash",
        "dash-dot" => "dashDot",
        "long-dash" => "lgDash",
        "long-dash-dot" => "lgDashDot",
        "long-dash-dot-dot" => "lgDashDotDot",
        other => return Err(bad(format!("{other} is not a dash style"))),
    })
}

/// The paragraphs for `text`: one `a:p` per line, the run properties taken from `rpr_attrs`
/// (`lang="en-GB" sz="1100"`), `ppr` is copied into each paragraph.
pub(super) fn paragraphs_xml(text: &str, ppr: &str, rpr_attrs: &str) -> String {
    let mut out = String::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        out.push_str("<a:p>");
        out.push_str(ppr);
        if line.is_empty() {
            out.push_str(&format!("<a:endParaRPr {rpr_attrs}/>"));
        } else {
            out.push_str(&format!(
                "<a:r><a:rPr {rpr_attrs}/><a:t>{}</a:t></a:r>",
                esc(line)
            ));
        }
        out.push_str("</a:p>");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        assert_eq!(
            color_xml("1f4e79").unwrap(),
            "<a:srgbClr val=\"1F4E79\"></a:srgbClr>"
        );
        assert_eq!(
            color_xml("accent1:lumMod=60000,lumOff=40000").unwrap(),
            "<a:schemeClr val=\"accent1\"><a:lumMod val=\"60000\"/><a:lumOff val=\"40000\"/></a:schemeClr>"
        );
        assert!(color_xml("nonsense").is_err());
        assert!(color_xml("accent1:lumMod=x").is_err());
        assert_eq!(fill_xml("none").unwrap(), "<a:noFill/>");
    }

    #[test]
    fn paragraphs_split_lines() {
        let x = paragraphs_xml("a\n\nb & c", "", "lang=\"en-GB\" sz=\"1100\"");
        assert_eq!(x.matches("<a:p>").count(), 3);
        assert!(x.contains("<a:endParaRPr lang=\"en-GB\" sz=\"1100\"/>"));
        assert!(x.contains("b &amp; c"));
    }
}
