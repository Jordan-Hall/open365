//! Display list: drawing objects to primitives in points.
//!
//! Every primitive is already in the coordinate space of the `parent` transform handed to
//! [`display_list`] (the page, a cell grid, ...): group transforms, rotation and flips are
//! applied. Painting (colour resolution, gradients, text layout) is the consumer's job.

use crate::geom::{self, PathFill, PresetSource, Seg};
use crate::model::{DrawingObject, Geometry, Kind, PictureData, ShapeData, SrcRect};
use crate::paint::{Fill, Line, StyleRef};
use crate::text::BodyProps;
use crate::transform::{Affine, box_matrix, pt};

/// A path in points with its paint.
#[derive(Debug, Clone, PartialEq)]
pub struct PathPrim {
    pub object_id: u32,
    pub segs: Vec<Seg>,
    /// Fill behaviour of this path in the geometry (`none`, `norm`, `lighten`, ...).
    pub fill_mode: PathFill,
    /// The shape's own fill; `None` means inherit from the style reference.
    pub fill: Option<Fill>,
    /// Whether the geometry strokes this path.
    pub stroke: bool,
    pub line: Option<Line>,
    /// The shape's style references: where an absent fill or line comes from.
    pub style: Option<StyleRef>,
}

/// A picture placed by a transform; the picture occupies `0..width_pt` by `0..height_pt`
/// in its own space.
#[derive(Debug, Clone, PartialEq)]
pub struct ImagePrim {
    pub object_id: u32,
    pub embed: Option<String>,
    pub src_rect: Option<SrcRect>,
    pub width_pt: f64,
    pub height_pt: f64,
    pub transform: Affine,
    /// Outline in points when the picture is clipped to a non-rectangular geometry.
    pub clip: Option<Vec<Seg>>,
}

/// A text frame: `rect` is `(l, t, r, b)` in the frame's own space, placed by `transform`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextFramePrim {
    pub object_id: u32,
    pub rect: (f64, f64, f64, f64),
    pub transform: Affine,
    pub props: BodyProps,
    pub story: crate::model::RawXml,
}

/// A display primitive.
#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    Path(Box<PathPrim>),
    Image(ImagePrim),
    TextFrame(TextFramePrim),
}

/// Primitives in z order plus anything that could not be drawn faithfully.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayList {
    pub prims: Vec<Prim>,
    pub diagnostics: Vec<String>,
}

/// Builds the display list of `obj` (children of groups follow their group, in order).
pub fn display_list(
    obj: &DrawingObject,
    presets: &dyn PresetSource,
    parent: Affine,
) -> DisplayList {
    let mut out = DisplayList::default();
    emit(obj, presets, &parent, &mut out);
    out
}

fn emit(obj: &DrawingObject, presets: &dyn PresetSource, parent: &Affine, out: &mut DisplayList) {
    if obj.hidden {
        return;
    }
    let m = parent.mul(&box_matrix(&obj.xfrm));
    let (w, h) = (pt(obj.xfrm.ext_cx), pt(obj.xfrm.ext_cy));
    match &obj.kind {
        Kind::Shape(s) => emit_shape(obj, s, presets, &m, (w, h), out),
        Kind::Picture(p) => emit_picture(obj, p, presets, &m, (w, h), out),
        Kind::Group(g) => {
            let (cw, ch) = (pt(g.ch_ext_cx), pt(g.ch_ext_cy));
            let sx = if cw > 0.0 { w / cw } else { 1.0 };
            let sy = if ch > 0.0 { h / ch } else { 1.0 };
            // Child space -> group box (local, unrotated), then the group's own matrix.
            let into = m
                .mul(&Affine::scale(sx, sy))
                .mul(&Affine::translate(-pt(g.ch_off_x), -pt(g.ch_off_y)));
            for c in &g.children {
                emit(c, presets, &into, out);
            }
        }
        Kind::Chart { .. } => out.diagnostics.push(format!(
            "object {}: chart is drawn by the chart crate",
            obj.id
        )),
        Kind::Unknown { uri } => out.diagnostics.push(format!(
            "object {}: graphic data `{uri}` is preserved, not drawn",
            obj.id
        )),
    }
}

