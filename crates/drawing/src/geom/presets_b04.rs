//! Clean-room presets of batch b04 (see `docs/architecture/drawing.md`).
//!
//! Every entry was written from the ECMA-376 guide-language prose and from Word's own output for the probe
//! documents in `fixtures/reference/drawing/presets/b04.*` (`tools/office/make-preset-probes.ps1`); the doc comment of
//! each entry says what was measured. No other project's preset tables and no copy of Ecma's
//! `presetShapeDefinitions.xml` were opened. Guide names are our own.

use crate::geom::ShapeDef;
use crate::parse::cust_geom_from_str;

/// `cube`.
///
/// Measured (b04, probes filled grey; sizes 144x144, 288x96, 96x288, 216x144 pt and the adjust pages): one adjust,
/// default 0.25. The depth is `ss * adj / 100000` (36 pt at 144), clamped 0..100000 (100000 and 400000 give 144 pt):
/// front face from (0, depth) to (w - depth, h), top face above it, side face to its right. Stroked: the hexagon
/// outline, the front top edge with the top-right diagonal, and the front right edge. Fill shades read from the PDF
/// (grey 0.753 base): front face as is, top face 0.8 (`lightenLess`), side face 0.604 (`darkenLess`).
fn cube() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a" fmla="pin 0 adj 100000"/>
    <a:gd name="d" fmla="*/ ss a 100000"/>
    <a:gd name="xf" fmla="+- r 0 d"/>
    <a:gd name="yf" fmla="+- b 0 d"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="l" y="d"/></a:moveTo>
    <a:lnTo><a:pt x="xf" y="d"/></a:lnTo>
    <a:lnTo><a:pt x="xf" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  <a:path fill="darkenLess" stroke="0">
    <a:moveTo><a:pt x="xf" y="d"/></a:moveTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yf"/></a:lnTo>
    <a:lnTo><a:pt x="xf" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  <a:path fill="lightenLess" stroke="0">
    <a:moveTo><a:pt x="l" y="d"/></a:moveTo>
    <a:lnTo><a:pt x="d" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xf" y="d"/></a:lnTo>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="l" y="d"/></a:moveTo>
    <a:lnTo><a:pt x="d" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yf"/></a:lnTo>
    <a:lnTo><a:pt x="xf" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="l" y="d"/></a:moveTo>
    <a:lnTo><a:pt x="xf" y="d"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:moveTo><a:pt x="xf" y="d"/></a:moveTo>
    <a:lnTo><a:pt x="xf" y="b"/></a:lnTo>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `can`.
///
/// Measured: one adjust, default 0.25. The top rim is an ellipse of height `ss * adj / 100000` (36 pt of 144, so the
/// half height `ry` is 18 pt) spanning the full width; the body runs from the rim's side points down to a bottom ellipse
/// half of the same size. Clamp: `adj` 0..`50000 * h / ss` (100000 and 400000 give a 72 pt rim ellipse, a quarter of the
/// height each side). Stroked as one outline: the rim ellipse (front half first), the right side, the front half of the
/// bottom ellipse, the left side. Fills: body as is, rim ellipse 0.851 (`lighten`, grey 0.753 base).
fn can() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="maxA" fmla="*/ 50000 h ss"/>
    <a:gd name="a" fmla="pin 0 adj maxA"/>
    <a:gd name="ry0" fmla="*/ ss a 200000"/>
    <a:gd name="ry" fmla="max ry0 0"/>
    <a:gd name="yb" fmla="+- b 0 ry"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="l" y="ry"/></a:moveTo>
    <a:arcTo wR="wd2" hR="ry" stAng="cd2" swAng="-10800000"/>
    <a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
    <a:arcTo wR="wd2" hR="ry" stAng="0" swAng="cd2"/>
    <a:close/>
  </a:path>
  <a:path fill="lighten" stroke="0">
    <a:moveTo><a:pt x="l" y="ry"/></a:moveTo>
    <a:arcTo wR="wd2" hR="ry" stAng="cd2" swAng="cd2"/>
    <a:arcTo wR="wd2" hR="ry" stAng="0" swAng="cd2"/>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="r" y="ry"/></a:moveTo>
    <a:arcTo wR="wd2" hR="ry" stAng="0" swAng="cd2"/>
    <a:arcTo wR="wd2" hR="ry" stAng="cd2" swAng="cd2"/>
    <a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
    <a:arcTo wR="wd2" hR="ry" stAng="0" swAng="cd2"/>
    <a:lnTo><a:pt x="l" y="ry"/></a:lnTo>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `lightningBolt`.
///
/// Measured: no adjusts. An eleven point polygon, every vertex a fixed fraction of the box (a 21600 x 21600 grid fits
/// the vertices of the 288x96 and 96x288 pages to 0.01 pt; the 144x144 and 216x144 pages are checked against it).
fn lightning_bolt() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst/>
  <a:gdLst/>
  <a:pathLst>
  <a:path w="21600" h="21600">
    <a:moveTo><a:pt x="8472" y="0"/></a:moveTo>
    <a:lnTo><a:pt x="12860" y="6080"/></a:lnTo>
    <a:lnTo><a:pt x="11050" y="6797"/></a:lnTo>
    <a:lnTo><a:pt x="16577" y="12007"/></a:lnTo>
    <a:lnTo><a:pt x="14767" y="12877"/></a:lnTo>
    <a:lnTo><a:pt x="21600" y="21600"/></a:lnTo>
    <a:lnTo><a:pt x="10012" y="14915"/></a:lnTo>
    <a:lnTo><a:pt x="12222" y="13987"/></a:lnTo>
    <a:lnTo><a:pt x="5022" y="9705"/></a:lnTo>
    <a:lnTo><a:pt x="7602" y="8382"/></a:lnTo>
    <a:lnTo><a:pt x="0" y="3890"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `heart`.
///
/// Measured: no adjusts. Two cubic curves from the top notch (w/2, h/4) round the left and right lobes to the bottom
/// point (w/2, h); the control points lie outside the box (-0.52 w .. 1.52 w and -h/3 above the top), as the 21600 grid
/// values in the path show. The curves were read from the 288x96 and 96x288 pages and checked on the others.
fn heart() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst/>
  <a:gdLst/>
  <a:pathLst>
  <a:path w="21600" h="21600">
    <a:moveTo><a:pt x="10800" y="5400"/></a:moveTo>
    <a:cubicBezTo><a:pt x="15300" y="-7200"/><a:pt x="32850" y="5400"/><a:pt x="10800" y="21600"/></a:cubicBezTo>
    <a:cubicBezTo><a:pt x="-11250" y="5400"/><a:pt x="6300" y="-7200"/><a:pt x="10800" y="5400"/></a:cubicBezTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `irregularSeal1`.
