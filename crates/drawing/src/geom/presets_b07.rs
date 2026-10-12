//! Preset batch b07 (flow chart shapes and the first five action buttons), rebuilt clean-room from
//! Word measurements (docs/architecture/drawing.md).
//!
//! Source: `fixtures/reference/drawing/presets/b07.{docx,pdf,probes.json,handles.json,paths.json}`
//! and the fill-enabled twin `b07.fill.*` (same presets with a solid fill, used to read which
//! sub-areas Word shades), both made by `tools/office/make-preset-probes.ps1`. Every formula below
//! was derived from those measured paths only; no other project's preset table and no copy of
//! Ecma's `presetShapeDefinitions.xml` was opened. Guide names are our own.
//!
//! None of these presets has adjusts (`Shape.Adjustments.Count` = 0 for all 23).
//!
//! Findings common to the batch:
//! - Flow chart outlines are fractions of the box (`w/5`, `h/5`, `h/10`, `w/6`, `h/6`, `45 degree`
//!   points of the ellipse) and follow the box exactly at every probed size.
//! - Action buttons: the frame is the box. The icon is drawn in a grid of `ss / 32` (`ss` = short
//!   side) centred on the box centre, at every box shape (checked at 144x144, 288x96, 96x288 and
//!   216x144 pt), so `x = hc + k * ss / 32` and `y = vc + k * ss / 32`. Word paints the frame and
//!   the icon outline as one even-odd fill, shades parts of the icon (fill modes read from the
//!   `b07.fill` twin: 0.8 x the base grey = darkenLess, 0.6 x = darken, base plus 60% of the way to
//!   white = lightenLess) and then strokes the icon and the frame.
//! - Paths with `fill="none"` are open or inner line work that Word strokes without a fill
//!   (inner lines of the summing junction and the disk rim, for example).
//! - The fill-only shading paths (`stroke="0"`) are checked by `tests/preset_fills_b07_b08.rs`
//!   against the `b07.fill` twin, not by the stroke comparison of `preset_probes.rs`.

use super::ShapeDef;
use crate::parse::cust_geom_from_str;
use std::collections::BTreeSet;
use std::fmt::Write as _;

fn parse(xml: &str) -> Option<ShapeDef> {
    cust_geom_from_str(xml).ok()
}

/// One path of a button icon: fill mode, stroke flag and a compact command string.
///
/// The commands are `M x y`, `L x y`, `C x1 y1 x2 y2 x y` and `Z`. A number is a distance from
/// the box centre in `ss / 32` units (x from `hc`, y from `vc`); a name (`l t r b hc vc`) is a
/// box guide.
pub(super) type UPath = (&'static str, bool, &'static str);

fn unit_key(v: &str) -> Option<i64> {
    v.parse::<f64>().ok().map(|f| (f * 1000.0).round() as i64)
}

fn unit_name(axis: char, k: i64) -> String {
    if k < 0 {
        format!("u{axis}n{}", -k)
    } else {
        format!("u{axis}p{k}")
    }
}

/// Builds a shape whose coordinates are written in `ss / 32` units about the centre (the
/// action-button icon grid). Returns `None` if the text is malformed.
pub(super) fn unit_shape(paths: &[UPath]) -> Option<ShapeDef> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut gd = String::new();
    let mut body = String::new();
    for (fill, stroke, text) in paths {
        let _ = write!(
            body,
            "<a:path fill=\"{fill}\" stroke=\"{}\">",
            u8::from(*stroke)
        );
        let toks: Vec<&str> = text.split_whitespace().collect();
        let mut i = 0;
        while i < toks.len() {
            let cmd = toks[i];
            i += 1;
            let n = match cmd {
                "M" | "L" => 2,
                "C" => 6,
                "Z" => 0,
                _ => return None,
            };
            let mut pts: Vec<String> = Vec::new();
            for j in 0..n {
                let t = *toks.get(i + j)?;
                if let Some(k) = unit_key(t) {
                    let axis = if j % 2 == 0 { 'x' } else { 'y' };
                    let name = unit_name(axis, k);
                    if seen.insert(name.clone()) {
                        let base = if axis == 'x' { "hc" } else { "vc" };
                        let _ = write!(
                            gd,
                            "<a:gd name=\"{name}o\" fmla=\"*/ ss {k} 32000\"/><a:gd name=\"{name}\" fmla=\"+- {base} {name}o 0\"/>"
                        );
                    }
                    pts.push(name);
                } else {
                    pts.push(t.to_owned());
                }
            }
            i += n;
            let pt = |a: usize| format!("<a:pt x=\"{}\" y=\"{}\"/>", pts[a], pts[a + 1]);
            match cmd {
                "M" => {
                    let _ = write!(body, "<a:moveTo>{}</a:moveTo>", pt(0));
                }
                "L" => {
                    let _ = write!(body, "<a:lnTo>{}</a:lnTo>", pt(0));
                }
                "C" => {
                    let _ = write!(
                        body,
                        "<a:cubicBezTo>{}{}{}</a:cubicBezTo>",
                        pt(0),
                        pt(2),
                        pt(4)
                    );
                }
                _ => body.push_str("<a:close/>"),
            }
        }
        body.push_str("</a:path>");
    }
    parse(&format!(
        "<a:custGeom><a:avLst/><a:gdLst>{gd}</a:gdLst><a:pathLst>{body}</a:pathLst></a:custGeom>"
    ))
}

