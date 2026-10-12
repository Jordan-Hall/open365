//! Preset batch b08 (the last seven action buttons, gears, funnel, math signs, corner/square/plaque
//! tabs and chart marks), rebuilt clean-room from Word measurements (docs/architecture/drawing.md).
//!
//! Source: `fixtures/reference/drawing/presets/b08.{docx,pdf,probes.json,handles.json,paths.json}`
//! and the fill-enabled twin `b08.fill.*` (used to read which sub-areas Word shades), both made by
//! `tools/office/make-preset-probes.ps1`. Every formula below was derived from those measured
//! paths and handle defaults only; no other project's preset table and no copy of Ecma's
//! `presetShapeDefinitions.xml` was opened. Guide names are our own.
//!
//! Findings common to the batch:
//! - The math signs keep their arms inside a box inset of 13.255% of each side (`w * 13255 / 100000`
//!   and the same of `h`), the thicknesses are fractions of `ss` (plus, multiply) or of `h` (minus,
//!   divide, equal, not equal), and every adjust is clamped before use.
//! - Word reports the angle adjust of `mathNotEqual` in degrees (110) and the other adjusts as
//!   fractions of 100000; the harness converts both.
//! - The action buttons use the same `ss / 32` icon grid as in b07 (`presets_b07::unit_shape`).
//! - Shading paths (`stroke="0"`) are checked by `tests/preset_fills_b07_b08.rs`.

use super::ShapeDef;
use super::presets_b07::{UPath, unit_shape};
use crate::parse::cust_geom_from_str;
use std::fmt::Write as _;

fn parse(xml: &str) -> Option<ShapeDef> {
    cust_geom_from_str(xml).ok()
}

/// `mathPlus`.
///
/// Measured: one adjust `adj1`, default 23520. A plus of two bars of thickness `t = ss * adj1 /
/// 100000` (33.87 of 144 pt, 22.58 of 96 pt), centred; the horizontal bar spans `0.13255 w` to
/// `0.86745 w` and the vertical bar `0.13255 h` to `0.86745 h` (19.09 and 124.91 pt of 144; 38.17
/// and 249.83 of 288). The adjust is clamped to 0..73490: at 100000 and 400000 the bars are
/// `0.7349 ss` thick (105.8 of 144 pt) and at 0 and -50000 they collapse to lines.
fn math_plus() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 23520"/></a:avLst><a:gdLst>
  <a:gd name="a" fmla="pin 0 adj1 73490"/>
  <a:gd name="t" fmla="*/ ss a 100000"/>
  <a:gd name="ht" fmla="*/ t 1 2"/>
  <a:gd name="xl" fmla="*/ w 13255 100000"/><a:gd name="xr" fmla="+- r 0 xl"/>
  <a:gd name="yt" fmla="*/ h 13255 100000"/><a:gd name="yb" fmla="+- b 0 yt"/>
  <a:gd name="x1" fmla="+- hc 0 ht"/><a:gd name="x2" fmla="+- hc ht 0"/>
  <a:gd name="y1" fmla="+- vc 0 ht"/><a:gd name="y2" fmla="+- vc ht 0"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="xl" y="y1"/></a:moveTo><a:lnTo><a:pt x="x1" y="y1"/></a:lnTo>
  <a:lnTo><a:pt x="x1" y="yt"/></a:lnTo><a:lnTo><a:pt x="x2" y="yt"/></a:lnTo>
  <a:lnTo><a:pt x="x2" y="y1"/></a:lnTo><a:lnTo><a:pt x="xr" y="y1"/></a:lnTo>
  <a:lnTo><a:pt x="xr" y="y2"/></a:lnTo><a:lnTo><a:pt x="x2" y="y2"/></a:lnTo>
  <a:lnTo><a:pt x="x2" y="yb"/></a:lnTo><a:lnTo><a:pt x="x1" y="yb"/></a:lnTo>
  <a:lnTo><a:pt x="x1" y="y2"/></a:lnTo><a:lnTo><a:pt x="xl" y="y2"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `mathMinus`.
///
/// Measured: one adjust `adj1`, default 23520. A bar from `0.13255 w` to `0.86745 w`, thickness
/// `h * adj1 / 100000` (33.87 of 144; 22.58 of 96; 67.74 of 288 on a 96x288 box, so the height and
/// not `ss` is the base), centred vertically. Clamped to 0..100000 (at 100000 and 400000 the bar
/// fills the box height; at 0 and -50000 it is a line).
fn math_minus() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 23520"/></a:avLst><a:gdLst>
  <a:gd name="a" fmla="pin 0 adj1 100000"/>
  <a:gd name="ht" fmla="*/ h a 200000"/>
  <a:gd name="xl" fmla="*/ w 13255 100000"/><a:gd name="xr" fmla="+- r 0 xl"/>
  <a:gd name="y1" fmla="+- vc 0 ht"/><a:gd name="y2" fmla="+- vc ht 0"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="xl" y="y1"/></a:moveTo><a:lnTo><a:pt x="xr" y="y1"/></a:lnTo>
  <a:lnTo><a:pt x="xr" y="y2"/></a:lnTo><a:lnTo><a:pt x="xl" y="y2"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `mathMultiply`.
