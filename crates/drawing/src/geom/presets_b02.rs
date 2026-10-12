//! Preset batch b02, rebuilt clean-room from Word measurements (docs/architecture/drawing.md).
//!
//! Source: `fixtures/reference/drawing/presets/b02.{docx,pdf,probes.json,handles.json,paths.json}`,
//! made by `tools/office/make-preset-probes.ps1` (Word's PDF export of one preset per page at four
//! sizes, five all-adjust settings and one scan page per adjust). Every formula below was derived
//! from those measured paths and handle defaults only; no other project's preset table and no copy
//! of Ecma's `presetShapeDefinitions.xml` was opened. Guide names are our own.
//!
//! `roundRect`, `ellipse` and `rightArrow` are in the pilot (`presets.rs`) and are not repeated.
//!
//! Common findings:
//! - Size-like adjusts (radius, snip, inset) are fractions of the short side `ss` and are clamped
//!   with `pin` (Word shows the nearest limit for -50000 and 400000).
//! - Angle adjusts are clamped to `0..21599999` (a negative angle behaves as 0; 400000 is 6.67 deg).
//!   Arc sweeps use the visual (radius-line) angle; a sweep of zero means a full turn. A full turn is
//!   written as 21600000 (the arc converter sweeps a full turn).

use super::ShapeDef;
use super::presets_b01::{StarSpec, star};
use crate::parse::cust_geom_from_str;

fn parse(xml: &str) -> Option<ShapeDef> {
    cust_geom_from_str(xml).ok()
}

/// `star32`: one adjust, default 37500. Same construction as the other stars (see `presets_b01`):
/// inner radius `adj / 50000` of the outer, clamped 0..50000, 32 outer vertices every 11.25 degrees
/// from the top (measured at 144x144 and 216x144 with adj 30000 and 100000).
fn star32() -> Option<ShapeDef> {
    star(&StarSpec {
        n: 32,
        adj: 37_500,
        hf: None,
        vf: None,
    })
}

/// `round1Rect`: one adjust, default 16667. Measured: only the top right corner is rounded, with
/// radius `ss * adj / 100000` (24 pt on 144x144, 16 pt on 288x96), clamped to 0..50000 (at 100000
/// and 400000 the radius is 72 pt on 216x144; at 0 and -50000 a plain rectangle).
fn round1_rect() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 16667"/></a:avLst>
  <a:gdLst>
    <a:gd name="aPin" fmla="pin 0 adj 50000"/>
    <a:gd name="rad" fmla="*/ ss aPin 100000"/>
    <a:gd name="xr" fmla="+- r 0 rad"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="xr" y="t"/></a:lnTo>
    <a:arcTo wR="rad" hR="rad" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `round2SameRect`: two adjusts, defaults 16667 and 0. Measured: `adj1` rounds both top corners,
/// `adj2` both bottom corners, each with radius `ss * adj / 100000` clamped to 0..50000 (scan pages:
/// adj2 = 0 leaves square bottom corners; both at 100000 give 72 pt on 216x144, the two arcs
/// meeting). Not measured: any limit tying the two radii together.
fn round2_same_rect() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 16667"/><a:gd name="adj2" fmla="val 0"/></a:avLst>
  <a:gdLst>
    <a:gd name="a1" fmla="pin 0 adj1 50000"/>
    <a:gd name="a2" fmla="pin 0 adj2 50000"/>
    <a:gd name="rt" fmla="*/ ss a1 100000"/>
    <a:gd name="rb" fmla="*/ ss a2 100000"/>
    <a:gd name="xrt" fmla="+- r 0 rt"/>
    <a:gd name="ybr" fmla="+- b 0 rb"/>
    <a:gd name="xrb" fmla="+- r 0 rb"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="rt" y="t"/></a:moveTo><a:lnTo><a:pt x="xrt" y="t"/></a:lnTo>
    <a:arcTo wR="rt" hR="rt" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="ybr"/></a:lnTo>
    <a:arcTo wR="rb" hR="rb" stAng="0" swAng="cd4"/>
    <a:lnTo><a:pt x="rb" y="b"/></a:lnTo>
    <a:arcTo wR="rb" hR="rb" stAng="cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="l" y="rt"/></a:lnTo>
    <a:arcTo wR="rt" hR="rt" stAng="cd2" swAng="cd4"/>
    <a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `round2DiagRect`: two adjusts, defaults 16667 and 0. Measured: `adj1` rounds the top left and