/// `flowChartManualOperation`.
///
/// Measured (four sizes): a trapezoid, wide edge on top. Top edge is the full width; the bottom
/// edge is inset by `w / 5` on each side (28.8 of 144, 43.2 of 216, 57.6 of 288 pt). No adjusts.
fn manual_operation() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="inset" fmla="*/ w 1 5"/>
  <a:gd name="xl" fmla="+- l inset 0"/>
  <a:gd name="xr" fmla="+- r 0 inset"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo>
  <a:lnTo><a:pt x="xr" y="b"/></a:lnTo><a:lnTo><a:pt x="xl" y="b"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartConnector`: no adjusts. An ellipse inscribed in the box (four quarter arcs from the
/// left middle point, control offsets 0.5523 of the radii), exactly as `ellipse`.
fn connector() -> Option<ShapeDef> {
    parse(ELLIPSE_PATH)
}

const ELLIPSE_PATH: &str = r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path>
  <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
  <a:arcTo wR="wd2" hR="hd2" stAng="cd2" swAng="cd4"/>
  <a:arcTo wR="wd2" hR="hd2" stAng="3cd4" swAng="cd4"/>
  <a:arcTo wR="wd2" hR="hd2" stAng="0" swAng="cd4"/>
  <a:arcTo wR="wd2" hR="hd2" stAng="cd4" swAng="cd4"/>
  <a:close/>
</a:path></a:pathLst></a:custGeom>"#;

/// `flowChartOffpageConnector`.
///
/// Measured: a pentagon pointing down. Top corners on the box, side edges run to `y = 0.8 h`
/// (115.2 of 144, 76.8 of 96 pt), then lines to the bottom middle point.
fn offpage_connector() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="ys" fmla="*/ h 4 5"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo>
  <a:lnTo><a:pt x="r" y="ys"/></a:lnTo><a:lnTo><a:pt x="hc" y="b"/></a:lnTo>
  <a:lnTo><a:pt x="l" y="ys"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartPunchedCard`.
///
/// Measured: a rectangle with the top left corner cut off. The cut runs from `(0, h / 5)` to
/// `(w / 5, 0)` (43.2 and 28.8 pt at 216x144; 57.6 and 19.2 pt at 288x96), so both legs scale
/// with their own side.
fn punched_card() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="cx" fmla="*/ w 1 5"/>
  <a:gd name="cy" fmla="*/ h 1 5"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="l" y="cy"/></a:moveTo><a:lnTo><a:pt x="cx" y="t"/></a:lnTo>
  <a:lnTo><a:pt x="r" y="t"/></a:lnTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo>
  <a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartPunchedTape`.
///
/// Measured: a box whose top and bottom edges are full sine-like waves of two half-ellipse arcs
/// each. The wave amplitude is `h / 10` (14.4 pt of 144, 9.6 of 96) either side of the edge line
/// and each arc spans a quarter of the width (`w / 4`). The top edge dips at `x = w / 4` and
/// rises at `x = 3 w / 4`; the bottom edge is the mirror (low at the right, high at the left
/// quarter). The left and right sides are straight between `h / 10` and `9 h / 10`.
fn punched_tape() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="amp" fmla="*/ h 1 10"/>
  <a:gd name="yLow" fmla="+- b 0 amp"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="l" y="amp"/></a:moveTo>
  <a:arcTo wR="wd4" hR="amp" stAng="cd2" swAng="-5400000"/>
  <a:arcTo wR="wd4" hR="amp" stAng="cd4" swAng="-5400000"/>
  <a:arcTo wR="wd4" hR="amp" stAng="cd2" swAng="cd4"/>
  <a:arcTo wR="wd4" hR="amp" stAng="3cd4" swAng="cd4"/>
  <a:lnTo><a:pt x="r" y="yLow"/></a:lnTo>
  <a:arcTo wR="wd4" hR="amp" stAng="0" swAng="-5400000"/>
  <a:arcTo wR="wd4" hR="amp" stAng="3cd4" swAng="-5400000"/>
  <a:arcTo wR="wd4" hR="amp" stAng="0" swAng="cd4"/>
  <a:arcTo wR="wd4" hR="amp" stAng="cd4" swAng="cd4"/>
  <a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartSummingJunction`.