///
/// Measured: one adjust `adj1`, default 23520. Two bars of thickness `t = ss * adj1 / 100000`
/// along the box diagonals (direction `(w, h)` and `(w, -h)`, not 45 degrees on a non-square
/// box), crossing at the centre, ends square to the bar. The centre of each bar end sits at
/// `(+-0.25981 w, +-0.25981 h)` from the centre. The corner points of an end are the end centre
/// `+- (t / 2)` along the perpendicular `(h, -w) / d` (or `(h, w) / d` for the other bar), with
/// `d = sqrt(w^2 + h^2)`; the four inner corners where the bars meet are at `t d / (2 w)` above
/// and below the centre and `t d / (2 h)` to the left and right. Clamped to 0..51962 (twice the
/// 0.25981 factor): at 100000 and 400000 the bars are 74.8 pt thick on 216x144; at 0 and -50000
/// they collapse to the diagonals.
fn math_multiply() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 23520"/></a:avLst><a:gdLst>
  <a:gd name="a" fmla="pin 0 adj1 51962"/>
  <a:gd name="t" fmla="*/ ss a 100000"/>
  <a:gd name="d" fmla="mod w h 0"/>
  <a:gd name="d2" fmla="*/ d 2 1"/>
  <a:gd name="w2" fmla="*/ w 2 1"/>
  <a:gd name="h2" fmla="*/ h 2 1"/>
  <a:gd name="ex" fmla="*/ w 25981 100000"/>
  <a:gd name="ey" fmla="*/ h 25981 100000"/>
  <a:gd name="dx" fmla="*/ t h d2"/>
  <a:gd name="dy" fmla="*/ t w d2"/>
  <a:gd name="ix" fmla="*/ t d h2"/>
  <a:gd name="iy" fmla="*/ t d w2"/>
  <a:gd name="xA" fmla="+- hc 0 ex"/><a:gd name="xA1" fmla="+- xA 0 dx"/><a:gd name="xB1" fmla="+- xA dx 0"/>
  <a:gd name="xC0" fmla="+- hc ex 0"/><a:gd name="xC1" fmla="+- xC0 0 dx"/><a:gd name="xD1" fmla="+- xC0 dx 0"/>
  <a:gd name="yA" fmla="+- vc 0 ey"/><a:gd name="yA1" fmla="+- yA 0 dy"/><a:gd name="yB1" fmla="+- yA dy 0"/>
  <a:gd name="yC0" fmla="+- vc ey 0"/><a:gd name="yC1" fmla="+- yC0 0 dy"/><a:gd name="yD1" fmla="+- yC0 dy 0"/>
  <a:gd name="yTop" fmla="+- vc 0 iy"/><a:gd name="yBot" fmla="+- vc iy 0"/>
  <a:gd name="xRt" fmla="+- hc ix 0"/><a:gd name="xLf" fmla="+- hc 0 ix"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="xA1" y="yB1"/></a:moveTo><a:lnTo><a:pt x="xB1" y="yA1"/></a:lnTo>
  <a:lnTo><a:pt x="hc" y="yTop"/></a:lnTo><a:lnTo><a:pt x="xC1" y="yA1"/></a:lnTo>
  <a:lnTo><a:pt x="xD1" y="yB1"/></a:lnTo><a:lnTo><a:pt x="xRt" y="vc"/></a:lnTo>
  <a:lnTo><a:pt x="xD1" y="yC1"/></a:lnTo><a:lnTo><a:pt x="xC1" y="yD1"/></a:lnTo>
  <a:lnTo><a:pt x="hc" y="yBot"/></a:lnTo><a:lnTo><a:pt x="xB1" y="yD1"/></a:lnTo>
  <a:lnTo><a:pt x="xA1" y="yC1"/></a:lnTo><a:lnTo><a:pt x="xLf" y="vc"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `mathDivide`.
///
/// Measured: three adjusts, defaults 23520 (bar thickness), 5880 (gap between bar and dots) and
/// 11760 (dot radius), all fractions of `h` (checked on a 96x288 box). The bar spans `0.13255 w`
/// to `0.86745 w`, centred vertically; the two dots are circles centred on `hc`, above and below
/// the bar, so the upper dot centre is `vc - t/2 - gap - r`. Clamps seen with the all-adjust
/// pages: `adj1` 1000..36745, `adj3` 1000..(73490 - adj1) / 4, `adj2` 0..73490 - adj1 - 4 adj3
/// (so a dot never leaves the box; at 30000 each, the dot radius becomes 15.66 of 144 pt and the
/// gap 0).
fn math_divide() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst>
  <a:gd name="adj1" fmla="val 23520"/><a:gd name="adj2" fmla="val 5880"/><a:gd name="adj3" fmla="val 11760"/>
</a:avLst><a:gdLst>
  <a:gd name="a1" fmla="pin 1000 adj1 36745"/>
  <a:gd name="room" fmla="+- 73490 0 a1"/>
  <a:gd name="a3max" fmla="*/ room 1 4"/>
  <a:gd name="a3" fmla="pin 1000 adj3 a3max"/>
  <a:gd name="a3x4" fmla="*/ a3 4 1"/>
  <a:gd name="a2max" fmla="+- room 0 a3x4"/>
  <a:gd name="a2" fmla="pin 0 adj2 a2max"/>
  <a:gd name="t" fmla="*/ h a1 100000"/>
  <a:gd name="rd" fmla="*/ h a3 100000"/>
  <a:gd name="gap" fmla="*/ h a2 100000"/>
  <a:gd name="ht" fmla="*/ t 1 2"/>
  <a:gd name="xl" fmla="*/ w 13255 100000"/><a:gd name="xr" fmla="+- r 0 xl"/>
  <a:gd name="y1" fmla="+- vc 0 ht"/><a:gd name="y2" fmla="+- vc ht 0"/>
  <a:gd name="yu0" fmla="+- y1 0 gap"/><a:gd name="yuc" fmla="+- yu0 0 rd"/><a:gd name="yut" fmla="+- yuc 0 rd"/>
  <a:gd name="yl0" fmla="+- y2 gap 0"/><a:gd name="ylc" fmla="+- yl0 rd 0"/><a:gd name="ylt" fmla="+- ylc 0 rd"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="hc" y="yut"/></a:moveTo>
  <a:arcTo wR="rd" hR="rd" stAng="3cd4" swAng="cd4"/><a:arcTo wR="rd" hR="rd" stAng="0" swAng="cd4"/>
  <a:arcTo wR="rd" hR="rd" stAng="cd4" swAng="cd4"/><a:arcTo wR="rd" hR="rd" stAng="cd2" swAng="cd4"/>
  <a:close/>
  <a:moveTo><a:pt x="hc" y="ylt"/></a:moveTo>
  <a:arcTo wR="rd" hR="rd" stAng="3cd4" swAng="cd4"/><a:arcTo wR="rd" hR="rd" stAng="0" swAng="cd4"/>
  <a:arcTo wR="rd" hR="rd" stAng="cd4" swAng="cd4"/><a:arcTo wR="rd" hR="rd" stAng="cd2" swAng="cd4"/>
  <a:close/>
  <a:moveTo><a:pt x="xl" y="y1"/></a:moveTo><a:lnTo><a:pt x="xr" y="y1"/></a:lnTo>
  <a:lnTo><a:pt x="xr" y="y2"/></a:lnTo><a:lnTo><a:pt x="xl" y="y2"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `mathEqual`.