/// Evaluates a geometry in EMU and returns paths in points (local space).
fn local_geometry(
    g: &Geometry,
    presets: &dyn PresetSource,
    ext_emu: (i64, i64),
    diags: &mut Vec<String>,
) -> geom::Evaluated {
    let (def, adjust) = match g {
        Geometry::Preset { name, adjust } => {
            let (d, gap) = geom::resolve_preset(presets, name);
            diags.extend(gap);
            (d, adjust.as_slice())
        }
        Geometry::Custom(d) => (d.as_ref(), &[][..]),
    };
    let mut e = geom::evaluate(def, adjust, ext_emu.0 as f64, ext_emu.1 as f64);
    diags.append(&mut e.diagnostics);
    let f = |p: geom::P| {
        geom::P::new(
            p.x / crate::model::EMU_PER_PT,
            p.y / crate::model::EMU_PER_PT,
        )
    };
    for p in &mut e.paths {
        p.segs = p.segs.iter().map(|s| s.map(&f)).collect();
    }
    if let Some(r) = &mut e.text_rect {
        let k = crate::model::EMU_PER_PT;
        r.l /= k;
        r.t /= k;
        r.r /= k;
        r.b /= k;
    }
    e
}

fn place(segs: &[Seg], m: &Affine) -> Vec<Seg> {
    segs.iter().map(|s| s.map(&|p| m.apply(p))).collect()
}

fn emit_shape(
    obj: &DrawingObject,
    s: &ShapeData,
    presets: &dyn PresetSource,
    m: &Affine,
    size: (f64, f64),
    out: &mut DisplayList,
) {
    let e = local_geometry(
        &s.geometry,
        presets,
        (obj.xfrm.ext_cx, obj.xfrm.ext_cy),
        &mut out.diagnostics,
    );
    for p in &e.paths {
        out.prims.push(Prim::Path(Box::new(PathPrim {
            object_id: obj.id,
            segs: place(&p.segs, m),
            fill_mode: p.fill,
            fill: s.fill.clone(),
            stroke: p.stroke,
            line: s.line.clone(),
            style: s.style.clone(),
        })));
    }
    if let Some(t) = &s.text {
        let r = e
            .text_rect
            .map_or((0.0, 0.0, size.0, size.1), |r| (r.l, r.t, r.r, r.b));
        out.prims.push(Prim::TextFrame(TextFramePrim {
            object_id: obj.id,
            rect: r,
            transform: *m,
            props: t.props.clone(),
            story: t.story.clone(),
        }));
    }
}