///
/// Measured: no adjusts. A twenty-four point explosion polygon, every vertex a fixed fraction of the box (21600 grid
/// read from the 288x96 and 96x288 pages and checked on the 144x144 and 216x144 pages).
fn irregular_seal1() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst/>
  <a:gdLst/>
  <a:pathLst>
  <a:path w="21600" h="21600">
    <a:moveTo><a:pt x="10800" y="5800"/></a:moveTo>
    <a:lnTo><a:pt x="14522" y="0"/></a:lnTo>
    <a:lnTo><a:pt x="14155" y="5325"/></a:lnTo>
    <a:lnTo><a:pt x="18380" y="4457"/></a:lnTo>
    <a:lnTo><a:pt x="16702" y="7315"/></a:lnTo>
    <a:lnTo><a:pt x="21097" y="8137"/></a:lnTo>
    <a:lnTo><a:pt x="17607" y="10475"/></a:lnTo>
    <a:lnTo><a:pt x="21600" y="13290"/></a:lnTo>
    <a:lnTo><a:pt x="16837" y="12942"/></a:lnTo>
    <a:lnTo><a:pt x="18145" y="18095"/></a:lnTo>
    <a:lnTo><a:pt x="14020" y="14457"/></a:lnTo>
    <a:lnTo><a:pt x="13247" y="19737"/></a:lnTo>
    <a:lnTo><a:pt x="10532" y="14935"/></a:lnTo>
    <a:lnTo><a:pt x="8485" y="21600"/></a:lnTo>
    <a:lnTo><a:pt x="7715" y="15627"/></a:lnTo>
    <a:lnTo><a:pt x="4762" y="17617"/></a:lnTo>
    <a:lnTo><a:pt x="5667" y="13937"/></a:lnTo>
    <a:lnTo><a:pt x="135" y="14587"/></a:lnTo>
    <a:lnTo><a:pt x="3722" y="11775"/></a:lnTo>
    <a:lnTo><a:pt x="0" y="8615"/></a:lnTo>
    <a:lnTo><a:pt x="4627" y="7617"/></a:lnTo>
    <a:lnTo><a:pt x="370" y="2295"/></a:lnTo>
    <a:lnTo><a:pt x="7312" y="6320"/></a:lnTo>
    <a:lnTo><a:pt x="8352" y="2295"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `irregularSeal2`.
///
/// Measured: no adjusts. A twenty-eight point explosion polygon, every vertex a fixed fraction of the box (21600 grid
/// read from the 288x96 and 96x288 pages and checked on the 144x144 and 216x144 pages).
fn irregular_seal2() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst/>
  <a:gdLst/>
  <a:pathLst>
  <a:path w="21600" h="21600">
    <a:moveTo><a:pt x="11462" y="4342"/></a:moveTo>
    <a:lnTo><a:pt x="14790" y="0"/></a:lnTo>
    <a:lnTo><a:pt x="14525" y="5777"/></a:lnTo>
    <a:lnTo><a:pt x="18007" y="3172"/></a:lnTo>
    <a:lnTo><a:pt x="16380" y="6532"/></a:lnTo>
    <a:lnTo><a:pt x="21600" y="6645"/></a:lnTo>
    <a:lnTo><a:pt x="16985" y="9402"/></a:lnTo>
    <a:lnTo><a:pt x="18270" y="11290"/></a:lnTo>
    <a:lnTo><a:pt x="16380" y="12310"/></a:lnTo>
    <a:lnTo><a:pt x="18877" y="15632"/></a:lnTo>
    <a:lnTo><a:pt x="14640" y="14350"/></a:lnTo>
    <a:lnTo><a:pt x="14942" y="17370"/></a:lnTo>
    <a:lnTo><a:pt x="12180" y="15935"/></a:lnTo>
    <a:lnTo><a:pt x="11612" y="18842"/></a:lnTo>
    <a:lnTo><a:pt x="9872" y="17370"/></a:lnTo>
    <a:lnTo><a:pt x="8700" y="19712"/></a:lnTo>
    <a:lnTo><a:pt x="7527" y="18125"/></a:lnTo>
    <a:lnTo><a:pt x="4917" y="21600"/></a:lnTo>
    <a:lnTo><a:pt x="4805" y="18240"/></a:lnTo>
    <a:lnTo><a:pt x="1285" y="17825"/></a:lnTo>
    <a:lnTo><a:pt x="3330" y="15370"/></a:lnTo>
    <a:lnTo><a:pt x="0" y="12877"/></a:lnTo>
    <a:lnTo><a:pt x="3935" y="11592"/></a:lnTo>
    <a:lnTo><a:pt x="1172" y="8270"/></a:lnTo>
    <a:lnTo><a:pt x="5372" y="7817"/></a:lnTo>
    <a:lnTo><a:pt x="4502" y="3625"/></a:lnTo>
    <a:lnTo><a:pt x="8550" y="6382"/></a:lnTo>
    <a:lnTo><a:pt x="9722" y="1887"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `sun`.
