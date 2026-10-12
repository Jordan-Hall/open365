//! Read-only parsers: a DrawingML fragment becomes a [`DrawingObject`].
//!
//! Supported roots: `a:graphic`, `xdr:graphicFrame`, `wps:wsp`, `xdr:sp`, `xdr:cxnSp`,
//! `pic:pic` / `xdr:pic`, and groups (`wpg:wgp`, `xdr:grpSp`). Elements are matched by
//! local name; the host checks namespaces. The whole fragment is kept verbatim in
//! [`DrawingObject::raw`] so the object can be written back unchanged.

use crate::error::ParseError;
use crate::geom::{
    Formula, GuideDef, Operand, PathCmd, PathDef, PathFill, Pt, ShapeDef, TextRectDef,
};
use crate::model::{
    DrawingObject, Geometry, GroupData, Kind, PictureData, RawXml, ShapeData, SrcRect, Xfrm,
};
use crate::paint::{
    Color, ColorBase, ColorMod, Effect, Fill, GradStop, Line, LineEnd, Rgb, SchemeColor, StyleRef,
    StyleSlot,
};
use crate::text::{Anchor, Autofit, BodyProps, TextBody, Vert};
use crate::xml::{self, Node};

/// Parses a drawing fragment.
pub fn parse_fragment(src: &str) -> Result<DrawingObject, ParseError> {
    let root = xml::parse(src)?;
    let o = object(&root, src)?;
    Ok(o)
}

/// Parses an `a:custGeom` element on its own.
pub fn cust_geom_from_str(src: &str) -> Result<ShapeDef, ParseError> {
    cust_geom(&xml::parse(src)?)
}

fn raw(n: &Node, src: &str) -> RawXml {
    RawXml(src.get(n.span.clone()).unwrap_or_default().to_owned())
}

fn object(n: &Node, src: &str) -> Result<DrawingObject, ParseError> {
    let mut o = match n.local_name() {
        "sp" | "wsp" => shape(n, src, false),
        "cxnSp" => shape(n, src, true),
        "pic" => picture(n, src),
        "grpSp" | "wgp" => group(n, src)?,
        "graphicFrame" => {
            let mut o = match n.find("graphic") {
                Some(g) => graphic(g, src)?,
                None => unknown(n, src, ""),
            };
            if let Some(x) = n.child("xfrm").map(xfrm) {
                o.xfrm = x;
            }
            if let Some(c) = n.find("cNvPr") {
                apply_cnvpr(&mut o, c);
            }
            o
        }
        "graphic" => graphic(n, src)?,
        other => return Err(ParseError::UnsupportedRoot(other.to_owned())),
    };
    o.raw = raw(n, src);
    Ok(o)
}

fn blank(n: &Node, src: &str, kind: Kind) -> DrawingObject {
    DrawingObject {
        id: 0,
        name: String::new(),
        descr: None,
        hidden: false,
        xfrm: Xfrm::default(),
        kind,
        raw: raw(n, src),
    }
}

fn unknown(n: &Node, src: &str, uri: &str) -> DrawingObject {
    blank(
        n,
        src,
        Kind::Unknown {
            uri: uri.to_owned(),
        },
    )
}

fn apply_cnvpr(o: &mut DrawingObject, c: &Node) {
    o.id = c.attr("id").and_then(|v| v.parse().ok()).unwrap_or(0);
    c.attr("name").unwrap_or_default().clone_into(&mut o.name);
    o.descr = c.attr("descr").map(str::to_owned);
    o.hidden = c.attr_bool("hidden").unwrap_or(false);
}