/// bottom right corners, `adj2` the top right and bottom left ones (default page: top left and bottom
/// right rounded; scan with adj2 = 0 the same), radius `ss * adj / 100000`, each clamped to 0..50000.
fn round2_diag_rect() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 16667"/><a:gd name="adj2" fmla="val 0"/></a:avLst>
  <a:gdLst>
    <a:gd name="a1" fmla="pin 0 adj1 50000"/>
    <a:gd name="a2" fmla="pin 0 adj2 50000"/>
    <a:gd name="r1" fmla="*/ ss a1 100000"/>
    <a:gd name="r2" fmla="*/ ss a2 100000"/>
    <a:gd name="xr2" fmla="+- r 0 r2"/>
    <a:gd name="yb1" fmla="+- b 0 r1"/>
    <a:gd name="xr1" fmla="+- r 0 r1"/>
    <a:gd name="yb2" fmla="+- b 0 r2"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="r1" y="t"/></a:moveTo><a:lnTo><a:pt x="xr2" y="t"/></a:lnTo>
    <a:arcTo wR="r2" hR="r2" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="yb1"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="0" swAng="cd4"/>
    <a:lnTo><a:pt x="r2" y="b"/></a:lnTo>
    <a:arcTo wR="r2" hR="r2" stAng="cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="l" y="r1"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="cd2" swAng="cd4"/>
    <a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `snipRoundRect`: two adjusts, both default 16667. Measured: the top left corner is rounded with
/// radius `ss * adj1 / 100000`, the top right corner is cut with a 45 degree snip of
/// `ss * adj2 / 100000`; the bottom corners stay square. Each is clamped to 0..50000 (scans: adj1 = 0
/// gives a square top left, adj2 = 0 a square top right).
fn snip_round_rect() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 16667"/><a:gd name="adj2" fmla="val 16667"/></a:avLst>
  <a:gdLst>
    <a:gd name="a1" fmla="pin 0 adj1 50000"/>
    <a:gd name="a2" fmla="pin 0 adj2 50000"/>
    <a:gd name="rad" fmla="*/ ss a1 100000"/>
    <a:gd name="snip" fmla="*/ ss a2 100000"/>
    <a:gd name="xs" fmla="+- r 0 snip"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="rad"/></a:moveTo>
    <a:arcTo wR="rad" hR="rad" stAng="cd2" swAng="cd4"/>
    <a:lnTo><a:pt x="xs" y="t"/></a:lnTo><a:lnTo><a:pt x="r" y="snip"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `snip1Rect`: one adjust, default 16667. Measured: only the top right corner is cut, by
/// `ss * adj / 100000` on both axes (16 pt on 288x96), clamped to 0..50000.
fn snip1_rect() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 16667"/></a:avLst>
  <a:gdLst>
    <a:gd name="aPin" fmla="pin 0 adj 50000"/>
    <a:gd name="snip" fmla="*/ ss aPin 100000"/>
    <a:gd name="xs" fmla="+- r 0 snip"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="xs" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="snip"/></a:lnTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `snip2SameRect`: two adjusts, defaults 16667 and 0. Measured: `adj1` cuts both top corners,
