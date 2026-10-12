//! Preset batch b05 (connectors, line callouts, wedge callouts), rebuilt clean-room from Word
//! measurements (docs/architecture/drawing.md).
//!
//! Source: `fixtures/reference/drawing/presets/b05.{docx,pdf,probes.json,handles.json,paths.json}`
//! made by `tools/office/make-preset-probes.ps1` (Word's PDF export, one preset per page: four
//! sizes, five all-adjust settings, one scan page per adjust). Every formula below was derived from
//! those measured paths and handle defaults only; no other project's preset table and no copy of
//! Ecma's `presetShapeDefinitions.xml` was opened. Guide names are our own.
//!
//! Findings common to the batch:
//! - Connector and line-callout adjusts are NOT clamped: -50000 and 400000 put the bends and the
//!   line ends far outside the box exactly as the arithmetic says.
//! - Word writes zero-length strokes as nothing (callouts at adjust 0), so those pages compare
//!   as empty outlines.
//! - Text rectangles and connection sites are not measurable with these probes and are left out.

use super::ShapeDef;
use crate::parse::cust_geom_from_str;

fn parse(xml: &str) -> Option<ShapeDef> {
    cust_geom_from_str(xml).ok()
}

/// `bentConnector2`: no adjusts. Measured at four sizes: an open elbow, top left to top right to
/// bottom right, along the top and right edges. Not filled.
fn bent_connector2() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `bentConnector3`: one adjust (named `adj1` in the file, default 0.5). Measured: open path
/// (0,0) -> (x1,0) -> (x1,h) -> (w,h) with `x1 = w * adj1 / 100000` (108 pt on 216, 64.8 with
/// 30000). Not clamped: -50000 gives -108 pt and 400000 gives 864 pt.
fn bent_connector3() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 50000"/></a:avLst>
    <a:gdLst><a:gd name="xBend" fmla="*/ w adj1 100000"/></a:gdLst>
    <a:pathLst><a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="xBend" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xBend" y="b"/></a:lnTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `bentConnector4`: two adjusts (default 0.5, 0.5). Measured: open path (0,0) -> (x1,0) ->
/// (x1,y2) -> (w,y2) -> (w,h), `x1 = w * adj1 / 100000`, `y2 = h * adj2 / 100000`, unclamped
/// (-50000 gives -108 pt and -72 pt on 216x144).
fn bent_connector4() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
    <a:gdLst><a:gd name="x1" fmla="*/ w adj1 100000"/><a:gd name="y2" fmla="*/ h adj2 100000"/></a:gdLst>
    <a:pathLst><a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="x1" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="x1" y="y2"/></a:lnTo><a:lnTo><a:pt x="r" y="y2"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `bentConnector5`: three adjusts (default 0.5 each). Measured: open path (0,0) -> (x1,0) ->
/// (x1,y2) -> (x3,y2) -> (x3,h) -> (w,h), `x1 = w * adj1`, `y2 = h * adj2`, `x3 = w * adj3`
/// (all / 100000), unclamped.
fn bent_connector5() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/><a:gd name="adj3" fmla="val 50000"/></a:avLst>
    <a:gdLst><a:gd name="x1" fmla="*/ w adj1 100000"/><a:gd name="y2" fmla="*/ h adj2 100000"/><a:gd name="x3" fmla="*/ w adj3 100000"/></a:gdLst>
    <a:pathLst><a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="x1" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="x1" y="y2"/></a:lnTo><a:lnTo><a:pt x="x3" y="y2"/></a:lnTo>
    <a:lnTo><a:pt x="x3" y="b"/></a:lnTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `curvedConnector2`: no adjusts. Measured: one cubic from (0,0) with control points