///
/// Measured: one adjust, default 0.25, clamped 12500..46875 (0 gives 0.125, 100000 and 400000 give 0.46875). A centre
/// ellipse and eight triangular rays. The ellipse has radii `k * w/2` and `k * h/2` with `k = 1 - 2 * adj` (81 x 54 pt at
/// adj 0.125, 54 x 36 at 0.25, 6.75 x 4.5 at 0.46875 on 216x144). Each ray is the triangle of a tip on the box ellipse
/// (at 0, 45, ..., 315 degrees) and two base corners; in the box ellipse's unit coordinates, along the ray's axis, the
/// base corners sit at `0.25 + 0.69292 * k` and `+-0.28702 * k` across (fits all four adjusts to 0.01 pt; the ratio of
/// the two slopes is tan 22.5 degrees), then the whole thing is scaled by `w/2` and `h/2`. All nine outlines are
/// stroked and filled with the base shade.
fn sun() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a" fmla="pin 12500 adj 46875"/>
    <a:gd name="a2" fmla="*/ a 2 1"/>
    <a:gd name="kk" fmla="+- 100000 0 a2"/>
    <a:gd name="rx" fmla="*/ wd2 kk 100000"/>
    <a:gd name="ry" fmla="*/ hd2 kk 100000"/>
    <a:gd name="bt" fmla="*/ kk 69292 100000"/>
    <a:gd name="bn" fmla="+- 25000 bt 0"/>
    <a:gd name="hn" fmla="*/ kk 28702 100000"/>
    <a:gd name="r0tx" fmla="*/ wd2 100000 100000"/>
    <a:gd name="r0ty" fmla="*/ hd2 0 100000"/>
    <a:gd name="r0A" fmla="*/ bn 100000 100000"/>
    <a:gd name="r0B" fmla="*/ hn 0 100000"/>
    <a:gd name="r0C" fmla="*/ bn 0 100000"/>
    <a:gd name="r0D" fmla="*/ hn 100000 100000"/>
    <a:gd name="r0x1n" fmla="+- r0A 0 r0B"/>
    <a:gd name="r0y1n" fmla="+- r0C r0D 0"/>
    <a:gd name="r0x2n" fmla="+- r0A r0B 0"/>
    <a:gd name="r0y2n" fmla="+- r0C 0 r0D"/>
    <a:gd name="r0x1" fmla="*/ wd2 r0x1n 100000"/>
    <a:gd name="r0y1" fmla="*/ hd2 r0y1n 100000"/>
    <a:gd name="r0x2" fmla="*/ wd2 r0x2n 100000"/>
    <a:gd name="r0y2" fmla="*/ hd2 r0y2n 100000"/>
    <a:gd name="r0X0" fmla="+- hc r0tx 0"/>
    <a:gd name="r0Y0" fmla="+- vc r0ty 0"/>
    <a:gd name="r0X1" fmla="+- hc r0x1 0"/>
    <a:gd name="r0Y1" fmla="+- vc r0y1 0"/>
    <a:gd name="r0X2" fmla="+- hc r0x2 0"/>
    <a:gd name="r0Y2" fmla="+- vc r0y2 0"/>
    <a:gd name="r1tx" fmla="*/ wd2 70711 100000"/>
    <a:gd name="r1ty" fmla="*/ hd2 70711 100000"/>
    <a:gd name="r1A" fmla="*/ bn 70711 100000"/>
    <a:gd name="r1B" fmla="*/ hn 70711 100000"/>
    <a:gd name="r1C" fmla="*/ bn 70711 100000"/>
    <a:gd name="r1D" fmla="*/ hn 70711 100000"/>
    <a:gd name="r1x1n" fmla="+- r1A 0 r1B"/>
    <a:gd name="r1y1n" fmla="+- r1C r1D 0"/>
    <a:gd name="r1x2n" fmla="+- r1A r1B 0"/>
    <a:gd name="r1y2n" fmla="+- r1C 0 r1D"/>
    <a:gd name="r1x1" fmla="*/ wd2 r1x1n 100000"/>
    <a:gd name="r1y1" fmla="*/ hd2 r1y1n 100000"/>
    <a:gd name="r1x2" fmla="*/ wd2 r1x2n 100000"/>
    <a:gd name="r1y2" fmla="*/ hd2 r1y2n 100000"/>
    <a:gd name="r1X0" fmla="+- hc r1tx 0"/>
    <a:gd name="r1Y0" fmla="+- vc r1ty 0"/>
    <a:gd name="r1X1" fmla="+- hc r1x1 0"/>
    <a:gd name="r1Y1" fmla="+- vc r1y1 0"/>
    <a:gd name="r1X2" fmla="+- hc r1x2 0"/>
    <a:gd name="r1Y2" fmla="+- vc r1y2 0"/>
    <a:gd name="r2tx" fmla="*/ wd2 0 100000"/>
    <a:gd name="r2ty" fmla="*/ hd2 100000 100000"/>
    <a:gd name="r2A" fmla="*/ bn 0 100000"/>
    <a:gd name="r2B" fmla="*/ hn 100000 100000"/>
    <a:gd name="r2C" fmla="*/ bn 100000 100000"/>
    <a:gd name="r2D" fmla="*/ hn 0 100000"/>
    <a:gd name="r2x1n" fmla="+- r2A 0 r2B"/>
    <a:gd name="r2y1n" fmla="+- r2C r2D 0"/>
    <a:gd name="r2x2n" fmla="+- r2A r2B 0"/>
    <a:gd name="r2y2n" fmla="+- r2C 0 r2D"/>
    <a:gd name="r2x1" fmla="*/ wd2 r2x1n 100000"/>
    <a:gd name="r2y1" fmla="*/ hd2 r2y1n 100000"/>
    <a:gd name="r2x2" fmla="*/ wd2 r2x2n 100000"/>
    <a:gd name="r2y2" fmla="*/ hd2 r2y2n 100000"/>
    <a:gd name="r2X0" fmla="+- hc r2tx 0"/>
    <a:gd name="r2Y0" fmla="+- vc r2ty 0"/>
    <a:gd name="r2X1" fmla="+- hc r2x1 0"/>
    <a:gd name="r2Y1" fmla="+- vc r2y1 0"/>
    <a:gd name="r2X2" fmla="+- hc r2x2 0"/>
    <a:gd name="r2Y2" fmla="+- vc r2y2 0"/>
    <a:gd name="r3tx" fmla="*/ wd2 -70711 100000"/>
    <a:gd name="r3ty" fmla="*/ hd2 70711 100000"/>
    <a:gd name="r3A" fmla="*/ bn -70711 100000"/>
    <a:gd name="r3B" fmla="*/ hn 70711 100000"/>
    <a:gd name="r3C" fmla="*/ bn 70711 100000"/>
    <a:gd name="r3D" fmla="*/ hn -70711 100000"/>
    <a:gd name="r3x1n" fmla="+- r3A 0 r3B"/>
    <a:gd name="r3y1n" fmla="+- r3C r3D 0"/>
    <a:gd name="r3x2n" fmla="+- r3A r3B 0"/>
    <a:gd name="r3y2n" fmla="+- r3C 0 r3D"/>
    <a:gd name="r3x1" fmla="*/ wd2 r3x1n 100000"/>
    <a:gd name="r3y1" fmla="*/ hd2 r3y1n 100000"/>
    <a:gd name="r3x2" fmla="*/ wd2 r3x2n 100000"/>
    <a:gd name="r3y2" fmla="*/ hd2 r3y2n 100000"/>
    <a:gd name="r3X0" fmla="+- hc r3tx 0"/>
    <a:gd name="r3Y0" fmla="+- vc r3ty 0"/>
    <a:gd name="r3X1" fmla="+- hc r3x1 0"/>
    <a:gd name="r3Y1" fmla="+- vc r3y1 0"/>
    <a:gd name="r3X2" fmla="+- hc r3x2 0"/>
    <a:gd name="r3Y2" fmla="+- vc r3y2 0"/>
    <a:gd name="r4tx" fmla="*/ wd2 -100000 100000"/>
    <a:gd name="r4ty" fmla="*/ hd2 0 100000"/>
    <a:gd name="r4A" fmla="*/ bn -100000 100000"/>
    <a:gd name="r4B" fmla="*/ hn 0 100000"/>
    <a:gd name="r4C" fmla="*/ bn 0 100000"/>
    <a:gd name="r4D" fmla="*/ hn -100000 100000"/>
    <a:gd name="r4x1n" fmla="+- r4A 0 r4B"/>
    <a:gd name="r4y1n" fmla="+- r4C r4D 0"/>
    <a:gd name="r4x2n" fmla="+- r4A r4B 0"/>
    <a:gd name="r4y2n" fmla="+- r4C 0 r4D"/>
    <a:gd name="r4x1" fmla="*/ wd2 r4x1n 100000"/>
    <a:gd name="r4y1" fmla="*/ hd2 r4y1n 100000"/>
    <a:gd name="r4x2" fmla="*/ wd2 r4x2n 100000"/>
    <a:gd name="r4y2" fmla="*/ hd2 r4y2n 100000"/>
    <a:gd name="r4X0" fmla="+- hc r4tx 0"/>
    <a:gd name="r4Y0" fmla="+- vc r4ty 0"/>
    <a:gd name="r4X1" fmla="+- hc r4x1 0"/>
    <a:gd name="r4Y1" fmla="+- vc r4y1 0"/>
    <a:gd name="r4X2" fmla="+- hc r4x2 0"/>
    <a:gd name="r4Y2" fmla="+- vc r4y2 0"/>
    <a:gd name="r5tx" fmla="*/ wd2 -70711 100000"/>
    <a:gd name="r5ty" fmla="*/ hd2 -70711 100000"/>
    <a:gd name="r5A" fmla="*/ bn -70711 100000"/>
    <a:gd name="r5B" fmla="*/ hn -70711 100000"/>
    <a:gd name="r5C" fmla="*/ bn -70711 100000"/>
    <a:gd name="r5D" fmla="*/ hn -70711 100000"/>
    <a:gd name="r5x1n" fmla="+- r5A 0 r5B"/>
    <a:gd name="r5y1n" fmla="+- r5C r5D 0"/>
    <a:gd name="r5x2n" fmla="+- r5A r5B 0"/>
    <a:gd name="r5y2n" fmla="+- r5C 0 r5D"/>
    <a:gd name="r5x1" fmla="*/ wd2 r5x1n 100000"/>
    <a:gd name="r5y1" fmla="*/ hd2 r5y1n 100000"/>
    <a:gd name="r5x2" fmla="*/ wd2 r5x2n 100000"/>
    <a:gd name="r5y2" fmla="*/ hd2 r5y2n 100000"/>
    <a:gd name="r5X0" fmla="+- hc r5tx 0"/>
    <a:gd name="r5Y0" fmla="+- vc r5ty 0"/>
    <a:gd name="r5X1" fmla="+- hc r5x1 0"/>
    <a:gd name="r5Y1" fmla="+- vc r5y1 0"/>
    <a:gd name="r5X2" fmla="+- hc r5x2 0"/>
    <a:gd name="r5Y2" fmla="+- vc r5y2 0"/>
    <a:gd name="r6tx" fmla="*/ wd2 0 100000"/>
    <a:gd name="r6ty" fmla="*/ hd2 -100000 100000"/>
    <a:gd name="r6A" fmla="*/ bn 0 100000"/>
    <a:gd name="r6B" fmla="*/ hn -100000 100000"/>
    <a:gd name="r6C" fmla="*/ bn -100000 100000"/>
    <a:gd name="r6D" fmla="*/ hn 0 100000"/>
    <a:gd name="r6x1n" fmla="+- r6A 0 r6B"/>
    <a:gd name="r6y1n" fmla="+- r6C r6D 0"/>
    <a:gd name="r6x2n" fmla="+- r6A r6B 0"/>
    <a:gd name="r6y2n" fmla="+- r6C 0 r6D"/>
    <a:gd name="r6x1" fmla="*/ wd2 r6x1n 100000"/>
    <a:gd name="r6y1" fmla="*/ hd2 r6y1n 100000"/>
    <a:gd name="r6x2" fmla="*/ wd2 r6x2n 100000"/>
    <a:gd name="r6y2" fmla="*/ hd2 r6y2n 100000"/>
    <a:gd name="r6X0" fmla="+- hc r6tx 0"/>
    <a:gd name="r6Y0" fmla="+- vc r6ty 0"/>
    <a:gd name="r6X1" fmla="+- hc r6x1 0"/>
    <a:gd name="r6Y1" fmla="+- vc r6y1 0"/>
    <a:gd name="r6X2" fmla="+- hc r6x2 0"/>
    <a:gd name="r6Y2" fmla="+- vc r6y2 0"/>
    <a:gd name="r7tx" fmla="*/ wd2 70711 100000"/>
    <a:gd name="r7ty" fmla="*/ hd2 -70711 100000"/>
    <a:gd name="r7A" fmla="*/ bn 70711 100000"/>
    <a:gd name="r7B" fmla="*/ hn -70711 100000"/>
    <a:gd name="r7C" fmla="*/ bn -70711 100000"/>
    <a:gd name="r7D" fmla="*/ hn 70711 100000"/>
    <a:gd name="r7x1n" fmla="+- r7A 0 r7B"/>
    <a:gd name="r7y1n" fmla="+- r7C r7D 0"/>
    <a:gd name="r7x2n" fmla="+- r7A r7B 0"/>
    <a:gd name="r7y2n" fmla="+- r7C 0 r7D"/>
    <a:gd name="r7x1" fmla="*/ wd2 r7x1n 100000"/>
    <a:gd name="r7y1" fmla="*/ hd2 r7y1n 100000"/>
    <a:gd name="r7x2" fmla="*/ wd2 r7x2n 100000"/>
    <a:gd name="r7y2" fmla="*/ hd2 r7y2n 100000"/>
    <a:gd name="r7X0" fmla="+- hc r7tx 0"/>
    <a:gd name="r7Y0" fmla="+- vc r7ty 0"/>
    <a:gd name="r7X1" fmla="+- hc r7x1 0"/>
    <a:gd name="r7Y1" fmla="+- vc r7y1 0"/>
    <a:gd name="r7X2" fmla="+- hc r7x2 0"/>
    <a:gd name="r7Y2" fmla="+- vc r7y2 0"/>
    <a:gd name="xl" fmla="+- hc 0 rx"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="r0X0" y="r0Y0"/></a:moveTo>
    <a:lnTo><a:pt x="r0X1" y="r0Y1"/></a:lnTo>
    <a:lnTo><a:pt x="r0X2" y="r0Y2"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="r1X0" y="r1Y0"/></a:moveTo>
    <a:lnTo><a:pt x="r1X1" y="r1Y1"/></a:lnTo>
    <a:lnTo><a:pt x="r1X2" y="r1Y2"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="r2X0" y="r2Y0"/></a:moveTo>
    <a:lnTo><a:pt x="r2X1" y="r2Y1"/></a:lnTo>
    <a:lnTo><a:pt x="r2X2" y="r2Y2"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="r3X0" y="r3Y0"/></a:moveTo>
    <a:lnTo><a:pt x="r3X1" y="r3Y1"/></a:lnTo>
    <a:lnTo><a:pt x="r3X2" y="r3Y2"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="r4X0" y="r4Y0"/></a:moveTo>
    <a:lnTo><a:pt x="r4X1" y="r4Y1"/></a:lnTo>
    <a:lnTo><a:pt x="r4X2" y="r4Y2"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="r5X0" y="r5Y0"/></a:moveTo>
    <a:lnTo><a:pt x="r5X1" y="r5Y1"/></a:lnTo>
    <a:lnTo><a:pt x="r5X2" y="r5Y2"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="r6X0" y="r6Y0"/></a:moveTo>
    <a:lnTo><a:pt x="r6X1" y="r6Y1"/></a:lnTo>
    <a:lnTo><a:pt x="r6X2" y="r6Y2"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="r7X0" y="r7Y0"/></a:moveTo>
    <a:lnTo><a:pt x="r7X1" y="r7Y1"/></a:lnTo>
    <a:lnTo><a:pt x="r7X2" y="r7Y2"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="xl" y="vc"/></a:moveTo>
    <a:arcTo wR="rx" hR="ry" stAng="cd2" swAng="cd4"/>
    <a:arcTo wR="rx" hR="ry" stAng="3cd4" swAng="cd4"/>
    <a:arcTo wR="rx" hR="ry" stAng="0" swAng="cd4"/>
    <a:arcTo wR="rx" hR="ry" stAng="cd4" swAng="cd4"/>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `moon`.