fn graphic(n: &Node, src: &str) -> Result<DrawingObject, ParseError> {
    let Some(data) = n.child("graphicData") else {
        return Ok(unknown(n, src, ""));
    };
    let uri = data.attr("uri").unwrap_or_default();
    let Some(inner) = data.children.first() else {
        return Ok(unknown(n, src, uri));
    };
    let mut o = match inner.local_name() {
        "wsp" | "pic" | "wgp" | "sp" | "grpSp" | "cxnSp" => object(inner, src)?,
        "chart" => blank(
            n,
            src,
            Kind::Chart {
                rid: inner.attr("id").unwrap_or_default().to_owned(),
            },
        ),
        _ => unknown(n, src, uri),
    };
    o.raw = raw(n, src);
    Ok(o)
}

fn xfrm(n: &Node) -> Xfrm {
    let off = n.child("off");
    let ext = n.child("ext");
    Xfrm {
        off_x: off.and_then(|o| o.attr_i64("x")).unwrap_or(0),
        off_y: off.and_then(|o| o.attr_i64("y")).unwrap_or(0),
        ext_cx: ext.and_then(|o| o.attr_i64("cx")).unwrap_or(0),
        ext_cy: ext.and_then(|o| o.attr_i64("cy")).unwrap_or(0),
        rot: n.attr_i64("rot").unwrap_or(0),
        flip_h: n.attr_bool("flipH").unwrap_or(false),
        flip_v: n.attr_bool("flipV").unwrap_or(false),
    }
}

fn sp_pr_xfrm(n: &Node) -> Xfrm {
    ["spPr", "grpSpPr"]
        .iter()
        .find_map(|k| n.child(k))
        .and_then(|p| p.child("xfrm"))
        .map(xfrm)
        .unwrap_or_default()
}

fn shape(n: &Node, src: &str, connector: bool) -> DrawingObject {
    let sp = n.child("spPr");
    let data = ShapeData {
        geometry: sp
            .and_then(|p| geometry(p).ok().flatten())
            .unwrap_or_else(Geometry::rect),
        fill: sp.and_then(fill_of),
        line: sp.and_then(|p| p.child("ln")).map(line),
        effects: sp.map(effects).unwrap_or_default(),
        style: n.child("style").map(style),
        text: text_of(n, src),
        is_text_box: n
            .find("cNvSpPr")
            .and_then(|c| c.attr_bool("txBox"))
            .unwrap_or(false),
        is_connector: connector,
    };
    let mut o = blank(n, src, Kind::Shape(Box::new(data)));
    o.xfrm = sp_pr_xfrm(n);
    if let Some(c) = n.find("cNvPr") {
        apply_cnvpr(&mut o, c);
    }
    o
}

fn picture(n: &Node, src: &str) -> DrawingObject {
    let sp = n.child("spPr");
    let bf = n.child("blipFill");
    let blip = bf.and_then(|b| b.child("blip"));
    let data = PictureData {
        embed: blip.and_then(|b| b.attr("embed")).map(str::to_owned),
        link: blip.and_then(|b| b.attr("link")).map(str::to_owned),
        src_rect: bf.and_then(|b| b.child("srcRect")).map(|r| SrcRect {
            l: r.attr_i64("l").unwrap_or(0),
            t: r.attr_i64("t").unwrap_or(0),
            r: r.attr_i64("r").unwrap_or(0),
            b: r.attr_i64("b").unwrap_or(0),
        }),
        geometry: sp
            .and_then(|p| geometry(p).ok().flatten())
            .unwrap_or_else(Geometry::rect),
        line: sp.and_then(|p| p.child("ln")).map(line),
        effects: sp.map(effects).unwrap_or_default(),
    };
    let mut o = blank(n, src, Kind::Picture(Box::new(data)));
    o.xfrm = sp_pr_xfrm(n);
    if let Some(c) = n.find("cNvPr") {
        apply_cnvpr(&mut o, c);
    }
    o
}