fn emit_picture(
    obj: &DrawingObject,
    p: &PictureData,
    presets: &dyn PresetSource,
    m: &Affine,
    size: (f64, f64),
    out: &mut DisplayList,
) {
    let plain = matches!(&p.geometry, Geometry::Preset { name, .. } if name == "rect");
    let clip = if plain {
        None
    } else {
        let e = local_geometry(
            &p.geometry,
            presets,
            (obj.xfrm.ext_cx, obj.xfrm.ext_cy),
            &mut out.diagnostics,
        );
        e.paths.first().map(|x| place(&x.segs, m))
    };
    out.prims.push(Prim::Image(ImagePrim {
        object_id: obj.id,
        embed: p.embed.clone(),
        src_rect: p.src_rect,
        width_pt: size.0,
        height_pt: size.1,
        transform: *m,
        clip,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Builtin, P};
    use crate::model::{GroupData, RawXml, Xfrm};

    fn shape(id: u32, x: Xfrm) -> DrawingObject {
        DrawingObject {
            id,
            name: format!("s{id}"),
            descr: None,
            hidden: false,
            xfrm: x,
            kind: Kind::Shape(Box::new(ShapeData {
                geometry: Geometry::rect(),
                fill: None,
                line: None,
                effects: vec![],
                style: None,
                text: None,
                is_text_box: false,
                is_connector: false,
            })),
            raw: RawXml::default(),
        }
    }

    fn xf(x: i64, y: i64, cx: i64, cy: i64) -> Xfrm {
        Xfrm {
            off_x: x,
            off_y: y,
            ext_cx: cx,
            ext_cy: cy,
            ..Xfrm::default()
        }
    }

    fn pts(p: &Prim) -> Vec<P> {
        let Prim::Path(p) = p else { panic!() };
        p.segs.iter().filter_map(Seg::end).collect()
    }

    #[test]
    fn rect_in_points() {
        let o = shape(1, xf(127_000, 254_000, 1_270_000, 635_000));
        let d = display_list(&o, &Builtin, Affine::IDENTITY);
        assert_eq!(d.prims.len(), 1);
        let p = pts(&d.prims[0]);
        assert_eq!(p[0], P::new(10.0, 20.0));
        assert_eq!(p[2], P::new(110.0, 70.0));
        assert!(d.diagnostics.is_empty());
    }

    #[test]
    fn rotation_and_parent_transform() {
        let mut x = xf(0, 0, 1_270_000, 635_000);
        x.rot = 5_400_000;
        let o = shape(1, x);
        let d = display_list(&o, &Builtin, Affine::translate(1.0, 2.0));
        let p = pts(&d.prims[0]);
        assert!((p[0].x - 76.0).abs() < 1e-9 && (p[0].y - -23.0).abs() < 1e-9);
    }

    #[test]
    fn group_maps_child_space_into_group_box() {
        // Group at (100pt, 100pt) sized 200x100 pt, child space 0..100 x 0..100 (so x2, y1).
        let mut child = shape(2, xf(12_700 * 10, 12_700 * 10, 12_700 * 20, 12_700 * 20));
        child.hidden = false;
        let g = DrawingObject {
            id: 1,
            name: "g".into(),
            descr: None,
            hidden: false,
            xfrm: xf(12_700 * 100, 12_700 * 100, 12_700 * 200, 12_700 * 100),
            kind: Kind::Group(GroupData {
                ch_off_x: 0,
                ch_off_y: 0,
                ch_ext_cx: 12_700 * 100,
                ch_ext_cy: 12_700 * 100,
                children: vec![child],
            }),
            raw: RawXml::default(),
        };
        let d = display_list(&g, &Builtin, Affine::IDENTITY);
        assert_eq!(d.prims.len(), 1);
        let p = pts(&d.prims[0]);
        assert_eq!(p[0], P::new(120.0, 110.0));
        assert_eq!(p[2], P::new(160.0, 130.0));
    }

    #[test]
    fn group_child_offset_is_subtracted() {
        let child = shape(2, xf(12_700 * 50, 12_700 * 50, 12_700 * 10, 12_700 * 10));
        let g = DrawingObject {
            id: 1,
            name: "g".into(),
            descr: None,
            hidden: false,
            xfrm: xf(0, 0, 12_700 * 100, 12_700 * 100),
            kind: Kind::Group(GroupData {
                ch_off_x: 12_700 * 50,
                ch_off_y: 12_700 * 50,
                ch_ext_cx: 12_700 * 100,
                ch_ext_cy: 12_700 * 100,
                children: vec![child],
            }),
            raw: RawXml::default(),
        };
        let d = display_list(&g, &Builtin, Affine::IDENTITY);
        assert_eq!(pts(&d.prims[0])[0], P::new(0.0, 0.0));
    }

    #[test]
    fn rotated_group_rotates_children() {
        let child = shape(2, xf(0, 0, 12_700 * 100, 12_700 * 50));
        let mut gx = xf(0, 0, 12_700 * 100, 12_700 * 50);
        gx.rot = 10_800_000; // 180 degrees
        let g = DrawingObject {
            id: 1,
            name: "g".into(),
            descr: None,
            hidden: false,
            xfrm: gx,
            kind: Kind::Group(GroupData {
                ch_off_x: 0,
                ch_off_y: 0,
                ch_ext_cx: 12_700 * 100,
                ch_ext_cy: 12_700 * 50,
                children: vec![child],
            }),
            raw: RawXml::default(),
        };
        let d = display_list(&g, &Builtin, Affine::IDENTITY);
        let p = pts(&d.prims[0])[0];
        assert!((p.x - 100.0).abs() < 1e-9 && (p.y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn hidden_objects_and_unknown_presets() {
        let mut o = shape(1, xf(0, 0, 12_700, 12_700));
        o.hidden = true;
        assert!(
            display_list(&o, &Builtin, Affine::IDENTITY)
                .prims
                .is_empty()
        );
        o.hidden = false;
        if let Kind::Shape(s) = &mut o.kind {
            s.geometry = Geometry::Preset {
                name: "star5".into(),
                adjust: vec![],
            };
        }
        let d = display_list(&o, &Builtin, Affine::IDENTITY);
        assert_eq!(d.prims.len(), 1);
        assert_eq!(d.diagnostics.len(), 1);
        assert!(d.diagnostics[0].contains("star5"));
    }
}