///
/// Measured: one adjust, default 0.5. A crescent: the outer edge is the left half of the box ellipse centred on the
/// right edge (radii `w` and `h/2`, from (r, b) through (0, vc) to (r, t)); the inner edge is an arc of a larger ellipse
/// through the top right and bottom right corners whose leftmost point is `adj * w` from the left edge. Clamp: `adj`
/// 0..87500 (100000 and 400000 give 87500; 0 and -50000 give a zero-width crescent, the inner arc retracing the outer
/// one). The inner ellipse is the box ellipse scaled by `s = (1 + q^2) / (2q)` with `q = 1 - adj` (radii `s * w`,
/// `s * h/2`, centre `adj * w + s * w` from the left; 1.25 at the default, 4.0625 at 87500), read off the third and
/// fourth control points of the 216x144 pages and confirmed by (162, 126) lying on it.
fn moon() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a" fmla="pin 0 adj 87500"/>
    <a:gd name="q" fmla="+- 100000 0 a"/>
    <a:gd name="qsq" fmla="*/ q q 1"/>
    <a:gd name="num" fmla="+- qsq 10000000000 0"/>
    <a:gd name="dq" fmla="*/ q 2 1"/>
    <a:gd name="s" fmla="*/ num 1 dq"/>
    <a:gd name="sw" fmla="*/ w s 100000"/>
    <a:gd name="sh" fmla="*/ hd2 s 100000"/>
    <a:gd name="xL" fmla="*/ w a 100000"/>
    <a:gd name="cx" fmla="+- xL sw 0"/>
    <a:gd name="dx" fmla="+- r 0 cx"/>
    <a:gd name="negHd2" fmla="+- 0 0 hd2"/>
    <a:gd name="a0" fmla="at2 dx negHd2"/>
    <a:gd name="a1" fmla="at2 dx hd2"/>
    <a:gd name="swI" fmla="+- a1 0 a0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="r" y="b"/></a:moveTo>
    <a:arcTo wR="w" hR="hd2" stAng="cd4" swAng="cd4"/>
    <a:arcTo wR="w" hR="hd2" stAng="cd2" swAng="cd4"/>
    <a:arcTo wR="sw" hR="sh" stAng="a0" swAng="swI"/>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `smileyFace`.
