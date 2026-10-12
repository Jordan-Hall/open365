//! Shape geometry: definitions, the guide evaluator and the preset source.
//!
//! A [`ShapeDef`] is data: adjust values, guides (formulas) and paths. [`evaluate`]
//! turns one into concrete paths for a box. Preset definitions come from a
//! [`PresetSource`]; the generated table is a slot that is empty for now (see
//! [`presets`]), and unknown presets fall back to the rectangle with a diagnostic.

pub mod formula;
pub mod path;
pub mod presets;

use std::collections::HashMap;
use std::sync::OnceLock;

pub use formula::{Formula, Op, Operand, Scope};
pub use path::{P, Seg};

/// A named formula (an adjust default or a guide).
#[derive(Debug, Clone, PartialEq)]
pub struct GuideDef {
    pub name: String,
    pub fmla: Formula,
}

/// A path point whose coordinates are operands.
#[derive(Debug, Clone, PartialEq)]
pub struct Pt {
    pub x: Operand,
    pub y: Operand,
}

/// A path command.
#[derive(Debug, Clone, PartialEq)]
pub enum PathCmd {
    MoveTo(Pt),
    LnTo(Pt),
    ArcTo {
        wr: Operand,
        hr: Operand,
        st_ang: Operand,
        sw_ang: Operand,
    },
    QuadBezTo(Pt, Pt),
    CubicBezTo(Pt, Pt, Pt),
    Close,
}

/// How a path is filled (`a:path fill`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PathFill {
    None,
    #[default]
    Norm,
    Lighten,
    LightenLess,
    Darken,
    DarkenLess,
}

impl PathFill {
    /// Parses the attribute value.
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "none" => Self::None,
            "norm" => Self::Norm,
            "lighten" => Self::Lighten,
            "lightenLess" => Self::LightenLess,
            "darken" => Self::Darken,
            "darkenLess" => Self::DarkenLess,
            _ => return None,
        })
    }
}

/// One `a:path`.
#[derive(Debug, Clone, PartialEq)]
pub struct PathDef {
    /// Coordinate space of the path; `0` means the shape's own size.
    pub w: i64,
    pub h: i64,
    pub fill: PathFill,
    pub stroke: bool,
    pub extrusion_ok: bool,
    pub cmds: Vec<PathCmd>,
}

/// Text rectangle as four operands `l t r b`.
pub type TextRectDef = [Operand; 4];

/// A complete geometry definition (`a:custGeom` or one preset).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShapeDef {
    pub adjust: Vec<GuideDef>,
    pub guides: Vec<GuideDef>,
    pub paths: Vec<PathDef>,
    pub text_rect: Option<TextRectDef>,
}

fn name(n: &str) -> Operand {
    Operand::Name(n.to_owned())
}

fn pt(x: &str, y: &str) -> Pt {
    Pt {
        x: name(x),
        y: name(y),
    }
}

impl ShapeDef {
    /// The plain rectangle, written by hand. It is the fallback for unknown presets.
    pub fn rect() -> Self {
        Self {
            adjust: Vec::new(),
            guides: Vec::new(),
            paths: vec![PathDef {
                w: 0,
                h: 0,
                fill: PathFill::Norm,
                stroke: true,
                extrusion_ok: true,
                cmds: vec![
                    PathCmd::MoveTo(pt("l", "t")),
                    PathCmd::LnTo(pt("r", "t")),
                    PathCmd::LnTo(pt("r", "b")),
                    PathCmd::LnTo(pt("l", "b")),
                    PathCmd::Close,
                ],
            }],
            text_rect: Some([name("l"), name("t"), name("r"), name("b")]),
        }
    }
}

/// A supplier of preset definitions.
pub trait PresetSource {
    /// The definition of preset `name` (`prstGeom prst`), if known.
    fn get(&self, name: &str) -> Option<&ShapeDef>;
}

/// A preset source over an owned map; for tests and for presets supplied by the host.
#[derive(Debug, Clone, Default)]
pub struct MapPresets(pub HashMap<String, ShapeDef>);