/// `adj2` both bottom corners, by `ss * adj / 100000`, each clamped to 0..50000.
fn snip2_same_rect() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 16667"/><a:gd name="adj2" fmla="val 0"/></a:avLst>
  <a:gdLst>
    <a:gd name="a1" fmla="pin 0 adj1 50000"/>
    <a:gd name="a2" fmla="pin 0 adj2 50000"/>
    <a:gd name="st" fmla="*/ ss a1 100000"/>
    <a:gd name="sb" fmla="*/ ss a2 100000"/>
    <a:gd name="xt" fmla="+- r 0 st"/>
    <a:gd name="xb" fmla="+- r 0 sb"/>
    <a:gd name="yb" fmla="+- b 0 sb"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="st" y="t"/></a:moveTo><a:lnTo><a:pt x="xt" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="st"/></a:lnTo><a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="xb" y="b"/></a:lnTo><a:lnTo><a:pt x="sb" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="yb"/></a:lnTo><a:lnTo><a:pt x="l" y="st"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `snip2DiagRect`: two adjusts, defaults 0 and 16667 (note the order). Measured: `adj1` cuts the
/// top left and bottom right corners, `adj2` the top right and bottom left ones (default page: only
/// the top right and bottom left corners are cut), by `ss * adj / 100000`, each clamped to 0..50000.
fn snip2_diag_rect() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 0"/><a:gd name="adj2" fmla="val 16667"/></a:avLst>
  <a:gdLst>
    <a:gd name="a1" fmla="pin 0 adj1 50000"/>
    <a:gd name="a2" fmla="pin 0 adj2 50000"/>
    <a:gd name="s1" fmla="*/ ss a1 100000"/>
    <a:gd name="s2" fmla="*/ ss a2 100000"/>
    <a:gd name="x2" fmla="+- r 0 s2"/>
    <a:gd name="x1" fmla="+- r 0 s1"/>
    <a:gd name="y1" fmla="+- b 0 s1"/>
    <a:gd name="y2" fmla="+- b 0 s2"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="s1" y="t"/></a:moveTo><a:lnTo><a:pt x="x2" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="s2"/></a:lnTo><a:lnTo><a:pt x="r" y="y1"/></a:lnTo>
    <a:lnTo><a:pt x="x1" y="b"/></a:lnTo><a:lnTo><a:pt x="s2" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="y2"/></a:lnTo><a:lnTo><a:pt x="l" y="s1"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `plaque`: one adjust, default 16667. Measured: a rectangle whose four corners are cut by concave