///
/// Measured: one adjust, default 0.04653, clamped -4653..4653 (100000 and 400000 give 4653, -50000 gives -4653, 0 a
/// straight mouth). A face ellipse, two eyes (ellipses of radii `w * 1125 / 21600` by `h * 1125 / 21600`, centred at
/// (7340, 7570) and (14260, 7570) on a 21600 grid) and a mouth: one cubic from (4946, ym) to (16640, ym) with both
/// control points at x = 8849 and 12747; `ym = h * 16515 / 21600 - h * adj / 100000` and the control points' y is
/// `h * 16515 / 21600 + 5 / 3 * h * adj / 100000` (a smile for positive adjust, a frown for negative). The grid
/// numbers were read from the 288x96 and 96x288 pages. Fills: face as is, eyes 0.604 (`darkenLess`).
fn smiley_face() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 4653"/></a:avLst>
  <a:gdLst>
    <a:gd name="a" fmla="pin -4653 adj 4653"/>
    <a:gd name="exL" fmla="*/ w 7340 21600"/>
    <a:gd name="exR" fmla="*/ w 14260 21600"/>
    <a:gd name="ey" fmla="*/ h 7570 21600"/>
    <a:gd name="erx" fmla="*/ w 1125 21600"/>
    <a:gd name="ery" fmla="*/ h 1125 21600"/>
    <a:gd name="exL0" fmla="+- exL 0 erx"/>
    <a:gd name="exR0" fmla="+- exR 0 erx"/>
    <a:gd name="mx0" fmla="*/ w 4946 21600"/>
    <a:gd name="mx1" fmla="*/ w 16640 21600"/>
    <a:gd name="mc1" fmla="*/ w 8849 21600"/>
    <a:gd name="mc2" fmla="*/ w 12747 21600"/>
    <a:gd name="mA" fmla="*/ h a 100000"/>
    <a:gd name="mBase" fmla="*/ h 16515 21600"/>
    <a:gd name="my0" fmla="+- mBase 0 mA"/>
    <a:gd name="mA53" fmla="*/ mA 5 3"/>
    <a:gd name="myc" fmla="+- mBase mA53 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd2" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="3cd4" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="0" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd4" swAng="cd4"/>
    <a:close/>
  </a:path>
  <a:path fill="darkenLess" stroke="0">
    <a:moveTo><a:pt x="exL0" y="ey"/></a:moveTo>
    <a:arcTo wR="erx" hR="ery" stAng="cd2" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="3cd4" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="0" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="cd4" swAng="cd4"/>
    <a:close/>
    <a:moveTo><a:pt x="exR0" y="ey"/></a:moveTo>
    <a:arcTo wR="erx" hR="ery" stAng="cd2" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="3cd4" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="0" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="cd4" swAng="cd4"/>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="exL0" y="ey"/></a:moveTo>
    <a:arcTo wR="erx" hR="ery" stAng="cd2" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="3cd4" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="0" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="cd4" swAng="cd4"/>
    <a:close/>
    <a:moveTo><a:pt x="exR0" y="ey"/></a:moveTo>
    <a:arcTo wR="erx" hR="ery" stAng="cd2" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="3cd4" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="0" swAng="cd4"/>
    <a:arcTo wR="erx" hR="ery" stAng="cd4" swAng="cd4"/>
    <a:close/>
    <a:moveTo><a:pt x="mx0" y="my0"/></a:moveTo>
    <a:cubicBezTo><a:pt x="mc1" y="myc"/><a:pt x="mc2" y="myc"/><a:pt x="mx1" y="my0"/></a:cubicBezTo>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd2" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="3cd4" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="0" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd4" swAng="cd4"/>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `foldedCorner`.
///
/// Measured: one adjust, default 0.16667, clamped 0..50000 (100000 and 400000 give 50000; 0 and -50000 give 0, no
/// visible fold). The fold size is `ss * adj / 100000` (24 pt of 144; 72 pt at the limit). The body is the box with the
/// bottom right corner cut by the diagonal from (r - fold, b) to (r, b - fold); the folded flap is the triangle of
/// those two points and (r - 0.8 fold, b - 0.8 fold) (4.8 pt in from the left and 19.2 pt up of the cut's start, as
/// measured at fold 24 and 43.2). Stroked: the flap's three edges, then the bottom, left, top and right edges as one
/// open outline. Fills: body as is, flap 0.604 (`darkenLess`).
fn folded_corner() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 16667"/></a:avLst>
  <a:gdLst>
    <a:gd name="a" fmla="pin 0 adj 50000"/>
    <a:gd name="f" fmla="*/ ss a 100000"/>
    <a:gd name="xf" fmla="+- r 0 f"/>
    <a:gd name="yf" fmla="+- b 0 f"/>
    <a:gd name="f8" fmla="*/ f 8 10"/>
    <a:gd name="xm" fmla="+- r 0 f8"/>
    <a:gd name="ym" fmla="+- b 0 f8"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yf"/></a:lnTo>
    <a:lnTo><a:pt x="xf" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  <a:path fill="darkenLess" stroke="0">
    <a:moveTo><a:pt x="xf" y="b"/></a:moveTo>
    <a:lnTo><a:pt x="xm" y="ym"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yf"/></a:lnTo>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="xf" y="b"/></a:moveTo>
    <a:lnTo><a:pt x="xm" y="ym"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yf"/></a:lnTo>
    <a:lnTo><a:pt x="xf" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yf"/></a:lnTo>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `bevel`.