///
/// Measured: two adjusts, defaults 23520 (bar thickness) and 11760 (gap between the bars), both
/// fractions of `h` (checked at 96x288: bars 67.74, gap 33.86). Two bars from `0.13255 w` to
/// `0.86745 w`, centred as a pair. Clamps: `adj1` 0..36745, `adj2` 0..100000 - 2 adj1 (at 100000
/// each on 216x144 the bars are 52.91 thick with a 38.18 gap and fill the box height; at 0 and
/// -50000 both collapse onto the centre line).
fn math_equal() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst>
  <a:gd name="adj1" fmla="val 23520"/><a:gd name="adj2" fmla="val 11760"/>
</a:avLst><a:gdLst>
  <a:gd name="a1" fmla="pin 0 adj1 36745"/>
  <a:gd name="a1x2" fmla="*/ a1 2 1"/>
  <a:gd name="a2max" fmla="+- 100000 0 a1x2"/>
  <a:gd name="a2" fmla="pin 0 adj2 a2max"/>
  <a:gd name="t" fmla="*/ h a1 100000"/>
  <a:gd name="gap" fmla="*/ h a2 100000"/>
  <a:gd name="hg" fmla="*/ gap 1 2"/>
  <a:gd name="xl" fmla="*/ w 13255 100000"/><a:gd name="xr" fmla="+- r 0 xl"/>
  <a:gd name="y1" fmla="+- vc 0 hg"/><a:gd name="y0" fmla="+- y1 0 t"/>
  <a:gd name="y2" fmla="+- vc hg 0"/><a:gd name="y3" fmla="+- y2 t 0"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="xl" y="y0"/></a:moveTo><a:lnTo><a:pt x="xr" y="y0"/></a:lnTo>
  <a:lnTo><a:pt x="xr" y="y1"/></a:lnTo><a:lnTo><a:pt x="xl" y="y1"/></a:lnTo><a:close/>
  <a:moveTo><a:pt x="xl" y="y2"/></a:moveTo><a:lnTo><a:pt x="xr" y="y2"/></a:lnTo>
  <a:lnTo><a:pt x="xr" y="y3"/></a:lnTo><a:lnTo><a:pt x="xl" y="y3"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `mathNotEqual`.
///
/// Measured: three adjusts, defaults 23520 (bar thickness, of `h`), 110 degrees (6600000, the
/// slash angle) and 11760 (gap, of `h`). The two bars of `mathEqual` crossed by a slash of the
/// same thickness `t`, drawn as one outline. The slash is a rectangle of width `t` through the
/// centre whose direction makes `adj2` with the x axis (110 degrees leans like `/`, 70 degrees
/// like `\`); its ends are cut square and its length is the one that makes the highest end
/// corner touch the box top (and the lowest the box bottom): half length `(h/2 - t/2 |cos a|) /
/// sin a`. The outline runs along the bars and turns onto the slash edges where they cross the
/// bar edges, so each bar edge is split by the slash edges (edge x at height y is the corner x
/// minus `(corner y - y) cot a`). Clamps: `adj1` 0..50000 (at 100000 the two bars meet at the
/// centre and fill the box height), `adj3` 0..100000 - 2 adj1, `adj2` 4200000..6600000 (70..110
/// degrees: 0, 30000, 100000, 400000 and -50000 all show the 70 degree slash). The `adj1`
/// ceiling is only seen at its far end (100000): it lies between 30000 and 50000 + epsilon and
/// 50000 matches the one probe at that end.
fn math_not_equal() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst>
  <a:gd name="adj1" fmla="val 23520"/><a:gd name="adj2" fmla="val 6600000"/><a:gd name="adj3" fmla="val 11760"/>