/// quarter circles (centre on the box corner) of radius `ss * adj / 100000` (16 pt on 288x96),
/// clamped to 0..50000 (at 100000 and 400000 the radius is 72 pt on 216x144, 0 and -50000 give a
/// plain rectangle).
fn plaque() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 16667"/></a:avLst>
  <a:gdLst>
    <a:gd name="aPin" fmla="pin 0 adj 50000"/>
    <a:gd name="c" fmla="*/ ss aPin 100000"/>
    <a:gd name="xr" fmla="+- r 0 c"/>
    <a:gd name="yb" fmla="+- b 0 c"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="c"/></a:moveTo>
    <a:arcTo wR="c" hR="c" stAng="cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="xr" y="t"/></a:lnTo>
    <a:arcTo wR="c" hR="c" stAng="cd2" swAng="-5400000"/>
    <a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
    <a:arcTo wR="c" hR="c" stAng="3cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="c" y="b"/></a:lnTo>
    <a:arcTo wR="c" hR="c" stAng="0" swAng="-5400000"/>
    <a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `teardrop`: one adjust, default 100000. Measured: three quadrants of the box ellipse (left
/// middle over the top, and from the right middle round the bottom back to the left middle) plus a
/// pointed corner at `T = (hc + wd2 * adj / 100000, vc - hd2 * adj / 100000)`; adj is clamped to
/// 0..200000 (at 400000 the tip is at (324, -72) on 216x144, 0 and -50000 put it on the centre).
/// The two sides to the tip are cubic parabolas: from the top middle P0 the control points are
/// `P0 + (dx/3, 0)` and `P0 + (2dx/3, dy/3)`, from the tip T to the right middle Q they are
/// `T + (2/3, 1/3) * (Q - T)` and `T + (1, 2/3) * (Q - T)` (checked at adj 30000, 100000, 400000).
fn teardrop() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 100000"/></a:avLst>
  <a:gdLst>
    <a:gd name="aPin" fmla="pin 0 adj 200000"/>
    <a:gd name="dx" fmla="*/ wd2 aPin 100000"/>
    <a:gd name="dy" fmla="*/ hd2 aPin 100000"/>
    <a:gd name="xt" fmla="+- hc dx 0"/>
    <a:gd name="yt" fmla="+- vc 0 dy"/>
    <a:gd name="dx3" fmla="*/ dx 1 3"/>
    <a:gd name="dx23" fmla="*/ dx 2 3"/>
    <a:gd name="c1x" fmla="+- hc dx3 0"/>
    <a:gd name="c2x" fmla="+- hc dx23 0"/>
    <a:gd name="c2y" fmla="*/ yt 1 3"/>
    <a:gd name="ex" fmla="+- r 0 xt"/>
    <a:gd name="ey" fmla="+- vc 0 yt"/>
    <a:gd name="ex23" fmla="*/ ex 2 3"/>
    <a:gd name="ey13" fmla="*/ ey 1 3"/>
    <a:gd name="ey23" fmla="*/ ey 2 3"/>
    <a:gd name="d1x" fmla="+- xt ex23 0"/>
    <a:gd name="d1y" fmla="+- yt ey13 0"/>
    <a:gd name="d2y" fmla="+- yt ey23 0"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd2" swAng="cd4"/>
    <a:cubicBezTo><a:pt x="c1x" y="t"/><a:pt x="c2x" y="c2y"/><a:pt x="xt" y="yt"/></a:cubicBezTo>
    <a:cubicBezTo><a:pt x="d1x" y="d1y"/><a:pt x="r" y="d2y"/><a:pt x="r" y="vc"/></a:cubicBezTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="0" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd4" swAng="cd4"/>
    <a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `homePlate` (pentagon arrow): one adjust, default 50000. Measured: a rectangle whose right end
/// is a point on the middle line; the point's run is `ss * adj / 100000` (72 pt on 144x144, 48 pt on
/// 288x96 and on 96x288), clamped to 0..`100000 * w / ss` (at 400000 the tip
/// base is on the left edge).
fn home_plate() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="maxA" fmla="*/ 100000 w ss"/>
    <a:gd name="aPin" fmla="pin 0 adj maxA"/>
    <a:gd name="run" fmla="*/ ss aPin 100000"/>
    <a:gd name="xb" fmla="+- r 0 run"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="xb" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="vc"/></a:lnTo><a:lnTo><a:pt x="xb" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `chevron`: one adjust, default 50000. Measured: like `homePlate` with a notch in the left edge
/// whose depth equals the point's run: the point at (r, vc), the notch at (`ss * adj / 100000`, vc);
/// clamped to 0..`100000 * w / ss` (at 400000 on 216x144 the notch reaches the right edge).
fn chevron() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="maxA" fmla="*/ 100000 w ss"/>
    <a:gd name="aPin" fmla="pin 0 adj maxA"/>
    <a:gd name="run" fmla="*/ ss aPin 100000"/>
    <a:gd name="xb" fmla="+- r 0 run"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="xb" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="vc"/></a:lnTo><a:lnTo><a:pt x="xb" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:lnTo><a:pt x="run" y="vc"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `pieWedge`: no adjusts. Measured at four sizes: a quarter of the box ellipse centred on the
/// bottom right corner (radii `w` and `h`), from the left bottom corner over the top right corner,
/// closed through the bottom right corner.
fn pie_wedge() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo>
    <a:arcTo wR="w" hR="h" stAng="cd2" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `pie`: two angle adjusts, `adj1` (start, default 0) and `adj2` (end, default 16200000, which
/// Word's object model reports as -90 degrees). Measured: the box ellipse from the start to the end
/// angle clockwise (y down; visual angles of the radius line), closed through the centre; default
/// is three quarters (right middle, over the bottom and left to the top middle). Both angles are
/// clamped to 0..21599999 (-50000 behaves as 0, an explicit -5400000 too); a sweep of zero or less
/// has 360 degrees added, so equal angles draw the full ellipse (adj 0/0, 30000/30000 and so on).
fn pie() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 0"/><a:gd name="adj2" fmla="val 16200000"/></a:avLst>
  <a:gdLst>
    <a:gd name="st" fmla="pin 0 adj1 21599999"/>
    <a:gd name="en" fmla="pin 0 adj2 21599999"/>
    <a:gd name="sw1" fmla="+- en 0 st"/>
    <a:gd name="sw2" fmla="+- sw1 21600000 0"/>
    <a:gd name="sw" fmla="?: sw1 sw1 sw2"/>
    <a:gd name="ca" fmla="cos hd2 st"/>
    <a:gd name="sa" fmla="sin wd2 st"/>
    <a:gd name="dx" fmla="cat2 wd2 ca sa"/>
    <a:gd name="dy" fmla="sat2 hd2 ca sa"/>
    <a:gd name="x1" fmla="+- hc dx 0"/>
    <a:gd name="y1" fmla="+- vc dy 0"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="x1" y="y1"/></a:moveTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="st" swAng="sw"/>
    <a:lnTo><a:pt x="hc" y="vc"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `blockArc`: three adjusts, `adj1` start angle (default 10800000, 180 degrees), `adj2` end angle
/// (default 0) and `adj3` thickness (default 25000). Measured: the outer box ellipse from the start
/// to the end angle clockwise (default: the upper half), a line to the inner ellipse at the end angle,
/// the inner arc back to the start angle, closed. The inner radii are the box radii minus
/// `ss * adj3 / 100000` on both axes (36 pt on 216x144 at the default), adj3 clamped to 0..50000
/// (at 100000 the inner vertical radius is 0). Angles as for `pie` (clamped, zero sweep = full turn).
/// The inner point at an angle is the ellipse point at that visual angle (so with a zero radius it
/// collapses onto the centre, as measured at adj3 = 100000).
///
/// Flat inner ellipse (`adj-all-100000` and `adj-all-400000`: inner vertical radius 0): Word draws
/// the inner arc as a line traced there and back, centred half a width left of the centre point;
/// the arc converter uses the visual angle as the parameter of a flat ellipse, which matches.
fn block_arc() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 10800000"/><a:gd name="adj2" fmla="val 0"/><a:gd name="adj3" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="st" fmla="pin 0 adj1 21599999"/>
    <a:gd name="en" fmla="pin 0 adj2 21599999"/>
    <a:gd name="a3" fmla="pin 0 adj3 50000"/>
    <a:gd name="dr" fmla="*/ ss a3 100000"/>
    <a:gd name="iw" fmla="+- wd2 0 dr"/>
    <a:gd name="ih" fmla="+- hd2 0 dr"/>
    <a:gd name="sw1" fmla="+- en 0 st"/>
    <a:gd name="sw2" fmla="+- sw1 21600000 0"/>
    <a:gd name="sw" fmla="?: sw1 sw1 sw2"/>
    <a:gd name="nsw" fmla="+- 0 0 sw"/>
    <a:gd name="ca" fmla="cos hd2 st"/>
    <a:gd name="sa" fmla="sin wd2 st"/>
    <a:gd name="dx" fmla="cat2 wd2 ca sa"/>
    <a:gd name="dy" fmla="sat2 hd2 ca sa"/>
    <a:gd name="x1" fmla="+- hc dx 0"/>
    <a:gd name="y1" fmla="+- vc dy 0"/>
    <a:gd name="cb" fmla="cos ih en"/>
    <a:gd name="sb" fmla="sin iw en"/>
    <a:gd name="dx2" fmla="cat2 iw cb sb"/>
    <a:gd name="dy2" fmla="sat2 ih cb sb"/>
    <a:gd name="x2" fmla="+- hc dx2 0"/>
    <a:gd name="y2" fmla="+- vc dy2 0"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="x1" y="y1"/></a:moveTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="st" swAng="sw"/>
    <a:lnTo><a:pt x="x2" y="y2"/></a:lnTo>
    <a:arcTo wR="iw" hR="ih" stAng="en" swAng="nsw"/>
    <a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `donut`: one adjust, default 25000. Measured: the box ellipse (clockwise from the left middle)
/// and an inner ellipse in the opposite direction, concentric, with radii reduced by
/// `ss * adj / 100000` on both axes (24 pt on 288x96, 43.2 pt at adj 30000 on 216x144); adj clamped to
/// 0..50000 (at 100000 and 400000 the inner ellipse is a flat line of half the box width, drawn by
/// the arc converter as Word does a flat ellipse; at 0 and -50000 it
/// coincides with the outer one).
fn donut() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="aPin" fmla="pin 0 adj 50000"/>
    <a:gd name="dr" fmla="*/ ss aPin 100000"/>
    <a:gd name="iw" fmla="+- wd2 0 dr"/>
    <a:gd name="ih" fmla="+- hd2 0 dr"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd2" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="3cd4" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="0" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd4" swAng="cd4"/>
    <a:close/>
    <a:moveTo><a:pt x="dr" y="vc"/></a:moveTo>
    <a:arcTo wR="iw" hR="ih" stAng="cd2" swAng="-5400000"/>
    <a:arcTo wR="iw" hR="ih" stAng="cd4" swAng="-5400000"/>
    <a:arcTo wR="iw" hR="ih" stAng="0" swAng="-5400000"/>
    <a:arcTo wR="iw" hR="ih" stAng="3cd4" swAng="-5400000"/>
    <a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `noSmoking`: one adjust, default 18750. Measured: the box ellipse (clockwise), plus two
/// half-discs of the inner ellipse (radii reduced by `dr = ss * adj / 100000`, adj clamped to
/// 0..50000; nothing is drawn inside at 50000) drawn counter-clockwise so that they cut the slash out
/// of the ring. The slash runs along the box diagonal (visual angle `td = atan2(h, w)`). Its half-width
/// is `dr / 2`: the half-disc edges are chords whose ends sit on the inner ellipse at the visual angles
/// `td - d` and `td + 180 deg + d` (first half), `td + 180 deg - d` and `td + d` (second), where
/// `d = atan2(dr / 2, n)` and `n` is the distance from the centre to the inner ellipse along the
/// diagonal. Each half-disc sweeps `-(180 deg - 2d)` (verified within 0.01 pt on 144x144, 288x96,
/// 96x288, 216x144 and with adj 0 and 30000).
fn no_smoking() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 18750"/></a:avLst>
  <a:gdLst>
    <a:gd name="aPin" fmla="pin 0 adj 50000"/>
    <a:gd name="dr" fmla="*/ ss aPin 100000"/>
    <a:gd name="iw" fmla="+- wd2 0 dr"/>
    <a:gd name="ih" fmla="+- hd2 0 dr"/>
    <a:gd name="td" fmla="at2 w h"/>
    <a:gd name="ct" fmla="cos ih td"/>
    <a:gd name="st" fmla="sin iw td"/>
    <a:gd name="m" fmla="mod ct st 0"/>
    <a:gd name="n" fmla="*/ iw ih m"/>
    <a:gd name="hw" fmla="*/ dr 1 2"/>
    <a:gd name="dd" fmla="at2 n hw"/>
    <a:gd name="sweep" fmla="+- dd dd 10800000"/>
    <a:gd name="aa" fmla="+- td 0 dd"/>
    <a:gd name="ab" fmla="+- td 10800000 0"/>
    <a:gd name="ac" fmla="+- ab 0 dd"/>
    <a:gd name="ca" fmla="cos ih aa"/>
    <a:gd name="sa" fmla="sin iw aa"/>
    <a:gd name="dxa" fmla="cat2 iw ca sa"/>
    <a:gd name="dya" fmla="sat2 ih ca sa"/>
    <a:gd name="xa" fmla="+- hc dxa 0"/>
    <a:gd name="ya" fmla="+- vc dya 0"/>
    <a:gd name="cc" fmla="cos ih ac"/>
    <a:gd name="sc" fmla="sin iw ac"/>
    <a:gd name="dxc" fmla="cat2 iw cc sc"/>
    <a:gd name="dyc" fmla="sat2 ih cc sc"/>
    <a:gd name="xc" fmla="+- hc dxc 0"/>
    <a:gd name="yc" fmla="+- vc dyc 0"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd2" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="3cd4" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="0" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd4" swAng="cd4"/>
    <a:close/>
    <a:moveTo><a:pt x="xa" y="ya"/></a:moveTo>
    <a:arcTo wR="iw" hR="ih" stAng="aa" swAng="sweep"/>
    <a:close/>
    <a:moveTo><a:pt x="xc" y="yc"/></a:moveTo>
    <a:arcTo wR="iw" hR="ih" stAng="ac" swAng="sweep"/>
    <a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `leftArrow`: two adjusts, both default 50000. Measured (mirror of the pilot `rightArrow`): seven
/// points, the tip at (l, vc); `adj1` is the shaft thickness as a fraction of the height (clamped
/// 0..100000), `adj2` the head length `ss * adj2 / 100000` (clamped 0..`100000 * w / ss`; at 400000
/// the head is the full width).
fn left_arrow() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a1" fmla="pin 0 adj1 100000"/>
    <a:gd name="maxA" fmla="*/ 100000 w ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 maxA"/>
    <a:gd name="hl" fmla="*/ ss a2 100000"/>
    <a:gd name="half" fmla="*/ h a1 200000"/>
    <a:gd name="yt" fmla="+- vc 0 half"/>
    <a:gd name="yb" fmla="+- vc half 0"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo><a:lnTo><a:pt x="hl" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yt"/></a:lnTo><a:lnTo><a:pt x="r" y="yt"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yb"/></a:lnTo><a:lnTo><a:pt x="hl" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="b"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `upArrow`: two adjusts, both default 50000. Measured: seven points, the tip at (hc, t); `adj1`
/// is the shaft thickness as a fraction of the WIDTH (clamped 0..100000), `adj2` the head length
/// `ss * adj2 / 100000` (clamped 0..`100000 * h / ss`; at 400000 the head is the full height).
fn up_arrow() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a1" fmla="pin 0 adj1 100000"/>
    <a:gd name="maxA" fmla="*/ 100000 h ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 maxA"/>
    <a:gd name="hl" fmla="*/ ss a2 100000"/>
    <a:gd name="half" fmla="*/ w a1 200000"/>
    <a:gd name="xl" fmla="+- hc 0 half"/>
    <a:gd name="xr" fmla="+- hc half 0"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="hl"/></a:moveTo><a:lnTo><a:pt x="hc" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="hl"/></a:lnTo><a:lnTo><a:pt x="xr" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="b"/></a:lnTo><a:lnTo><a:pt x="xl" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xl" y="hl"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `downArrow`: two adjusts, both default 50000. Measured: like `upArrow` turned over, the tip at
/// (hc, b): the head base is `ss * adj2 / 100000` above the bottom edge, shaft thickness `adj1` of
/// the width; same clamps as `upArrow`.
fn down_arrow() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a1" fmla="pin 0 adj1 100000"/>
    <a:gd name="maxA" fmla="*/ 100000 h ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 maxA"/>
    <a:gd name="hl" fmla="*/ ss a2 100000"/>
    <a:gd name="yh" fmla="+- b 0 hl"/>
    <a:gd name="half" fmla="*/ w a1 200000"/>
    <a:gd name="xl" fmla="+- hc 0 half"/>
    <a:gd name="xr" fmla="+- hc half 0"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="yh"/></a:moveTo><a:lnTo><a:pt x="xl" y="yh"/></a:lnTo>
    <a:lnTo><a:pt x="xl" y="t"/></a:lnTo><a:lnTo><a:pt x="xr" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="yh"/></a:lnTo><a:lnTo><a:pt x="r" y="yh"/></a:lnTo>
    <a:lnTo><a:pt x="hc" y="b"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `stripedRightArrow`: two adjusts, both default 50000. Measured: the shaft and head of
/// `rightArrow` (shaft thickness `adj1` of the height, clamped 0..100000; head length
/// `ss * adj2 / 100000`, clamped 0..`84375 * w / ss`, so at 400000 on 216x144 the head is 182.25 pt),
/// but with the shaft's left end replaced by two stripes (closed rectangles spanning the shaft
/// thickness) at x = 0..`ss/32`, `ss/16`..`ss/8` and a body starting at x = `5 * ss / 32`
/// (4.5, 9, 18 and 22.5 pt on 144 short side; 3, 6, 12 and 15 on 96).
fn striped_right_arrow() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a1" fmla="pin 0 adj1 100000"/>
    <a:gd name="maxA" fmla="*/ 84375 w ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 maxA"/>
    <a:gd name="hl" fmla="*/ ss a2 100000"/>
    <a:gd name="xh" fmla="+- r 0 hl"/>
    <a:gd name="half" fmla="*/ h a1 200000"/>
    <a:gd name="yt" fmla="+- vc 0 half"/>
    <a:gd name="yb" fmla="+- vc half 0"/>
    <a:gd name="s1" fmla="*/ ss 3125 100000"/>
    <a:gd name="s2" fmla="*/ ss 6250 100000"/>
    <a:gd name="s3" fmla="*/ ss 12500 100000"/>
    <a:gd name="s4" fmla="*/ ss 15625 100000"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="yt"/></a:moveTo><a:lnTo><a:pt x="s1" y="yt"/></a:lnTo>
    <a:lnTo><a:pt x="s1" y="yb"/></a:lnTo><a:lnTo><a:pt x="l" y="yb"/></a:lnTo><a:close/>
    <a:moveTo><a:pt x="s2" y="yt"/></a:moveTo><a:lnTo><a:pt x="s3" y="yt"/></a:lnTo>
    <a:lnTo><a:pt x="s3" y="yb"/></a:lnTo><a:lnTo><a:pt x="s2" y="yb"/></a:lnTo><a:close/>
    <a:moveTo><a:pt x="s4" y="yt"/></a:moveTo><a:lnTo><a:pt x="xh" y="yt"/></a:lnTo>
    <a:lnTo><a:pt x="xh" y="t"/></a:lnTo><a:lnTo><a:pt x="r" y="vc"/></a:lnTo>
    <a:lnTo><a:pt x="xh" y="b"/></a:lnTo><a:lnTo><a:pt x="xh" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="s4" y="yb"/></a:lnTo><a:close/>
  </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// The b02 entries (those not already in the pilot).
pub fn entries() -> Vec<(&'static str, ShapeDef)> {
    let all = [
        ("star32", star32()),
        ("round1Rect", round1_rect()),
        ("round2SameRect", round2_same_rect()),
        ("round2DiagRect", round2_diag_rect()),
        ("snipRoundRect", snip_round_rect()),
        ("snip1Rect", snip1_rect()),
        ("snip2SameRect", snip2_same_rect()),
        ("snip2DiagRect", snip2_diag_rect()),
        ("plaque", plaque()),
        ("teardrop", teardrop()),
        ("homePlate", home_plate()),
        ("chevron", chevron()),
        ("pieWedge", pie_wedge()),
        ("pie", pie()),
        ("blockArc", block_arc()),
        ("donut", donut()),
        ("noSmoking", no_smoking()),
        ("leftArrow", left_arrow()),
        ("upArrow", up_arrow()),
        ("downArrow", down_arrow()),
        ("stripedRightArrow", striped_right_arrow()),
    ];
    all.into_iter()
        .filter_map(|(n, d)| d.map(|d| (n, d)))
        .collect()
}