///
/// Measured: one adjust, default 0.125, clamped 0..50000 (100000 and 400000 give 50000: the inner rectangle collapses to
/// a line through the middle). The bevel width is `ss * adj / 100000` (18 pt of 144). Stroked: the outer rectangle, the
/// inner rectangle inset by the width on every side, and the four corner diagonals. Fills (grey 0.753 base): centre as
/// is, top 0.8 (`lightenLess`), left 0.851 (`lighten`), bottom 0.604 (`darkenLess`), right 0.451 (`darken`).
fn bevel() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 12500"/></a:avLst>
  <a:gdLst>
    <a:gd name="a" fmla="pin 0 adj 50000"/>
    <a:gd name="d" fmla="*/ ss a 100000"/>
    <a:gd name="xr" fmla="+- r 0 d"/>
    <a:gd name="yb" fmla="+- b 0 d"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="d" y="d"/></a:moveTo>
    <a:lnTo><a:pt x="xr" y="d"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="d" y="yb"/></a:lnTo>
    <a:close/>
  </a:path>
  <a:path fill="lightenLess" stroke="0">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="d"/></a:lnTo>
    <a:lnTo><a:pt x="d" y="d"/></a:lnTo>
    <a:close/>
  </a:path>
  <a:path fill="darkenLess" stroke="0">
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo>
    <a:lnTo><a:pt x="d" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  <a:path fill="lighten" stroke="0">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="d" y="d"/></a:lnTo>
    <a:lnTo><a:pt x="d" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  <a:path fill="darken" stroke="0">
    <a:moveTo><a:pt x="r" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="d"/></a:lnTo>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="d" y="d"/></a:moveTo>
    <a:lnTo><a:pt x="xr" y="d"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="d" y="yb"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="d" y="d"/></a:lnTo>
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo>
    <a:lnTo><a:pt x="d" y="yb"/></a:lnTo>
    <a:moveTo><a:pt x="r" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="xr" y="d"/></a:lnTo>
    <a:moveTo><a:pt x="r" y="b"/></a:moveTo>
    <a:lnTo><a:pt x="xr" y="yb"/></a:lnTo>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `frame`.
///
/// Measured: one adjust, named `adj1`, default 0.125, clamped 0..50000 (100000 and 400000 give 50000, the hole
/// collapsing to a line through the middle; 0 and -50000 give 0, the hole equal to the box). The border width is
/// `ss * adj1 / 100000` (18 pt of 144). One outline of two rectangles: the box, then the hole inset by the width,
/// drawn the other way round.
fn frame() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 12500"/></a:avLst>
  <a:gdLst>
    <a:gd name="a" fmla="pin 0 adj1 50000"/>
    <a:gd name="d" fmla="*/ ss a 100000"/>
    <a:gd name="xr" fmla="+- r 0 d"/>
    <a:gd name="yb" fmla="+- b 0 d"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
    <a:moveTo><a:pt x="d" y="d"/></a:moveTo>
    <a:lnTo><a:pt x="d" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="d"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `halfFrame`.
///
/// Measured: two adjusts, defaults 0.33333 each. An L-shaped bar along the top and left edges whose free ends are cut
/// parallel to the box diagonal; six points: (l, t), (r, t), (xr, y1), (x1, y1), (x1, yb), (l, b). `adj1` is the top
/// bar's thickness `y1 = ss * adj1 / 100000` (48 pt of 144), `adj2` the left bar's `x1 = ss * adj2 / 100000`. The cut
/// ends lie on lines parallel to the diagonal: `xr = r - y1 * w / h`, `yb = b - x1 * h / w`. Clamps seen on 216x144:
/// `adj2` 0..`100000 * w / ss` (400000 gives x1 = 216); `adj1` 0..`(w - x1) * h / w` in length (the inner corner
/// never passes the diagonal: with both at 100000, y1 = 48 and x1 = 144). Negative values act as 0.
fn half_frame() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 33333"/><a:gd name="adj2" fmla="val 33333"/></a:avLst>
  <a:gdLst>
    <a:gd name="maxA2" fmla="*/ 100000 w ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 maxA2"/>
    <a:gd name="x1" fmla="*/ ss a2 100000"/>
    <a:gd name="a1" fmla="max 0 adj1"/>
    <a:gd name="y1r" fmla="*/ ss a1 100000"/>
    <a:gd name="wx" fmla="+- w 0 x1"/>
    <a:gd name="ymax" fmla="*/ wx h w"/>
    <a:gd name="y1" fmla="min y1r ymax"/>
    <a:gd name="dxr" fmla="*/ y1 w h"/>
    <a:gd name="xr" fmla="+- r 0 dxr"/>
    <a:gd name="dyb" fmla="*/ x1 h w"/>
    <a:gd name="yb" fmla="+- b 0 dyb"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xr" y="y1"/></a:lnTo>
    <a:lnTo><a:pt x="x1" y="y1"/></a:lnTo>
    <a:lnTo><a:pt x="x1" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `corner`.
///
/// Measured: two adjusts, defaults 0.5 each. An L-shaped corner along the left and bottom edges, six points: (l, t),
/// (x1, t), (x1, y1), (r, y1), (r, b), (l, b). `adj1` is the bottom bar's thickness (`y1 = b - ss * adj1 / 100000`,
/// 72 pt of 144, 48 pt on 96x288), `adj2` the left bar's width (`x1 = ss * adj2 / 100000`). Clamps: `adj1`
/// 0..`100000 * h / ss` and `adj2` 0..`100000 * w / ss` (400000 for both gives a full-height bar and a full-width bar);
/// negative values act as 0.
fn corner() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="maxA1" fmla="*/ 100000 h ss"/>
    <a:gd name="a1" fmla="pin 0 adj1 maxA1"/>
    <a:gd name="maxA2" fmla="*/ 100000 w ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 maxA2"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="y1" fmla="+- b 0 th"/>
    <a:gd name="x1" fmla="*/ ss a2 100000"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="x1" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="x1" y="y1"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="y1"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `diagStripe`.
///
/// Measured: one adjust, default 0.5, clamped 0..100000 (-50000 gives 0, 400000 gives 100000). A diagonal band: the
/// quadrilateral (l, y1), (x1, t), (r, t), (l, b) with `x1 = w * adj / 100000` and `y1 = h * adj / 100000` (108 and 72
/// pt at the default on 216x144).
fn diag_stripe() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a" fmla="pin 0 adj 100000"/>
    <a:gd name="x1" fmla="*/ w a 100000"/>
    <a:gd name="y1" fmla="*/ h a 100000"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="y1"/></a:moveTo>
    <a:lnTo><a:pt x="x1" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `chord`.
///
/// Measured: two adjusts, the start angle (default 45 degrees, 2700000) and the end angle (default 270 degrees,
/// 16200000; Word's object model reports it as -90). The outline is the box ellipse's arc from the start angle clockwise
/// to the end angle, closed by the chord. Both angles are clamped to 0..21599999 (negative values act as 0); when the
/// end angle is not past the start (equal values included) the arc is the whole ellipse, `sweep = end - start` or that
/// plus 360 degrees. The angles are visual angles of the ellipse, so the start point is the ellipse point on that ray.
/// Note for the harness: the end angle default is 16200000, but `Shape.Adjustments` reports -90, so the default check
/// that compares `-90 * 60000` fails for it although the outline is right on every page.
fn chord() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 2700000"/><a:gd name="adj2" fmla="val 16200000"/></a:avLst>
  <a:gdLst>
    <a:gd name="stAng" fmla="pin 0 adj1 21599999"/>
    <a:gd name="enAng" fmla="pin 0 adj2 21599999"/>
    <a:gd name="sw0" fmla="+- enAng 0 stAng"/>
    <a:gd name="sw1" fmla="+- sw0 21600000 0"/>
    <a:gd name="swAng" fmla="?: sw0 sw0 sw1"/>
    <a:gd name="cx1" fmla="cos hd2 stAng"/>
    <a:gd name="sx1" fmla="sin wd2 stAng"/>
    <a:gd name="dx1" fmla="cat2 wd2 cx1 sx1"/>
    <a:gd name="dy1" fmla="sat2 hd2 cx1 sx1"/>
    <a:gd name="xs" fmla="+- hc dx1 0"/>
    <a:gd name="ys" fmla="+- vc dy1 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="xs" y="ys"/></a:moveTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="stAng" swAng="swAng"/>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `arc`.