impl PresetSource for MapPresets {
    fn get(&self, name: &str) -> Option<&ShapeDef> {
        self.0.get(name)
    }
}

/// The built-in presets: the hand-written `rect` plus whatever the generated table holds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Builtin;

fn builtin_map() -> &'static HashMap<String, ShapeDef> {
    static MAP: OnceLock<HashMap<String, ShapeDef>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m: HashMap<String, ShapeDef> = presets::table()
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect();
        m.entry("rect".to_owned()).or_insert_with(ShapeDef::rect);
        m
    })
}

impl PresetSource for Builtin {
    fn get(&self, name: &str) -> Option<&ShapeDef> {
        builtin_map().get(name)
    }
}

fn fallback_rect() -> &'static ShapeDef {
    static RECT: OnceLock<ShapeDef> = OnceLock::new();
    RECT.get_or_init(ShapeDef::rect)
}

/// Looks `name` up; an unknown preset yields the rectangle and a diagnostic.
pub fn resolve_preset<'a>(src: &'a dyn PresetSource, name: &str) -> (&'a ShapeDef, Option<String>) {
    match src.get(name) {
        Some(d) => (d, None),
        None => (
            fallback_rect(),
            Some(format!(
                "preset geometry `{name}` unavailable: drawn as rect"
            )),
        ),
    }
}

/// An axis-aligned rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub l: f64,
    pub t: f64,
    pub r: f64,
    pub b: f64,
}

/// A path in shape space (same unit as the box handed to [`evaluate`]).
#[derive(Debug, Clone, PartialEq)]
pub struct EvalPath {
    pub segs: Vec<Seg>,
    pub fill: PathFill,
    pub stroke: bool,
}

/// The result of evaluating a definition for a box.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Evaluated {
    pub paths: Vec<EvalPath>,
    pub text_rect: Option<Rect>,
    /// Problems met while evaluating (unknown names, bad formulas). Values fall back to 0.
    pub diagnostics: Vec<String>,
}

/// Evaluates `def` for a `w` by `h` box. `overrides` are `(adjust name, formula)` pairs from
/// the instance's `a:avLst`; unknown names are ignored, bad formulas keep the default.
pub fn evaluate(def: &ShapeDef, overrides: &[(String, String)], w: f64, h: f64) -> Evaluated {
    let mut out = Evaluated::default();
    let mut scope = Scope::new(w, h);
    for a in &def.adjust {
        let mut v = None;
        if let Some((_, f)) = overrides.iter().rev().find(|(n, _)| *n == a.name) {
            match Formula::parse(f).and_then(|f| f.eval(&scope)) {
                Ok(x) => v = Some(x),
                Err(e) => out
                    .diagnostics
                    .push(format!("adjust `{}` override `{f}`: {e}", a.name)),
            }
        }
        let v = v.unwrap_or_else(|| guide_value(&a.fmla, &a.name, &scope, &mut out.diagnostics));
        scope.set(&a.name, v);
    }
    for g in &def.guides {
        let v = guide_value(&g.fmla, &g.name, &scope, &mut out.diagnostics);
        scope.set(&g.name, v);
    }
    for p in &def.paths {
        let sx = if p.w > 0 { w / p.w as f64 } else { 1.0 };
        let sy = if p.h > 0 { h / p.h as f64 } else { 1.0 };
        let mut segs = Vec::new();
        let mut cur = P::default();
        let mut start = P::default();
        let diags = &mut out.diagnostics;
        let mut num = |o: &Operand| -> f64 {
            scope.operand(o).unwrap_or_else(|e| {
                diags.push(e);
                0.0
            })
        };
        for c in &p.cmds {
            match c {
                PathCmd::MoveTo(a) => {
                    cur = P::new(num(&a.x) * sx, num(&a.y) * sy);
                    start = cur;
                    segs.push(Seg::Move(cur));
                }
                PathCmd::LnTo(a) => {
                    cur = P::new(num(&a.x) * sx, num(&a.y) * sy);
                    segs.push(Seg::Line(cur));
                }
                PathCmd::ArcTo {
                    wr,
                    hr,
                    st_ang,
                    sw_ang,
                } => {
                    // The arc is built in path space, then scaled with the rest of the path.
                    let from = P::new(cur.x / sx, cur.y / sy);
                    let (arc, end) = path::arc_to(from, num(wr), num(hr), num(st_ang), num(sw_ang));
                    let f = |q: P| P::new(q.x * sx, q.y * sy);
                    for s in arc {
                        segs.push(s.map(&f));
                    }
                    cur = f(end);
                }
                PathCmd::QuadBezTo(a, b) => {
                    cur = P::new(num(&b.x) * sx, num(&b.y) * sy);
                    segs.push(Seg::Quad(P::new(num(&a.x) * sx, num(&a.y) * sy), cur));
                }
                PathCmd::CubicBezTo(a, b, e) => {
                    cur = P::new(num(&e.x) * sx, num(&e.y) * sy);
                    segs.push(Seg::Cubic(
                        P::new(num(&a.x) * sx, num(&a.y) * sy),
                        P::new(num(&b.x) * sx, num(&b.y) * sy),
                        cur,
                    ));
                }
                PathCmd::Close => {
                    segs.push(Seg::Close);
                    cur = start;
                }
            }
        }
        out.paths.push(EvalPath {
            segs,
            fill: p.fill,
            stroke: p.stroke,
        });
    }
    if let Some([l, t, r, b]) = &def.text_rect {
        let num = |o: &Operand| scope.operand(o).unwrap_or(0.0);
        out.text_rect = Some(Rect {
            l: num(l),
            t: num(t),
            r: num(r),
            b: num(b),
        });
    }
    out
}