fn group(n: &Node, src: &str) -> Result<DrawingObject, ParseError> {
    let gp = n.child("grpSpPr");
    let x = gp.and_then(|p| p.child("xfrm"));
    let ch_off = x.and_then(|x| x.child("chOff"));
    let ch_ext = x.and_then(|x| x.child("chExt"));
    let mut children = Vec::new();
    for c in &n.children {
        if matches!(
            c.local_name(),
            "sp" | "wsp" | "cxnSp" | "pic" | "grpSp" | "wgp" | "graphicFrame"
        ) {
            children.push(object(c, src)?);
        }
    }
    let data = GroupData {
        ch_off_x: ch_off.and_then(|o| o.attr_i64("x")).unwrap_or(0),
        ch_off_y: ch_off.and_then(|o| o.attr_i64("y")).unwrap_or(0),
        ch_ext_cx: ch_ext.and_then(|o| o.attr_i64("cx")).unwrap_or(0),
        ch_ext_cy: ch_ext.and_then(|o| o.attr_i64("cy")).unwrap_or(0),
        children,
    };
    let mut o = blank(n, src, Kind::Group(data));
    o.xfrm = x.map(xfrm).unwrap_or_default();
    // `wpg:cNvPr` is a direct child; `xdr:grpSp` nests it in `nvGrpSpPr`.
    let own = n.child("cNvPr").or_else(|| {
        n.children
            .iter()
            .find(|c| c.local_name().starts_with("nv"))
            .and_then(|nv| nv.find("cNvPr"))
    });
    if let Some(c) = own {
        apply_cnvpr(&mut o, c);
    }
    Ok(o)
}

// --- geometry ---------------------------------------------------------------------------