///
/// Measured: two adjusts, the start angle (default 270 degrees, 16200000; Word reports -90) and the end angle (default
/// 0). A stroked open arc of the box ellipse from the start angle clockwise to the end angle, and an unstroked pie
/// (arc plus the lines to the centre) for the fill. Angles are clamped to 0..21599999 (negative values act as 0, which is
/// why the probes that set the reported -90 degrees as -5400000 show a full ellipse); equal start and end give the whole
/// ellipse. Same default-check note as `chord`.
fn arc() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 16200000"/><a:gd name="adj2" fmla="val 0"/></a:avLst>
  <a:gdLst>
    <a:gd name="stAng" fmla="pin 0 adj1 21599999"/>
    <a:gd name="enAng" fmla="pin 0 adj2 21599999"/>
    <a:gd name="sw0" fmla="+- enAng 0 stAng"/>
    <a:gd name="sw1" fmla="+- sw0 21600000 0"/>
    <a:gd name="swAng" fmla="?: sw0 sw0 sw1"/>
    <a:gd name="cx1" fmla="cos hd2 stAng"/>
    <a:gd name="sx1" fmla="sin wd2 stAng"/>
    <a:gd name="dx1" fmla="cat2 wd2 cx1 sx1"/>
    <a:gd name="dy1" fmla="sat2 hd2 cx1 sx1"/>
    <a:gd name="xs" fmla="+- hc dx1 0"/>
    <a:gd name="ys" fmla="+- vc dy1 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="xs" y="ys"/></a:moveTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="stAng" swAng="swAng"/>
    <a:lnTo><a:pt x="hc" y="vc"/></a:lnTo>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="xs" y="ys"/></a:moveTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="stAng" swAng="swAng"/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `leftBracket`.
///
/// Measured: one adjust, default 0.08333. An open bracket on the left edge with rounded ends: from the bottom right
/// (r, b) a quarter ellipse (radii `w` and the corner size) to the left edge, up the edge, a second quarter ellipse to
/// (r, t). The corner size is `ss * adj / 100000` (12 pt of 144), clamped 0..`50000 * h / ss` (100000 and 400000 give half
/// the height, 72 pt; 0 and -50000 give 0). The fill is the same outline closed along the right edge.
fn left_bracket() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 8333"/></a:avLst>
  <a:gdLst>
    <a:gd name="maxA" fmla="*/ 50000 h ss"/>
    <a:gd name="a" fmla="pin 0 adj maxA"/>
    <a:gd name="r10" fmla="*/ ss a 100000"/>
    <a:gd name="r1" fmla="max r10 0"/>
    <a:gd name="yb" fmla="+- b 0 r1"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="r" y="b"/></a:moveTo>
    <a:arcTo wR="w" hR="r1" stAng="cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="l" y="r1"/></a:lnTo>
    <a:arcTo wR="w" hR="r1" stAng="cd2" swAng="cd4"/>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="r" y="b"/></a:moveTo>
    <a:arcTo wR="w" hR="r1" stAng="cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="l" y="r1"/></a:lnTo>
    <a:arcTo wR="w" hR="r1" stAng="cd2" swAng="cd4"/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `rightBracket`.
///
/// Measured: one adjust, default 0.08333. The mirror image of `leftBracket` (bracket on the right edge, drawn from the
/// top left (l, t)); same corner size and clamp.
fn right_bracket() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 8333"/></a:avLst>
  <a:gdLst>
    <a:gd name="maxA" fmla="*/ 50000 h ss"/>
    <a:gd name="a" fmla="pin 0 adj maxA"/>
    <a:gd name="r10" fmla="*/ ss a 100000"/>
    <a:gd name="r1" fmla="max r10 0"/>
    <a:gd name="yb" fmla="+- b 0 r1"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:arcTo wR="w" hR="r1" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
    <a:arcTo wR="w" hR="r1" stAng="0" swAng="cd4"/>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:arcTo wR="w" hR="r1" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
    <a:arcTo wR="w" hR="r1" stAng="0" swAng="cd4"/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `leftBrace`.
///
/// Measured: two adjusts, defaults 0.08333 and 0.5. A brace on the left: the stem is the vertical line at the middle of the
/// width, joined to the top and bottom right corners by quarter ellipses (radii `w/2` and the corner size) and to the
/// point on the left edge by two more, meeting at (l, ym). `adj1` is the corner size `ss * adj1 / 100000` (12 pt of 144),
/// `adj2` the point's height as a fraction of the height (72 pt of 144), clamped 0..100000. The corner size is further
/// limited to half the distance from the point to the top and to the bottom (`min(r1, ym / 2, (h - ym) / 2)`: 21.6 pt
/// with both adjusts 30000 on 216x144, 0 with both 100000). The fill is the outline closed along the right edge.
fn left_brace() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 8333"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a2" fmla="pin 0 adj2 100000"/>
    <a:gd name="a1" fmla="max 0 adj1"/>
    <a:gd name="ym" fmla="*/ h a2 100000"/>
    <a:gd name="hmy" fmla="+- h 0 ym"/>
    <a:gd name="r1a" fmla="*/ ss a1 100000"/>
    <a:gd name="ymh" fmla="*/ ym 1 2"/>
    <a:gd name="hmh" fmla="*/ hmy 1 2"/>
    <a:gd name="r1b" fmla="min r1a ymh"/>
    <a:gd name="r1c" fmla="min r1b hmh"/>
    <a:gd name="r1" fmla="max r1c 0"/>
    <a:gd name="yb" fmla="+- b 0 r1"/>
    <a:gd name="yu" fmla="+- ym 0 r1"/>
    <a:gd name="yl" fmla="+- ym r1 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="r" y="b"/></a:moveTo>
    <a:arcTo wR="wd2" hR="r1" stAng="cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="hc" y="yl"/></a:lnTo>
    <a:arcTo wR="wd2" hR="r1" stAng="0" swAng="-5400000"/>
    <a:arcTo wR="wd2" hR="r1" stAng="cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="hc" y="r1"/></a:lnTo>
    <a:arcTo wR="wd2" hR="r1" stAng="cd2" swAng="cd4"/>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="r" y="b"/></a:moveTo>
    <a:arcTo wR="wd2" hR="r1" stAng="cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="hc" y="yl"/></a:lnTo>
    <a:arcTo wR="wd2" hR="r1" stAng="0" swAng="-5400000"/>
    <a:arcTo wR="wd2" hR="r1" stAng="cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="hc" y="r1"/></a:lnTo>
    <a:arcTo wR="wd2" hR="r1" stAng="cd2" swAng="cd4"/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `rightBrace`.