</a:avLst><a:gdLst>
  <a:gd name="a1" fmla="pin 0 adj1 50000"/>
  <a:gd name="a1x2" fmla="*/ a1 2 1"/>
  <a:gd name="a3max" fmla="+- 100000 0 a1x2"/>
  <a:gd name="a3" fmla="pin 0 adj3 a3max"/>
  <a:gd name="ang" fmla="pin 4200000 adj2 6600000"/>
  <a:gd name="t" fmla="*/ h a1 100000"/>
  <a:gd name="gap" fmla="*/ h a3 100000"/>
  <a:gd name="hg" fmla="*/ gap 1 2"/>
  <a:gd name="xl" fmla="*/ w 13255 100000"/><a:gd name="xr" fmla="+- r 0 xl"/>
  <a:gd name="y1" fmla="+- vc 0 hg"/><a:gd name="y0" fmla="+- y1 0 t"/>
  <a:gd name="y2" fmla="+- vc hg 0"/><a:gd name="y3" fmla="+- y2 t 0"/>
  <a:gd name="cs" fmla="cos 100000 ang"/>
  <a:gd name="sn" fmla="sin 100000 ang"/>
  <a:gd name="acs" fmla="abs cs"/>
  <a:gd name="cot" fmla="*/ cs 1 sn"/>
  <a:gd name="tcs" fmla="*/ t acs 200000"/>
  <a:gd name="lnum" fmla="+- vc 0 tcs"/>
  <a:gd name="len" fmla="*/ lnum 100000 sn"/>
  <a:gd name="lc" fmla="*/ len cs 100000"/>
  <a:gd name="ls" fmla="*/ len sn 100000"/>
  <a:gd name="tc" fmla="*/ t cs 200000"/>
  <a:gd name="ts" fmla="*/ t sn 200000"/>
  <a:gd name="axa" fmla="+- hc 0 lc"/><a:gd name="ax" fmla="+- axa 0 ts"/><a:gd name="bx" fmla="+- axa ts 0"/>
  <a:gd name="ay0" fmla="+- vc 0 ls"/><a:gd name="ay" fmla="+- ay0 tc 0"/><a:gd name="by" fmla="+- ay0 0 tc"/>
  <a:gd name="cxa" fmla="+- hc lc 0"/><a:gd name="cx" fmla="+- cxa ts 0"/><a:gd name="dx" fmla="+- cxa 0 ts"/>
  <a:gd name="cy0" fmla="+- vc ls 0"/><a:gd name="cy" fmla="+- cy0 0 tc"/><a:gd name="dy" fmla="+- cy0 tc 0"/>
  <a:gd name="dLy0" fmla="+- ay 0 y0"/><a:gd name="oLy0" fmla="*/ dLy0 cot 1"/><a:gd name="xLy0" fmla="+- ax 0 oLy0"/>
  <a:gd name="dLy1" fmla="+- ay 0 y1"/><a:gd name="oLy1" fmla="*/ dLy1 cot 1"/><a:gd name="xLy1" fmla="+- ax 0 oLy1"/>
  <a:gd name="dLy2" fmla="+- ay 0 y2"/><a:gd name="oLy2" fmla="*/ dLy2 cot 1"/><a:gd name="xLy2" fmla="+- ax 0 oLy2"/>
  <a:gd name="dLy3" fmla="+- ay 0 y3"/><a:gd name="oLy3" fmla="*/ dLy3 cot 1"/><a:gd name="xLy3" fmla="+- ax 0 oLy3"/>
  <a:gd name="dRy0" fmla="+- by 0 y0"/><a:gd name="oRy0" fmla="*/ dRy0 cot 1"/><a:gd name="xRy0" fmla="+- bx 0 oRy0"/>
  <a:gd name="dRy1" fmla="+- by 0 y1"/><a:gd name="oRy1" fmla="*/ dRy1 cot 1"/><a:gd name="xRy1" fmla="+- bx 0 oRy1"/>
  <a:gd name="dRy2" fmla="+- by 0 y2"/><a:gd name="oRy2" fmla="*/ dRy2 cot 1"/><a:gd name="xRy2" fmla="+- bx 0 oRy2"/>
  <a:gd name="dRy3" fmla="+- by 0 y3"/><a:gd name="oRy3" fmla="*/ dRy3 cot 1"/><a:gd name="xRy3" fmla="+- bx 0 oRy3"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="xl" y="y0"/></a:moveTo><a:lnTo><a:pt x="xLy0" y="y0"/></a:lnTo>
  <a:lnTo><a:pt x="ax" y="ay"/></a:lnTo><a:lnTo><a:pt x="bx" y="by"/></a:lnTo>
  <a:lnTo><a:pt x="xRy0" y="y0"/></a:lnTo><a:lnTo><a:pt x="xr" y="y0"/></a:lnTo>
  <a:lnTo><a:pt x="xr" y="y1"/></a:lnTo><a:lnTo><a:pt x="xRy1" y="y1"/></a:lnTo>
  <a:lnTo><a:pt x="xRy2" y="y2"/></a:lnTo><a:lnTo><a:pt x="xr" y="y2"/></a:lnTo>
  <a:lnTo><a:pt x="xr" y="y3"/></a:lnTo><a:lnTo><a:pt x="xRy3" y="y3"/></a:lnTo>
  <a:lnTo><a:pt x="cx" y="cy"/></a:lnTo><a:lnTo><a:pt x="dx" y="dy"/></a:lnTo>
  <a:lnTo><a:pt x="xLy3" y="y3"/></a:lnTo><a:lnTo><a:pt x="xl" y="y3"/></a:lnTo>
  <a:lnTo><a:pt x="xl" y="y2"/></a:lnTo><a:lnTo><a:pt x="xLy2" y="y2"/></a:lnTo>
  <a:lnTo><a:pt x="xLy1" y="y1"/></a:lnTo><a:lnTo><a:pt x="xl" y="y1"/></a:lnTo><a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `cornerTabs`.
///
/// Measured: no adjusts. Four separate right isosceles triangles in the box corners, legs
/// `c = 0.05 * sqrt(w^2 + h^2)` (10.18 pt at 144x144, 12.98 at 216x144, from the box diagonal and
/// not from `ss`), each closed.
fn corner_tabs() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="d" fmla="mod w h 0"/><a:gd name="c" fmla="*/ d 5 100"/>
  <a:gd name="xr" fmla="+- r 0 c"/><a:gd name="yb" fmla="+- b 0 c"/>
</a:gdLst><a:pathLst>
<a:path><a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="c" y="t"/></a:lnTo><a:lnTo><a:pt x="l" y="c"/></a:lnTo><a:close/></a:path>
<a:path><a:moveTo><a:pt x="xr" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo><a:lnTo><a:pt x="r" y="c"/></a:lnTo><a:close/></a:path>
<a:path><a:moveTo><a:pt x="r" y="yb"/></a:moveTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:lnTo><a:pt x="xr" y="b"/></a:lnTo><a:close/></a:path>
<a:path><a:moveTo><a:pt x="l" y="yb"/></a:moveTo><a:lnTo><a:pt x="c" y="b"/></a:lnTo><a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:close/></a:path>
</a:pathLst></a:custGeom>"#,
    )
}

/// `squareTabs`: no adjusts. Four separate squares of side `c = 0.05 * sqrt(w^2 + h^2)` in the box
/// corners (see `cornerTabs`).
fn square_tabs() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="d" fmla="mod w h 0"/><a:gd name="c" fmla="*/ d 5 100"/>
  <a:gd name="xr" fmla="+- r 0 c"/><a:gd name="yb" fmla="+- b 0 c"/>
</a:gdLst><a:pathLst>
<a:path><a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="c" y="t"/></a:lnTo><a:lnTo><a:pt x="c" y="c"/></a:lnTo><a:lnTo><a:pt x="l" y="c"/></a:lnTo><a:close/></a:path>
<a:path><a:moveTo><a:pt x="xr" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo><a:lnTo><a:pt x="r" y="c"/></a:lnTo><a:lnTo><a:pt x="xr" y="c"/></a:lnTo><a:close/></a:path>
<a:path><a:moveTo><a:pt x="xr" y="yb"/></a:moveTo><a:lnTo><a:pt x="r" y="yb"/></a:lnTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:lnTo><a:pt x="xr" y="b"/></a:lnTo><a:close/></a:path>
<a:path><a:moveTo><a:pt x="l" y="yb"/></a:moveTo><a:lnTo><a:pt x="c" y="yb"/></a:lnTo><a:lnTo><a:pt x="c" y="b"/></a:lnTo><a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:close/></a:path>
</a:pathLst></a:custGeom>"#,
    )
}

