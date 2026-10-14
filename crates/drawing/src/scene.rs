//! A resolved scene: the display list with colours, widths and style references applied, and
//! curves flattened to polylines. Hosts draw it with any backend (egui painter, gfx draw list).
//!
//! Pictures stay symbolic (`embed` is a relationship id): the host owns the package and the
//! image decoder. Text frames and fills that need more than a colour (gradients, patterns,
//! picture fills) degrade to a representative colour or are dropped with a diagnostic.

use crate::geom::{P, PathFill, PresetSource, Seg};
use crate::model::{DrawingObject, EMU_PER_PT, Xfrm};
use crate::paint::{Fill, Line, Rgba, StyleRef};
use crate::render::{DisplayList, ImagePrim, PathPrim, Prim, TextFramePrim, display_list};
use crate::text::{Align, Anchor, paragraphs};
use crate::theme::ThemeData;
use crate::transform::Affine;

/// Width in points of a line that gives none and has no style to take one from (`a:ln` default
/// of 9525 EMU).
const DEFAULT_LINE_PT: f64 = 0.75;

/// Stroke parameters of a resolved path.
#[derive(Debug, Clone, PartialEq)]
pub struct StrokeSpec {
    pub color: Rgba,
    pub width_pt: f64,
    /// On/off lengths in points; empty is solid.
    pub dash: Vec<f64>,
}

/// One flattened subpath.
#[derive(Debug, Clone, PartialEq)]
pub struct SubPath {
    pub points: Vec<P>,
    pub closed: bool,
}

/// A filled and/or stroked path.
#[derive(Debug, Clone, PartialEq)]
pub struct Poly {
    pub object_id: u32,
    pub subpaths: Vec<SubPath>,
    pub fill: Option<Rgba>,
    pub stroke: Option<StrokeSpec>,
}

/// One paragraph of a text item, colour resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    pub text: String,
    pub size_pt: f64,
    pub bold: bool,
    pub italic: bool,
    pub color: Rgba,
    pub align: Align,
}

/// The text of a shape or text box: paragraphs laid into `rect` (after the insets), a bounding box
/// in the scene's space. A rotated shape's text turns with it: `angle_deg` is the rotation of the
/// frame, clockwise, and `rect` is the text rectangle, upright, centred where the rotated rectangle's
/// centre lands.
#[derive(Debug, Clone, PartialEq)]
pub struct TextItem {
    pub object_id: u32,
    /// `(left, top, right, bottom)` of the text rectangle inside the insets, scene points.
    pub rect: (f64, f64, f64, f64),
    pub angle_deg: f64,
    /// Centre of the text rectangle in the scene (the pivot of `angle_deg`).
    pub pivot: P,
    pub lines: Vec<TextLine>,
    pub anchor: Anchor,
    pub wrap: bool,
}

/// A scene item in z order.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneItem {
    Poly(Poly),
    Image(ImagePrim),
    Text(TextItem),
}

/// Items in z order plus what could not be drawn faithfully.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene {
    pub items: Vec<SceneItem>,
    pub diagnostics: Vec<String>,
}

/// Returns `obj` placed in the rectangle `(x, y, w, h)` points: the position and size of a
/// worksheet object come from its anchor, not from the `xfrm` in the shape. Rotation and flips
/// are kept (the rotation turns about the centre of the rectangle).
pub fn place_in_rect(obj: &DrawingObject, x: f64, y: f64, w: f64, h: f64) -> DrawingObject {
    let mut o = obj.clone();
    let k = EMU_PER_PT;
    o.xfrm = Xfrm {
        off_x: (x * k).round() as i64,
        off_y: (y * k).round() as i64,
        ext_cx: (w * k).round() as i64,
        ext_cy: (h * k).round() as i64,
        ..obj.xfrm
    };
    o
}

/// Draws `obj` (already placed, see [`place_in_rect`]) into a scene in points.
pub fn build_scene(
    obj: &DrawingObject,
    presets: &dyn PresetSource,
    theme: &ThemeData,
    parent: Affine,
) -> Scene {
    scene_of(&display_list(obj, presets, parent), theme)
}

/// Resolves a display list.
pub fn scene_of(list: &DisplayList, theme: &ThemeData) -> Scene {
    let mut scene = Scene {
        items: Vec::new(),
        diagnostics: list.diagnostics.clone(),
    };
    for prim in &list.prims {
        match prim {
            Prim::Path(p) => {
                if let Some(poly) = resolve_path(p, theme, &mut scene.diagnostics) {
                    scene.items.push(SceneItem::Poly(poly));
                }
            }
            Prim::Image(i) => scene.items.push(SceneItem::Image(i.clone())),
            Prim::TextFrame(t) => {
                if let Some(item) = resolve_text(t, theme) {
                    scene.items.push(SceneItem::Text(item));
                }
            }
        }
    }
    scene
}