///
/// Measured: an ellipse inscribed in the box plus two diagonals joining the points of the ellipse
/// at 45, 135, 225 and 315 degrees of the ellipse parameter (`hc +- wd2 cos 45`, `vc +- hd2 sin
/// 45`: 31.63 and 184.37 by 21.09 and 122.91 pt on 216x144). The diagonals are open strokes.
fn summing_junction() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="dx" fmla="cos wd2 2700000"/>
  <a:gd name="dy" fmla="sin hd2 2700000"/>
  <a:gd name="xl" fmla="+- hc 0 dx"/><a:gd name="xr" fmla="+- hc dx 0"/>
  <a:gd name="yt" fmla="+- vc 0 dy"/><a:gd name="yb" fmla="+- vc dy 0"/>
</a:gdLst><a:pathLst>
<a:path fill="none"><a:moveTo><a:pt x="xl" y="yt"/></a:moveTo><a:lnTo><a:pt x="xr" y="yb"/></a:lnTo></a:path>
<a:path fill="none"><a:moveTo><a:pt x="xr" y="yt"/></a:moveTo><a:lnTo><a:pt x="xl" y="yb"/></a:lnTo></a:path>
<a:path>
  <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
  <a:arcTo wR="wd2" hR="hd2" stAng="cd2" swAng="cd4"/>
  <a:arcTo wR="wd2" hR="hd2" stAng="3cd4" swAng="cd4"/>
  <a:arcTo wR="wd2" hR="hd2" stAng="0" swAng="cd4"/>
  <a:arcTo wR="wd2" hR="hd2" stAng="cd4" swAng="cd4"/>
  <a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartOr`: an ellipse inscribed in the box plus the vertical and horizontal centre lines
/// (open strokes from edge to edge). No adjusts.
fn or() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst>
<a:path fill="none"><a:moveTo><a:pt x="hc" y="t"/></a:moveTo><a:lnTo><a:pt x="hc" y="b"/></a:lnTo></a:path>
<a:path fill="none"><a:moveTo><a:pt x="l" y="vc"/></a:moveTo><a:lnTo><a:pt x="r" y="vc"/></a:lnTo></a:path>
<a:path>
  <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
  <a:arcTo wR="wd2" hR="hd2" stAng="cd2" swAng="cd4"/>
  <a:arcTo wR="wd2" hR="hd2" stAng="3cd4" swAng="cd4"/>
  <a:arcTo wR="wd2" hR="hd2" stAng="0" swAng="cd4"/>
  <a:arcTo wR="wd2" hR="hd2" stAng="cd4" swAng="cd4"/>
  <a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartCollate`: an hourglass. One closed path: top left, top right, centre, bottom right,
/// bottom left, centre again (measured as six points; the centre appears twice).
fn collate() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path>
  <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo>
  <a:lnTo><a:pt x="hc" y="vc"/></a:lnTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo>
  <a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:lnTo><a:pt x="hc" y="vc"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartSort`: a diamond through the four edge midpoints plus the horizontal centre line
/// (open stroke, left edge to right edge).
fn sort() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst>
<a:path fill="none"><a:moveTo><a:pt x="l" y="vc"/></a:moveTo><a:lnTo><a:pt x="r" y="vc"/></a:lnTo></a:path>
<a:path>
  <a:moveTo><a:pt x="l" y="vc"/></a:moveTo><a:lnTo><a:pt x="hc" y="t"/></a:lnTo>
  <a:lnTo><a:pt x="r" y="vc"/></a:lnTo><a:lnTo><a:pt x="hc" y="b"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartExtract`: a triangle, apex at the top middle, base on the bottom edge.