/// (w/2,0) and (w,h/2) to (w,h) (108,0 and 216,72 on 216x144). Not filled.
fn curved_connector2() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:cubicBezTo><a:pt x="wd2" y="t"/><a:pt x="r" y="hd2"/><a:pt x="r" y="b"/></a:cubicBezTo>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `curvedConnector3`: one adjust (`adj1`, default 0.5). Measured: two cubics through the
/// mid point (x2, h/2) with `x2 = w * adj1 / 100000` (unclamped). First cubic (0,0) with
/// controls (x2/2,0), (x2,h/4); second from (x2,h/2) with controls (x2,3h/4), (x3,h) where
/// `x3 = (w + x2) / 2`, ending at (w,h). Checked at 30000, 100000 and -50000.
fn curved_connector3() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 50000"/></a:avLst>
    <a:gdLst>
      <a:gd name="x2" fmla="*/ w adj1 100000"/>
      <a:gd name="x1" fmla="+/ l x2 2"/>
      <a:gd name="x3" fmla="+/ r x2 2"/>
      <a:gd name="y3" fmla="*/ h 3 4"/>
    </a:gdLst>
    <a:pathLst><a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:cubicBezTo><a:pt x="x1" y="t"/><a:pt x="x2" y="hd4"/><a:pt x="x2" y="vc"/></a:cubicBezTo>
    <a:cubicBezTo><a:pt x="x2" y="y3"/><a:pt x="x3" y="b"/><a:pt x="r" y="b"/></a:cubicBezTo>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `curvedConnector4`: two adjusts (default 0.5, 0.5). Measured: three cubics through
/// (x2, y2) and (x5, y4), with `x2 = w * adj1`, `y4 = h * adj2` (/ 100000, unclamped),
/// `y2 = y4 / 2`, `x5 = (x2 + w) / 2`. Cubic 1: (0,0), controls (x2/2,0), (x2,y4/4), end
/// (x2,y2). Cubic 2: controls (x2,(y2+y4)/2), ((x2+x5)/2,y4), end (x5,y4). Cubic 3: controls
/// ((x5+w)/2,y4), (w,(y4+h)/2), end (w,h). Checked at 30000 and -50000 on 216x144 and at
/// default on 96x288.
fn curved_connector4() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
    <a:gdLst>
      <a:gd name="x2" fmla="*/ w adj1 100000"/>
      <a:gd name="y4" fmla="*/ h adj2 100000"/>
      <a:gd name="x1" fmla="+/ l x2 2"/>
      <a:gd name="x5" fmla="+/ x2 r 2"/>
      <a:gd name="x3" fmla="+/ x2 x5 2"/>
      <a:gd name="x6" fmla="+/ x5 r 2"/>
      <a:gd name="y1" fmla="+/ t y4 4"/>
      <a:gd name="y2" fmla="+/ t y4 2"/>
      <a:gd name="y3" fmla="+/ y2 y4 2"/>
      <a:gd name="y5" fmla="+/ y4 b 2"/>
    </a:gdLst>
    <a:pathLst><a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:cubicBezTo><a:pt x="x1" y="t"/><a:pt x="x2" y="y1"/><a:pt x="x2" y="y2"/></a:cubicBezTo>
    <a:cubicBezTo><a:pt x="x2" y="y3"/><a:pt x="x3" y="y4"/><a:pt x="x5" y="y4"/></a:cubicBezTo>
    <a:cubicBezTo><a:pt x="x6" y="y4"/><a:pt x="r" y="y5"/><a:pt x="r" y="b"/></a:cubicBezTo>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `curvedConnector5`: three adjusts (default 0.5 each). Measured: four cubics, with
/// `x2 = w * adj1`, `y4 = h * adj2`, `x6 = w * adj3` (/ 100000, unclamped), `y2 = y4 / 2`,
/// `x4 = (x2 + x6) / 2`, `y6 = (y4 + h) / 2`. Cubic 1: controls (x2/2,0), (x2,y4/4), end (x2,y2).
/// Cubic 2: controls (x2,(y2+y4)/2), ((x2+x4)/2,y4), end (x4,y4). Cubic 3: controls
/// ((x4+x6)/2,y4), (x6,(y4+y6)/2), end (x6,y6). Cubic 4: controls (x6,(y6+h)/2), ((x6+w)/2,h),
/// end (w,h). Checked at default, 30000, -50000 and with each adjust at 0.
fn curved_connector5() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/><a:gd name="adj3" fmla="val 50000"/></a:avLst>
    <a:gdLst>
      <a:gd name="x2" fmla="*/ w adj1 100000"/>
      <a:gd name="y4" fmla="*/ h adj2 100000"/>
      <a:gd name="x6" fmla="*/ w adj3 100000"/>
      <a:gd name="x1" fmla="+/ l x2 2"/>
      <a:gd name="x4" fmla="+/ x2 x6 2"/>
      <a:gd name="x3" fmla="+/ x2 x4 2"/>
      <a:gd name="x5" fmla="+/ x4 x6 2"/>
      <a:gd name="x7" fmla="+/ x6 r 2"/>
      <a:gd name="y1" fmla="+/ t y4 4"/>
      <a:gd name="y2" fmla="+/ t y4 2"/>
      <a:gd name="y3" fmla="+/ y2 y4 2"/>
      <a:gd name="y6" fmla="+/ y4 b 2"/>
      <a:gd name="y5" fmla="+/ y4 y6 2"/>
      <a:gd name="y7" fmla="+/ y6 b 2"/>
    </a:gdLst>
    <a:pathLst><a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:cubicBezTo><a:pt x="x1" y="t"/><a:pt x="x2" y="y1"/><a:pt x="x2" y="y2"/></a:cubicBezTo>
    <a:cubicBezTo><a:pt x="x2" y="y3"/><a:pt x="x3" y="y4"/><a:pt x="x4" y="y4"/></a:cubicBezTo>
    <a:cubicBezTo><a:pt x="x5" y="y4"/><a:pt x="x6" y="y5"/><a:pt x="x6" y="y6"/></a:cubicBezTo>
    <a:cubicBezTo><a:pt x="x6" y="y7"/><a:pt x="x7" y="b"/><a:pt x="r" y="b"/></a:cubicBezTo>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