/// Default size of text when a run gives none (Office's 11 pt).
const DEFAULT_TEXT_PT: f64 = 11.0;

fn resolve_text(t: &TextFramePrim, theme: &ThemeData) -> Option<TextItem> {
    let paras = paragraphs(&t.story);
    if paras.iter().all(|p| p.text.is_empty()) {
        return None;
    }
    // Text colour: the run's, else the style's `fontRef`, else dark text.
    let default = t
        .style
        .as_ref()
        .and_then(|s| s.font.as_ref())
        .and_then(|f| f.color.as_ref())
        .map_or(
            Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
            |c| c.resolve(theme, None),
        );
    let lines = paras
        .into_iter()
        .map(|p| TextLine {
            color: p.color.map_or(default, |c| c.resolve(theme, Some(default))),
            size_pt: p.size_pt.unwrap_or(DEFAULT_TEXT_PT),
            text: p.text,
            bold: p.bold,
            italic: p.italic,
            align: p.align,
        })
        .collect();
    let k = EMU_PER_PT;
    let (l, tp, r, b) = t.rect;
    let (l, tp, r, b) = (
        l + t.props.left_inset as f64 / k,
        tp + t.props.top_inset as f64 / k,
        r - t.props.right_inset as f64 / k,
        b - t.props.bottom_inset as f64 / k,
    );
    // The frame's own space is the box at the origin, so the shape centre is half its extent; the
    // transform places it (with any rotation) in the scene.
    let tl = t.transform.apply(P::new(l, tp));
    let br = t.transform.apply(P::new(r, b));
    let mid = P::new((tl.x + br.x) / 2.0, (tl.y + br.y) / 2.0);
    let angle = t.transform.b.atan2(t.transform.a).to_degrees();
    // The rectangle in the scene space, unrotated about its own centre.
    let (w, h) = ((r - l).max(0.0), (b - tp).max(0.0));
    Some(TextItem {
        object_id: t.object_id,
        rect: (
            mid.x - w / 2.0,
            mid.y - h / 2.0,
            mid.x + w / 2.0,
            mid.y + h / 2.0,
        ),
        angle_deg: angle,
        pivot: mid,
        lines,
        anchor: t.props.anchor,
        wrap: t.props.wrap,
    })
}

fn resolve_path(p: &PathPrim, theme: &ThemeData, diags: &mut Vec<String>) -> Option<Poly> {
    let style = p.style.as_ref();
    let ph_fill = style
        .and_then(|s| s.fill.as_ref())
        .and_then(|s| s.color.as_ref())
        .map(|c| c.resolve(theme, None));
    let fill = if p.fill_mode == PathFill::None {
        None
    } else {
        match &p.fill {
            Some(f) => fill_color(f, theme, ph_fill, diags),
            None => style_fill(style, theme, ph_fill, diags),
        }
    };
    let stroke = if p.stroke {
        stroke_of(p.line.as_ref(), style, theme, diags)
    } else {
        None
    };
    if fill.is_none() && stroke.is_none() {
        return None;
    }
    Some(Poly {
        object_id: p.object_id,
        subpaths: flatten(&p.segs),
        fill,
        stroke,
    })
}

fn style_fill(
    style: Option<&StyleRef>,
    theme: &ThemeData,
    ph: Option<Rgba>,
    diags: &mut Vec<String>,
) -> Option<Rgba> {
    let idx = style?.fill.as_ref()?.idx as usize;
    // 0 is no fill; 1001 and up index the background fill styles, which Office does not use
    // for worksheet shapes.
    let fill = theme.fills.get(idx.checked_sub(1)?)?;
    fill_color(fill, theme, ph, diags)
}

/// The colour a fill is drawn with. Gradients draw as the average of their stops, patterns as
/// the foreground colour; picture fills are dropped.
fn fill_color(
    f: &Fill,
    theme: &ThemeData,
    ph: Option<Rgba>,
    diags: &mut Vec<String>,
) -> Option<Rgba> {
    match f {
        Fill::None | Fill::Group => None,
        Fill::Solid(c) => Some(c.resolve(theme, ph)),
        Fill::Gradient { stops, .. } => {
            diags.push("gradient fill drawn as its average colour".to_owned());
            average(stops.iter().map(|s| s.color.resolve(theme, ph)))
        }
        Fill::Pattern { fg, .. } => {
            diags.push("pattern fill drawn as its foreground colour".to_owned());
            fg.as_ref().map(|c| c.resolve(theme, ph))
        }
        Fill::Blip { .. } => {
            diags.push("picture fill is not drawn".to_owned());
            None
        }
    }
}