fn extract() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path>
  <a:moveTo><a:pt x="l" y="b"/></a:moveTo><a:lnTo><a:pt x="hc" y="t"/></a:lnTo>
  <a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartMerge`: a triangle, base on the top edge, apex at the bottom middle.
fn merge() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path>
  <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo>
  <a:lnTo><a:pt x="hc" y="b"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartOfflineStorage`.
///
/// Measured: a downward triangle (as `flowChartMerge`) plus a short open stroke inside it at
/// `y = 0.8 h` from `x = 0.4 w` to `x = 0.6 w` (57.6 to 86.4 of 144 pt; 115.2 to 172.8 of 288).
fn offline_storage() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="y8" fmla="*/ h 4 5"/>
  <a:gd name="x4" fmla="*/ w 2 5"/>
  <a:gd name="x6" fmla="*/ w 3 5"/>
</a:gdLst><a:pathLst>
<a:path fill="none"><a:moveTo><a:pt x="x4" y="y8"/></a:moveTo><a:lnTo><a:pt x="x6" y="y8"/></a:lnTo></a:path>
<a:path>
  <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo>
  <a:lnTo><a:pt x="hc" y="b"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartOnlineStorage`.
///
/// Measured: a drum on its side. Top and bottom edges run from `x = w / 6` to the right edge; the
/// right side is a half ellipse bulging inwards (centre on the right edge, radii `w / 6` by
/// `h / 2`, so the deepest point is at `5 w / 6`); the left side bulges outwards with the same
/// radii centred at `x = w / 6`. 24, 36 and 48 pt at 144, 216 and 288 pt wide.
fn online_storage() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path>
  <a:moveTo><a:pt x="wd6" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo>
  <a:arcTo wR="wd6" hR="hd2" stAng="3cd4" swAng="-10800000"/>
  <a:lnTo><a:pt x="wd6" y="b"/></a:lnTo>
  <a:arcTo wR="wd6" hR="hd2" stAng="cd4" swAng="cd2"/>
  <a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartMagneticTape`.
///
/// Measured: an ellipse inscribed in the box, left open at the lower right: from the bottom
/// middle point clockwise (left, top, right) and on to the ellipse point at parameter 45 degrees
/// (`hc + wd2 cos 45`, `vc + hd2 sin 45`: 122.91 of 144 pt), then a horizontal line to the right
/// edge, down to the bottom right corner and back along the bottom edge (closing line).
fn magnetic_tape() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="dx" fmla="cos wd2 2700000"/>
  <a:gd name="dy" fmla="sin hd2 2700000"/>
  <a:gd name="ye" fmla="+- vc dy 0"/>
  <a:gd name="angE" fmla="at2 dx dy"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="hc" y="b"/></a:moveTo>
  <a:arcTo wR="wd2" hR="hd2" stAng="cd4" swAng="3cd4"/>
  <a:arcTo wR="wd2" hR="hd2" stAng="0" swAng="angE"/>
  <a:lnTo><a:pt x="r" y="ye"/></a:lnTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartMagneticDisk`.
///
/// Measured: a can. Elliptical caps with vertical radius `h / 6` (24 of 144 pt, 16 of 96) and the
/// full half width; the outline is the top cap, the straight sides and the bottom half of the lower
/// cap. A second open stroke is the front half of the top cap's rim (from the right to the left at
/// `y = h / 6`, dipping to `h / 3`). Word shades nothing here (`b07.fill`: one fill colour).
fn magnetic_disk() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="cap" fmla="*/ h 1 6"/>
  <a:gd name="yb" fmla="+- b 0 cap"/>
</a:gdLst><a:pathLst>
<a:path>
  <a:moveTo><a:pt x="l" y="cap"/></a:moveTo>
  <a:arcTo wR="wd2" hR="cap" stAng="cd2" swAng="cd2"/>
  <a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
  <a:arcTo wR="wd2" hR="cap" stAng="0" swAng="cd2"/>
  <a:close/>
</a:path>
<a:path fill="none">
  <a:moveTo><a:pt x="r" y="cap"/></a:moveTo>
  <a:arcTo wR="wd2" hR="cap" stAng="0" swAng="cd2"/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartMagneticDrum`.