// ---- line callouts -------------------------------------------------------------------------

/// Which parts a line callout draws.
struct LineCallout {
    /// Number of line points: 1 (two points), 2 (three points) or 3 (four points).
    n: usize,
    /// The box is stroked.
    border: bool,
    /// A vertical accent bar at the x of the first point.
    accent: bool,
}

/// Line callouts. Measured (Word 16, `callout1..3`, `accentCallout1..3`, `borderCallout1..3`,
/// `accentBorderCallout1..3`): adjust pairs are (y, x) fractions of height and width, in order:
/// point k is `(w * adj(2k) / 100000, h * adj(2k-1) / 100000)`. Defaults (Word object model):
/// point 1 (0.1875 h, -0.08333 w); then for `*2` point 2 (0.1875, -0.16667) and point 3
/// (1.125, -0.46667); for `*1` point 2 is (1.125, -0.38333); for `*3` points 2..4 are
/// (0.1875, -0.16667), (1.0, -0.16667), (1.12963, -0.08333). None of the adjusts is clamped.
/// Outline: the callout polyline (open, joined points) is always stroked; the `border` variants
/// stroke the box too (the `callout`/`accentCallout` variants show no box outline, only the
/// filled box); the `accent` variants stroke a vertical bar from top to bottom edge at the x of
/// the first point (a closed two-point path as Word draws it). Word draws nothing for a
/// zero-length polyline (all adjusts equal), and so do we (a single point).
fn line_callout(spec: &LineCallout) -> Option<ShapeDef> {
    let defaults: &[i64] = match spec.n {
        1 => &[18_750, -8_333, 112_500, -38_333],
        2 => &[18_750, -8_333, 18_750, -16_667, 112_500, -46_667],
        _ => &[
            18_750, -8_333, 18_750, -16_667, 100_000, -16_667, 112_963, -8_333,
        ],
    };
    let mut av = String::new();
    let mut gd = String::new();
    let mut line = String::new();
    for k in 0..spec.n + 1 {
        let (ya, xa) = (2 * k + 1, 2 * k + 2);
        av += &format!(
            r#"<a:gd name="adj{ya}" fmla="val {}"/><a:gd name="adj{xa}" fmla="val {}"/>"#,
            defaults[2 * k],
            defaults[2 * k + 1]
        );
        gd += &format!(
            r#"<a:gd name="py{k}" fmla="*/ h adj{ya} 100000"/><a:gd name="px{k}" fmla="*/ w adj{xa} 100000"/>"#
        );
        line += &if k == 0 {
            format!(r#"<a:moveTo><a:pt x="px{k}" y="py{k}"/></a:moveTo>"#)
        } else {
            format!(r#"<a:lnTo><a:pt x="px{k}" y="py{k}"/></a:lnTo>"#)
        };
    }
    let boxpath = format!(
        r#"<a:path{}><a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:close/></a:path>"#,
        if spec.border { "" } else { r#" stroke="0""# }
    );
    let accent = if spec.accent {
        r#"<a:path fill="none"><a:moveTo><a:pt x="px0" y="t"/></a:moveTo><a:lnTo><a:pt x="px0" y="b"/></a:lnTo><a:close/></a:path>"#
    } else {
        ""
    };
    parse(&format!(
        r#"<a:custGeom><a:avLst>{av}</a:avLst><a:gdLst>{gd}</a:gdLst><a:pathLst>{boxpath}{accent}<a:path fill="none">{line}</a:path></a:pathLst></a:custGeom>"#
    ))
}

// ---- wedge callouts ------------------------------------------------------------------------

/// Guides shared by the two rectangular wedge callouts: the tip, the edge it leaves from and the
/// base points of the tail.
///
/// Measured (`wedgeRectCallout`, `wedgeRoundRectCallout`, same logic in both): the tip is at
/// `(hc + w * adj1 / 100000, vc + h * adj2 / 100000)`, not clamped. The tail leaves from the edge
/// that the tip lies beyond: vertical (top or bottom) when `|dy| > |dx * h / w|`, otherwise
/// horizontal (a tie, as in every all-adjusts page, gives left or right), left when `dx <= 0`,
/// top when `dy < 0`. The coordinate of the tip across that edge is pulled onto the edge when the
/// tip is inside the box (seen: adj2 = 0 puts the tip of a left tail at x = 0, not at xPos). The
/// tail base on a horizontal edge spans `w/6 .. 5w/12` when the tip is at or left of the centre and
/// `7w/12 .. 10w/12` when right of it; on a vertical edge it is `h/6 .. 5h/12` (tip at or above
/// the centre line) or `7h/12 .. 10h/12`.
const WEDGE_GUIDES: &str = r#"
      <a:gd name="wSev" fmla="*/ w 7 12"/>
      <a:gd name="wTen" fmla="*/ w 10 12"/>
      <a:gd name="wTwo" fmla="*/ w 2 12"/>
      <a:gd name="wFiv" fmla="*/ w 5 12"/>
      <a:gd name="hSev" fmla="*/ h 7 12"/>
      <a:gd name="hTen" fmla="*/ h 10 12"/>
      <a:gd name="hTwo" fmla="*/ h 2 12"/>
      <a:gd name="hFiv" fmla="*/ h 5 12"/>
      <a:gd name="dxp" fmla="*/ w adj1 100000"/>
      <a:gd name="dyp" fmla="*/ h adj2 100000"/>
      <a:gd name="xPos" fmla="+- hc dxp 0"/>
      <a:gd name="yPos" fmla="+- vc dyp 0"/>
      <a:gd name="dq" fmla="*/ dxp h w"/>
      <a:gd name="adq" fmla="abs dq"/>
      <a:gd name="ady" fmla="abs dyp"/>
      <a:gd name="isV" fmla="+- ady 0 adq"/>
      <a:gd name="negDy" fmla="+- 0 0 dyp"/>
      <a:gd name="fTop" fmla="min isV negDy"/>
      <a:gd name="fBot" fmla="min isV dyp"/>
      <a:gd name="fH" fmla="?: isV 0 1"/>
      <a:gd name="dxLe" fmla="?: dxp 0 1"/>
      <a:gd name="fLeft" fmla="min fH dxLe"/>
      <a:gd name="fRight" fmla="min fH dxp"/>
      <a:gd name="xRight" fmla="+- xPos 0 hc"/>
      <a:gd name="xb1" fmla="?: xRight wSev wTwo"/>
      <a:gd name="xb2" fmla="?: xRight wTen wFiv"/>
      <a:gd name="yDown" fmla="+- yPos 0 vc"/>
      <a:gd name="yb1" fmla="?: yDown hSev hTwo"/>
      <a:gd name="yb2" fmla="?: yDown hTen hFiv"/>
      <a:gd name="yTopT" fmla="min yPos t"/>
      <a:gd name="yBotT" fmla="max yPos b"/>
      <a:gd name="xLeftT" fmla="min xPos l"/>
      <a:gd name="xRightT" fmla="max xPos r"/>
      <a:gd name="topX" fmla="?: fTop xPos xb1"/>
      <a:gd name="topY" fmla="?: fTop yTopT t"/>
      <a:gd name="botX" fmla="?: fBot xPos xb2"/>
      <a:gd name="botY" fmla="?: fBot yBotT b"/>
      <a:gd name="leftX" fmla="?: fLeft xLeftT l"/>
      <a:gd name="leftY" fmla="?: fLeft yPos yb2"/>
      <a:gd name="rightX" fmla="?: fRight xRightT r"/>
      <a:gd name="rightY" fmla="?: fRight yPos yb1"/>
"#;

/// `wedgeRectCallout`: two adjusts, defaults -20833 and 62500 (Word object model); the tip is at
/// (hc + w*adj1, vc + h*adj2), see `WEDGE_GUIDES`. A rectangle whose edge is cut by the tail.
fn wedge_rect_callout() -> Option<ShapeDef> {
    parse(&format!(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val -20833"/><a:gd name="adj2" fmla="val 62500"/></a:avLst>
    <a:gdLst>{WEDGE_GUIDES}</a:gdLst>
    <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="xb1" y="t"/></a:lnTo><a:lnTo><a:pt x="topX" y="topY"/></a:lnTo><a:lnTo><a:pt x="xb2" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yb1"/></a:lnTo><a:lnTo><a:pt x="rightX" y="rightY"/></a:lnTo><a:lnTo><a:pt x="r" y="yb2"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xb2" y="b"/></a:lnTo><a:lnTo><a:pt x="botX" y="botY"/></a:lnTo><a:lnTo><a:pt x="xb1" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="yb2"/></a:lnTo><a:lnTo><a:pt x="leftX" y="leftY"/></a:lnTo><a:lnTo><a:pt x="l" y="yb1"/></a:lnTo>
    <a:close/>
    </a:path></a:pathLst></a:custGeom>"#
    ))
}

/// `wedgeRoundRectCallout`: three adjusts, defaults -20833, 62500, 16667. The outline of
/// `wedgeRectCallout` with quarter-ellipse corners of radius `rad = ss * adj3 / 100000`. `adj3` is
/// NOT clamped: with -50000 the radius is -72 pt and Word keeps the signed value for the points
/// but draws each corner with the absolute radius (so the outline leaves the box); with 400000 the
/// corners are 576 pt. The same signed/absolute split is reproduced here (`rad` for positions,
/// `radA` for the arcs).
fn wedge_round_rect_callout() -> Option<ShapeDef> {
    parse(&format!(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val -20833"/><a:gd name="adj2" fmla="val 62500"/><a:gd name="adj3" fmla="val 16667"/></a:avLst>
    <a:gdLst>{WEDGE_GUIDES}
      <a:gd name="rad" fmla="*/ ss adj3 100000"/>
      <a:gd name="radA" fmla="abs rad"/>
      <a:gd name="xr" fmla="+- r 0 rad"/>
      <a:gd name="yb" fmla="+- b 0 rad"/>
    </a:gdLst>
    <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="rad"/></a:moveTo>
    <a:arcTo wR="radA" hR="radA" stAng="cd2" swAng="cd4"/>
    <a:lnTo><a:pt x="xb1" y="t"/></a:lnTo><a:lnTo><a:pt x="topX" y="topY"/></a:lnTo><a:lnTo><a:pt x="xb2" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="t"/></a:lnTo>
    <a:arcTo wR="radA" hR="radA" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="yb1"/></a:lnTo><a:lnTo><a:pt x="rightX" y="rightY"/></a:lnTo><a:lnTo><a:pt x="r" y="yb2"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
    <a:arcTo wR="radA" hR="radA" stAng="0" swAng="cd4"/>
    <a:lnTo><a:pt x="xb2" y="b"/></a:lnTo><a:lnTo><a:pt x="botX" y="botY"/></a:lnTo><a:lnTo><a:pt x="xb1" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="rad" y="b"/></a:lnTo>
    <a:arcTo wR="radA" hR="radA" stAng="cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="l" y="yb2"/></a:lnTo><a:lnTo><a:pt x="leftX" y="leftY"/></a:lnTo><a:lnTo><a:pt x="l" y="yb1"/></a:lnTo>
    <a:close/>
    </a:path></a:pathLst></a:custGeom>"#
    ))
}

/// `wedgeEllipseCallout`: two adjusts, defaults -20833 and 62500. Measured: an ellipse inscribed
/// in the box with a wedge to the tip. Let `dx = w*adj1/100000`, `dy = h*adj2/100000` and `th`
/// the parametric angle of the tip direction (`atan2(dy/hd2, dx/wd2)`). The tip is
/// `(hc + dx, vc + dy)` when it is outside the ellipse and the ellipse point at `th` when inside
/// (seen with dx = dy = 0: tip at the right middle point; adj2 = 0: tip at the left middle).
/// The outline starts at the tip, runs to the ellipse point at parametric angle `th + 11` degrees,
/// follows the ellipse clockwise (338 degrees) to the point at `th - 11` degrees and closes back
/// to the tip. The 11 degrees were fitted on three tip directions (error below 0.01 pt).
fn wedge_ellipse_callout() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val -20833"/><a:gd name="adj2" fmla="val 62500"/></a:avLst>
    <a:gdLst>
      <a:gd name="dxp" fmla="*/ w adj1 100000"/>
      <a:gd name="dyp" fmla="*/ h adj2 100000"/>
      <a:gd name="xPos" fmla="+- hc dxp 0"/>
      <a:gd name="yPos" fmla="+- vc dyp 0"/>
      <a:gd name="gx" fmla="*/ dxp hd2 1"/>
      <a:gd name="gy" fmla="*/ dyp wd2 1"/>
      <a:gd name="th" fmla="at2 gx gy"/>
      <a:gd name="len" fmla="mod gx gy 0"/>
      <a:gd name="lim" fmla="*/ wd2 hd2 1"/>
      <a:gd name="inside" fmla="+- lim 0 len"/>
      <a:gd name="bx" fmla="cos wd2 th"/>
      <a:gd name="by" fmla="sin hd2 th"/>
      <a:gd name="bxa" fmla="+- hc bx 0"/>
      <a:gd name="bya" fmla="+- vc by 0"/>
      <a:gd name="tipX" fmla="?: inside bxa xPos"/>
      <a:gd name="tipY" fmla="?: inside bya yPos"/>
      <a:gd name="a1" fmla="+- th 660000 0"/>
      <a:gd name="a2" fmla="+- th 0 660000"/>
      <a:gd name="q1x" fmla="cos wd2 a1"/>
      <a:gd name="q1y" fmla="sin hd2 a1"/>
      <a:gd name="q2x" fmla="cos wd2 a2"/>
      <a:gd name="q2y" fmla="sin hd2 a2"/>
      <a:gd name="p1x" fmla="+- hc q1x 0"/>
      <a:gd name="p1y" fmla="+- vc q1y 0"/>
      <a:gd name="v1" fmla="at2 q1x q1y"/>
      <a:gd name="v2" fmla="at2 q2x q2y"/>
      <a:gd name="swRaw" fmla="+- v2 0 v1"/>
      <a:gd name="swPlus" fmla="+- swRaw 21600000 0"/>
      <a:gd name="sw" fmla="?: swRaw swRaw swPlus"/>
    </a:gdLst>
    <a:pathLst><a:path>
    <a:moveTo><a:pt x="tipX" y="tipY"/></a:moveTo>
    <a:lnTo><a:pt x="p1x" y="p1y"/></a:lnTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="v1" swAng="sw"/>
    <a:close/>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// The b05 entries.
pub fn entries() -> Vec<(&'static str, ShapeDef)> {
    let lc = |n, border, accent| line_callout(&LineCallout { n, border, accent });
    let all = [
        ("bentConnector2", bent_connector2()),
        ("bentConnector3", bent_connector3()),
        ("bentConnector4", bent_connector4()),
        ("bentConnector5", bent_connector5()),
        ("curvedConnector2", curved_connector2()),
        ("curvedConnector3", curved_connector3()),
        ("curvedConnector4", curved_connector4()),
        ("curvedConnector5", curved_connector5()),
        ("callout1", lc(1, false, false)),
        ("callout2", lc(2, false, false)),
        ("callout3", lc(3, false, false)),
        ("accentCallout1", lc(1, false, true)),
        ("accentCallout2", lc(2, false, true)),
        ("accentCallout3", lc(3, false, true)),
        ("borderCallout1", lc(1, true, false)),
        ("borderCallout2", lc(2, true, false)),
        ("borderCallout3", lc(3, true, false)),
        ("accentBorderCallout1", lc(1, true, true)),
        ("accentBorderCallout2", lc(2, true, true)),
        ("accentBorderCallout3", lc(3, true, true)),
        ("wedgeRectCallout", wedge_rect_callout()),
        ("wedgeRoundRectCallout", wedge_round_rect_callout()),
        ("wedgeEllipseCallout", wedge_ellipse_callout()),
    ];
    all.into_iter()
        .filter_map(|(n, d)| d.map(|d| (n, d)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_b05_entries_parse() {
        assert_eq!(entries().len(), 23);
    }
}