fn geometry(sp: &Node) -> Result<Option<Geometry>, ParseError> {
    if let Some(p) = sp.child("prstGeom") {
        let adjust = p
            .child("avLst")
            .map(|a| {
                a.children_named("gd")
                    .filter_map(|g| Some((g.attr("name")?.to_owned(), g.attr("fmla")?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        return Ok(Some(Geometry::Preset {
            name: p.attr("prst").unwrap_or("rect").to_owned(),
            adjust,
        }));
    }
    if let Some(c) = sp.child("custGeom") {
        return Ok(Some(Geometry::Custom(Box::new(cust_geom(c)?))));
    }
    Ok(None)
}

fn guides(n: Option<&Node>) -> Result<Vec<GuideDef>, ParseError> {
    let mut v = Vec::new();
    for g in n.into_iter().flat_map(|n| n.children_named("gd")) {
        let name = g.attr("name").unwrap_or_default().to_owned();
        let f = g.attr("fmla").unwrap_or_default();
        let fmla =
            Formula::parse(f).map_err(|e| ParseError::Geometry(format!("guide `{name}`: {e}")))?;
        v.push(GuideDef { name, fmla });
    }
    Ok(v)
}

fn point(n: &Node) -> Pt {
    let p = n.child("pt");
    let op = |k| Operand::parse(p.and_then(|p| p.attr(k)).unwrap_or("0"));
    Pt {
        x: op("x"),
        y: op("y"),
    }
}

fn points(n: &Node) -> Vec<Pt> {
    n.children_named("pt")
        .map(|p| Pt {
            x: Operand::parse(p.attr("x").unwrap_or("0")),
            y: Operand::parse(p.attr("y").unwrap_or("0")),
        })
        .collect()
}

fn cust_geom(n: &Node) -> Result<ShapeDef, ParseError> {
    let mut def = ShapeDef {
        adjust: guides(n.child("avLst"))?,
        guides: guides(n.child("gdLst"))?,
        paths: Vec::new(),
        text_rect: n.child("rect").map(|r| -> TextRectDef {
            ["l", "t", "r", "b"].map(|k| Operand::parse(r.attr(k).unwrap_or("0")))
        }),
    };
    for p in n
        .child("pathLst")
        .into_iter()
        .flat_map(|l| l.children_named("path"))
    {
        let mut cmds = Vec::new();
        for c in &p.children {
            let cmd = match c.local_name() {
                "moveTo" => PathCmd::MoveTo(point(c)),
                "lnTo" => PathCmd::LnTo(point(c)),
                "arcTo" => PathCmd::ArcTo {
                    wr: Operand::parse(c.attr("wR").unwrap_or("0")),
                    hr: Operand::parse(c.attr("hR").unwrap_or("0")),
                    st_ang: Operand::parse(c.attr("stAng").unwrap_or("0")),
                    sw_ang: Operand::parse(c.attr("swAng").unwrap_or("0")),
                },
                "quadBezTo" => {
                    let v = points(c);
                    let [a, b]: [Pt; 2] = v
                        .try_into()
                        .map_err(|_| ParseError::Geometry("quadBezTo needs 2 points".into()))?;
                    PathCmd::QuadBezTo(a, b)
                }
                "cubicBezTo" => {
                    let v = points(c);
                    let [a, b, d]: [Pt; 3] = v
                        .try_into()
                        .map_err(|_| ParseError::Geometry("cubicBezTo needs 3 points".into()))?;
                    PathCmd::CubicBezTo(a, b, d)
                }
                "close" => PathCmd::Close,
                _ => continue,
            };
            cmds.push(cmd);
        }
        def.paths.push(PathDef {
            w: p.attr_i64("w").unwrap_or(0),
            h: p.attr_i64("h").unwrap_or(0),
            fill: p
                .attr("fill")
                .and_then(PathFill::from_name)
                .unwrap_or_default(),
            stroke: p.attr_bool("stroke").unwrap_or(true),
            extrusion_ok: p.attr_bool("extrusionOk").unwrap_or(true),
            cmds,
        });
    }
    Ok(def)
}

// --- paint ------------------------------------------------------------------------------

fn pct(n: &Node) -> f64 {
    n.attr_i64("val").unwrap_or(0) as f64 / 100_000.0
}

fn color_of(n: &Node) -> Option<Color> {
    let base = match n.local_name() {
        "srgbClr" => {
            let v = n.attr("val")?;
            let r = u32::from_str_radix(v, 16).ok()?;
            ColorBase::Srgb(Rgb((r >> 16) as u8, (r >> 8) as u8, r as u8))
        }
        "schemeClr" => ColorBase::Scheme(SchemeColor::from_name(n.attr("val")?)?),
        "sysClr" => ColorBase::System {
            name: n.attr("val")?.to_owned(),
            last: n
                .attr("lastClr")
                .and_then(|v| u32::from_str_radix(v, 16).ok())
                .map(|r| Rgb((r >> 16) as u8, (r >> 8) as u8, r as u8)),
        },
        "prstClr" => ColorBase::Preset(n.attr("val")?.to_owned()),
        _ => return None,
    };
    let mods = n
        .children
        .iter()
        .filter_map(|m| {
            Some(match m.local_name() {
                "alpha" => ColorMod::Alpha(pct(m)),
                "alphaMod" => ColorMod::AlphaMod(pct(m)),
                "alphaOff" => ColorMod::AlphaOff(pct(m)),
                "lumMod" => ColorMod::LumMod(pct(m)),
                "lumOff" => ColorMod::LumOff(pct(m)),
                "satMod" => ColorMod::SatMod(pct(m)),
                "satOff" => ColorMod::SatOff(pct(m)),
                "tint" => ColorMod::Tint(pct(m)),
                "shade" => ColorMod::Shade(pct(m)),
                _ => return None,
            })
        })
        .collect();
    Some(Color { base, mods })
}

fn first_color(n: &Node) -> Option<Color> {
    n.children.iter().find_map(color_of)
}

/// The fill child of `parent` (`noFill`, `solidFill`, ...), if any.
pub(crate) fn fill_of(parent: &Node) -> Option<Fill> {
    parent.children.iter().find_map(|c| match c.local_name() {
        "noFill" => Some(Fill::None),
        "solidFill" => Some(first_color(c).map_or(Fill::None, Fill::Solid)),
        "gradFill" => Some(Fill::Gradient {
            stops: c
                .child("gsLst")
                .into_iter()
                .flat_map(|l| l.children_named("gs"))
                .filter_map(|g| {
                    Some(GradStop {
                        pos: g.attr_i64("pos")? as f64 / 100_000.0,
                        color: first_color(g)?,
                    })
                })
                .collect(),
            linear_angle: c.child("lin").and_then(|l| l.attr_i64("ang")),
            path: c
                .child("path")
                .and_then(|p| p.attr("path"))
                .map(str::to_owned),
        }),
        "blipFill" => Some(Fill::Blip {
            embed: c
                .child("blip")
                .and_then(|b| b.attr("embed"))
                .unwrap_or_default()
                .to_owned(),
        }),
        "pattFill" => Some(Fill::Pattern {
            preset: c.attr("prst").unwrap_or_default().to_owned(),
            fg: c.child("fgClr").and_then(first_color),
            bg: c.child("bgClr").and_then(first_color),
        }),
        "grpFill" => Some(Fill::Group),
        _ => None,
    })
}

fn line_end(n: &Node) -> LineEnd {
    LineEnd {
        kind: n.attr("type").unwrap_or("none").to_owned(),
        width: n.attr("w").map(str::to_owned),
        length: n.attr("len").map(str::to_owned),
    }
}

pub(crate) fn line(n: &Node) -> Line {
    Line {
        width: n.attr_i64("w"),
        fill: fill_of(n),
        dash: n
            .child("prstDash")
            .and_then(|d| d.attr("val"))
            .map(str::to_owned),
        cap: n.attr("cap").map(str::to_owned),
        join: n
            .children
            .iter()
            .find(|c| matches!(c.local_name(), "round" | "bevel" | "miter"))
            .map(|c| c.local_name().to_owned()),
        head_end: n.child("headEnd").map(line_end),
        tail_end: n.child("tailEnd").map(line_end),
    }
}

fn effects(sp: &Node) -> Vec<Effect> {
    sp.child("effectLst")
        .map(|l| {
            l.children
                .iter()
                .map(|e| Effect {
                    kind: e.local_name().to_owned(),
                    params: e.attrs.clone(),
                    color: first_color(e),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn style(n: &Node) -> StyleRef {
    let slot = |k| {
        n.child(k).map(|r| StyleSlot {
            idx: r.attr("idx").and_then(|v| v.parse().ok()).unwrap_or(0),
            color: first_color(r),
        })
    };
    StyleRef {
        ln: slot("lnRef"),
        fill: slot("fillRef"),
        effect: slot("effectRef"),
        font: slot("fontRef"),
    }
}

// --- text -------------------------------------------------------------------------------

fn body_props(n: &Node) -> BodyProps {
    let d = BodyProps::default();
    BodyProps {
        left_inset: n.attr_i64("lIns").unwrap_or(d.left_inset),
        top_inset: n.attr_i64("tIns").unwrap_or(d.top_inset),
        right_inset: n.attr_i64("rIns").unwrap_or(d.right_inset),
        bottom_inset: n.attr_i64("bIns").unwrap_or(d.bottom_inset),
        wrap: n.attr("wrap") != Some("none"),
        anchor: match n.attr("anchor") {
            Some("ctr") => Anchor::Middle,
            Some("b") => Anchor::Bottom,
            Some("just") => Anchor::Justified,
            Some("dist") => Anchor::Distributed,
            _ => Anchor::Top,
        },
        vert: match n.attr("vert") {
            None | Some("horz") => Vert::Horizontal,
            Some("vert") => Vert::Vertical,
            Some("vert270") => Vert::Vertical270,
            Some("eaVert") => Vert::EastAsianVertical,
            Some("mongolianVert") => Vert::MongolianVertical,
            Some("wordArtVert") => Vert::WordArtVertical,
            Some(_) => Vert::Other,
        },
        autofit: if n.child("spAutoFit").is_some() {
            Autofit::ShapeToFitText
        } else if let Some(a) = n.child("normAutofit") {
            Autofit::ShrinkText {
                font_scale_pct: a
                    .attr_i64("fontScale")
                    .map(|v| u32::try_from(v / 1000).unwrap_or(100)),
                line_reduction_pct: a
                    .attr_i64("lnSpcReduction")
                    .map(|v| u32::try_from(v / 1000).unwrap_or(0)),
            }
        } else {
            Autofit::None
        },
        rot: n.attr_i64("rot").unwrap_or(0),
    }
}

fn text_of(n: &Node, src: &str) -> Option<TextBody> {
    // Word: `wps:txbx` holds `w:txbxContent`, `wps:bodyPr` is a sibling.
    if let Some(tb) = n.child("txbx") {
        let story = tb
            .child("txbxContent")
            .map_or_else(|| raw(tb, src), |c| raw(c, src));
        let props = n.child("bodyPr").map(body_props).unwrap_or_default();
        return Some(TextBody { props, story });
    }
    // Excel/PowerPoint: `xdr:txBody` holds `a:bodyPr` and the paragraphs.
    let tb = n.child("txBody")?;
    Some(TextBody {
        props: tb.child("bodyPr").map(body_props).unwrap_or_default(),
        story: raw(tb, src),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const WSP: &str = r#"<wps:wsp xmlns:wps="x" xmlns:a="y" xmlns:w="z">
  <wps:cNvPr id="4" name="Rectangle 3" descr="alt text"/>
  <wps:cNvSpPr txBox="1"/>
  <wps:spPr>
    <a:xfrm rot="5400000" flipH="1"><a:off x="914400" y="457200"/><a:ext cx="1828800" cy="914400"/></a:xfrm>
    <a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 25000"/></a:avLst></a:prstGeom>
    <a:solidFill><a:schemeClr val="accent1"><a:lumMod val="75000"/></a:schemeClr></a:solidFill>
    <a:ln w="19050"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:prstDash val="dash"/><a:round/><a:tailEnd type="triangle"/></a:ln>
    <a:effectLst><a:outerShdw blurRad="40000" dist="23000"><a:srgbClr val="000000"><a:alpha val="35000"/></a:srgbClr></a:outerShdw></a:effectLst>
  </wps:spPr>
  <wps:style><a:lnRef idx="2"><a:schemeClr val="accent1"/></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef></wps:style>
  <wps:txbx><w:txbxContent><w:p><w:r><w:t>Hi</w:t></w:r></w:p></w:txbxContent></wps:txbx>
  <wps:bodyPr lIns="0" anchor="ctr" wrap="none"><a:spAutoFit/></wps:bodyPr>
</wps:wsp>"#;

    #[test]
    fn wsp_reads_every_field_and_keeps_raw() {
        let o = parse_fragment(WSP).unwrap();
        assert_eq!(
            (o.id, o.name.as_str(), o.descr.as_deref()),
            (4, "Rectangle 3", Some("alt text"))
        );
        assert_eq!(
            o.xfrm,
            Xfrm {
                off_x: 914_400,
                off_y: 457_200,
                ext_cx: 1_828_800,
                ext_cy: 914_400,
                rot: 5_400_000,
                flip_h: true,
                flip_v: false
            }
        );
        assert_eq!(o.raw.as_str(), WSP);
        let Kind::Shape(s) = &o.kind else { panic!() };
        assert!(s.is_text_box && !s.is_connector);
        assert_eq!(
            s.geometry,
            Geometry::Preset {
                name: "roundRect".into(),
                adjust: vec![("adj".into(), "val 25000".into())]
            }
        );
        let Some(Fill::Solid(c)) = &s.fill else {
            panic!()
        };
        assert_eq!(c.base, ColorBase::Scheme(SchemeColor::Accent1));
        assert_eq!(c.mods, vec![ColorMod::LumMod(0.75)]);
        let l = s.line.as_ref().unwrap();
        assert_eq!(l.width, Some(19050));
        assert_eq!(l.dash.as_deref(), Some("dash"));
        assert_eq!(l.join.as_deref(), Some("round"));
        assert_eq!(l.tail_end.as_ref().unwrap().kind, "triangle");
        assert_eq!(s.effects.len(), 1);
        assert_eq!(s.effects[0].kind, "outerShdw");
        let st = s.style.as_ref().unwrap();
        assert_eq!(st.ln.as_ref().unwrap().idx, 2);
        assert_eq!(st.fill.as_ref().unwrap().idx, 1);
        let t = s.text.as_ref().unwrap();
        assert_eq!(
            t.story.as_str(),
            "<w:txbxContent><w:p><w:r><w:t>Hi</w:t></w:r></w:p></w:txbxContent>"
        );
        assert_eq!(t.props.left_inset, 0);
        assert_eq!(t.props.top_inset, 45_720);
        assert!(!t.props.wrap);
        assert_eq!(t.props.anchor, Anchor::Middle);
        assert_eq!(t.props.autofit, Autofit::ShapeToFitText);
    }

    #[test]
    fn xdr_sp_reads_text_body() {
        let src = r#"<xdr:sp xmlns:xdr="x" xmlns:a="y"><xdr:nvSpPr><xdr:cNvPr id="2" name="TextBox 1"/><xdr:cNvSpPr txBox="1"/></xdr:nvSpPr>
        <xdr:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="100" cy="200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></xdr:spPr>
        <xdr:txBody><a:bodyPr vertOverflow="clip" wrap="square"/><a:p><a:r><a:t>x</a:t></a:r></a:p></xdr:txBody></xdr:sp>"#;
        let o = parse_fragment(src).unwrap();
        assert_eq!((o.id, o.name.as_str()), (2, "TextBox 1"));
        assert_eq!((o.xfrm.ext_cx, o.xfrm.ext_cy), (100, 200));
        let Kind::Shape(s) = &o.kind else { panic!() };
        assert_eq!(s.fill, Some(Fill::None));
        assert!(
            s.text
                .as_ref()
                .unwrap()
                .story
                .as_str()
                .starts_with("<xdr:txBody>")
        );
    }

    #[test]
    fn pic_reads_blip_and_crop() {
        let src = r#"<pic:pic xmlns:pic="p" xmlns:a="a" xmlns:r="r"><pic:nvPicPr><pic:cNvPr id="0" name="photo.png"/><pic:cNvPicPr/></pic:nvPicPr>
        <pic:blipFill><a:blip r:embed="rId5"/><a:srcRect l="10000" b="5000"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>
        <pic:spPr><a:xfrm><a:off x="1" y="2"/><a:ext cx="3" cy="4"/></a:xfrm><a:prstGeom prst="ellipse"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>"#;
        let o = parse_fragment(src).unwrap();
        assert_eq!(o.name, "photo.png");
        let Kind::Picture(p) = &o.kind else { panic!() };
        assert_eq!(p.embed.as_deref(), Some("rId5"));
        assert_eq!(
            p.src_rect,
            Some(SrcRect {
                l: 10000,
                t: 0,
                r: 0,
                b: 5000
            })
        );
        assert!(matches!(&p.geometry, Geometry::Preset { name, .. } if name == "ellipse"));
    }

    #[test]
    fn graphic_dispatches_on_the_payload() {
        let pic = r#"<a:graphic xmlns:a="a" xmlns:pic="p"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="7" name="n"/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId9" xmlns:r="r"/></pic:blipFill><pic:spPr/></pic:pic></a:graphicData></a:graphic>"#;
        let o = parse_fragment(pic).unwrap();
        assert_eq!(o.id, 7);
        assert!(matches!(o.kind, Kind::Picture(_)));
        assert_eq!(o.raw.as_str(), pic);

        let chart = r#"<a:graphic xmlns:a="a"><a:graphicData uri="c"><c:chart xmlns:c="c" xmlns:r="r" r:id="rId3"/></a:graphicData></a:graphic>"#;
        assert_eq!(
            parse_fragment(chart).unwrap().kind,
            Kind::Chart { rid: "rId3".into() }
        );

        let smart = r#"<a:graphic xmlns:a="a"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds xmlns:dgm="d"/></a:graphicData></a:graphic>"#;
        assert_eq!(
            parse_fragment(smart).unwrap().kind,
            Kind::Unknown {
                uri: "http://schemas.openxmlformats.org/drawingml/2006/diagram".into()
            }
        );
    }

    #[test]
    fn group_reads_child_space_and_children() {
        let src = r#"<wpg:wgp xmlns:wpg="g" xmlns:wps="s" xmlns:a="a"><wpg:cNvPr id="10" name="Group"/><wpg:cNvGrpSpPr/>
        <wpg:grpSpPr><a:xfrm><a:off x="100" y="200"/><a:ext cx="1000" cy="500"/><a:chOff x="0" y="0"/><a:chExt cx="2000" cy="1000"/></a:xfrm></wpg:grpSpPr>
        <wps:wsp><wps:cNvPr id="11" name="A"/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="10" cy="10"/></a:xfrm></wps:spPr></wps:wsp>
        <wpg:grpSp><wpg:cNvPr id="12" name="Inner"/><wpg:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1" cy="1"/><a:chOff x="0" y="0"/><a:chExt cx="1" cy="1"/></a:xfrm></wpg:grpSpPr></wpg:grpSp>
        </wpg:wgp>"#;
        let o = parse_fragment(src).unwrap();
        assert_eq!((o.id, o.name.as_str()), (10, "Group"));
        assert_eq!(o.count(), 3);
        let Kind::Group(g) = &o.kind else { panic!() };
        assert_eq!((g.ch_ext_cx, g.ch_ext_cy), (2000, 1000));
        assert_eq!(g.children[0].id, 11);
        assert_eq!(g.children[1].id, 12);
        assert_eq!(o.xfrm.ext_cx, 1000);
    }

    #[test]
    fn gradient_and_pattern_fills() {
        let src = r#"<xdr:sp xmlns:xdr="x" xmlns:a="a"><xdr:spPr><a:gradFill><a:gsLst>
          <a:gs pos="0"><a:srgbClr val="000000"/></a:gs><a:gs pos="100000"><a:srgbClr val="FFFFFF"/></a:gs></a:gsLst><a:lin ang="5400000"/></a:gradFill></xdr:spPr></xdr:sp>"#;
        let o = parse_fragment(src).unwrap();
        let Kind::Shape(s) = &o.kind else { panic!() };
        let Some(Fill::Gradient {
            stops,
            linear_angle,
            ..
        }) = &s.fill
        else {
            panic!()
        };
        assert_eq!(stops.len(), 2);
        assert_eq!(*linear_angle, Some(5_400_000));
        assert_eq!(stops[1].pos, 1.0);
    }

    #[test]
    fn cust_geom_errors_are_reported() {
        assert!(matches!(
            cust_geom_from_str(
                r#"<a:custGeom><a:gdLst><a:gd name="x" fmla="bogus 1"/></a:gdLst></a:custGeom>"#
            ),
            Err(ParseError::Geometry(_))
        ));
        assert!(matches!(
            cust_geom_from_str(
                r#"<a:custGeom><a:pathLst><a:path><a:quadBezTo><a:pt x="1" y="1"/></a:quadBezTo></a:path></a:pathLst></a:custGeom>"#
            ),
            Err(ParseError::Geometry(_))
        ));
    }

    #[test]
    fn unsupported_root_and_bad_xml() {
        assert!(matches!(
            parse_fragment("<a:foo/>"),
            Err(ParseError::UnsupportedRoot(_))
        ));
        assert!(parse_fragment("<a:sp>").is_err());
    }
}