fn guide_value(f: &Formula, name: &str, scope: &Scope, diags: &mut Vec<String>) -> f64 {
    f.eval(scope).unwrap_or_else(|e| {
        diags.push(format!("guide `{name}`: {e}"));
        0.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::cust_geom_from_str;

    fn def(xml: &str) -> ShapeDef {
        cust_geom_from_str(xml).unwrap()
    }

    #[test]
    fn rect_evaluates_to_a_closed_box() {
        let e = evaluate(&ShapeDef::rect(), &[], 200.0, 100.0);
        let p = &e.paths[0];
        assert_eq!(
            p.segs,
            vec![
                Seg::Move(P::new(0.0, 0.0)),
                Seg::Line(P::new(200.0, 0.0)),
                Seg::Line(P::new(200.0, 100.0)),
                Seg::Line(P::new(0.0, 100.0)),
                Seg::Close
            ]
        );
        assert_eq!(
            e.text_rect,
            Some(Rect {
                l: 0.0,
                t: 0.0,
                r: 200.0,
                b: 100.0
            })
        );
    }

    #[test]
    fn adjust_default_and_override_drive_guides() {
        // Own definition: a rectangle whose right edge sits at adj/100000 of the width.
        let d = def(r#"<a:custGeom>
            <a:avLst><a:gd name="adj" fmla="val 50000"/></a:avLst>
            <a:gdLst><a:gd name="x1" fmla="*/ w adj 100000"/></a:gdLst>
            <a:pathLst><a:path><a:moveTo><a:pt x="l" y="t"/></a:moveTo>
              <a:lnTo><a:pt x="x1" y="t"/></a:lnTo><a:lnTo><a:pt x="x1" y="b"/></a:lnTo><a:close/></a:path></a:pathLst>
            </a:custGeom>"#);
        let e = evaluate(&d, &[], 400.0, 100.0);
        assert_eq!(e.paths[0].segs[1], Seg::Line(P::new(200.0, 0.0)));
        let ov = [("adj".to_owned(), "val 25000".to_owned())];
        let e = evaluate(&d, &ov, 400.0, 100.0);
        assert_eq!(e.paths[0].segs[1], Seg::Line(P::new(100.0, 0.0)));
        assert!(e.diagnostics.is_empty());
    }

    #[test]
    fn path_coordinate_space_is_scaled_to_the_box() {
        let d = def(r#"<a:custGeom><a:pathLst><a:path w="10" h="10">
            <a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="10" y="5"/></a:lnTo>
            </a:path></a:pathLst></a:custGeom>"#);
        let e = evaluate(&d, &[], 200.0, 100.0);
        assert_eq!(e.paths[0].segs[1], Seg::Line(P::new(200.0, 50.0)));
    }

    #[test]
    fn arc_follows_current_point_and_close_returns_to_start() {
        // A circle of radius 50 drawn as arc from the east point, in a 100x100 box.
        let d = def(r#"<a:custGeom><a:pathLst><a:path>
            <a:moveTo><a:pt x="r" y="vc"/></a:moveTo>
            <a:arcTo wR="wd2" hR="hd2" stAng="0" swAng="21600000"/>
            <a:close/><a:moveTo><a:pt x="5" y="5"/></a:moveTo><a:lnTo><a:pt x="6" y="6"/></a:lnTo></a:path>
            </a:pathLst></a:custGeom>"#);
        let e = evaluate(&d, &[], 100.0, 100.0);
        let segs = &e.paths[0].segs;
        assert_eq!(segs.len(), 1 + 4 + 1 + 2);
        let Seg::Cubic(_, _, last) = segs[4] else {
            panic!()
        };
        assert!((last.x - 100.0).abs() < 1e-9 && (last.y - 50.0).abs() < 1e-9);
        for s in &segs[1..5] {
            let q = s.end().unwrap();
            assert!(((q.x - 50.0).hypot(q.y - 50.0) - 50.0).abs() < 1e-9);
        }
    }

    #[test]
    fn bezier_commands_and_flags() {
        let d = def(r#"<a:custGeom><a:pathLst><a:path fill="none" stroke="0">
            <a:moveTo><a:pt x="0" y="0"/></a:moveTo>
            <a:quadBezTo><a:pt x="1" y="2"/><a:pt x="3" y="4"/></a:quadBezTo>
            <a:cubicBezTo><a:pt x="1" y="1"/><a:pt x="2" y="2"/><a:pt x="3" y="3"/></a:cubicBezTo>
            </a:path></a:pathLst></a:custGeom>"#);
        let e = evaluate(&d, &[], 10.0, 10.0);
        assert_eq!(e.paths[0].fill, PathFill::None);
        assert!(!e.paths[0].stroke);
        assert_eq!(
            e.paths[0].segs[1],
            Seg::Quad(P::new(1.0, 2.0), P::new(3.0, 4.0))
        );
        assert!(matches!(e.paths[0].segs[2], Seg::Cubic(..)));
    }

    #[test]
    fn bad_names_are_diagnosed_and_zeroed() {
        let d = def(
            r#"<a:custGeom><a:gdLst><a:gd name="g" fmla="val nope"/></a:gdLst>
            <a:pathLst><a:path><a:moveTo><a:pt x="g" y="g"/></a:moveTo></a:path></a:pathLst></a:custGeom>"#,
        );
        let e = evaluate(&d, &[], 10.0, 10.0);
        assert_eq!(e.paths[0].segs[0], Seg::Move(P::new(0.0, 0.0)));
        assert_eq!(e.diagnostics.len(), 1);
    }

    #[test]
    fn unknown_preset_falls_back_to_rect_with_a_gap() {
        let (d, gap) = resolve_preset(&Builtin, "someUnknownShape");
        assert_eq!(d, &ShapeDef::rect());
        assert!(gap.unwrap().contains("someUnknownShape"));
        let (_, none) = resolve_preset(&Builtin, "rect");
        assert!(none.is_none());
    }

    #[test]
    fn map_source_supplies_presets() {
        let mut m = MapPresets::default();
        m.0.insert("mine".into(), ShapeDef::rect());
        assert!(m.get("mine").is_some());
        assert!(resolve_preset(&m, "mine").1.is_none());
    }
}