fn average(colors: impl Iterator<Item = Rgba>) -> Option<Rgba> {
    let v: Vec<Rgba> = colors.collect();
    if v.is_empty() {
        return None;
    }
    let n = v.len() as u32;
    let avg = |f: fn(&Rgba) -> u8| (v.iter().map(|c| u32::from(f(c))).sum::<u32>() / n) as u8;
    Some(Rgba {
        r: avg(|c| c.r),
        g: avg(|c| c.g),
        b: avg(|c| c.b),
        a: avg(|c| c.a),
    })
}

fn stroke_of(
    own: Option<&Line>,
    style: Option<&StyleRef>,
    theme: &ThemeData,
    diags: &mut Vec<String>,
) -> Option<StrokeSpec> {
    let slot = style.and_then(|s| s.ln.as_ref());
    let ph = slot
        .and_then(|s| s.color.as_ref())
        .map(|c| c.resolve(theme, None));
    let themed: Option<&Line> = slot
        .and_then(|s| (s.idx as usize).checked_sub(1))
        .and_then(|i| theme.lines.get(i));
    let width_emu = own
        .and_then(|l| l.width)
        .or_else(|| themed.and_then(|l| l.width));
    let fill = own
        .and_then(|l| l.fill.as_ref())
        .or_else(|| themed.and_then(|l| l.fill.as_ref()));
    let color = match fill {
        Some(f) => fill_color(f, theme, ph, diags)?,
        // No line fill anywhere: a line is drawn only when the style asks for one.
        None => return None,
    };
    let width_pt = width_emu.map_or(DEFAULT_LINE_PT, |w| w as f64 / EMU_PER_PT);
    let dash = own
        .and_then(|l| l.dash.as_deref())
        .or_else(|| themed.and_then(|l| l.dash.as_deref()))
        .map(|d| dash_pattern(d, width_pt))
        .unwrap_or_default();
    Some(StrokeSpec {
        color,
        width_pt,
        dash,
    })
}

/// On/off lengths of a preset dash, in multiples of the line width (ECMA-376 20.1.10.48).
fn dash_pattern(name: &str, width_pt: f64) -> Vec<f64> {
    let units: &[f64] = match name {
        "sysDot" => &[1.0, 1.0],
        "sysDash" => &[3.0, 1.0],
        "sysDashDot" => &[3.0, 1.0, 1.0, 1.0],
        "sysDashDotDot" => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        "dot" => &[1.0, 3.0],
        "dash" => &[4.0, 3.0],
        "dashDot" => &[4.0, 3.0, 1.0, 3.0],
        "lgDash" => &[8.0, 3.0],
        "lgDashDot" => &[8.0, 3.0, 1.0, 3.0],
        "lgDashDotDot" => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
        _ => &[],
    };
    units.iter().map(|u| u * width_pt).collect()
}

/// Flattens segments to polylines (curves in steps fine enough for a 1/8 pt error at the sizes
/// sheets use).
pub fn flatten(segs: &[Seg]) -> Vec<SubPath> {
    let mut out: Vec<SubPath> = Vec::new();
    let mut cur: Vec<P> = Vec::new();
    let mut start = P::default();
    let mut at = P::default();
    let finish = |cur: &mut Vec<P>, closed: bool, out: &mut Vec<SubPath>| {
        if cur.len() > 1 {
            out.push(SubPath {
                points: std::mem::take(cur),
                closed,
            });
        } else {
            cur.clear();
        }
    };
    for s in segs {
        match *s {
            Seg::Move(p) => {
                finish(&mut cur, false, &mut out);
                cur.push(p);
                start = p;
                at = p;
            }
            Seg::Line(p) => {
                if cur.is_empty() {
                    cur.push(at);
                }
                cur.push(p);
                at = p;
            }
            Seg::Quad(c, p) => {
                if cur.is_empty() {
                    cur.push(at);
                }
                let n = steps(&[at, c, p]);
                for i in 1..=n {
                    let t = f64::from(i) / f64::from(n);
                    let u = 1.0 - t;
                    cur.push(P::new(
                        u * u * at.x + 2.0 * u * t * c.x + t * t * p.x,
                        u * u * at.y + 2.0 * u * t * c.y + t * t * p.y,
                    ));
                }
                at = p;
            }
            Seg::Cubic(c1, c2, p) => {
                if cur.is_empty() {
                    cur.push(at);
                }
                let n = steps(&[at, c1, c2, p]);
                for i in 1..=n {
                    let t = f64::from(i) / f64::from(n);
                    let u = 1.0 - t;
                    cur.push(P::new(
                        u * u * u * at.x
                            + 3.0 * u * u * t * c1.x
                            + 3.0 * u * t * t * c2.x
                            + t * t * t * p.x,
                        u * u * u * at.y
                            + 3.0 * u * u * t * c1.y
                            + 3.0 * u * t * t * c2.y
                            + t * t * t * p.y,
                    ));
                }
                at = p;
            }
            Seg::Close => {
                finish(&mut cur, true, &mut out);
                at = start;
            }
        }
    }
    finish(&mut cur, false, &mut out);
    out
}