/// `plaqueTabs`: no adjusts. Four separate quarter discs of radius `c = 0.05 * sqrt(w^2 + h^2)`
/// centred on the box corners (see `cornerTabs`); the outline of each is two legs along the box
/// edges and a quarter circle arc (control offset 0.5523 c).
fn plaque_tabs() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="d" fmla="mod w h 0"/><a:gd name="c" fmla="*/ d 5 100"/>
  <a:gd name="xr" fmla="+- r 0 c"/><a:gd name="yb" fmla="+- b 0 c"/>
</a:gdLst><a:pathLst>
<a:path><a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="c" y="t"/></a:lnTo>
  <a:arcTo wR="c" hR="c" stAng="0" swAng="cd4"/><a:close/></a:path>
<a:path><a:moveTo><a:pt x="r" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="c"/></a:lnTo>
  <a:arcTo wR="c" hR="c" stAng="cd4" swAng="cd4"/><a:close/></a:path>
<a:path><a:moveTo><a:pt x="xr" y="b"/></a:moveTo>
  <a:arcTo wR="c" hR="c" stAng="cd2" swAng="cd4"/><a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:close/></a:path>
<a:path><a:moveTo><a:pt x="l" y="yb"/></a:moveTo>
  <a:arcTo wR="c" hR="c" stAng="3cd4" swAng="cd4"/><a:lnTo><a:pt x="l" y="b"/></a:lnTo><a:close/></a:path>
</a:pathLst></a:custGeom>"#,
    )
}

/// `chartX`: no adjusts. Two open diagonal strokes corner to corner (top left to bottom right and
/// bottom left to top right). Word also fills the box rectangle (stroke off), which only shows when the
/// shape has a fill.
fn chart_x() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst>
<a:path stroke="0"><a:moveTo><a:pt x="l" y="b"/></a:moveTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo><a:lnTo><a:pt x="l" y="t"/></a:lnTo><a:close/></a:path>
<a:path fill="none"><a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo>
  <a:moveTo><a:pt x="l" y="b"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo></a:path>
</a:pathLst></a:custGeom>"#,
    )
}

/// `chartStar`: no adjusts. The two diagonals of `chartX` plus the vertical centre line (top
/// middle to bottom middle), all open strokes, over a filled (unstroked) box rectangle.
fn chart_star() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst>
<a:path stroke="0"><a:moveTo><a:pt x="l" y="b"/></a:moveTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo><a:lnTo><a:pt x="l" y="t"/></a:lnTo><a:close/></a:path>
<a:path fill="none"><a:moveTo><a:pt x="l" y="t"/></a:moveTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo>
  <a:moveTo><a:pt x="l" y="b"/></a:moveTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo>
  <a:moveTo><a:pt x="hc" y="t"/></a:moveTo><a:lnTo><a:pt x="hc" y="b"/></a:lnTo></a:path>
</a:pathLst></a:custGeom>"#,
    )
}

/// `chartPlus`: no adjusts. The vertical and horizontal centre lines, edge to edge, open strokes,
/// over a filled (unstroked) box rectangle.
fn chart_plus() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst>
<a:path stroke="0"><a:moveTo><a:pt x="l" y="b"/></a:moveTo><a:lnTo><a:pt x="r" y="b"/></a:lnTo><a:lnTo><a:pt x="r" y="t"/></a:lnTo><a:lnTo><a:pt x="l" y="t"/></a:lnTo><a:close/></a:path>
<a:path fill="none"><a:moveTo><a:pt x="hc" y="t"/></a:moveTo><a:lnTo><a:pt x="hc" y="b"/></a:lnTo>
  <a:moveTo><a:pt x="l" y="vc"/></a:moveTo><a:lnTo><a:pt x="r" y="vc"/></a:lnTo></a:path>
</a:pathLst></a:custGeom>"#,
    )
}

/// `funnel`.
///
/// Measured: no adjusts. Two subpaths in one path. (1) The outline: an arc of the ellipse inscribed
/// in the upper half of the box (centre `(hc, h/4)`, radii `w/2` by `h/4`, i.e. the "rim") from the
/// ellipse parameter 172 degrees through the top to 8 degrees (a 196 degree sweep), a straight line
/// to the spout, the spout arc (centre `(hc, 15 h / 16)`, radii `w/8` by `h/16`) from parameter 8
/// degrees through the bottom to 172 degrees, and the closing line back to the rim start. The
/// outline therefore sticks out of the box by about 0.7 pt at the top and 0.6 pt at the bottom.
/// (2) The inner opening: an ellipse with centre `(hc, h/4)` and radii `w/2 - m` by `h/4 - m`,
/// `m = ss / 20` (7.2 of 144 pt). Fit of the spout ellipse from the cubic control points at 144x144
/// and 288x96: radii 17.995 by 8.99 and 36.00 by 6.01.
fn funnel() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
  <a:gd name="m" fmla="*/ ss 1 20"/>
  <a:gd name="rxi" fmla="+- wd2 0 m"/>
  <a:gd name="ryi" fmla="+- hd4 0 m"/>
  <a:gd name="rcs" fmla="cos wd2 10320000"/>
  <a:gd name="rsn" fmla="sin hd4 10320000"/>
  <a:gd name="xRim" fmla="+- hc rcs 0"/>
  <a:gd name="yRim" fmla="+- hd4 rsn 0"/>
  <a:gd name="stRim" fmla="at2 rcs rsn"/>
  <a:gd name="ecs" fmla="cos wd2 480000"/>
  <a:gd name="esn" fmla="sin hd4 480000"/>
  <a:gd name="enRim" fmla="at2 ecs esn"/>
  <a:gd name="swRim" fmla="+- enRim 21600000 stRim"/>
  <a:gd name="sa" fmla="*/ w 1 8"/>
  <a:gd name="sb" fmla="*/ h 1 16"/>
  <a:gd name="cyS" fmla="+- b 0 sb"/>
  <a:gd name="scs" fmla="cos sa 480000"/>
  <a:gd name="ssn" fmla="sin sb 480000"/>
  <a:gd name="xSp" fmla="+- hc scs 0"/>
  <a:gd name="ySp" fmla="+- cyS ssn 0"/>
  <a:gd name="stSp" fmla="at2 scs ssn"/>
  <a:gd name="s2cs" fmla="cos sa 10320000"/>
  <a:gd name="s2sn" fmla="sin sb 10320000"/>
  <a:gd name="enSp" fmla="at2 s2cs s2sn"/>
  <a:gd name="swSp" fmla="+- enSp 0 stSp"/>
  <a:gd name="xIn" fmla="+- l m 0"/>
