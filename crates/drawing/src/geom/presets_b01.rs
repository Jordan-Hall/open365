//! Preset batch b01, rebuilt clean-room from Word measurements (docs/architecture/drawing.md).
//!
//! Source: `fixtures/reference/drawing/presets/b01.{docx,pdf,probes.json,handles.json,paths.json}`,
//! made by `tools/office/make-preset-probes.ps1` (Word's PDF export of one preset per page at four
//! sizes, five all-adjust settings and one scan page per adjust). Every formula below was derived
//! from those measured paths and handle defaults only; no other project's preset table and no copy
//! of Ecma's `presetShapeDefinitions.xml` was opened. Guide names are our own.
//!
//! `triangle` and `rect` are in the pilot (`presets.rs`) and are not repeated here.
//!
//! Common findings:
//! - An adjust that is a size (inset, radius) is clamped with `pin` before use; Word shows the
//!   nearest limit for -50000 and 400000.
//! - The `hf` and `vf` adjusts of the regular polygons and stars (default `1 / sin` of the
//!   extreme vertex angle) scale the circumscribed ellipse horizontally and vertically and are
//!   NOT clamped (a negative value mirrors the shape, 400000 grows it far outside the box).
//! - The polygons and stars are built on an ellipse of radii `wd2 * hf` and `hd2 * vf`; for an
//!   odd vertex count the apex sits on the top edge and the ellipse centre is `radius_y` below
//!   it, for an even count the centre is the box centre.

use super::ShapeDef;
use crate::parse::cust_geom_from_str;

fn parse(xml: &str) -> Option<ShapeDef> {
    cust_geom_from_str(xml).ok()
}

/// Angle in 60000ths of a degree for `num / den` of a full turn.
fn ang(num: f64, den: f64) -> String {
    format!("{:.3}", 21_600_000.0 * num / den)
}

/// `line`: no adjusts. Measured at four sizes: one open segment from the top left to the bottom
/// right corner (not closed, not filled).
fn line() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `lineInv`: no adjusts. Measured: one open segment from the bottom left to the top right corner.
fn line_inv() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path fill="none">
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `rtTriangle`: no adjusts. Measured: closed, left bottom, left top, right bottom, at every size.
fn rt_triangle() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo><a:lnTo><a:pt x="l" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:close/>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `diamond`: no adjusts. Measured: the four edge midpoints at every size, closed.
fn diamond() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo><a:lnTo><a:pt x="hc" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="vc"/></a:lnTo><a:lnTo><a:pt x="hc" y="b"/></a:lnTo><a:close/>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `parallelogram`: one adjust, default 25000. Measured: the top edge is shifted right by
/// `ss * adj / 100000` (36 pt on 144 and 216x144, 24 pt on 288x96 and 96x288); the bottom edge runs
/// from the left edge to `w - shift`. 100000 gives 144 pt on 216x144, 400000 gives the full width
/// (216 pt), 0 and -50000 give a rectangle: the adjust is clamped to 0..`100000 * w / ss`.
fn parallelogram() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="maxA" fmla="*/ 100000 w ss"/>
    <a:gd name="aPin" fmla="pin 0 adj maxA"/>
    <a:gd name="off" fmla="*/ ss aPin 100000"/>
    <a:gd name="xr" fmla="+- r 0 off"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo><a:lnTo><a:pt x="off" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo><a:lnTo><a:pt x="xr" y="b"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `trapezoid`: one adjust, default 25000. Measured: wide base on the bottom, the top edge inset
/// on both sides by `ss * adj / 100000` (36 pt on 216x144). 100000 and 400000 give an apex at the
/// top middle, 0 and -50000 a rectangle: clamped to 0..`50000 * w / ss`.
fn trapezoid() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="maxA" fmla="*/ 50000 w ss"/>
    <a:gd name="aPin" fmla="pin 0 adj maxA"/>
    <a:gd name="inset" fmla="*/ ss aPin 100000"/>
    <a:gd name="xr" fmla="+- r 0 inset"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo><a:lnTo><a:pt x="inset" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="t"/></a:lnTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `nonIsoscelesTrapezoid`: two adjusts, default 25000 each. Measured: like `trapezoid` but the