fn steps(ctrl: &[P]) -> u32 {
    let len: f64 = ctrl
        .windows(2)
        .map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y))
        .sum();
    (len / 2.0).ceil().clamp(4.0, 64.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Builtin;
    use crate::parse_fragment;

    const SP: &str = r#"<xdr:sp xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><xdr:nvSpPr><xdr:cNvPr id="2" name="Rect"/><xdr:cNvSpPr/></xdr:nvSpPr>
<xdr:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="635000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr>
<xdr:style><a:lnRef idx="2"><a:schemeClr val="accent1"><a:shade val="50000"/></a:schemeClr></a:lnRef><a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef><a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef><a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef></xdr:style></xdr:sp>"#;

    #[test]
    fn style_reference_supplies_fill_and_line() {
        let obj = parse_fragment(SP).unwrap();
        let placed = place_in_rect(&obj, 10.0, 20.0, 100.0, 50.0);
        let scene = build_scene(&placed, &Builtin, &ThemeData::default(), Affine::IDENTITY);
        let SceneItem::Poly(p) = &scene.items[0] else {
            panic!("poly")
        };
        // fillRef idx 1: solid phClr = accent1 of the default theme.
        assert_eq!(p.fill.map(|c| (c.r, c.g, c.b)), Some((0x44, 0x72, 0xC4)));
        // lnRef idx 2: 12700 EMU = 1 pt, accent1 darkened by shade 50 %.
        let s = p.stroke.as_ref().unwrap();
        assert!((s.width_pt - 1.0).abs() < 1e-9);
        assert_eq!((s.color.r, s.color.g, s.color.b), (0x22, 0x39, 0x62));
        let xs: Vec<f64> = p.subpaths[0].points.iter().map(|q| q.x).collect();
        assert!(xs.iter().all(|x| (10.0..=110.0).contains(x)));
    }

    #[test]
    fn text_in_a_shape_becomes_a_text_item_with_the_style_colour() {
        let with_text = SP.replace(
            "</xdr:sp>",
            "<xdr:txBody><a:bodyPr lIns=\"127000\" tIns=\"63500\" rIns=\"127000\" bIns=\"63500\" anchor=\"ctr\"/><a:lstStyle/><a:p><a:pPr algn=\"ctr\"/><a:r><a:rPr lang=\"en-GB\" sz=\"1200\"/><a:t>Hello</a:t></a:r></a:p></xdr:txBody></xdr:sp>",
        );
        let obj = parse_fragment(&with_text).unwrap();
        let placed = place_in_rect(&obj, 10.0, 20.0, 100.0, 50.0);
        let scene = build_scene(&placed, &Builtin, &ThemeData::default(), Affine::IDENTITY);
        let t = scene
            .items
            .iter()
            .find_map(|i| match i {
                SceneItem::Text(t) => Some(t),
                _ => None,
            })
            .expect("a text item");
        assert_eq!(t.lines[0].text, "Hello");
        assert_eq!(t.lines[0].size_pt, 12.0);
        assert_eq!(t.lines[0].align, Align::Center);
        // fontRef lt1: white text on the accent fill.
        assert_eq!(
            (t.lines[0].color.r, t.lines[0].color.g, t.lines[0].color.b),
            (255, 255, 255)
        );
        // Insets of 10 pt x 5 pt inside the 100 x 50 box at (10, 20).
        assert!((t.rect.0 - 20.0).abs() < 1e-9 && (t.rect.1 - 25.0).abs() < 1e-9);
        assert!((t.rect.2 - 100.0).abs() < 1e-9 && (t.rect.3 - 65.0).abs() < 1e-9);
        assert_eq!(t.anchor, Anchor::Middle);
        assert!(t.angle_deg.abs() < 1e-9);
    }

    #[test]
    fn flatten_closes_subpaths_and_samples_curves() {
        let segs = [
            Seg::Move(P::new(0.0, 0.0)),
            Seg::Cubic(P::new(0.0, 10.0), P::new(10.0, 10.0), P::new(10.0, 0.0)),
            Seg::Close,
        ];
        let sub = flatten(&segs);
        assert_eq!(sub.len(), 1);
        assert!(sub[0].closed);
        assert!(sub[0].points.len() >= 5);
    }
}