</a:gdLst><a:pathLst><a:path>
  <a:moveTo><a:pt x="xRim" y="yRim"/></a:moveTo>
  <a:arcTo wR="wd2" hR="hd4" stAng="stRim" swAng="swRim"/>
  <a:lnTo><a:pt x="xSp" y="ySp"/></a:lnTo>
  <a:arcTo wR="sa" hR="sb" stAng="stSp" swAng="swSp"/>
  <a:close/>
  <a:moveTo><a:pt x="xIn" y="hd4"/></a:moveTo>
  <a:arcTo wR="rxi" hR="ryi" stAng="cd2" swAng="cd4"/>
  <a:arcTo wR="rxi" hR="ryi" stAng="3cd4" swAng="cd4"/>
  <a:arcTo wR="rxi" hR="ryi" stAng="0" swAng="cd4"/>
  <a:arcTo wR="rxi" hR="ryi" stAng="cd4" swAng="cd4"/>
  <a:close/>
</a:path></a:pathLst></a:custGeom>"#,
    )
}

/// Gear with `n` teeth.
///
/// Measured on `gear6` (and cross-checked on `gear9`): two adjusts, `adj1` the tooth height as a
/// fraction of `ss` (clamped 0..20000: 30000, 100000 and 400000 all give 0.2 ss; 0 and -50000
/// give 0) and `adj2` the lateral offset between the base and the tip of a tooth edge as a
/// fraction of `ss` (clamped 0..`a2_max`, 5359 for six teeth and 2679 for nine teeth, as measured
/// with the all-adjust pages at the 0.2 ss tooth height; the ceiling at other tooth heights is not
/// visible in the probes).
///
/// Shape: the body is an ellipse of radii `rx = w/2 - t`, `ry = h/2 - t` (`t` = tooth height). Tooth
/// `k` is centred on the body ellipse point of parameter `phi_k = 360 k / n` (measured clockwise
/// from the top, point `(hc + rx sin phi, vc - ry cos phi)`). Its two base corners are the body
/// ellipse points of parameters `phi_k -+ d` with `d = atan((t + lh) / (2 rm))`, `rm = ss/2 - t`,
/// `lh = ss adj2 / 100000` (this single angle reproduces `gear6` at 144x144, 216x144, 288x96,
/// 96x288 and `gear9` likewise, to 0.01 pt, for every probed adjust pair). The tip corners are the
/// base corners moved by `t` along the outward normal of the body ellipse at `phi_k` (angle
/// `a = atan2(ry sin phi, rx cos phi)` from the vertical) and by `lh` along the tangent towards
/// the tooth centre, i.e. a rigid tooth, not scaled with the box. Between teeth the outline
/// follows the body ellipse.
fn gear(n: u32, a1: i64, a2: i64, a2_max: i64) -> Option<ShapeDef> {
    let mut g = String::new();
    let _ = write!(
        g,
        "<a:gd name=\"a1p\" fmla=\"pin 0 adj1 20000\"/><a:gd name=\"a2p\" fmla=\"pin 0 adj2 {a2_max}\"/>\
<a:gd name=\"th\" fmla=\"*/ ss a1p 100000\"/><a:gd name=\"lh\" fmla=\"*/ ss a2p 100000\"/>\
<a:gd name=\"rx\" fmla=\"+- wd2 0 th\"/><a:gd name=\"ry\" fmla=\"+- hd2 0 th\"/>\
<a:gd name=\"rm\" fmla=\"+- ssd2 0 th\"/><a:gd name=\"rm2\" fmla=\"*/ rm 2 1\"/>\
<a:gd name=\"tl\" fmla=\"+- th lh 0\"/><a:gd name=\"dl\" fmla=\"at2 rm2 tl\"/>"
    );
    for k in 0..n {
        let ph = (21_600_000.0 * f64::from(k) / f64::from(n)).round() as i64;
        let _ = write!(
            g,
            "<a:gd name=\"p{k}\" fmla=\"val {ph}\"/>\
<a:gd name=\"q{k}a\" fmla=\"+- p{k} 0 dl\"/><a:gd name=\"q{k}b\" fmla=\"+- p{k} dl 0\"/>\
<a:gd name=\"gx{k}a\" fmla=\"sin rx q{k}a\"/><a:gd name=\"gy{k}a\" fmla=\"cos ry q{k}a\"/>\
<a:gd name=\"gx{k}b\" fmla=\"sin rx q{k}b\"/><a:gd name=\"gy{k}b\" fmla=\"cos ry q{k}b\"/>\
<a:gd name=\"ng{k}a\" fmla=\"+- 0 0 gy{k}a\"/><a:gd name=\"ng{k}b\" fmla=\"+- 0 0 gy{k}b\"/>\
<a:gd name=\"X{k}a\" fmla=\"+- hc gx{k}a 0\"/><a:gd name=\"Y{k}a\" fmla=\"+- vc 0 gy{k}a\"/>\
<a:gd name=\"X{k}b\" fmla=\"+- hc gx{k}b 0\"/><a:gd name=\"Y{k}b\" fmla=\"+- vc 0 gy{k}b\"/>\
<a:gd name=\"cx{k}\" fmla=\"cos rx p{k}\"/><a:gd name=\"sy{k}\" fmla=\"sin ry p{k}\"/>\
<a:gd name=\"al{k}\" fmla=\"at2 cx{k} sy{k}\"/>\
<a:gd name=\"lc{k}\" fmla=\"cos lh al{k}\"/><a:gd name=\"ls{k}\" fmla=\"sin lh al{k}\"/>\
<a:gd name=\"tc{k}\" fmla=\"cos th al{k}\"/><a:gd name=\"ts{k}\" fmla=\"sin th al{k}\"/>\
<a:gd name=\"ux{k}\" fmla=\"+- X{k}a lc{k} 0\"/><a:gd name=\"TX{k}a\" fmla=\"+- ux{k} ts{k} 0\"/>\
<a:gd name=\"TY{k}a\" fmla=\"+- Y{k}a ls{k} tc{k}\"/>\
<a:gd name=\"TX{k}b\" fmla=\"+- X{k}b ts{k} lc{k}\"/>\
<a:gd name=\"uy{k}\" fmla=\"+- Y{k}b 0 ls{k}\"/><a:gd name=\"TY{k}b\" fmla=\"+- uy{k} 0 tc{k}\"/>\
<a:gd name=\"vs{k}\" fmla=\"at2 gx{k}b ng{k}b\"/><a:gd name=\"ve{k}\" fmla=\"at2 gx{k}a ng{k}a\"/>"
        );
    }
    // Valley k runs from base corner b of tooth k to base corner a of tooth k + 1.
    let mut path = String::new();
    for k in 0..n {
        let nk = (k + 1) % n;
        let _ = write!(
            g,
            "<a:gd name=\"w0{k}\" fmla=\"+- ve{nk} 0 vs{k}\"/><a:gd name=\"w1{k}\" fmla=\"+- w0{k} 10800000 0\"/>\
<a:gd name=\"w2{k}\" fmla=\"+- w0{k} 21600000 0\"/><a:gd name=\"sw{k}\" fmla=\"?: w1{k} w0{k} w2{k}\"/>"
        );
        if k == 0 {
            let _ = write!(path, "<a:moveTo><a:pt x=\"X0a\" y=\"Y0a\"/></a:moveTo>");
        }
        let _ = write!(
            path,
            "<a:lnTo><a:pt x=\"TX{k}a\" y=\"TY{k}a\"/></a:lnTo><a:lnTo><a:pt x=\"TX{k}b\" y=\"TY{k}b\"/></a:lnTo>\
<a:lnTo><a:pt x=\"X{k}b\" y=\"Y{k}b\"/></a:lnTo><a:arcTo wR=\"rx\" hR=\"ry\" stAng=\"vs{k}\" swAng=\"sw{k}\"/>"
        );
    }
    parse(&format!(
        "<a:custGeom><a:avLst><a:gd name=\"adj1\" fmla=\"val {a1}\"/><a:gd name=\"adj2\" fmla=\"val {a2}\"/></a:avLst>\
<a:gdLst>{g}</a:gdLst><a:pathLst><a:path>{path}<a:close/></a:path></a:pathLst></a:custGeom>"
    ))
}