/// top edge is inset on the left by `ss * adj1 / 100000` and on the right by `ss * adj2 / 100000`
/// (scan pages: adj1 = 0 leaves the top left corner on the edge, adj2 = 0 the top right one). Each is
/// clamped to 0..`50000 * w / ss` (both at 100000 or more meet at the top middle). Not measured:
/// whether one adjust's limit depends on the other (the probes set both to the same values).
fn non_isosceles_trapezoid() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="maxA" fmla="*/ 50000 w ss"/>
    <a:gd name="aPin1" fmla="pin 0 adj1 maxA"/>
    <a:gd name="aPin2" fmla="pin 0 adj2 maxA"/>
    <a:gd name="inL" fmla="*/ ss aPin1 100000"/>
    <a:gd name="inR" fmla="*/ ss aPin2 100000"/>
    <a:gd name="xr" fmla="+- r 0 inR"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo><a:lnTo><a:pt x="inL" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="t"/></a:lnTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// Regular polygon with an odd vertex count (`pentagon` n=5, `heptagon` n=7).
///
/// Measured (pentagon and heptagon): two adjusts named `hf` and `vf`, defaults 1.05146 / 1.10557
/// (pentagon) and 1.02572 / 1.0521 (heptagon), i.e. the values at which the outline just fits the
/// box (`1 / sin` of the widest vertex angle horizontally, `2 / (1 + cos)` of the lowest vertex
/// vertically). Vertices lie on an ellipse with radii `rx = wd2 * hf`, `ry = hd2 * vf` at angles
/// `k * 360 / n` from the top: x = `hc + rx sin a`, y = `ry - ry cos a` (apex on the top edge, so
/// the ellipse centre is `ry` below the top). Neither adjust is clamped: at 30000, 100000, -50000
/// (mirrored) and 400000 the outline follows the same formula. With hf = 0 or vf = 0 the shape
/// collapses to a vertical or horizontal line (scan pages).
fn odd_polygon(n: u32, hf: u32, vf: u32) -> Option<ShapeDef> {
    let mut gd = String::new();
    let mut pts = String::new();
    gd.push_str(
        r#"<a:gd name="rx" fmla="*/ wd2 hf 100000"/><a:gd name="ry" fmla="*/ hd2 vf 100000"/>"#,
    );
    for k in 0..n {
        let a = ang(f64::from(k), f64::from(n));
        gd.push_str(&format!(
            r#"<a:gd name="dx{k}" fmla="sin rx {a}"/><a:gd name="dy{k}" fmla="cos ry {a}"/><a:gd name="x{k}" fmla="+- hc dx{k} 0"/><a:gd name="y{k}" fmla="+- ry 0 dy{k}"/>"#
        ));
        let tag = if k == 0 { "moveTo" } else { "lnTo" };
        pts.push_str(&format!(r#"<a:{tag}><a:pt x="x{k}" y="y{k}"/></a:{tag}>"#));
    }
    parse(&format!(
        r#"<a:custGeom><a:avLst><a:gd name="hf" fmla="val {hf}"/><a:gd name="vf" fmla="val {vf}"/></a:avLst>
  <a:gdLst>{gd}</a:gdLst><a:pathLst><a:path>{pts}<a:close/></a:path></a:pathLst></a:custGeom>"#
    ))
}

/// `hexagon`: two adjusts, `adj` (default 25000) and `vf` (default 115470). Measured: six vertices;
/// left and right points on the middle line, the other four on the top and bottom lines inset by
/// `x1 = ss * adj / 100000` (clamped to 0..`50000 * w / ss`: 100000 and 400000 give half the width,
/// 0 and -50000 give no inset). The vertical offset of the four points from the middle line is
/// `hd2 * vf / 100000 * sin 60 deg` (72 pt at the default on 144 high, 62.35 at vf = 100000,
/// 18.71 at 30000, 249.4 at 400000, mirrored at -50000): `vf` is not clamped.
fn hexagon() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 25000"/><a:gd name="vf" fmla="val 115470"/></a:avLst>
  <a:gdLst>
    <a:gd name="maxA" fmla="*/ 50000 w ss"/>
    <a:gd name="aPin" fmla="pin 0 adj maxA"/>
    <a:gd name="x1" fmla="*/ ss aPin 100000"/>
    <a:gd name="x2" fmla="+- r 0 x1"/>
    <a:gd name="ryf" fmla="*/ hd2 vf 100000"/>
    <a:gd name="dy" fmla="sin ryf 3600000"/>
    <a:gd name="y1" fmla="+- vc 0 dy"/>
    <a:gd name="y2" fmla="+- vc dy 0"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo><a:lnTo><a:pt x="x1" y="y1"/></a:lnTo>
    <a:lnTo><a:pt x="x2" y="y1"/></a:lnTo><a:lnTo><a:pt x="r" y="vc"/></a:lnTo>
    <a:lnTo><a:pt x="x2" y="y2"/></a:lnTo><a:lnTo><a:pt x="x1" y="y2"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `octagon`: one adjust, default 29289. Measured: eight vertices, corners cut by
/// `ss * adj / 100000` on both axes (42.18 pt on 144, 28.12 on 96 short side), clamped to
/// 0..50000 (at 100000 and 400000 the cut is half the short side; 0 and -50000 give a rectangle with
/// doubled corner points).
fn octagon() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 29289"/></a:avLst>
  <a:gdLst>
    <a:gd name="aPin" fmla="pin 0 adj 50000"/>
    <a:gd name="c" fmla="*/ ss aPin 100000"/>
    <a:gd name="x2" fmla="+- r 0 c"/>
    <a:gd name="y2" fmla="+- b 0 c"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="c"/></a:moveTo><a:lnTo><a:pt x="c" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="x2" y="t"/></a:lnTo><a:lnTo><a:pt x="r" y="c"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="y2"/></a:lnTo><a:lnTo><a:pt x="x2" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="c" y="b"/></a:lnTo><a:lnTo><a:pt x="l" y="y2"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `decagon`: one adjust named `vf`, default 1.05146 (105146). Measured: ten vertices at angles
/// `k * 36 deg` from the right (so left and right points sit on the middle line, no vertex on the
/// top edge); horizontally on the box ellipse (`x = hc + wd2 cos a`), vertically on an ellipse of
/// radius `hd2 * vf / 100000` (`y = vc - ry sin a`). `vf` is not clamped (mirrors at -50000, grows
/// at 400000, flat line at 0).
fn decagon() -> Option<ShapeDef> {
    let mut gd = String::from(r#"<a:gd name="ry" fmla="*/ hd2 vf 100000"/>"#);
    let mut pts = String::new();
    for k in 0..10u32 {
        let a = ang(f64::from(k), 10.0);
        gd.push_str(&format!(
            r#"<a:gd name="dx{k}" fmla="cos wd2 {a}"/><a:gd name="dy{k}" fmla="sin ry {a}"/><a:gd name="x{k}" fmla="+- hc dx{k} 0"/><a:gd name="y{k}" fmla="+- vc 0 dy{k}"/>"#
        ));
        let tag = if k == 0 { "moveTo" } else { "lnTo" };
        pts.push_str(&format!(r#"<a:{tag}><a:pt x="x{k}" y="y{k}"/></a:{tag}>"#));
    }
    parse(&format!(
        r#"<a:custGeom><a:avLst><a:gd name="vf" fmla="val 105146"/></a:avLst>
  <a:gdLst>{gd}</a:gdLst><a:pathLst><a:path>{pts}<a:close/></a:path></a:pathLst></a:custGeom>"#
    ))
}

/// `dodecagon`: no adjusts. Measured at four sizes: twelve vertices, on each side the corner cut is
/// 0.13397 of the width horizontally and 0.13397 of the height vertically (`wd2 * tan 15 deg`,
/// `hd2 * tan 15 deg`: 19.29 pt on 144x144); the edge points on the box edges are at 0.36603 of the
/// size from the corner (`hc - cut`).
fn dodecagon() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/>
  <a:gdLst>
    <a:gd name="x1" fmla="tan wd2 900000"/>
    <a:gd name="x2" fmla="+- hc 0 x1"/>
    <a:gd name="x3" fmla="+- hc x1 0"/>
    <a:gd name="x4" fmla="+- r 0 x1"/>
    <a:gd name="y1" fmla="tan hd2 900000"/>
    <a:gd name="y2" fmla="+- vc 0 y1"/>
    <a:gd name="y3" fmla="+- vc y1 0"/>
    <a:gd name="y4" fmla="+- b 0 y1"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="y2"/></a:moveTo><a:lnTo><a:pt x="x1" y="y1"/></a:lnTo>
    <a:lnTo><a:pt x="x2" y="t"/></a:lnTo><a:lnTo><a:pt x="x3" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="x4" y="y1"/></a:lnTo><a:lnTo><a:pt x="r" y="y2"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="y3"/></a:lnTo><a:lnTo><a:pt x="x4" y="y4"/></a:lnTo>
    <a:lnTo><a:pt x="x3" y="b"/></a:lnTo><a:lnTo><a:pt x="x2" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="x1" y="y4"/></a:lnTo><a:lnTo><a:pt x="l" y="y3"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// Which extra adjusts a star has, with Word's defaults (1/100000).
pub(super) struct StarSpec {
    pub(super) n: u32,
    pub(super) adj: u32,
    pub(super) hf: Option<u32>,
    pub(super) vf: Option<u32>,
}

/// `starN`: n tips. Measured (star4 .. star24): `adj` is the inner radius as a fraction of the
/// outer one, `adj / 50000`, clamped to 0..50000 (at 100000 and 400000 the inner vertices sit on
/// the outer radius; at 0 and -50000 on the centre). Outer vertices at `k * 360 / n` from the
/// top, inner ones halfway between. Defaults: star4 12500, star5 19098, star6 28868, star7 34601,
/// star8 37500, star10 42533, star12 37500, star16 37500, star24 37500. The shapes with an odd
/// count (5, 7) have `hf` and `vf` (apex on the top edge, as for the odd polygons), star6 and
/// star10 only `hf` (`1 / sin` of the widest tip angle: 1.1547 and 1.05146; vertically the tips
/// reach the box), the others none (the tips reach the box). `hf`/`vf` scale the radii of the
/// ellipse (`wd2 * hf`, `hd2 * vf`) and are not clamped.
pub(super) fn star(s: &StarSpec) -> Option<ShapeDef> {
    let StarSpec { n, adj, hf, vf } = *s;
    let mut av = format!(r#"<a:gd name="adj" fmla="val {adj}"/>"#);
    if let Some(v) = hf {
        av.push_str(&format!(r#"<a:gd name="hf" fmla="val {v}"/>"#));
    }
    if let Some(v) = vf {
        av.push_str(&format!(r#"<a:gd name="vf" fmla="val {v}"/>"#));
    }
    let rx = if hf.is_some() {
        "*/ wd2 hf 100000"
    } else {
        "val wd2"
    };
    let (ry, cy) = if vf.is_some() {
        ("*/ hd2 vf 100000", "ryO")
    } else {
        ("val hd2", "vc")
    };
    let mut gd = format!(
        r#"<a:gd name="rxO" fmla="{rx}"/><a:gd name="ryO" fmla="{ry}"/><a:gd name="cy" fmla="val {cy}"/><a:gd name="aPin" fmla="pin 0 adj 50000"/><a:gd name="rxI" fmla="*/ rxO aPin 50000"/><a:gd name="ryI" fmla="*/ ryO aPin 50000"/>"#
    );
    let mut pts = String::new();
    let mut put = |i: u32, a: String, rxn: &str, ryn: &str| {
        gd.push_str(&format!(
            r#"<a:gd name="dx{i}" fmla="sin {rxn} {a}"/><a:gd name="dy{i}" fmla="cos {ryn} {a}"/><a:gd name="x{i}" fmla="+- hc dx{i} 0"/><a:gd name="y{i}" fmla="+- cy 0 dy{i}"/>"#
        ));
        let tag = if i == 0 { "moveTo" } else { "lnTo" };
        pts.push_str(&format!(r#"<a:{tag}><a:pt x="x{i}" y="y{i}"/></a:{tag}>"#));
    };
    for k in 0..n {
        put(2 * k, ang(f64::from(k), f64::from(n)), "rxO", "ryO");
        put(
            2 * k + 1,
            ang(f64::from(k) + 0.5, f64::from(n)),
            "rxI",
            "ryI",
        );
    }
    parse(&format!(
        r#"<a:custGeom><a:avLst>{av}</a:avLst><a:gdLst>{gd}</a:gdLst><a:pathLst><a:path>{pts}<a:close/></a:path></a:pathLst></a:custGeom>"#
    ))
}

/// The b01 entries (those not already in the pilot).
pub fn entries() -> Vec<(&'static str, ShapeDef)> {
    let st = |n, adj, hf, vf| star(&StarSpec { n, adj, hf, vf });
    let all = [
        ("line", line()),
        ("lineInv", line_inv()),
        ("rtTriangle", rt_triangle()),
        ("diamond", diamond()),
        ("parallelogram", parallelogram()),
        ("trapezoid", trapezoid()),
        ("nonIsoscelesTrapezoid", non_isosceles_trapezoid()),
        ("pentagon", odd_polygon(5, 105_146, 110_557)),
        ("hexagon", hexagon()),
        ("heptagon", odd_polygon(7, 102_572, 105_210)),
        ("octagon", octagon()),
        ("decagon", decagon()),
        ("dodecagon", dodecagon()),
        ("star4", st(4, 12_500, None, None)),
        ("star5", st(5, 19_098, Some(105_146), Some(110_557))),
        ("star6", st(6, 28_868, Some(115_470), None)),
        ("star7", st(7, 34_601, Some(102_572), Some(105_210))),
        ("star8", st(8, 37_500, None, None)),
        ("star10", st(10, 42_533, Some(105_146), None)),
        ("star12", st(12, 37_500, None, None)),
        ("star16", st(16, 37_500, None, None)),
        ("star24", st(24, 37_500, None, None)),
    ];
    all.into_iter()
        .filter_map(|(n, d)| d.map(|d| (n, d)))
        .collect()
}