///
/// Measured: a can on its side, as `flowChartOnlineStorage` mirrored on the right side. Both end
/// caps are half ellipses of radii `w / 6` by `h / 2` bulging outwards (centres at `w / 6` and
/// `5 w / 6`). The inner stroke is the left half of the right cap (from the bottom of the right
/// cap to its top, through `x = 2 w / 3`).
fn magnetic_drum() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="xr" fmla="+- r 0 wd6"/>
</a:gdLst><a:pathLst>
<a:path>
  <a:moveTo><a:pt x="wd6" y="t"/></a:moveTo><a:lnTo><a:pt x="xr" y="t"/></a:lnTo>
  <a:arcTo wR="wd6" hR="hd2" stAng="3cd4" swAng="cd2"/>
  <a:lnTo><a:pt x="wd6" y="b"/></a:lnTo>
  <a:arcTo wR="wd6" hR="hd2" stAng="cd4" swAng="cd2"/>
  <a:close/>
</a:path>
<a:path fill="none">
  <a:moveTo><a:pt x="xr" y="b"/></a:moveTo>
  <a:arcTo wR="wd6" hR="hd2" stAng="cd4" swAng="cd2"/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartDisplay`.
///
/// Measured: a point on the left middle, slanted edges to `(w / 6, 0)` and `(w / 6, h)`, straight
/// top and bottom to `x = 5 w / 6`, and a right half ellipse (radii `w / 6` by `h / 2`).
fn display() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="xr" fmla="+- r 0 wd6"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="l" y="vc"/></a:moveTo><a:lnTo><a:pt x="wd6" y="t"/></a:lnTo>
  <a:lnTo><a:pt x="xr" y="t"/></a:lnTo>
  <a:arcTo wR="wd6" hR="hd2" stAng="3cd4" swAng="cd2"/>
  <a:lnTo><a:pt x="wd6" y="b"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartDelay`: a D shape. Straight left side, top and bottom edges to `x = w / 2`, then a
/// half ellipse (radii `w / 2` by `h / 2`) on the right.
fn delay() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path>
  <a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="hc" y="t"/></a:lnTo>
  <a:arcTo wR="wd2" hR="hd2" stAng="3cd4" swAng="cd2"/>
  <a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `actionButtonBlank`: no adjusts. The frame is the box: filled and stroked, exactly the box