/// `actionButtonBackPrevious`: no adjusts. A left pointing triangle `(-12,0) (12,-12) (12,12)` in
/// `ss / 32` units about the centre, shaded darken.
const BTN_BACK_PREVIOUS: &[UPath] = &[
    (
        "norm",
        false,
        "M l t L r t L r b L l b Z M -12 0 L 12 -12 L 12 12 Z",
    ),
    ("darken", false, "M -12 0 L 12 -12 L 12 12 Z"),
    ("none", true, "M -12 0 L 12 -12 L 12 12 Z"),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// `actionButtonEnd`: no adjusts. A right pointing triangle `(6,0) (-12,-12) (-12,12)` and a bar
/// `9..12` by `-12..12` (units of `ss / 32` about the centre), shaded darken.
const BTN_END: &[UPath] = &[
    (
        "norm",
        false,
        "M l t L r t L r b L l b Z M 6 0 L -12 -12 L -12 12 Z M 9 -12 L 12 -12 L 12 12 L 9 12 Z",
    ),
    (
        "darken",
        false,
        "M 6 0 L -12 -12 L -12 12 Z M 9 -12 L 12 -12 L 12 12 L 9 12 Z",
    ),
    (
        "none",
        true,
        "M 6 0 L -12 12 L -12 -12 Z M 9 -12 L 12 -12 L 12 12 L 9 12 Z",
    ),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// `actionButtonBeginning`: no adjusts. A left pointing triangle `(-6,0) (12,-12) (12,12)` and a bar
/// `-12..-9` by `-12..12` (units of `ss / 32` about the centre), shaded darken.
const BTN_BEGINNING: &[UPath] = &[
    (
        "norm",
        false,
        "M l t L r t L r b L l b Z M -6 0 L 12 -12 L 12 12 Z M -9 -12 L -12 -12 L -12 12 L -9 12 Z",
    ),
    (
        "darken",
        false,
        "M -6 0 L 12 -12 L 12 12 Z M -9 -12 L -12 -12 L -12 12 L -9 12 Z",
    ),
    (
        "none",
        true,
        "M -6 0 L 12 -12 L 12 12 Z M -9 -12 L -9 12 L -12 12 L -12 -12 Z",
    ),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// `actionButtonReturn`: no adjusts. A U turn arrow in `ss / 32` units about the centre: arrow head
/// `(0,-6) (6,-12) (12,-6)`, a shaft that comes down at `x = 3 .. 9` and returns along a rounded
/// bottom (cubic arcs of radius 3 to 9 units) to `x = -12 .. -6` and up to `y = -6`. Shaded darken.
const BTN_RETURN: &[UPath] = &[
    (
        "norm",
        false,
        "M l t L r t L r b L l b Z M 12 -6 L 6 -12 L 0 -6 L 3 -6 L 3 3 C 3 4.658 1.658 6 0 6 L -3 6 C -4.658 6 -6 4.658 -6 3 L -6 -6 L -12 -6 L -12 3 C -12 7.971 -7.971 12 -3 12 L 0 12 C 4.971 12 9 7.971 9 3 L 9 -6 Z",
    ),
    (
        "darken",
        false,
        "M 12 -6 L 6 -12 L 0 -6 L 3 -6 L 3 3 C 3 4.658 1.658 6 0 6 L -3 6 C -4.658 6 -6 4.658 -6 3 L -6 -6 L -12 -6 L -12 3 C -12 7.971 -7.971 12 -3 12 L 0 12 C 4.971 12 9 7.971 9 3 L 9 -6 Z",
    ),
    (
        "none",
        true,
        "M 12 -6 L 9 -6 L 9 3 C 9 7.971 4.971 12 0 12 L -3 12 C -7.971 12 -12 7.971 -12 3 L -12 -6 L -6 -6 L -6 3 C -6 4.658 -4.658 6 -3 6 L 0 6 C 1.658 6 3 4.658 3 3 L 3 -6 L 0 -6 L 6 -12 Z",
    ),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// `actionButtonDocument`: no adjusts. A page with a folded corner in `ss / 32` units about the
/// centre: `x = -9..9`, `y = -12..12`, fold `3..9` by `-12..-6`. The page is shaded darkenLess, the
/// fold darken.
const BTN_DOCUMENT: &[UPath] = &[
    (
        "norm",
        false,
        "M l t L r t L r b L l b Z M -9 -12 L 3 -12 L 9 -6 L 9 12 L -9 12 Z",
    ),
    (
        "darkenLess",
        false,
        "M -9 -12 L 3 -12 L 3 -6 L 9 -6 L 9 12 L -9 12 Z",
    ),
    ("darken", false, "M 3 -12 L 3 -6 L 9 -6 Z"),
    (
        "none",
        true,
        "M -9 -12 L 3 -12 L 9 -6 L 9 12 L -9 12 Z M 9 -6 L 3 -6 L 3 -12",
    ),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// `actionButtonSound`: no adjusts. A loudspeaker (box `-12..-4.5` by `-4.5..4.5`, cone out to
/// `x = 3` between `y = -12` and `12`) and three open sound-wave strokes from `x = 6` to `12`
/// at `y = -4.5 -> -9`, `0 -> 0` and `4.5 -> 9` (units of `ss / 32` about the centre). Speaker shaded darken.
const BTN_SOUND: &[UPath] = &[
    (
        "norm",
        false,
        "M l t L r t L r b L l b Z M -12 -4.5 L -12 4.5 L -4.5 4.5 L 3 12 L 3 -12 L -4.5 -4.5 Z",
    ),
    (
        "darken",
        false,
        "M -12 -4.5 L -12 4.5 L -4.5 4.5 L 3 12 L 3 -12 L -4.5 -4.5 Z",
    ),
    (
        "none",
        true,
        "M -12 -4.5 L -4.5 -4.5 L 3 -12 L 3 12 L -4.5 4.5 L -12 4.5 Z M 6 -4.5 L 12 -9 M 6 0 L 12 0 M 6 4.5 L 12 9",
    ),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// `actionButtonMovie`: no adjusts. A film camera outline in `ss / 32` units about the centre (the
/// measured 19 point outline between `x = -12 .. 12` and `y = -6.13 .. 5.32`), shaded darken.
const BTN_MOVIE: &[UPath] = &[
    (
        "norm",
        false,
        "M l t L r t L r b L l b Z M -12 -6.133 L -12 -1.384 L -10.382 -1.384 L -9.882 -1.927 L -9.416 -1.927 L -9.416 5.324 L 6.9 5.324 L 6.9 2.824 L 9.482 2.824 L 10.882 4.2 L 12 4.2 L 12 -4.633 L 10.882 -4.633 L 9.918 -3.676 L 6.9 -3.676 L 6.9 -4.633 L 5.949 -5.633 L -9.882 -5.633 L -10.382 -6.133 Z",
    ),
    (
        "darken",
        false,
        "M -12 -6.133 L -12 -1.384 L -10.382 -1.384 L -9.882 -1.927 L -9.416 -1.927 L -9.416 5.324 L 6.9 5.324 L 6.9 2.824 L 9.482 2.824 L 10.882 4.2 L 12 4.2 L 12 -4.633 L 10.882 -4.633 L 9.918 -3.676 L 6.9 -3.676 L 6.9 -4.633 L 5.949 -5.633 L -9.882 -5.633 L -10.382 -6.133 Z",
    ),
    (
        "none",
        true,
        "M -12 -6.133 L -10.382 -6.133 L -9.882 -5.633 L 5.949 -5.633 L 6.9 -4.633 L 6.9 -3.676 L 9.918 -3.676 L 10.882 -4.633 L 12 -4.633 L 12 4.2 L 10.882 4.2 L 9.482 2.824 L 6.9 2.824 L 6.9 5.324 L -9.416 5.324 L -9.416 -1.927 L -9.882 -1.927 L -10.382 -1.384 L -12 -1.384 Z",
    ),
    ("none", true, "M l b L r b L r t L l t Z"),
];

/// Entries of this batch `(name, definition)`.
pub(super) fn entries() -> Vec<(&'static str, ShapeDef)> {
    let all = [
        ("actionButtonBackPrevious", unit_shape(BTN_BACK_PREVIOUS)),
        ("actionButtonEnd", unit_shape(BTN_END)),
        ("actionButtonBeginning", unit_shape(BTN_BEGINNING)),
        ("actionButtonReturn", unit_shape(BTN_RETURN)),
        ("actionButtonDocument", unit_shape(BTN_DOCUMENT)),
        ("actionButtonSound", unit_shape(BTN_SOUND)),
        ("actionButtonMovie", unit_shape(BTN_MOVIE)),
        ("gear6", gear(6, 15000, 3526, 5359)),
        ("gear9", gear(9, 10000, 1763, 2679)),
        ("funnel", funnel()),
        ("mathPlus", math_plus()),
        ("mathMinus", math_minus()),
        ("mathMultiply", math_multiply()),
        ("mathDivide", math_divide()),
        ("mathEqual", math_equal()),
        ("mathNotEqual", math_not_equal()),
        ("cornerTabs", corner_tabs()),
        ("squareTabs", square_tabs()),
        ("plaqueTabs", plaque_tabs()),
        ("chartX", chart_x()),
        ("chartStar", chart_star()),
        ("chartPlus", chart_plus()),
    ];
    all.into_iter()
        .filter_map(|(n, d)| d.map(|d| (n, d)))
        .collect()
}