///
/// Measured: two adjusts, defaults 0.08333 and 0.5. The mirror image of `leftBrace` (stem at the middle, point on the right
/// edge, drawn from the top left); same units and clamps.
fn right_brace() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 8333"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a2" fmla="pin 0 adj2 100000"/>
    <a:gd name="a1" fmla="max 0 adj1"/>
    <a:gd name="ym" fmla="*/ h a2 100000"/>
    <a:gd name="hmy" fmla="+- h 0 ym"/>
    <a:gd name="r1a" fmla="*/ ss a1 100000"/>
    <a:gd name="ymh" fmla="*/ ym 1 2"/>
    <a:gd name="hmh" fmla="*/ hmy 1 2"/>
    <a:gd name="r1b" fmla="min r1a ymh"/>
    <a:gd name="r1c" fmla="min r1b hmh"/>
    <a:gd name="r1" fmla="max r1c 0"/>
    <a:gd name="yb" fmla="+- b 0 r1"/>
    <a:gd name="yu" fmla="+- ym 0 r1"/>
    <a:gd name="yl" fmla="+- ym r1 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:arcTo wR="wd2" hR="r1" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="hc" y="yu"/></a:lnTo>
    <a:arcTo wR="wd2" hR="r1" stAng="cd2" swAng="-5400000"/>
    <a:arcTo wR="wd2" hR="r1" stAng="3cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="hc" y="yb"/></a:lnTo>
    <a:arcTo wR="wd2" hR="r1" stAng="0" swAng="cd4"/>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:arcTo wR="wd2" hR="r1" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="hc" y="yu"/></a:lnTo>
    <a:arcTo wR="wd2" hR="r1" stAng="cd2" swAng="-5400000"/>
    <a:arcTo wR="wd2" hR="r1" stAng="3cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="hc" y="yb"/></a:lnTo>
    <a:arcTo wR="wd2" hR="r1" stAng="0" swAng="cd4"/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `bracketPair`.
///
/// Measured: one adjust, default 0.16667. Two brackets, one on each side, each with quarter-circle corners of radius
/// `ss * adj / 100000` (24 pt of 144), clamped 0..50000 (100000 and 400000 give 72 pt, the brackets then nearly
/// meeting at the middle). Stroked: the left bracket (open, from the bottom), the right bracket (open, from the top).
/// The fill is the rounded rectangle of the same corner radius.
fn bracket_pair() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 16667"/></a:avLst>
  <a:gdLst>
    <a:gd name="a" fmla="pin 0 adj 50000"/>
    <a:gd name="r1" fmla="*/ ss a 100000"/>
    <a:gd name="xr" fmla="+- r 0 r1"/>
    <a:gd name="yb" fmla="+- b 0 r1"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="l" y="r1"/></a:moveTo>
    <a:arcTo wR="r1" hR="r1" stAng="cd2" swAng="cd4"/>
    <a:lnTo><a:pt x="xr" y="t"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="0" swAng="cd4"/>
    <a:lnTo><a:pt x="r1" y="b"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="cd4" swAng="cd4"/>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="r1" y="b"/></a:moveTo>
    <a:arcTo wR="r1" hR="r1" stAng="cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="l" y="r1"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="cd2" swAng="cd4"/>
    <a:moveTo><a:pt x="xr" y="t"/></a:moveTo>
    <a:arcTo wR="r1" hR="r1" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="0" swAng="cd4"/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `bracePair`.
///
/// Measured: one adjust, default 0.08333. Two braces, one on each side; each is two quarter circles of radius
/// `r1 = ss * adj / 100000` (12 pt of 144) at the top and bottom corners joined by a stem at distance r1 from the edge, and
/// two more to the point on the edge at mid height, so each brace is 2 * r1 wide. Clamp: `adj` 0..25000 (30000, 100000
/// and 400000 give 25000, 36 pt, a quarter of the short side). The fill is the outline of both braces joined by the top
/// and bottom edges.
fn brace_pair() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 8333"/></a:avLst>
  <a:gdLst>
    <a:gd name="a" fmla="pin 0 adj 25000"/>
    <a:gd name="r1" fmla="*/ ss a 100000"/>
    <a:gd name="r2" fmla="*/ r1 2 1"/>
    <a:gd name="xr" fmla="+- r 0 r1"/>
    <a:gd name="xr2" fmla="+- r 0 r2"/>
    <a:gd name="yb" fmla="+- b 0 r1"/>
    <a:gd name="yu" fmla="+- vc 0 r1"/>
    <a:gd name="yl" fmla="+- vc r1 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path stroke="0">
    <a:moveTo><a:pt x="r2" y="b"/></a:moveTo>
    <a:arcTo wR="r1" hR="r1" stAng="cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r1" y="yl"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="0" swAng="-5400000"/>
    <a:arcTo wR="r1" hR="r1" stAng="cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="r1" y="r1"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="cd2" swAng="cd4"/>
    <a:lnTo><a:pt x="xr2" y="t"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="xr" y="yu"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="cd2" swAng="-5400000"/>
    <a:arcTo wR="r1" hR="r1" stAng="3cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="xr" y="yb"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="0" swAng="cd4"/>
    <a:close/>
  </a:path>
  <a:path fill="none">
    <a:moveTo><a:pt x="r2" y="b"/></a:moveTo>
    <a:arcTo wR="r1" hR="r1" stAng="cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r1" y="yl"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="0" swAng="-5400000"/>
    <a:arcTo wR="r1" hR="r1" stAng="cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="r1" y="r1"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="cd2" swAng="cd4"/>
    <a:moveTo><a:pt x="xr2" y="t"/></a:moveTo>
    <a:arcTo wR="r1" hR="r1" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="xr" y="yu"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="cd2" swAng="-5400000"/>
    <a:arcTo wR="r1" hR="r1" stAng="3cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="xr" y="yb"/></a:lnTo>
    <a:arcTo wR="r1" hR="r1" stAng="0" swAng="cd4"/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `straightConnector1`.
///
/// Measured: no adjusts. A straight line from the top left (l, t) to the bottom right (r, b) at all four sizes, not filled.
fn straight_connector1() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst/>
  <a:gdLst/>
  <a:pathLst>
  <a:path fill="none">
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// The presets of this batch `(name, definition)`.
pub(super) fn entries() -> Vec<(&'static str, ShapeDef)> {
    [
        ("cube", cube()),
        ("can", can()),
        ("lightningBolt", lightning_bolt()),
        ("heart", heart()),
        ("irregularSeal1", irregular_seal1()),
        ("irregularSeal2", irregular_seal2()),
        ("sun", sun()),
        ("moon", moon()),
        ("smileyFace", smiley_face()),
        ("foldedCorner", folded_corner()),
        ("bevel", bevel()),
        ("frame", frame()),
        ("halfFrame", half_frame()),
        ("corner", corner()),
        ("diagStripe", diag_stripe()),
        ("chord", chord()),
        ("arc", arc()),
        ("leftBracket", left_bracket()),
        ("rightBracket", right_bracket()),
        ("leftBrace", left_brace()),
        ("rightBrace", right_brace()),
        ("bracketPair", bracket_pair()),
        ("bracePair", brace_pair()),
        ("straightConnector1", straight_connector1()),
    ]
    .into_iter()
    .filter_map(|(n, d)| d.map(|d| (n, d)))
    .collect()
}