/// rectangle at every size.
const BTN_BLANK: &[UPath] = &[
    ("norm", false, "M l b L r b L r t L l t Z"),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// `actionButtonHome`: no adjusts. Icon in `ss / 32` units about the centre: a house (roof apex
/// `(0, -12)`, eaves at `+-12`, walls at `+-9` down to `y = 12`), a chimney (`4.5..7.5` by
/// `-10.5..-4.5`), a roof base line at `y = 0` and a door (`+-1.5`, `6..12`). Shading read from the
/// fill twin: roof and door darken, walls and chimney darkenLess.
const BTN_HOME: &[UPath] = &[
    (
        "norm",
        false,
        "M l t L r t L r b L l b Z M 0 -12 L -12 0 L -9 0 L -9 12 L 9 12 L 9 0 L 12 0 L 7.5 -4.5 L 7.5 -10.5 L 4.5 -10.5 L 4.5 -7.5 Z",
    ),
    (
        "darkenLess",
        false,
        "M 7.5 -4.5 L 7.5 -10.5 L 4.5 -10.5 L 4.5 -7.5 Z M -9 0 L -9 12 L -1.5 12 L -1.5 6 L 1.5 6 L 1.5 12 L 9 12 L 9 0 Z",
    ),
    (
        "darken",
        false,
        "M 0 -12 L -12 0 L 12 0 Z M -1.5 6 L 1.5 6 L 1.5 12 L -1.5 12 Z",
    ),
    (
        "none",
        true,
        "M 0 -12 L 4.5 -7.5 L 4.5 -10.5 L 7.5 -10.5 L 7.5 -4.5 L 12 0 L 9 0 L 9 12 L -9 12 L -9 0 L -12 0 Z M 4.5 -7.5 L 7.5 -4.5 M 9 0 L -9 0 M -1.5 12 L -1.5 6 L 1.5 6 L 1.5 12",
    ),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// `actionButtonHelp`: no adjusts. A question mark in `ss / 32` units about the centre, as cubic
/// curves (hook from `(-6.858, -5.142)` over the top at `y = -12`, stem to `y = 6`, dot centred at
/// `(0, 9.43)`), shaded darken. Curve control points are the measured values.
const BTN_HELP: &[UPath] = &[
    (
        "norm",
        false,
        "M l t L r t L r b L l b Z M -6.858 -5.142 C -6.858 -8.929 -3.787 -12 0 -12 C 3.787 -12 6.858 -8.929 6.858 -5.142 C 6.858 -2.302 5.322 0 3.429 0 C 2.482 0 1.713 1.151 1.713 2.571 L 1.713 6 L -1.713 6 L -1.713 2.571 C -1.713 -0.269 -0.18 -2.571 1.713 -2.571 C 2.66 -2.571 3.429 -3.722 3.429 -5.142 C 3.429 -7.036 1.893 -8.571 0 -8.571 C -1.893 -8.571 -3.429 -7.036 -3.429 -5.142 Z M 0 6.858 C 1.42 6.858 2.571 8.009 2.571 9.429 C 2.571 10.849 1.42 12 0 12 C -1.42 12 -2.571 10.849 -2.571 9.429 C -2.571 8.009 -1.42 6.858 0 6.858 Z",
    ),
    (
        "darken",
        false,
        "M -6.858 -5.142 C -6.858 -8.929 -3.787 -12 0 -12 C 3.787 -12 6.858 -8.929 6.858 -5.142 C 6.858 -2.302 5.322 0 3.429 0 C 2.482 0 1.713 1.151 1.713 2.571 L 1.713 6 L -1.713 6 L -1.713 2.571 C -1.713 -0.269 -0.18 -2.571 1.713 -2.571 C 2.66 -2.571 3.429 -3.722 3.429 -5.142 C 3.429 -7.036 1.893 -8.571 0 -8.571 C -1.893 -8.571 -3.429 -7.036 -3.429 -5.142 Z M 0 6.858 C 1.42 6.858 2.571 8.009 2.571 9.429 C 2.571 10.849 1.42 12 0 12 C -1.42 12 -2.571 10.849 -2.571 9.429 C -2.571 8.009 -1.42 6.858 0 6.858 Z",
    ),
    (
        "none",
        true,
        "M -6.858 -5.142 C -6.858 -8.929 -3.787 -12 0 -12 C 3.787 -12 6.858 -8.929 6.858 -5.142 C 6.858 -2.302 5.322 0 3.429 0 C 2.482 0 1.713 1.151 1.713 2.571 L 1.713 6 L -1.713 6 L -1.713 2.571 C -1.713 -0.269 -0.18 -2.571 1.713 -2.571 C 2.66 -2.571 3.429 -3.722 3.429 -5.142 C 3.429 -7.036 1.893 -8.571 0 -8.571 C -1.893 -8.571 -3.429 -7.036 -3.429 -5.142 Z M 0 6.858 C 1.42 6.858 2.571 8.009 2.571 9.429 C 2.571 10.849 1.42 12 0 12 C -1.42 12 -2.571 10.849 -2.571 9.429 C -2.571 8.009 -1.42 6.858 0 6.858 Z",
    ),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// `actionButtonInformation`: no adjusts. A circle of radius 12 units about the centre (darken)
/// with a lighter (lightenLess) letter i: dot centred at `(0, -9)`, radius about 2.25, and the
/// serifed stem `(-4.5..4.5) x (-4.5..9)`. Units are `ss / 32`.
const BTN_INFORMATION: &[UPath] = &[
    (
        "norm",
        false,
        "M l t L r t L r b L l b Z M 0 -12 C 6.627 -12 12 -6.627 12 0 C 12 6.627 6.627 12 0 12 C -6.627 12 -12 6.627 -12 0 C -12 -6.627 -6.627 -12 0 -12 Z",
    ),
    (
        "darken",
        false,
        "M 0 -12 C 6.627 -12 12 -6.627 12 0 C 12 6.627 6.627 12 0 12 C -6.627 12 -12 6.627 -12 0 C -12 -6.627 -6.627 -12 0 -12 Z M 0 -11.251 C 1.242 -11.251 2.251 -10.242 2.251 -9 C 2.251 -7.758 1.242 -6.751 0 -6.751 C -1.242 -6.751 -2.251 -7.758 -2.251 -9 C -2.251 -10.242 -1.242 -11.251 0 -11.251 M -4.5 -4.5 L -4.5 -3 L -2.251 -3 L -2.251 7.5 L -4.5 7.5 L -4.5 9 L 4.5 9 L 4.5 7.5 L 2.251 7.5 L 2.251 -4.5 Z",
    ),
    (
        "lightenLess",
        false,
        "M 0 -11.251 C 1.242 -11.251 2.251 -10.242 2.251 -9 C 2.251 -7.758 1.242 -6.751 0 -6.751 C -1.242 -6.751 -2.251 -7.758 -2.251 -9 C -2.251 -10.242 -1.242 -11.251 0 -11.251 M -4.5 -4.5 L 2.251 -4.5 L 2.251 7.5 L 4.5 7.5 L 4.5 9 L -4.5 9 L -4.5 7.5 L -2.251 7.5 L -2.251 -3 L -4.5 -3 Z",
    ),
    (
        "none",
        true,
        "M 0 -12 C 6.627 -12 12 -6.627 12 0 C 12 6.627 6.627 12 0 12 C -6.627 12 -12 6.627 -12 0 C -12 -6.627 -6.627 -12 0 -12 Z M 0 -11.251 C 1.242 -11.251 2.251 -10.242 2.251 -9 C 2.251 -7.758 1.242 -6.751 0 -6.751 C -1.242 -6.751 -2.251 -7.758 -2.251 -9 C -2.251 -10.242 -1.242 -11.251 0 -11.251 M -4.5 -4.5 L 2.251 -4.5 L 2.251 7.5 L 4.5 7.5 L 4.5 9 L -4.5 9 L -4.5 7.5 L -2.251 7.5 L -2.251 -3 L -4.5 -3 Z",
    ),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// `actionButtonForwardNext`: no adjusts. A right pointing triangle `(12,0) (-12,-12) (-12,12)`
/// in `ss / 32` units about the centre, shaded darken.
const BTN_FORWARD_NEXT: &[UPath] = &[
    (
        "norm",
        false,
        "M l t L r t L r b L l b Z M 12 0 L -12 -12 L -12 12 Z",
    ),
    ("darken", false, "M 12 0 L -12 -12 L -12 12 Z"),
    ("none", true, "M 12 0 L -12 12 L -12 -12 Z"),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// Entries of this batch `(name, definition)`.
pub(super) fn entries() -> Vec<(&'static str, ShapeDef)> {
    let all = [
        ("flowChartManualOperation", manual_operation()),
        ("flowChartConnector", connector()),
        ("flowChartOffpageConnector", offpage_connector()),
        ("flowChartPunchedCard", punched_card()),
        ("flowChartPunchedTape", punched_tape()),
        ("flowChartSummingJunction", summing_junction()),
        ("flowChartOr", or()),
        ("flowChartCollate", collate()),
        ("flowChartSort", sort()),
        ("flowChartExtract", extract()),
        ("flowChartMerge", merge()),
        ("flowChartOfflineStorage", offline_storage()),
        ("flowChartOnlineStorage", online_storage()),
        ("flowChartMagneticTape", magnetic_tape()),
        ("flowChartMagneticDisk", magnetic_disk()),
        ("flowChartMagneticDrum", magnetic_drum()),
        ("flowChartDisplay", display()),
        ("flowChartDelay", delay()),
        ("actionButtonBlank", unit_shape(BTN_BLANK)),
        ("actionButtonHome", unit_shape(BTN_HOME)),
        ("actionButtonHelp", unit_shape(BTN_HELP)),
        ("actionButtonInformation", unit_shape(BTN_INFORMATION)),
        ("actionButtonForwardNext", unit_shape(BTN_FORWARD_NEXT)),
    ];
    all.into_iter()
        .filter_map(|(n, d)| d.map(|d| (n, d)))
        .collect()
}
