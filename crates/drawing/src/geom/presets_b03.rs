//! Clean-room presets of batch b03 (see `docs/architecture/drawing.md`).
//!
//! Every entry was written from the ECMA-376 guide-language prose and from Word's own output for the probe
//! documents in `fixtures/reference/drawing/presets/b03.*` (`tools/office/make-preset-probes.ps1`); the doc comment of
//! each entry says what was measured. No other project's preset tables and no copy of Ecma's
//! `presetShapeDefinitions.xml` were opened. Guide names are our own.

use crate::geom::ShapeDef;
use crate::parse::cust_geom_from_str;

/// `notchedRightArrow`.
///
/// Measured (b03, 144x144, 288x96, 96x288, 216x144 pt and the adjust pages): two adjusts, both default 0.5. Eight points:
/// shaft top left, shaft top at the head base, head top corner, tip, head bottom corner, shaft bottom at the head base,
/// shaft bottom left, and the notch point on the middle line. `adj1` is the shaft thickness as a fraction of the height
/// (72 of 144 pt at 50000), clamped 0..100000. `adj2` is the head length as a fraction of the short side (72 pt at
/// 144x144 and 216x144, 48 pt at 288x96 and 96x288), clamped 0..`100000 * w / ss` (400000 gives a 216 pt head).
/// The notch depth is `half shaft * head length / (h / 2)`: 36 pt at the defaults on 216x144, 12.96 pt with 30000/30000,
/// 144 pt with 100000/100000, 0 when either adjust is 0.
fn notched_right_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="shaftPin" fmla="pin 0 adj1 100000"/>
    <a:gd name="headMax" fmla="*/ 100000 w ss"/>
    <a:gd name="headPin" fmla="pin 0 adj2 headMax"/>
    <a:gd name="headLen" fmla="*/ ss headPin 100000"/>
    <a:gd name="xHead" fmla="+- r 0 headLen"/>
    <a:gd name="halfShaft" fmla="*/ h shaftPin 200000"/>
    <a:gd name="yTop" fmla="+- vc 0 halfShaft"/>
    <a:gd name="yBot" fmla="+- vc halfShaft 0"/>
    <a:gd name="xNotch" fmla="*/ halfShaft headLen hd2"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="yTop"/></a:moveTo>
    <a:lnTo><a:pt x="xHead" y="yTop"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="vc"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="yBot"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="yBot"/></a:lnTo>
    <a:lnTo><a:pt x="xNotch" y="vc"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `bentUpArrow`.
///
/// Measured: three adjusts, all default 0.25. A horizontal bar along the bottom edge turns up into a vertical shaft whose
/// centre line sits at `r - head half width`; the head points up at the top edge. All three are fractions of the short
/// side: `adj1` shaft thickness (36 pt at 144 short side), `adj2` head half width (36 pt), `adj3` head length (36 pt).
/// Each is clamped to 0..50000 (100000 and 400000 give 72 pt for all three on 216x144). The shaft may leave the box on
/// the right when `adj2` is 0 (probe `scan-adj2`). The clamp for `adj3` is 50000 of `ss` at the only size where it was
/// seen (`ss` = 144); other clamp forms that agree there cannot be told apart with the fixed probe sizes.
fn bent_up_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/><a:gd name="adj3" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a1" fmla="pin 0 adj1 50000"/>
    <a:gd name="a2" fmla="pin 0 adj2 50000"/>
    <a:gd name="a3" fmla="pin 0 adj3 50000"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="xTip" fmla="+- r 0 hw"/>
    <a:gd name="thHalf" fmla="*/ th 1 2"/>
    <a:gd name="xS0" fmla="+- xTip 0 thHalf"/>
    <a:gd name="xS1" fmla="+- xTip thHalf 0"/>
    <a:gd name="xWing" fmla="+- xTip 0 hw"/>
    <a:gd name="yBar" fmla="+- b 0 th"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="yBar"/></a:moveTo>
    <a:lnTo><a:pt x="xS0" y="yBar"/></a:lnTo>
    <a:lnTo><a:pt x="xS0" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xWing" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xTip" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xS1" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xS1" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `leftRightArrow`.
///
/// Measured: two adjusts, both default 0.5. Ten points: a head at each end, one shaft between. `adj1` shaft thickness as a
/// fraction of the height (72 of 144 pt), clamped 0..100000. `adj2` head length as a fraction of the short side (72 pt at
/// 216x144, 48 pt at 288x96 and 96x288), clamped 0..`50000 * w / ss` (100000 and 400000 both give heads of half the width,
/// 108 pt on 216x144, that meet in the middle).
fn left_right_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="shaftPin" fmla="pin 0 adj1 100000"/>
    <a:gd name="headMax" fmla="*/ 50000 w ss"/>
    <a:gd name="headPin" fmla="pin 0 adj2 headMax"/>
    <a:gd name="headLen" fmla="*/ ss headPin 100000"/>
    <a:gd name="xL" fmla="+- l headLen 0"/>
    <a:gd name="xR" fmla="+- r 0 headLen"/>
    <a:gd name="halfShaft" fmla="*/ h shaftPin 200000"/>
    <a:gd name="yTop" fmla="+- vc 0 halfShaft"/>
    <a:gd name="yBot" fmla="+- vc halfShaft 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
    <a:lnTo><a:pt x="xL" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xL" y="yTop"/></a:lnTo>
    <a:lnTo><a:pt x="xR" y="yTop"/></a:lnTo>
    <a:lnTo><a:pt x="xR" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="vc"/></a:lnTo>
    <a:lnTo><a:pt x="xR" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xR" y="yBot"/></a:lnTo>
    <a:lnTo><a:pt x="xL" y="yBot"/></a:lnTo>
    <a:lnTo><a:pt x="xL" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `upDownArrow`.
///
/// Measured: two adjusts, both default 0.5. Ten points: a head at the top and at the bottom, one shaft between. `adj1`
/// shaft width as a fraction of the width (108 of 216 pt), clamped 0..100000. `adj2` head length as a fraction of the
/// short side (72 pt at 216x144, 48 pt at 96x288), clamped 0..`50000 * h / ss` (100000 and 400000 give heads of half
/// the height that meet in the middle).
fn up_down_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="shaftPin" fmla="pin 0 adj1 100000"/>
    <a:gd name="headMax" fmla="*/ 50000 h ss"/>
    <a:gd name="headPin" fmla="pin 0 adj2 headMax"/>
    <a:gd name="headLen" fmla="*/ ss headPin 100000"/>
    <a:gd name="yT" fmla="+- t headLen 0"/>
    <a:gd name="yB" fmla="+- b 0 headLen"/>
    <a:gd name="halfShaft" fmla="*/ w shaftPin 200000"/>
    <a:gd name="xL" fmla="+- hc 0 halfShaft"/>
    <a:gd name="xR" fmla="+- hc halfShaft 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="yT"/></a:moveTo>
    <a:lnTo><a:pt x="hc" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yT"/></a:lnTo>
    <a:lnTo><a:pt x="xR" y="yT"/></a:lnTo>
    <a:lnTo><a:pt x="xR" y="yB"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yB"/></a:lnTo>
    <a:lnTo><a:pt x="hc" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="yB"/></a:lnTo>
    <a:lnTo><a:pt x="xL" y="yB"/></a:lnTo>
    <a:lnTo><a:pt x="xL" y="yT"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `leftUpArrow`.
///
/// Measured: three adjusts, all default 0.25. A left arrow whose centre line is `head half width` above the bottom edge
/// and an up arrow whose centre line is `head half width` left of the right edge, joined by an L-shaped shaft; twelve
/// points. All three are fractions of the short side (36 pt at 144): `adj1` shaft thickness, `adj2` head half width,
/// `adj3` head length. Clamps seen: `adj2` 0..50000; `adj1` 0..`2 * adj2` (with `adj2` 0 the shaft vanishes even with
/// `adj1` 25000); `adj3` 0..`100000 - 2 * adj2` (100000 gives a zero-length head when `adj2` is clamped to 50000, and
/// 30000 stays 30000 when `adj2` is 30000). Another form, `100000 - adj2 - adj1 / 2`, fits the same pages.
fn left_up_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/><a:gd name="adj3" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a2" fmla="pin 0 adj2 50000"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="max3" fmla="+- 100000 0 max1"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thHalf" fmla="*/ th 1 2"/>
    <a:gd name="xc" fmla="+- r 0 hw"/>
    <a:gd name="yc" fmla="+- b 0 hw"/>
    <a:gd name="yHeadTop" fmla="+- yc 0 hw"/>
    <a:gd name="yShTop" fmla="+- yc 0 thHalf"/>
    <a:gd name="yShBot" fmla="+- yc thHalf 0"/>
    <a:gd name="xShL" fmla="+- xc 0 thHalf"/>
    <a:gd name="xShR" fmla="+- xc thHalf 0"/>
    <a:gd name="xWingL" fmla="+- xc 0 hw"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="yc"/></a:moveTo>
    <a:lnTo><a:pt x="hl" y="yHeadTop"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShTop"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yShTop"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xWingL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xc" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yShBot"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShBot"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `leftRightUpArrow`.
///
/// Measured: three adjusts, all default 0.25. A T-shaped arrow: left and right heads on a horizontal shaft whose centre
/// line is `head half width` above the bottom edge, and an up head on a vertical shaft in the middle; seventeen points.
/// All three are fractions of the short side (36 pt at 144): `adj1` shaft thickness, `adj2` head half width, `adj3` head
/// length. Clamps seen: `adj2` 0..50000; `adj1` 0..`2 * adj2`; `adj3` 0..`50000 - adj2` (30000 on 216x144 with all adjusts
/// 30000 gives 20000, 28.8 pt; 100000 gives 0). Which of `ss`, `w` or `h` the last clamp uses cannot be told apart at
/// the probe sizes (the clamp is only reached on 216x144, where `ss` = `h`).
fn left_right_up_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/><a:gd name="adj3" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a2" fmla="pin 0 adj2 50000"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="max3" fmla="+- 50000 0 a2"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thHalf" fmla="*/ th 1 2"/>
    <a:gd name="yc" fmla="+- b 0 hw"/>
    <a:gd name="yHeadTop" fmla="+- yc 0 hw"/>
    <a:gd name="yShTop" fmla="+- yc 0 thHalf"/>
    <a:gd name="yShBot" fmla="+- yc thHalf 0"/>
    <a:gd name="xShL" fmla="+- hc 0 thHalf"/>
    <a:gd name="xShR" fmla="+- hc thHalf 0"/>
    <a:gd name="xWingL" fmla="+- hc 0 hw"/>
    <a:gd name="xWingR" fmla="+- hc hw 0"/>
    <a:gd name="xHeadR" fmla="+- r 0 hl"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="yc"/></a:moveTo>
    <a:lnTo><a:pt x="hl" y="yHeadTop"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShTop"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yShTop"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xWingL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="hc" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xWingR" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yShTop"/></a:lnTo>
    <a:lnTo><a:pt x="xHeadR" y="yShTop"/></a:lnTo>
    <a:lnTo><a:pt x="xHeadR" y="yHeadTop"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yc"/></a:lnTo>
    <a:lnTo><a:pt x="xHeadR" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xHeadR" y="yShBot"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShBot"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `quadArrow`.
///
/// Measured: three adjusts, all default 0.225 (22500). A cross of four arrows, twenty-four points. `adj1` shaft
/// thickness, `adj2` head half width, `adj3` head length, all fractions of the short side (32.4 pt at 144 short side).
/// Clamps seen on 216x144: `adj2` 0..50000; `adj1` 0..`2 * adj2` (with `adj2` 0 the shafts vanish); `adj3`
/// 0..`50000 - adj2` (30000 on all three gives 20000, 28.8 pt; 100000 gives 0). The clamp has the same form as the one of
/// `leftRightUpArrow`; whether it uses `ss`, `w` or `h` is not separable at the probe sizes.
fn quad_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 22500"/><a:gd name="adj2" fmla="val 22500"/><a:gd name="adj3" fmla="val 22500"/></a:avLst>
  <a:gdLst>
    <a:gd name="a2" fmla="pin 0 adj2 50000"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="max3" fmla="+- 50000 0 a2"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="yWingT" fmla="+- vc 0 hw"/>
    <a:gd name="yWingB" fmla="+- vc hw 0"/>
    <a:gd name="yShT" fmla="+- vc 0 thH"/>
    <a:gd name="yShB" fmla="+- vc thH 0"/>
    <a:gd name="xWingL" fmla="+- hc 0 hw"/>
    <a:gd name="xWingR" fmla="+- hc hw 0"/>
    <a:gd name="xShL" fmla="+- hc 0 thH"/>
    <a:gd name="xShR" fmla="+- hc thH 0"/>
    <a:gd name="xHR" fmla="+- r 0 hl"/>
    <a:gd name="yHB" fmla="+- b 0 hl"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
    <a:lnTo><a:pt x="hl" y="yWingT"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xWingL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="hc" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xWingR" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yWingT"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="vc"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yWingB"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="xWingR" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="hc" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xWingL" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yWingB"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `leftArrowCallout`.
///
/// Measured: four adjusts, defaults 0.25, 0.25, 0.25 and 0.64977. A left arrow attached to a rectangle that fills the
/// right part of the box; eleven points. `adj1` shaft thickness, `adj2` head half width, `adj3` head length are fractions
/// of the short side (36 pt at 144); `adj4` is the width of the box as a fraction of the width (140.35 pt of 216, 62.38 pt
/// of 96). Clamps seen on 216x144: `adj2` 0..`50000 * h / ss` (the head half width is at most half the height);
/// `adj1` 0..`2 * adj2`; `adj3` 0..`100000 * w / ss` (400000 gives a 216 pt head); `adj4` 0..100000, and the box never
/// starts left of the head base (with `adj3` 100000 and `adj4` 100000 the box starts at x = 144, the head base). With
/// `adj4` 0 the box is empty at the right edge.
fn left_arrow_callout() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/><a:gd name="adj3" fmla="val 25000"/><a:gd name="adj4" fmla="val 64977"/></a:avLst>
  <a:gdLst>
    <a:gd name="max2" fmla="*/ 50000 h ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 max2"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="max3" fmla="*/ 100000 w ss"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="a4" fmla="pin 0 adj4 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="boxLen" fmla="*/ w a4 100000"/>
    <a:gd name="yWingT" fmla="+- vc 0 hw"/>
    <a:gd name="yWingB" fmla="+- vc hw 0"/>
    <a:gd name="yShT" fmla="+- vc 0 thH"/>
    <a:gd name="yShB" fmla="+- vc thH 0"/>
    <a:gd name="xBoxRaw" fmla="+- r 0 boxLen"/>
    <a:gd name="xBox" fmla="max hl xBoxRaw"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
    <a:lnTo><a:pt x="hl" y="yWingT"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xBox" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xBox" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xBox" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xBox" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yWingB"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `rightArrowCallout`.
///
/// Measured: four adjusts, defaults 0.25, 0.25, 0.25 and 0.64977. A right arrow attached to a rectangle that fills the
/// left part of the box; eleven points. `adj1` shaft thickness, `adj2` head half width, `adj3` head length are fractions
/// of the short side (36 pt at 144); `adj4` is the width of the box as a fraction of the width (140.35 pt of 216).
/// Clamps as for `leftArrowCallout`, mirrored: `adj2` 0..`50000 * h / ss`, `adj1` 0..`2 * adj2`, `adj3`
/// 0..`100000 * w / ss`, `adj4` 0..100000 with the box never reaching past the head base.
fn right_arrow_callout() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/><a:gd name="adj3" fmla="val 25000"/><a:gd name="adj4" fmla="val 64977"/></a:avLst>
  <a:gdLst>
    <a:gd name="max2" fmla="*/ 50000 h ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 max2"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="max3" fmla="*/ 100000 w ss"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="a4" fmla="pin 0 adj4 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="boxLen" fmla="*/ w a4 100000"/>
    <a:gd name="yWingT" fmla="+- vc 0 hw"/>
    <a:gd name="yWingB" fmla="+- vc hw 0"/>
    <a:gd name="yShT" fmla="+- vc 0 thH"/>
    <a:gd name="yShB" fmla="+- vc thH 0"/>
    <a:gd name="xHead" fmla="+- r 0 hl"/>
    <a:gd name="xBox" fmla="min xHead boxLen"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="xBox" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xBox" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="yWingT"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="vc"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="yWingB"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="xBox" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="xBox" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `upArrowCallout`.
///
/// Measured: four adjusts, defaults 0.25, 0.25, 0.25 and 0.64977. An up arrow attached to a rectangle that fills the
/// lower part of the box; eleven points. `adj1` shaft thickness, `adj2` head half width, `adj3` head length are fractions
/// of the short side (36 pt at 144); `adj4` is the height of the box as a fraction of the height (93.57 pt of 144, 187.1
/// of 288). Clamps seen on 216x144: `adj2` 0..`50000 * w / ss` (the head half width is at most half the width: 108 pt);
/// `adj1` 0..`2 * adj2`; `adj3` 0..`100000 * h / ss`; `adj4` 0..100000 with the box never starting above the head base.
fn up_arrow_callout() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/><a:gd name="adj3" fmla="val 25000"/><a:gd name="adj4" fmla="val 64977"/></a:avLst>
  <a:gdLst>
    <a:gd name="max2" fmla="*/ 50000 w ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 max2"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="max3" fmla="*/ 100000 h ss"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="a4" fmla="pin 0 adj4 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="boxLen" fmla="*/ h a4 100000"/>
    <a:gd name="xWingL" fmla="+- hc 0 hw"/>
    <a:gd name="xWingR" fmla="+- hc hw 0"/>
    <a:gd name="xShL" fmla="+- hc 0 thH"/>
    <a:gd name="xShR" fmla="+- hc thH 0"/>
    <a:gd name="yBoxRaw" fmla="+- b 0 boxLen"/>
    <a:gd name="yBox" fmla="max hl yBoxRaw"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="yBox"/></a:moveTo>
    <a:lnTo><a:pt x="xShL" y="yBox"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xWingL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="hc" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xWingR" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yBox"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yBox"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `downArrowCallout`.
///
/// Measured: four adjusts, defaults 0.25, 0.25, 0.25 and 0.64977. A down arrow attached to a rectangle that fills the
/// upper part of the box; eleven points. `adj1` shaft thickness, `adj2` head half width, `adj3` head length are
/// fractions of the short side (36 pt at 144); `adj4` is the height of the box as a fraction of the height. Clamps as
/// for `upArrowCallout`, mirrored: `adj2` 0..`50000 * w / ss`, `adj1` 0..`2 * adj2`, `adj3` 0..`100000 * h / ss`,
/// `adj4` 0..100000 with the box never reaching past the head base.
fn down_arrow_callout() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/><a:gd name="adj3" fmla="val 25000"/><a:gd name="adj4" fmla="val 64977"/></a:avLst>
  <a:gdLst>
    <a:gd name="max2" fmla="*/ 50000 w ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 max2"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="max3" fmla="*/ 100000 h ss"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="a4" fmla="pin 0 adj4 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="boxLen" fmla="*/ h a4 100000"/>
    <a:gd name="xWingL" fmla="+- hc 0 hw"/>
    <a:gd name="xWingR" fmla="+- hc hw 0"/>
    <a:gd name="xShL" fmla="+- hc 0 thH"/>
    <a:gd name="xShR" fmla="+- hc thH 0"/>
    <a:gd name="yHead" fmla="+- b 0 hl"/>
    <a:gd name="yBox" fmla="min yHead boxLen"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="t"/></a:moveTo>
    <a:lnTo><a:pt x="r" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yBox"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yBox"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yHead"/></a:lnTo>
    <a:lnTo><a:pt x="xWingR" y="yHead"/></a:lnTo>
    <a:lnTo><a:pt x="hc" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xWingL" y="yHead"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yHead"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yBox"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="yBox"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `leftRightArrowCallout`.
///
/// Measured: four adjusts, defaults 0.25, 0.25, 0.25 and 0.48123. A box centred in the shape with a left and a right
/// arrow; eighteen points. `adj1` shaft thickness, `adj2` head half width, `adj3` head length are fractions of the short
/// side (36 pt at 144); `adj4` is the width of the box as a fraction of the width (103.94 pt of 216, 46.2 pt of 96).
/// Clamps seen on 216x144: `adj2` 0..`50000 * h / ss` (72 pt); `adj1` 0..`2 * adj2`; `adj3` 0..`50000 * w / ss` (heads
/// of half the width meet at the middle, 108 pt); `adj4` 0..100000, and the box edges never pass the head bases.
fn left_right_arrow_callout() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/><a:gd name="adj3" fmla="val 25000"/><a:gd name="adj4" fmla="val 48123"/></a:avLst>
  <a:gdLst>
    <a:gd name="max2" fmla="*/ 50000 h ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 max2"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="max3" fmla="*/ 50000 w ss"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="a4" fmla="pin 0 adj4 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="halfBox" fmla="*/ w a4 200000"/>
    <a:gd name="xBoxL0" fmla="+- hc 0 halfBox"/>
    <a:gd name="xBoxR0" fmla="+- hc halfBox 0"/>
    <a:gd name="xHR" fmla="+- r 0 hl"/>
    <a:gd name="xBL" fmla="max hl xBoxL0"/>
    <a:gd name="xBR" fmla="min xHR xBoxR0"/>
    <a:gd name="yWingT" fmla="+- vc 0 hw"/>
    <a:gd name="yWingB" fmla="+- vc hw 0"/>
    <a:gd name="yShT" fmla="+- vc 0 thH"/>
    <a:gd name="yShB" fmla="+- vc thH 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
    <a:lnTo><a:pt x="hl" y="yWingT"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xBL" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xBL" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xBR" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xBR" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yWingT"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="vc"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yWingB"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="xBR" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="xBR" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xBL" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xBL" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yWingB"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `upDownArrowCallout`.
///
/// Measured: four adjusts, defaults 0.25, 0.25, 0.25 and 0.48123. A box centred in the shape with an up and a down arrow;
/// eighteen points. `adj1` shaft thickness, `adj2` head half width, `adj3` head length are fractions of the short side
/// (36 pt at 144); `adj4` is the height of the box as a fraction of the height (69.3 pt of 144, 138.6 of 288). Clamps seen
/// on 216x144: `adj2` 0..`50000 * w / ss` (108 pt, half the width); `adj1` 0..`2 * adj2`; `adj3` 0..`50000 * h / ss`
/// (heads of half the height meet at the middle, 72 pt); `adj4` 0..100000, and the box edges never pass the head bases.
fn up_down_arrow_callout() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/><a:gd name="adj3" fmla="val 25000"/><a:gd name="adj4" fmla="val 48123"/></a:avLst>
  <a:gdLst>
    <a:gd name="max2" fmla="*/ 50000 w ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 max2"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="max3" fmla="*/ 50000 h ss"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="a4" fmla="pin 0 adj4 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="halfBox" fmla="*/ h a4 200000"/>
    <a:gd name="yBoxT0" fmla="+- vc 0 halfBox"/>
    <a:gd name="yBoxB0" fmla="+- vc halfBox 0"/>
    <a:gd name="yHB" fmla="+- b 0 hl"/>
    <a:gd name="yBT" fmla="max hl yBoxT0"/>
    <a:gd name="yBB" fmla="min yHB yBoxB0"/>
    <a:gd name="xWingL" fmla="+- hc 0 hw"/>
    <a:gd name="xWingR" fmla="+- hc hw 0"/>
    <a:gd name="xShL" fmla="+- hc 0 thH"/>
    <a:gd name="xShR" fmla="+- hc thH 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="yBT"/></a:moveTo>
    <a:lnTo><a:pt x="xShL" y="yBT"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xWingL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="hc" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xWingR" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yBT"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yBT"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yBB"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yBB"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="xWingR" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="hc" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xWingL" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yBB"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="yBB"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `quadArrowCallout`.
///
/// Measured: four adjusts, defaults 0.18515, 0.18515, 0.18515 and 0.48123. A box centred in the shape with an arrow
/// on each side; thirty-two points. `adj1` shaft thickness, `adj2` head half width, `adj3` head length are fractions of
/// the short side (26.66 pt at 144); `adj4` is the size of the box as a fraction of the width and, separately, of the
/// height (103.94 x 69.3 pt on 216x144). Clamps seen on 216x144: `adj2` 0..50000; `adj1` 0..`2 * adj2`; `adj3`
/// 0..`50000 - adj2` (30000 on all four gives 20000, 28.8 pt); `adj4` 0..100000 with the box edges never passing the head
/// bases, and at least `adj1` (with `adj4` 0 the box is 40 x 26.66 pt, which is `adj1` of the width and of the height,
/// the shaft thickness in the vertical direction). Whether the lower limit is `adj1` or `adj1 * ss / min(w, h)` cannot be
/// separated at the probe sizes (it is the same there).
fn quad_arrow_callout() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 18515"/><a:gd name="adj2" fmla="val 18515"/><a:gd name="adj3" fmla="val 18515"/><a:gd name="adj4" fmla="val 48123"/></a:avLst>
  <a:gdLst>
    <a:gd name="a2" fmla="pin 0 adj2 50000"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="max3" fmla="+- 50000 0 a2"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="a4" fmla="pin a1 adj4 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="halfBoxW" fmla="*/ w a4 200000"/>
    <a:gd name="halfBoxH" fmla="*/ h a4 200000"/>
    <a:gd name="xHR" fmla="+- r 0 hl"/>
    <a:gd name="yHB" fmla="+- b 0 hl"/>
    <a:gd name="xBoxL0" fmla="+- hc 0 halfBoxW"/>
    <a:gd name="xBL" fmla="max hl xBoxL0"/>
    <a:gd name="xBoxR0" fmla="+- hc halfBoxW 0"/>
    <a:gd name="xBR" fmla="min xHR xBoxR0"/>
    <a:gd name="yBoxT0" fmla="+- vc 0 halfBoxH"/>
    <a:gd name="yBoxB0" fmla="+- vc halfBoxH 0"/>
    <a:gd name="yBT" fmla="max hl yBoxT0"/>
    <a:gd name="yBB" fmla="min yHB yBoxB0"/>
    <a:gd name="yWingT" fmla="+- vc 0 hw"/>
    <a:gd name="yWingB" fmla="+- vc hw 0"/>
    <a:gd name="yShT" fmla="+- vc 0 thH"/>
    <a:gd name="yShB" fmla="+- vc thH 0"/>
    <a:gd name="xWingL" fmla="+- hc 0 hw"/>
    <a:gd name="xWingR" fmla="+- hc hw 0"/>
    <a:gd name="xShL" fmla="+- hc 0 thH"/>
    <a:gd name="xShR" fmla="+- hc thH 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
    <a:lnTo><a:pt x="hl" y="yWingT"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xBL" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xBL" y="yBT"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yBT"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xWingL" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="hc" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xWingR" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yBT"/></a:lnTo>
    <a:lnTo><a:pt x="xBR" y="yBT"/></a:lnTo>
    <a:lnTo><a:pt x="xBR" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yShT"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yWingT"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="vc"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yWingB"/></a:lnTo>
    <a:lnTo><a:pt x="xHR" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="xBR" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="xBR" y="yBB"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yBB"/></a:lnTo>
    <a:lnTo><a:pt x="xShR" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="xWingR" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="hc" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xWingL" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yHB"/></a:lnTo>
    <a:lnTo><a:pt x="xShL" y="yBB"/></a:lnTo>
    <a:lnTo><a:pt x="xBL" y="yBB"/></a:lnTo>
    <a:lnTo><a:pt x="xBL" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yShB"/></a:lnTo>
    <a:lnTo><a:pt x="hl" y="yWingB"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `bentArrow`.
///
/// Measured: four adjusts, defaults 0.25, 0.25, 0.25 and 0.4375. A vertical shaft along the left edge bends through a
/// quarter circle into a horizontal shaft that ends in a head pointing right; the outline has two straight runs, two
/// head runs and two quarter-ellipse arcs (the outer one cubic from (0, 81) to (63, 18) on 216x144, the inner one of
/// radius `outer - thickness`). `adj1` shaft thickness (36 pt at 144), `adj2` head half width (the horizontal centre
/// line sits at that distance below the top, 36 pt), `adj3` head length (36 pt), `adj4` outer bend radius (63 pt); all
/// are fractions of the short side. Clamps seen on 216x144: `adj2` 0..50000; `adj1` 0..`2 * adj2` (with `adj2` 0 the
/// shaft vanishes); `adj4` 0..`(h - top edge) / ss` (100000 gives a radius of 144 when the shaft reaches the top edge);
/// `adj3` 0..`(w - radius) / ss` (100000 gives a 72 pt head when the radius is 144). The inner radius is
/// `max(0, outer - thickness)` and the inner corner stays at the shaft thickness (with `adj4` 0 both corners are square).
fn bent_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/><a:gd name="adj3" fmla="val 25000"/><a:gd name="adj4" fmla="val 43750"/></a:avLst>
  <a:gdLst>
    <a:gd name="a2" fmla="pin 0 adj2 50000"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="y0" fmla="+- hw 0 thH"/>
    <a:gd name="y1" fmla="+- hw thH 0"/>
    <a:gd name="hMinusY0" fmla="+- h 0 y0"/>
    <a:gd name="max4" fmla="*/ hMinusY0 100000 ss"/>
    <a:gd name="a4" fmla="pin 0 adj4 max4"/>
    <a:gd name="rad" fmla="*/ ss a4 100000"/>
    <a:gd name="wMinusRad" fmla="+- w 0 rad"/>
    <a:gd name="max3" fmla="*/ wMinusRad 100000 ss"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="xHead" fmla="+- r 0 hl"/>
    <a:gd name="hw2" fmla="*/ hw 2 1"/>
    <a:gd name="yArcStart" fmla="+- y0 rad 0"/>
    <a:gd name="radIn0" fmla="+- rad 0 th"/>
    <a:gd name="radIn" fmla="max 0 radIn0"/>
    <a:gd name="xInArc" fmla="+- th radIn 0"/>
    <a:gd name="yInArc" fmla="+- y1 radIn 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo>
    <a:lnTo><a:pt x="l" y="yArcStart"/></a:lnTo>
    <a:arcTo wR="rad" hR="rad" stAng="cd2" swAng="cd4"/>
    <a:lnTo><a:pt x="xHead" y="y0"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="hw"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="hw2"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="y1"/></a:lnTo>
    <a:lnTo><a:pt x="xInArc" y="y1"/></a:lnTo>
    <a:arcTo wR="radIn" hR="radIn" stAng="3cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="th" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `uturnArrow`.
///
/// Measured: five adjusts, defaults 0.25, 0.25, 0.25, 0.4375 and 0.75. An arrow that rises along the left edge, turns over
/// the top through two quarter-ellipse arcs and comes down the right side into a head pointing down. `adj1` shaft
/// thickness (36 pt at 144), `adj2` head half width (the right shaft centre line is that far left of the right edge),
/// `adj3` head length, `adj4` outer bend radius (63 pt), all fractions of the short side; `adj5` is the tip height as a
/// fraction of the height (108 of 144 pt, 72 of 96, 216 of 288). Clamps seen on 216x144: `adj2` 0..25000 (a head half width
/// of a quarter of the short side whatever the box; 30000 and 100000 both give 36 pt); `adj1` 0..`2 * adj2`; `adj5`
/// 0..100000; `adj4` 0..the smaller of 50000 and half the outer right edge (at the default of 0.4375 on 144x144 and
/// 96x288 the two arcs just meet, so the default is that limit); `adj3` 0..`(h - radius) / ss`; the head base is never
/// above the radius (`max(radius, tip - head length)`), the tip then sits one head length below it, and the radius is
/// limited to `tip height + head length` (with `adj5` 0 the radius drops from 63 to 36 pt). The last two rules were fitted
/// to one page each (`scan-adj5` and the 30000 page); the inner radius is `max(0, outer - thickness)`.
fn uturn_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 25000"/><a:gd name="adj3" fmla="val 25000"/><a:gd name="adj4" fmla="val 43750"/><a:gd name="adj5" fmla="val 75000"/></a:avLst>
  <a:gdLst>
    <a:gd name="a2" fmla="pin 0 adj2 25000"/>
    <a:gd name="max1" fmla="*/ a2 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="a5" fmla="pin 0 adj5 100000"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hw" fmla="*/ ss a2 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="xc" fmla="+- r 0 hw"/>
    <a:gd name="xo" fmla="+- xc thH 0"/>
    <a:gd name="xi" fmla="+- xc 0 thH"/>
    <a:gd name="xwing" fmla="+- xc 0 hw"/>
    <a:gd name="max4x" fmla="*/ xo 50000 ss"/>
    <a:gd name="max4" fmla="min 50000 max4x"/>
    <a:gd name="a4" fmla="pin 0 adj4 max4"/>
    <a:gd name="radP" fmla="*/ ss a4 100000"/>
    <a:gd name="hMinusRad" fmla="+- h 0 radP"/>
    <a:gd name="max3" fmla="*/ hMinusRad 100000 ss"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="tipNom" fmla="*/ h a5 100000"/>
    <a:gd name="radLim" fmla="+- tipNom hl 0"/>
    <a:gd name="rad" fmla="min radP radLim"/>
    <a:gd name="ybNom" fmla="+- tipNom 0 hl"/>
    <a:gd name="yb" fmla="max rad ybNom"/>
    <a:gd name="yTip" fmla="+- yb hl 0"/>
    <a:gd name="radIn0" fmla="+- rad 0 th"/>
    <a:gd name="radIn" fmla="max 0 radIn0"/>
    <a:gd name="xOutR" fmla="+- xo 0 rad"/>
    <a:gd name="xInTop" fmla="+- th radIn 0"/>
    <a:gd name="xInR" fmla="+- xi 0 radIn"/>
    <a:gd name="yInArc" fmla="+- th radIn 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo>
    <a:lnTo><a:pt x="l" y="rad"/></a:lnTo>
    <a:arcTo wR="rad" hR="rad" stAng="cd2" swAng="cd4"/>
    <a:lnTo><a:pt x="xOutR" y="t"/></a:lnTo>
    <a:arcTo wR="rad" hR="rad" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="xo" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="xc" y="yTip"/></a:lnTo>
    <a:lnTo><a:pt x="xwing" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="xi" y="yb"/></a:lnTo>
    <a:lnTo><a:pt x="xi" y="yInArc"/></a:lnTo>
    <a:arcTo wR="radIn" hR="radIn" stAng="0" swAng="-5400000"/>
    <a:lnTo><a:pt x="xInTop" y="th"/></a:lnTo>
    <a:arcTo wR="radIn" hR="radIn" stAng="3cd4" swAng="-5400000"/>
    <a:lnTo><a:pt x="th" y="b"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `curvedRightArrow`.
///
/// Measured: three adjusts, defaults 0.25, 0.5 and 0.25. A curved band whose tail is the left edge and whose head points
/// right at the bottom right. Every curve is an arc of one of two ellipses of equal size, `E_A` and `E_B`, centred on the
/// right edge (rx = w, ry = (h - head half width - band half thickness) / 2, so 45 pt on 144x144 and 216x144, 126 pt on
/// 96x288); `E_B` is `E_A` moved down by the band thickness. One open outline: `E_A` from its left point (0, ry) down to
/// the head base, the head (wing, tip at (r, h - head half width), wing), `E_B` back to (0, ry + thickness), then `E_A`
/// from (0, ry) over its top to (r, 0), the vertical edge to (r, thickness), and `E_B` from its top to where it meets
/// `E_A` (at y = ry + thickness / 2). `adj1` band thickness, `adj2` head width (the wings stand out by half of
/// `head width - thickness` on each side) and `adj3` head length, all fractions of the short side (36, 72 and 36 pt at
/// 144). The arcs end at visual angles worked out from the head base (`at2`), so the same guides give the odd outlines
/// Word draws for out-of-range input. Clamps seen on 216x144: `adj2` 0..`50000 * h / ss`; `adj1` 0..`adj2`; `adj3`
/// 0..`100000 - adj1 - adj2` (all adjusts 100000 give thickness 72, width 72, length 0). In the curved arrows Word's
/// `sqrt` of a negative number behaves as `sqrt(abs(x))` (seen on `curvedDownArrow`), so `abs` is applied first.
fn curved_right_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 50000"/><a:gd name="adj3" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="max2" fmla="*/ 50000 h ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 max2"/>
    <a:gd name="a1" fmla="pin 0 adj1 a2"/>
    <a:gd name="t13" fmla="+- 100000 0 a1"/>
    <a:gd name="max3" fmla="+- t13 0 a2"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hwTot" fmla="*/ ss a2 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="hwH" fmla="*/ hwTot 1 2"/>
    <a:gd name="hmh" fmla="+- h 0 hwH"/>
    <a:gd name="hmt" fmla="+- hmh 0 thH"/>
    <a:gd name="ry" fmla="*/ hmt 1 2"/>
    <a:gd name="wsq" fmla="*/ w w 1"/>
    <a:gd name="hlsq" fmla="*/ hl hl 1"/>
    <a:gd name="dd" fmla="+- wsq 0 hlsq"/>
    <a:gd name="ddA" fmla="abs dd"/>
    <a:gd name="sqh" fmla="sqrt ddA"/>
    <a:gd name="dyH" fmla="*/ ry sqh w"/>
    <a:gd name="negHl" fmla="+- 0 0 hl"/>
    <a:gd name="angH" fmla="at2 negHl dyH"/>
    <a:gd name="yAb" fmla="+- ry dyH 0"/>
    <a:gd name="yBb" fmla="+- yAb th 0"/>
    <a:gd name="wd" fmla="+- hwTot 0 th"/>
    <a:gd name="wingOff" fmla="*/ wd 1 2"/>
    <a:gd name="wingTop" fmla="+- yAb 0 wingOff"/>
    <a:gd name="wingBot" fmla="+- yBb wingOff 0"/>
    <a:gd name="tipY" fmla="+- h 0 hwH"/>
    <a:gd name="rysq" fmla="*/ ry ry 1"/>
    <a:gd name="thsq" fmla="*/ thH thH 1"/>
    <a:gd name="d2" fmla="+- rysq 0 thsq"/>
    <a:gd name="d2A" fmla="abs d2"/>
    <a:gd name="sq2" fmla="sqrt d2A"/>
    <a:gd name="dxI0" fmla="*/ w sq2 ry"/>
    <a:gd name="negThH" fmla="+- 0 0 thH"/>
    <a:gd name="dxIn" fmla="+- 0 0 dxI0"/>
    <a:gd name="angI" fmla="at2 dxIn negThH"/>
    <a:gd name="xh" fmla="+- r 0 hl"/>
    <a:gd name="swA" fmla="+- angH 0 cd2"/>
    <a:gd name="swB" fmla="+- cd2 0 angH"/>
    <a:gd name="swI" fmla="+- angI 0 3cd4"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="ry"/></a:moveTo>
    <a:arcTo wR="w" hR="ry" stAng="cd2" swAng="swA"/>
    <a:lnTo><a:pt x="xh" y="wingTop"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="tipY"/></a:lnTo>
    <a:lnTo><a:pt x="xh" y="wingBot"/></a:lnTo>
    <a:lnTo><a:pt x="xh" y="yBb"/></a:lnTo>
    <a:arcTo wR="w" hR="ry" stAng="angH" swAng="swB"/>
    <a:lnTo><a:pt x="l" y="ry"/></a:lnTo>
    <a:arcTo wR="w" hR="ry" stAng="cd2" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="th"/></a:lnTo>
    <a:arcTo wR="w" hR="ry" stAng="3cd4" swAng="swI"/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `curvedLeftArrow`.
///
/// Measured: three adjusts, defaults 0.25, 0.5 and 0.25. The mirror image (left to right) of `curvedRightArrow` on every
/// page seen: tail on the right edge, head pointing left at the bottom left, ellipses centred on the left edge. Same
/// adjusts, units and clamps as `curvedRightArrow`.
fn curved_left_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 50000"/><a:gd name="adj3" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="max2" fmla="*/ 50000 h ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 max2"/>
    <a:gd name="a1" fmla="pin 0 adj1 a2"/>
    <a:gd name="t13" fmla="+- 100000 0 a1"/>
    <a:gd name="max3" fmla="+- t13 0 a2"/>
    <a:gd name="a3" fmla="pin 0 adj3 max3"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hwTot" fmla="*/ ss a2 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="hwH" fmla="*/ hwTot 1 2"/>
    <a:gd name="hmh" fmla="+- h 0 hwH"/>
    <a:gd name="hmt" fmla="+- hmh 0 thH"/>
    <a:gd name="ry" fmla="*/ hmt 1 2"/>
    <a:gd name="wsq" fmla="*/ w w 1"/>
    <a:gd name="hlsq" fmla="*/ hl hl 1"/>
    <a:gd name="dd" fmla="+- wsq 0 hlsq"/>
    <a:gd name="ddA" fmla="abs dd"/>
    <a:gd name="sqh" fmla="sqrt ddA"/>
    <a:gd name="dyH" fmla="*/ ry sqh w"/>
    <a:gd name="negHl" fmla="+- 0 0 hl"/>
    <a:gd name="angH" fmla="at2 negHl dyH"/>
    <a:gd name="yAb" fmla="+- ry dyH 0"/>
    <a:gd name="yBb" fmla="+- yAb th 0"/>
    <a:gd name="wd" fmla="+- hwTot 0 th"/>
    <a:gd name="wingOff" fmla="*/ wd 1 2"/>
    <a:gd name="wingTop" fmla="+- yAb 0 wingOff"/>
    <a:gd name="wingBot" fmla="+- yBb wingOff 0"/>
    <a:gd name="tipY" fmla="+- h 0 hwH"/>
    <a:gd name="rysq" fmla="*/ ry ry 1"/>
    <a:gd name="thsq" fmla="*/ thH thH 1"/>
    <a:gd name="d2" fmla="+- rysq 0 thsq"/>
    <a:gd name="d2A" fmla="abs d2"/>
    <a:gd name="sq2" fmla="sqrt d2A"/>
    <a:gd name="dxI0" fmla="*/ w sq2 ry"/>
    <a:gd name="negThH" fmla="+- 0 0 thH"/>
    <a:gd name="dxIn" fmla="+- 0 0 dxI0"/>
    <a:gd name="angI" fmla="at2 dxIn negThH"/>
    <a:gd name="xh" fmla="+- l hl 0"/>
    <a:gd name="angHm" fmla="+- cd2 0 angH"/>
    <a:gd name="swA" fmla="+- cd2 0 angH"/>
    <a:gd name="angIA" fmla="at2 dxI0 thH"/>
    <a:gd name="swE" fmla="+- angIA 0 angHm"/>
    <a:gd name="yB0" fmla="+- ry th 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="r" y="yB0"/></a:moveTo>
    <a:arcTo wR="w" hR="ry" stAng="0" swAng="-5400000"/>
    <a:lnTo><a:pt x="l" y="t"/></a:lnTo>
    <a:arcTo wR="w" hR="ry" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="yB0"/></a:lnTo>
    <a:arcTo wR="w" hR="ry" stAng="0" swAng="swA"/>
    <a:lnTo><a:pt x="xh" y="wingBot"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="tipY"/></a:lnTo>
    <a:lnTo><a:pt x="xh" y="wingTop"/></a:lnTo>
    <a:lnTo><a:pt x="xh" y="yAb"/></a:lnTo>
    <a:arcTo wR="w" hR="ry" stAng="angHm" swAng="swE"/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `curvedDownArrow`.
///
/// Measured: three adjusts, defaults 0.25, 0.5 and 0.25. A curved band that rises from the bottom left edge over the top
/// and comes down on the right to a head pointing down at the bottom right. Like `curvedRightArrow` the curves are arcs
/// of two equal ellipses, `E_A` and `E_B`, here centred on the bottom edge with ry = h and rx = (w - head half width -
/// band half thickness) / 2 (81 pt on 216x144), `E_B` moved right by the band thickness. `adj1` band thickness, `adj2`
/// head width, `adj3` head length, all fractions of the short side. The outline starts at the crossing of the two
/// ellipses, runs along `E_B` to the bottom edge, along `E_A` over the top, then along `E_B` to the head base, round the
/// head and back along `E_A`. Clamps seen on 216x144: `adj2` 0..`50000 * w / ss` (75000: half the width); `adj1`
/// 0..100000; `adj3` has no limit at all (400000 gives a head base 576 pt above the box, -50000 one 72 pt below it; `adj1` and `adj2` below 0 act as 0). The arcs are worked out from
/// visual angles (`at2`) and `sqrt(abs(x))` as in `curvedRightArrow`, which reproduces the odd outlines Word draws for
/// out-of-range input.
fn curved_down_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 50000"/><a:gd name="adj3" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="max2" fmla="*/ 50000 w ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 max2"/>
    <a:gd name="a1" fmla="pin 0 adj1 100000"/>
    <a:gd name="a3" fmla="+- adj3 0 0"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hwTot" fmla="*/ ss a2 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="hwH" fmla="*/ hwTot 1 2"/>
    <a:gd name="wmh" fmla="+- w 0 hwH"/>
    <a:gd name="wmt" fmla="+- wmh 0 thH"/>
    <a:gd name="rx" fmla="*/ wmt 1 2"/>
    <a:gd name="rxsq" fmla="*/ rx rx 1"/>
    <a:gd name="thsq" fmla="*/ thH thH 1"/>
    <a:gd name="d2" fmla="+- rxsq 0 thsq"/>
    <a:gd name="d2A" fmla="abs d2"/>
    <a:gd name="sq2" fmla="sqrt d2A"/>
    <a:gd name="dyI0" fmla="*/ h sq2 rx"/>
    <a:gd name="xI" fmla="+- rx thH 0"/>
    <a:gd name="hsq" fmla="*/ h h 1"/>
    <a:gd name="hlsq" fmla="*/ hl hl 1"/>
    <a:gd name="dd" fmla="+- hsq 0 hlsq"/>
    <a:gd name="ddA" fmla="abs dd"/>
    <a:gd name="sqh" fmla="sqrt ddA"/>
    <a:gd name="dxB" fmla="*/ rx sqh h"/>
    <a:gd name="xT" fmla="+- rx th 0"/>
    <a:gd name="xBo" fmla="+- xT dxB 0"/>
    <a:gd name="xAi" fmla="+- rx dxB 0"/>
    <a:gd name="wd" fmla="+- hwTot 0 th"/>
    <a:gd name="wingOff" fmla="*/ wd 1 2"/>
    <a:gd name="xWo" fmla="+- xBo wingOff 0"/>
    <a:gd name="xWi" fmla="+- xAi 0 wingOff"/>
    <a:gd name="tipX" fmla="+- w 0 hwH"/>
    <a:gd name="negThH" fmla="+- 0 0 thH"/>
    <a:gd name="negHl" fmla="+- 0 0 hl"/>
    <a:gd name="dyIn" fmla="+- 0 0 dyI0"/>
    <a:gd name="angI" fmla="at2 negThH dyIn"/>
    <a:gd name="swI" fmla="+- cd2 0 angI"/>
    <a:gd name="angHB0" fmla="at2 dxB negHl"/>
    <a:gd name="negD" fmla="+- 3cd4 0 angHB0"/>
    <a:gd name="angHB1" fmla="+- angHB0 21600000 0"/>
    <a:gd name="angHB" fmla="?: negD angHB1 angHB0"/>
    <a:gd name="swH" fmla="+- angHB 0 3cd4"/>
    <a:gd name="swE" fmla="+- 3cd4 0 angHB"/>
    <a:gd name="yH" fmla="+- h 0 hl"/>
    <a:gd name="yI" fmla="+- h 0 dyI0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="xI" y="yI"/></a:moveTo>
    <a:arcTo wR="rx" hR="h" stAng="angI" swAng="swI"/>
    <a:lnTo><a:pt x="l" y="b"/></a:lnTo>
    <a:arcTo wR="rx" hR="h" stAng="cd2" swAng="cd4"/>
    <a:lnTo><a:pt x="xT" y="t"/></a:lnTo>
    <a:arcTo wR="rx" hR="h" stAng="3cd4" swAng="swH"/>
    <a:lnTo><a:pt x="xWo" y="yH"/></a:lnTo>
    <a:lnTo><a:pt x="tipX" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xWi" y="yH"/></a:lnTo>
    <a:lnTo><a:pt x="xAi" y="yH"/></a:lnTo>
    <a:arcTo wR="rx" hR="h" stAng="angHB" swAng="swE"/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `curvedUpArrow`.
///
/// Measured: three adjusts, defaults 0.25, 0.5 and 0.25. The vertical flip of `curvedDownArrow` in the ellipses and the
/// geometry (tail on the top left edge, head pointing up at the top right, ellipses centred on the top edge), but a
/// different set of stroked pieces: from the crossing of the ellipses along `E_A` to the head base, round the head, along
/// `E_B` to the bottom edge, then `E_A` and `E_B` each as a full quarter from the bottom edge to the left edge. Same
/// adjusts and clamps as `curvedDownArrow`.
fn curved_up_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 50000"/><a:gd name="adj3" fmla="val 25000"/></a:avLst>
  <a:gdLst>
    <a:gd name="max2" fmla="*/ 50000 w ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 max2"/>
    <a:gd name="a1" fmla="pin 0 adj1 100000"/>
    <a:gd name="a3" fmla="+- adj3 0 0"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="hwTot" fmla="*/ ss a2 100000"/>
    <a:gd name="hl" fmla="*/ ss a3 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="hwH" fmla="*/ hwTot 1 2"/>
    <a:gd name="wmh" fmla="+- w 0 hwH"/>
    <a:gd name="wmt" fmla="+- wmh 0 thH"/>
    <a:gd name="rx" fmla="*/ wmt 1 2"/>
    <a:gd name="rxsq" fmla="*/ rx rx 1"/>
    <a:gd name="thsq" fmla="*/ thH thH 1"/>
    <a:gd name="d2" fmla="+- rxsq 0 thsq"/>
    <a:gd name="d2A" fmla="abs d2"/>
    <a:gd name="sq2" fmla="sqrt d2A"/>
    <a:gd name="dyI0" fmla="*/ h sq2 rx"/>
    <a:gd name="xI" fmla="+- rx thH 0"/>
    <a:gd name="hsq" fmla="*/ h h 1"/>
    <a:gd name="hlsq" fmla="*/ hl hl 1"/>
    <a:gd name="dd" fmla="+- hsq 0 hlsq"/>
    <a:gd name="ddA" fmla="abs dd"/>
    <a:gd name="sqh" fmla="sqrt ddA"/>
    <a:gd name="dxB" fmla="*/ rx sqh h"/>
    <a:gd name="xT" fmla="+- rx th 0"/>
    <a:gd name="xBo" fmla="+- xT dxB 0"/>
    <a:gd name="xAi" fmla="+- rx dxB 0"/>
    <a:gd name="wd" fmla="+- hwTot 0 th"/>
    <a:gd name="wingOff" fmla="*/ wd 1 2"/>
    <a:gd name="xWo" fmla="+- xBo wingOff 0"/>
    <a:gd name="xWi" fmla="+- xAi 0 wingOff"/>
    <a:gd name="tipX" fmla="+- w 0 hwH"/>
    <a:gd name="negThH" fmla="+- 0 0 thH"/>
    <a:gd name="negHl" fmla="+- 0 0 hl"/>
    <a:gd name="angIu" fmla="at2 thH dyI0"/>
    <a:gd name="angHu0" fmla="at2 dxB hl"/>
    <a:gd name="dU" fmla="+- angHu0 0 angIu"/>
    <a:gd name="dUm" fmla="+- dU 0 cd2"/>
    <a:gd name="dUn" fmla="+- dU 0 21600000"/>
    <a:gd name="swU" fmla="?: dUm dUn dU"/>
    <a:gd name="angHu" fmla="+- angIu swU 0"/>
    <a:gd name="swD" fmla="+- cd4 0 angHu"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="xI" y="dyI0"/></a:moveTo>
    <a:arcTo wR="rx" hR="h" stAng="angIu" swAng="swU"/>
    <a:lnTo><a:pt x="xWi" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="tipX" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="xWo" y="hl"/></a:lnTo>
    <a:lnTo><a:pt x="xBo" y="hl"/></a:lnTo>
    <a:arcTo wR="rx" hR="h" stAng="angHu" swAng="swD"/>
    <a:lnTo><a:pt x="rx" y="b"/></a:lnTo>
    <a:arcTo wR="rx" hR="h" stAng="cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="th" y="t"/></a:lnTo>
    <a:arcTo wR="rx" hR="h" stAng="cd2" swAng="-5400000"/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `swooshArrow`.
///
/// Measured: two adjusts, defaults 0.25 and 0.16667. A curved swoosh from the bottom left corner to a small head at the
/// right edge: one cubic along the upper edge, the head (a slanted base line through two wing points, the tip on the
/// right edge), one cubic back along the lower edge; eight points, closed. `adj1` sets how far the head hangs below the
/// upper edge: the inner head point is `adj1 * h` below the end of the upper curve (36 pt of 144) and the tip is at
/// `0.075 * h + ss * adj1 / 2` (28.8 pt); `adj2` is the distance of the head base from the right edge as a fraction of the
/// short side (24 pt of 144). The end of the upper curve is `ss / 8` below the top (18 pt); the head wings lie on a line
/// that leans 0.1127 to the right per unit of height (6.43 degrees) and run from the top edge down to `ss / 8` below the
/// inner head point. The control points are fixed fractions of the box (upper curve: (w/9, 5h/9), (xB/3 + w/9,
/// 2h/9 + ss/24); lower curve: (xI/3 + w/6, yI + h/18), (w/6, 7h/18 + ss/12 + 2h*adj1/3), where xB is the end of the
/// upper curve and (xI, yI) the inner head point); they were fitted to the 11 pages (the four sizes included), the lower
/// curve's constants separately for `h` and `ss` by the 144x144 and 96x288 pages. Clamps seen on 216x144: `adj1`
/// 0..75000 (100000 gives 75000); `adj2` 0..`70000 * w / ss` (100000 stays, 400000 gives 105000, xB = 0.3 w). The
/// exact form of these two limits and of the lean (0.1127, fitted to within 0.0003) is a fit, not a derivation.
fn swoosh_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 16667"/></a:avLst>
  <a:gdLst>
    <a:gd name="a1" fmla="pin 0 adj1 75000"/>
    <a:gd name="max2" fmla="*/ 70000 w ss"/>
    <a:gd name="a2" fmla="pin 0 adj2 max2"/>
    <a:gd name="sa2" fmla="*/ ss a2 100000"/>
    <a:gd name="xB" fmla="+- r 0 sa2"/>
    <a:gd name="yB" fmla="val ssd8"/>
    <a:gd name="lean" fmla="*/ yB 11270 100000"/>
    <a:gd name="xU" fmla="+- xB 0 lean"/>
    <a:gd name="hA1" fmla="*/ h a1 100000"/>
    <a:gd name="yI" fmla="+- yB hA1 0"/>
    <a:gd name="leanI" fmla="*/ hA1 11270 100000"/>
    <a:gd name="xI" fmla="+- xB leanI 0"/>
    <a:gd name="yL2" fmla="+- yI yB 0"/>
    <a:gd name="hy" fmla="+- hA1 yB 0"/>
    <a:gd name="leanL2" fmla="*/ hy 11270 100000"/>
    <a:gd name="xL2" fmla="+- xB leanL2 0"/>
    <a:gd name="ty1" fmla="*/ h 7500 100000"/>
    <a:gd name="ty2" fmla="*/ ss a1 200000"/>
    <a:gd name="ty" fmla="+- ty1 ty2 0"/>
    <a:gd name="wd9" fmla="*/ w 1 9"/>
    <a:gd name="hd9" fmla="*/ h 5 9"/>
    <a:gd name="xB3" fmla="*/ xB 1 3"/>
    <a:gd name="xC2" fmla="+- xB3 wd9 0"/>
    <a:gd name="yC2a" fmla="*/ h 2 9"/>
    <a:gd name="yC2b" fmla="*/ ss 1 24"/>
    <a:gd name="yC2" fmla="+- yC2a yC2b 0"/>
    <a:gd name="wd6" fmla="*/ w 1 6"/>
    <a:gd name="xI3" fmla="*/ xI 1 3"/>
    <a:gd name="xC3" fmla="+- xI3 wd6 0"/>
    <a:gd name="hd18" fmla="*/ h 1 18"/>
    <a:gd name="yC3" fmla="+- yI hd18 0"/>
    <a:gd name="yC4a" fmla="*/ h 7 18"/>
    <a:gd name="yC4b" fmla="*/ ss 1 12"/>
    <a:gd name="yC4c" fmla="*/ h a1 150000"/>
    <a:gd name="yC4d" fmla="+- yC4a yC4b 0"/>
    <a:gd name="yC4" fmla="+- yC4d yC4c 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo>
    <a:cubicBezTo><a:pt x="wd9" y="hd9"/><a:pt x="xC2" y="yC2"/><a:pt x="xB" y="yB"/></a:cubicBezTo>
    <a:lnTo><a:pt x="xU" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="ty"/></a:lnTo>
    <a:lnTo><a:pt x="xL2" y="yL2"/></a:lnTo>
    <a:lnTo><a:pt x="xI" y="yI"/></a:lnTo>
    <a:cubicBezTo><a:pt x="xC3" y="yC3"/><a:pt x="wd6" y="yC4"/><a:pt x="l" y="b"/></a:cubicBezTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `circularArrow`.
///
/// Measured: five adjusts, defaults 0.125 (band thickness), 19.04 degrees (head angle), -19.04 degrees (end angle; Word
/// reports it as negative, the file default is 20457681), 180 degrees (start angle) and 0.125 (head width). A band along
/// the box ellipse, clockwise from the start angle, ending in an arrow head. The middle line of the band is the
/// ellipse `(w/2 - head, h/2 - head)` with head = `ss * adj5`; the band lies `ss * adj1 / 2` outside and inside it
/// (outer 99 x 63 pt, inner 81 x 45 pt on 216x144). The head's base line passes through the middle-line point at the end
/// angle, in the direction of the tip angle (`end + head angle`); its wings are `max(head, distance to the band edge)` from
/// that point on either side; the tip is the middle-line point at the tip angle; the band's arcs end where the base
/// line crosses the outer and the inner ellipse (found with a quadratic that has no division by the radii, which also
/// copes with a collapsed inner ellipse). All angles are visual angles of the ellipse, clamped to 0..21599999 (negative
/// values act as 0, so the probes that set the reported -19.04 degrees explicitly behave as end angle 0); an arc whose end
/// is before its start sweeps the rest of the circle. Clamps: `adj5` 0..25000, `adj1` 0..`2 * adj5`, and the head angle
/// at most `acos(thickness / (2 * head width))` (this makes the 100000 and 400000 pages come out as head angle 0, tip at
/// the base).
/// Not reproduced: the page with all adjusts 100000 (0.79 pt off, in the flat inner arc of the collapsed inner ellipse)
/// and the page with all adjusts 400000 (the same inner arc missing). Word's collapsed-ellipse arithmetic there depends
/// on exact float noise (its inner end lands on the centre); every other page, 12 of 14, is within 0.05 pt.
fn circular_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 12500"/><a:gd name="adj2" fmla="val 1142319"/><a:gd name="adj3" fmla="val 20457681"/><a:gd name="adj4" fmla="val 10800000"/><a:gd name="adj5" fmla="val 12500"/></a:avLst>
  <a:gdLst>
    <a:gd name="a5" fmla="pin 0 adj5 25000"/>
    <a:gd name="max1" fmla="*/ a5 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="ha" fmla="*/ ss a5 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="hw2" fmla="*/ ha 2 1"/>
    <a:gd name="negHa" fmla="+- 0 0 ha"/>
    <a:gd name="rxM" fmla="+- wd2 0 ha"/>
    <a:gd name="ryM" fmla="+- hd2 0 ha"/>
    <a:gd name="rxO" fmla="+- rxM thH 0"/>
    <a:gd name="ryO" fmla="+- ryM thH 0"/>
    <a:gd name="rxI" fmla="+- rxM 0 thH"/>
    <a:gd name="ryI" fmla="+- ryM 0 thH"/>
    <a:gd name="rxIa" fmla="max rxI 0"/>
    <a:gd name="ryIa" fmla="max ryI 0"/>
    <a:gd name="rxOa" fmla="max rxO 0"/>
    <a:gd name="ryOa" fmla="max ryO 0"/>
    <a:gd name="cc" fmla="*/ th 100000 hw2"/>
    <a:gd name="cs0" fmla="*/ cc cc 1"/>
    <a:gd name="cs1" fmla="+- 10000000000 0 cs0"/>
    <a:gd name="cs" fmla="sqrt cs1"/>
    <a:gd name="hlim" fmla="at2 cc cs"/>
    <a:gd name="st" fmla="pin 0 adj4 21599999"/>
    <a:gd name="en" fmla="pin 0 adj3 21599999"/>
    <a:gd name="hd0" fmla="pin 0 adj2 21599999"/>
    <a:gd name="hd" fmla="min hd0 hlim"/>
    <a:gd name="tp" fmla="+- en hd 0"/>
    <a:gd name="ecB" fmla="cos ryM en"/>
    <a:gd name="esB" fmla="sin rxM en"/>
    <a:gd name="ebx" fmla="cat2 rxM ecB esB"/>
    <a:gd name="eby" fmla="sat2 ryM ecB esB"/>
    <a:gd name="ecT" fmla="cos ryM tp"/>
    <a:gd name="esT" fmla="sin rxM tp"/>
    <a:gd name="etx" fmla="cat2 rxM ecT esT"/>
    <a:gd name="ety" fmla="sat2 ryM ecT esT"/>
    <a:gd name="eux" fmla="cos 100000 tp"/>
    <a:gd name="euy" fmla="sin 100000 tp"/>
    <a:gd name="eux2" fmla="*/ eux eux 1"/>
    <a:gd name="euy2" fmla="*/ euy euy 1"/>
    <a:gd name="ebxux" fmla="*/ ebx eux 1"/>
    <a:gd name="ebyuy" fmla="*/ eby euy 1"/>
    <a:gd name="ebx2" fmla="*/ ebx ebx 1"/>
    <a:gd name="eby2" fmla="*/ eby eby 1"/>
    <a:gd name="eox2" fmla="*/ rxO rxO 1"/>
    <a:gd name="eoy2" fmla="*/ ryO ryO 1"/>
    <a:gd name="eoA1" fmla="*/ eux2 eoy2 1"/>
    <a:gd name="eoA2" fmla="*/ euy2 eox2 1"/>
    <a:gd name="eoA" fmla="+- eoA1 eoA2 0"/>
    <a:gd name="eoC1" fmla="*/ ebxux eoy2 1"/>
    <a:gd name="eoC2" fmla="*/ ebyuy eox2 1"/>
    <a:gd name="eoC" fmla="+- eoC1 eoC2 0"/>
    <a:gd name="eoD1" fmla="*/ ebx2 eoy2 1"/>
    <a:gd name="eoD2" fmla="*/ eby2 eox2 1"/>
    <a:gd name="eoD3" fmla="*/ eox2 eoy2 1"/>
    <a:gd name="eoD0" fmla="+- eoD1 eoD2 0"/>
    <a:gd name="eoD" fmla="+- eoD0 0 eoD3"/>
    <a:gd name="eoCC" fmla="*/ eoC eoC 1"/>
    <a:gd name="eoAD" fmla="*/ eoA eoD 1"/>
    <a:gd name="eoQ" fmla="+- eoCC 0 eoAD"/>
    <a:gd name="eoS" fmla="sqrt eoQ"/>
    <a:gd name="eoN" fmla="+- eoS 0 eoC"/>
    <a:gd name="eot" fmla="*/ eoN 100000 eoA"/>
    <a:gd name="eoex" fmla="*/ eot eux 100000"/>
    <a:gd name="eoey" fmla="*/ eot euy 100000"/>
    <a:gd name="eoX" fmla="+- ebx eoex 0"/>
    <a:gd name="eoY" fmla="+- eby eoey 0"/>
    <a:gd name="eoaX" fmla="abs eoX"/>
    <a:gd name="eoaY" fmla="abs eoY"/>
    <a:gd name="eomX" fmla="+- eoaX 0 1"/>
    <a:gd name="eomY" fmla="+- eoaY 0 1"/>
    <a:gd name="eosX" fmla="?: eomX eoX 0"/>
    <a:gd name="eosY" fmla="?: eomY eoY 0"/>
    <a:gd name="eoang" fmla="at2 eosX eosY"/>
    <a:gd name="eix2" fmla="*/ rxI rxI 1"/>
    <a:gd name="eiy2" fmla="*/ ryI ryI 1"/>
    <a:gd name="eiA1" fmla="*/ eux2 eiy2 1"/>
    <a:gd name="eiA2" fmla="*/ euy2 eix2 1"/>
    <a:gd name="eiA" fmla="+- eiA1 eiA2 0"/>
    <a:gd name="eiC1" fmla="*/ ebxux eiy2 1"/>
    <a:gd name="eiC2" fmla="*/ ebyuy eix2 1"/>
    <a:gd name="eiC" fmla="+- eiC1 eiC2 0"/>
    <a:gd name="eiD1" fmla="*/ ebx2 eiy2 1"/>
    <a:gd name="eiD2" fmla="*/ eby2 eix2 1"/>
    <a:gd name="eiD3" fmla="*/ eix2 eiy2 1"/>
    <a:gd name="eiD0" fmla="+- eiD1 eiD2 0"/>
    <a:gd name="eiD" fmla="+- eiD0 0 eiD3"/>
    <a:gd name="eiCC" fmla="*/ eiC eiC 1"/>
    <a:gd name="eiAD" fmla="*/ eiA eiD 1"/>
    <a:gd name="eiQ" fmla="+- eiCC 0 eiAD"/>
    <a:gd name="eiS" fmla="sqrt eiQ"/>
    <a:gd name="eiN" fmla="+- eiS 0 eiC"/>
    <a:gd name="eit" fmla="*/ eiN 100000 eiA"/>
    <a:gd name="eiex" fmla="*/ eit eux 100000"/>
    <a:gd name="eiey" fmla="*/ eit euy 100000"/>
    <a:gd name="eiX" fmla="+- ebx eiex 0"/>
    <a:gd name="eiY" fmla="+- eby eiey 0"/>
    <a:gd name="eiaX" fmla="abs eiX"/>
    <a:gd name="eiaY" fmla="abs eiY"/>
    <a:gd name="eimX" fmla="+- eiaX 0 1"/>
    <a:gd name="eimY" fmla="+- eiaY 0 1"/>
    <a:gd name="eisX" fmla="?: eimX eiX 0"/>
    <a:gd name="eisY" fmla="?: eimY eiY 0"/>
    <a:gd name="eiang" fmla="at2 eisX eisY"/>
    <a:gd name="ewoT" fmla="max ha eot"/>
    <a:gd name="ewiT" fmla="min negHa eit"/>
    <a:gd name="ewox" fmla="*/ ewoT eux 100000"/>
    <a:gd name="ewoy" fmla="*/ ewoT euy 100000"/>
    <a:gd name="ewix" fmla="*/ ewiT eux 100000"/>
    <a:gd name="ewiy" fmla="*/ ewiT euy 100000"/>
    <a:gd name="eWox" fmla="+- ebx ewox 0"/>
    <a:gd name="eWoy" fmla="+- eby ewoy 0"/>
    <a:gd name="eWix" fmla="+- ebx ewix 0"/>
    <a:gd name="eWiy" fmla="+- eby ewiy 0"/>
    <a:gd name="cSo" fmla="cos ryO st"/>
    <a:gd name="sSo" fmla="sin rxO st"/>
    <a:gd name="sox" fmla="cat2 rxO cSo sSo"/>
    <a:gd name="soy" fmla="sat2 ryO cSo sSo"/>
    <a:gd name="Sx" fmla="+- hc sox 0"/>
    <a:gd name="Sy" fmla="+- vc soy 0"/>
    <a:gd name="swO0" fmla="+- eoang 0 st"/>
    <a:gd name="swO1" fmla="+- swO0 21600000 0"/>
    <a:gd name="nO" fmla="+- 0 0 swO0"/>
    <a:gd name="swO" fmla="?: nO swO1 swO0"/>
    <a:gd name="swI0" fmla="+- eiang 0 st"/>
    <a:gd name="swI1" fmla="+- swI0 21600000 0"/>
    <a:gd name="nI" fmla="+- 0 0 swI0"/>
    <a:gd name="swI2" fmla="?: nI swI1 swI0"/>
    <a:gd name="swI" fmla="+- 0 0 swI2"/>
    <a:gd name="Wex" fmla="+- hc eWox 0"/>
    <a:gd name="Wey" fmla="+- vc eWoy 0"/>
    <a:gd name="Wgx" fmla="+- hc eWix 0"/>
    <a:gd name="Wgy" fmla="+- vc eWiy 0"/>
    <a:gd name="Tx" fmla="+- hc etx 0"/>
    <a:gd name="Ty" fmla="+- vc ety 0"/>
    <a:gd name="Ixe" fmla="+- hc eiX 0"/>
    <a:gd name="Iye" fmla="+- vc eiY 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="Sx" y="Sy"/></a:moveTo>
    <a:arcTo wR="rxOa" hR="ryOa" stAng="st" swAng="swO"/>
    <a:lnTo><a:pt x="Wex" y="Wey"/></a:lnTo>
    <a:lnTo><a:pt x="Tx" y="Ty"/></a:lnTo>
    <a:lnTo><a:pt x="Wgx" y="Wgy"/></a:lnTo>
    <a:lnTo><a:pt x="Ixe" y="Iye"/></a:lnTo>
    <a:arcTo wR="rxIa" hR="ryIa" stAng="eiang" swAng="swI"/>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `leftCircularArrow`.
///
/// Measured: five adjusts, defaults 0.125, -19.04 degrees (head angle), 19.04 degrees (end angle), 180 degrees (start
/// angle) and 0.125. The mirror image of `circularArrow` about the horizontal axis: the band runs counter-clockwise from the
/// start angle (down and round the bottom), ending in a head at the lower right. The geometry is that of
/// `circularArrow` with every angle negated and y flipped; the head angle (`adj2`) is clamped to -21599999..0, the end and
/// start angles to 0..21599999.
/// Not reproduced: the five pages whose adjusts are all equal (0, 30000, 100000, -50000, 400000) and the page with the
/// head width 0; Word's outline there comes from degenerate arithmetic (collapsed ellipses, zero-size heads) that
/// the guides above do not reproduce. The eight other pages (four sizes, scans of the other four adjusts) are within 0.05 pt.
fn left_circular_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 12500"/><a:gd name="adj2" fmla="val -1142319"/><a:gd name="adj3" fmla="val 1142319"/><a:gd name="adj4" fmla="val 10800000"/><a:gd name="adj5" fmla="val 12500"/></a:avLst>
  <a:gdLst>
    <a:gd name="a5" fmla="pin 0 adj5 25000"/>
    <a:gd name="max1" fmla="*/ a5 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="ha" fmla="*/ ss a5 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="hw2" fmla="*/ ha 2 1"/>
    <a:gd name="negHa" fmla="+- 0 0 ha"/>
    <a:gd name="rxM" fmla="+- wd2 0 ha"/>
    <a:gd name="ryM" fmla="+- hd2 0 ha"/>
    <a:gd name="rxO" fmla="+- rxM thH 0"/>
    <a:gd name="ryO" fmla="+- ryM thH 0"/>
    <a:gd name="rxI" fmla="+- rxM 0 thH"/>
    <a:gd name="ryI" fmla="+- ryM 0 thH"/>
    <a:gd name="rxIa" fmla="max rxI 0"/>
    <a:gd name="ryIa" fmla="max ryI 0"/>
    <a:gd name="rxOa" fmla="max rxO 0"/>
    <a:gd name="ryOa" fmla="max ryO 0"/>
    <a:gd name="cc" fmla="*/ th 100000 hw2"/>
    <a:gd name="cs0" fmla="*/ cc cc 1"/>
    <a:gd name="cs1" fmla="+- 10000000000 0 cs0"/>
    <a:gd name="cs" fmla="sqrt cs1"/>
    <a:gd name="hlim" fmla="at2 cc cs"/>
    <a:gd name="st0" fmla="pin 0 adj4 21599999"/>
    <a:gd name="en0" fmla="pin 0 adj3 21599999"/>
    <a:gd name="hd1" fmla="pin -21599999 adj2 0"/>
    <a:gd name="st1" fmla="+- 21600000 0 st0"/>
    <a:gd name="st" fmla="?: st0 st1 0"/>
    <a:gd name="en1" fmla="+- 21600000 0 en0"/>
    <a:gd name="en" fmla="?: en0 en1 0"/>
    <a:gd name="hd0" fmla="+- 0 0 hd1"/>
    <a:gd name="hd" fmla="min hd0 hlim"/>
    <a:gd name="tp" fmla="+- en hd 0"/>
    <a:gd name="ecB" fmla="cos ryM en"/>
    <a:gd name="esB" fmla="sin rxM en"/>
    <a:gd name="ebx" fmla="cat2 rxM ecB esB"/>
    <a:gd name="eby" fmla="sat2 ryM ecB esB"/>
    <a:gd name="ecT" fmla="cos ryM tp"/>
    <a:gd name="esT" fmla="sin rxM tp"/>
    <a:gd name="etx" fmla="cat2 rxM ecT esT"/>
    <a:gd name="ety" fmla="sat2 ryM ecT esT"/>
    <a:gd name="eux" fmla="cos 100000 tp"/>
    <a:gd name="euy" fmla="sin 100000 tp"/>
    <a:gd name="eux2" fmla="*/ eux eux 1"/>
    <a:gd name="euy2" fmla="*/ euy euy 1"/>
    <a:gd name="ebxux" fmla="*/ ebx eux 1"/>
    <a:gd name="ebyuy" fmla="*/ eby euy 1"/>
    <a:gd name="ebx2" fmla="*/ ebx ebx 1"/>
    <a:gd name="eby2" fmla="*/ eby eby 1"/>
    <a:gd name="eox2" fmla="*/ rxO rxO 1"/>
    <a:gd name="eoy2" fmla="*/ ryO ryO 1"/>
    <a:gd name="eoA1" fmla="*/ eux2 eoy2 1"/>
    <a:gd name="eoA2" fmla="*/ euy2 eox2 1"/>
    <a:gd name="eoA" fmla="+- eoA1 eoA2 0"/>
    <a:gd name="eoC1" fmla="*/ ebxux eoy2 1"/>
    <a:gd name="eoC2" fmla="*/ ebyuy eox2 1"/>
    <a:gd name="eoC" fmla="+- eoC1 eoC2 0"/>
    <a:gd name="eoD1" fmla="*/ ebx2 eoy2 1"/>
    <a:gd name="eoD2" fmla="*/ eby2 eox2 1"/>
    <a:gd name="eoD3" fmla="*/ eox2 eoy2 1"/>
    <a:gd name="eoD0" fmla="+- eoD1 eoD2 0"/>
    <a:gd name="eoD" fmla="+- eoD0 0 eoD3"/>
    <a:gd name="eoCC" fmla="*/ eoC eoC 1"/>
    <a:gd name="eoAD" fmla="*/ eoA eoD 1"/>
    <a:gd name="eoQ" fmla="+- eoCC 0 eoAD"/>
    <a:gd name="eoS" fmla="sqrt eoQ"/>
    <a:gd name="eoN" fmla="+- eoS 0 eoC"/>
    <a:gd name="eot" fmla="*/ eoN 100000 eoA"/>
    <a:gd name="eoex" fmla="*/ eot eux 100000"/>
    <a:gd name="eoey" fmla="*/ eot euy 100000"/>
    <a:gd name="eoX" fmla="+- ebx eoex 0"/>
    <a:gd name="eoY" fmla="+- eby eoey 0"/>
    <a:gd name="eoaX" fmla="abs eoX"/>
    <a:gd name="eoaY" fmla="abs eoY"/>
    <a:gd name="eomX" fmla="+- eoaX 0 1"/>
    <a:gd name="eomY" fmla="+- eoaY 0 1"/>
    <a:gd name="eosX" fmla="?: eomX eoX 0"/>
    <a:gd name="eosY" fmla="?: eomY eoY 0"/>
    <a:gd name="eoang" fmla="at2 eosX eosY"/>
    <a:gd name="eix2" fmla="*/ rxI rxI 1"/>
    <a:gd name="eiy2" fmla="*/ ryI ryI 1"/>
    <a:gd name="eiA1" fmla="*/ eux2 eiy2 1"/>
    <a:gd name="eiA2" fmla="*/ euy2 eix2 1"/>
    <a:gd name="eiA" fmla="+- eiA1 eiA2 0"/>
    <a:gd name="eiC1" fmla="*/ ebxux eiy2 1"/>
    <a:gd name="eiC2" fmla="*/ ebyuy eix2 1"/>
    <a:gd name="eiC" fmla="+- eiC1 eiC2 0"/>
    <a:gd name="eiD1" fmla="*/ ebx2 eiy2 1"/>
    <a:gd name="eiD2" fmla="*/ eby2 eix2 1"/>
    <a:gd name="eiD3" fmla="*/ eix2 eiy2 1"/>
    <a:gd name="eiD0" fmla="+- eiD1 eiD2 0"/>
    <a:gd name="eiD" fmla="+- eiD0 0 eiD3"/>
    <a:gd name="eiCC" fmla="*/ eiC eiC 1"/>
    <a:gd name="eiAD" fmla="*/ eiA eiD 1"/>
    <a:gd name="eiQ" fmla="+- eiCC 0 eiAD"/>
    <a:gd name="eiS" fmla="sqrt eiQ"/>
    <a:gd name="eiN" fmla="+- eiS 0 eiC"/>
    <a:gd name="eit" fmla="*/ eiN 100000 eiA"/>
    <a:gd name="eiex" fmla="*/ eit eux 100000"/>
    <a:gd name="eiey" fmla="*/ eit euy 100000"/>
    <a:gd name="eiX" fmla="+- ebx eiex 0"/>
    <a:gd name="eiY" fmla="+- eby eiey 0"/>
    <a:gd name="eiaX" fmla="abs eiX"/>
    <a:gd name="eiaY" fmla="abs eiY"/>
    <a:gd name="eimX" fmla="+- eiaX 0 1"/>
    <a:gd name="eimY" fmla="+- eiaY 0 1"/>
    <a:gd name="eisX" fmla="?: eimX eiX 0"/>
    <a:gd name="eisY" fmla="?: eimY eiY 0"/>
    <a:gd name="eiang" fmla="at2 eisX eisY"/>
    <a:gd name="ewoT" fmla="max ha eot"/>
    <a:gd name="ewiT" fmla="min negHa eit"/>
    <a:gd name="ewox" fmla="*/ ewoT eux 100000"/>
    <a:gd name="ewoy" fmla="*/ ewoT euy 100000"/>
    <a:gd name="ewix" fmla="*/ ewiT eux 100000"/>
    <a:gd name="ewiy" fmla="*/ ewiT euy 100000"/>
    <a:gd name="eWox" fmla="+- ebx ewox 0"/>
    <a:gd name="eWoy" fmla="+- eby ewoy 0"/>
    <a:gd name="eWix" fmla="+- ebx ewix 0"/>
    <a:gd name="eWiy" fmla="+- eby ewiy 0"/>
    <a:gd name="cSo" fmla="cos ryO st"/>
    <a:gd name="sSo" fmla="sin rxO st"/>
    <a:gd name="sox" fmla="cat2 rxO cSo sSo"/>
    <a:gd name="soy" fmla="sat2 ryO cSo sSo"/>
    <a:gd name="Sx" fmla="+- hc sox 0"/>
    <a:gd name="Sy" fmla="+- vc 0 soy"/>
    <a:gd name="swO0" fmla="+- eoang 0 st"/>
    <a:gd name="swO1" fmla="+- swO0 21600000 0"/>
    <a:gd name="nO" fmla="+- 0 0 swO0"/>
    <a:gd name="swO" fmla="?: nO swO1 swO0"/>
    <a:gd name="swI0" fmla="+- eiang 0 st"/>
    <a:gd name="swI1" fmla="+- swI0 21600000 0"/>
    <a:gd name="nI" fmla="+- 0 0 swI0"/>
    <a:gd name="swI2" fmla="?: nI swI1 swI0"/>
    <a:gd name="swI" fmla="+- 0 0 swI2"/>
    <a:gd name="Wex" fmla="+- hc eWox 0"/>
    <a:gd name="Wey" fmla="+- vc 0 eWoy"/>
    <a:gd name="Wgx" fmla="+- hc eWix 0"/>
    <a:gd name="Wgy" fmla="+- vc 0 eWiy"/>
    <a:gd name="Tx" fmla="+- hc etx 0"/>
    <a:gd name="Ty" fmla="+- vc 0 ety"/>
    <a:gd name="Ixe" fmla="+- hc eiX 0"/>
    <a:gd name="Iye" fmla="+- vc 0 eiY"/>
    <a:gd name="stS" fmla="+- 0 0 st"/>
    <a:gd name="swOS" fmla="+- 0 0 swO"/>
    <a:gd name="angIS" fmla="+- 0 0 eiang"/>
    <a:gd name="swIS" fmla="+- 0 0 swI"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="Sx" y="Sy"/></a:moveTo>
    <a:arcTo wR="rxOa" hR="ryOa" stAng="stS" swAng="swOS"/>
    <a:lnTo><a:pt x="Wex" y="Wey"/></a:lnTo>
    <a:lnTo><a:pt x="Tx" y="Ty"/></a:lnTo>
    <a:lnTo><a:pt x="Wgx" y="Wgy"/></a:lnTo>
    <a:lnTo><a:pt x="Ixe" y="Iye"/></a:lnTo>
    <a:arcTo wR="rxIa" hR="ryIa" stAng="angIS" swAng="swIS"/>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `leftRightCircularArrow`.
///
/// Measured: five adjusts, defaults 0.125, 19.04 degrees (head angle), -19.04 degrees (end angle; 20457681 in the file),
/// -160.96 degrees (start angle; 11942319 in the file) and 0.125. The band of `circularArrow` between the start angle and
/// the end angle with a head at each end: the right head as in `circularArrow` (base at the end angle, tip at
/// `end + head angle`), the left head its counterpart (base at the start angle, tip at `start - head angle`, pointing
/// counter-clockwise), with the same wings and band ends. The outline runs left tip, left wing, outer arc, right head,
/// inner arc, left wing. Same clamps as `circularArrow`.
/// Not reproduced: the pages with all adjusts 0, 100000, -50000 and 400000 and the page with head width 0 (degenerate
/// outlines, as for `circularArrow`); the other nine pages are within 0.05 pt.
fn left_right_circular_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj1" fmla="val 12500"/><a:gd name="adj2" fmla="val 1142319"/><a:gd name="adj3" fmla="val 20457681"/><a:gd name="adj4" fmla="val 11942319"/><a:gd name="adj5" fmla="val 12500"/></a:avLst>
  <a:gdLst>
    <a:gd name="a5" fmla="pin 0 adj5 25000"/>
    <a:gd name="max1" fmla="*/ a5 2 1"/>
    <a:gd name="a1" fmla="pin 0 adj1 max1"/>
    <a:gd name="th" fmla="*/ ss a1 100000"/>
    <a:gd name="ha" fmla="*/ ss a5 100000"/>
    <a:gd name="thH" fmla="*/ th 1 2"/>
    <a:gd name="hw2" fmla="*/ ha 2 1"/>
    <a:gd name="negHa" fmla="+- 0 0 ha"/>
    <a:gd name="rxM" fmla="+- wd2 0 ha"/>
    <a:gd name="ryM" fmla="+- hd2 0 ha"/>
    <a:gd name="rxO" fmla="+- rxM thH 0"/>
    <a:gd name="ryO" fmla="+- ryM thH 0"/>
    <a:gd name="rxI" fmla="+- rxM 0 thH"/>
    <a:gd name="ryI" fmla="+- ryM 0 thH"/>
    <a:gd name="rxIa" fmla="max rxI 0"/>
    <a:gd name="ryIa" fmla="max ryI 0"/>
    <a:gd name="rxOa" fmla="max rxO 0"/>
    <a:gd name="ryOa" fmla="max ryO 0"/>
    <a:gd name="cc" fmla="*/ th 100000 hw2"/>
    <a:gd name="cs0" fmla="*/ cc cc 1"/>
    <a:gd name="cs1" fmla="+- 10000000000 0 cs0"/>
    <a:gd name="cs" fmla="sqrt cs1"/>
    <a:gd name="hlim" fmla="at2 cc cs"/>
    <a:gd name="st" fmla="pin 0 adj4 21599999"/>
    <a:gd name="en" fmla="pin 0 adj3 21599999"/>
    <a:gd name="hd0" fmla="pin 0 adj2 21599999"/>
    <a:gd name="hd" fmla="min hd0 hlim"/>
    <a:gd name="tp" fmla="+- en hd 0"/>
    <a:gd name="tq" fmla="+- st 0 hd"/>
    <a:gd name="ecB" fmla="cos ryM en"/>
    <a:gd name="esB" fmla="sin rxM en"/>
    <a:gd name="ebx" fmla="cat2 rxM ecB esB"/>
    <a:gd name="eby" fmla="sat2 ryM ecB esB"/>
    <a:gd name="ecT" fmla="cos ryM tp"/>
    <a:gd name="esT" fmla="sin rxM tp"/>
    <a:gd name="etx" fmla="cat2 rxM ecT esT"/>
    <a:gd name="ety" fmla="sat2 ryM ecT esT"/>
    <a:gd name="eux" fmla="cos 100000 tp"/>
    <a:gd name="euy" fmla="sin 100000 tp"/>
    <a:gd name="eux2" fmla="*/ eux eux 1"/>
    <a:gd name="euy2" fmla="*/ euy euy 1"/>
    <a:gd name="ebxux" fmla="*/ ebx eux 1"/>
    <a:gd name="ebyuy" fmla="*/ eby euy 1"/>
    <a:gd name="ebx2" fmla="*/ ebx ebx 1"/>
    <a:gd name="eby2" fmla="*/ eby eby 1"/>
    <a:gd name="eox2" fmla="*/ rxO rxO 1"/>
    <a:gd name="eoy2" fmla="*/ ryO ryO 1"/>
    <a:gd name="eoA1" fmla="*/ eux2 eoy2 1"/>
    <a:gd name="eoA2" fmla="*/ euy2 eox2 1"/>
    <a:gd name="eoA" fmla="+- eoA1 eoA2 0"/>
    <a:gd name="eoC1" fmla="*/ ebxux eoy2 1"/>
    <a:gd name="eoC2" fmla="*/ ebyuy eox2 1"/>
    <a:gd name="eoC" fmla="+- eoC1 eoC2 0"/>
    <a:gd name="eoD1" fmla="*/ ebx2 eoy2 1"/>
    <a:gd name="eoD2" fmla="*/ eby2 eox2 1"/>
    <a:gd name="eoD3" fmla="*/ eox2 eoy2 1"/>
    <a:gd name="eoD0" fmla="+- eoD1 eoD2 0"/>
    <a:gd name="eoD" fmla="+- eoD0 0 eoD3"/>
    <a:gd name="eoCC" fmla="*/ eoC eoC 1"/>
    <a:gd name="eoAD" fmla="*/ eoA eoD 1"/>
    <a:gd name="eoQ" fmla="+- eoCC 0 eoAD"/>
    <a:gd name="eoS" fmla="sqrt eoQ"/>
    <a:gd name="eoN" fmla="+- eoS 0 eoC"/>
    <a:gd name="eot" fmla="*/ eoN 100000 eoA"/>
    <a:gd name="eoex" fmla="*/ eot eux 100000"/>
    <a:gd name="eoey" fmla="*/ eot euy 100000"/>
    <a:gd name="eoX" fmla="+- ebx eoex 0"/>
    <a:gd name="eoY" fmla="+- eby eoey 0"/>
    <a:gd name="eoaX" fmla="abs eoX"/>
    <a:gd name="eoaY" fmla="abs eoY"/>
    <a:gd name="eomX" fmla="+- eoaX 0 1"/>
    <a:gd name="eomY" fmla="+- eoaY 0 1"/>
    <a:gd name="eosX" fmla="?: eomX eoX 0"/>
    <a:gd name="eosY" fmla="?: eomY eoY 0"/>
    <a:gd name="eoang" fmla="at2 eosX eosY"/>
    <a:gd name="eix2" fmla="*/ rxI rxI 1"/>
    <a:gd name="eiy2" fmla="*/ ryI ryI 1"/>
    <a:gd name="eiA1" fmla="*/ eux2 eiy2 1"/>
    <a:gd name="eiA2" fmla="*/ euy2 eix2 1"/>
    <a:gd name="eiA" fmla="+- eiA1 eiA2 0"/>
    <a:gd name="eiC1" fmla="*/ ebxux eiy2 1"/>
    <a:gd name="eiC2" fmla="*/ ebyuy eix2 1"/>
    <a:gd name="eiC" fmla="+- eiC1 eiC2 0"/>
    <a:gd name="eiD1" fmla="*/ ebx2 eiy2 1"/>
    <a:gd name="eiD2" fmla="*/ eby2 eix2 1"/>
    <a:gd name="eiD3" fmla="*/ eix2 eiy2 1"/>
    <a:gd name="eiD0" fmla="+- eiD1 eiD2 0"/>
    <a:gd name="eiD" fmla="+- eiD0 0 eiD3"/>
    <a:gd name="eiCC" fmla="*/ eiC eiC 1"/>
    <a:gd name="eiAD" fmla="*/ eiA eiD 1"/>
    <a:gd name="eiQ" fmla="+- eiCC 0 eiAD"/>
    <a:gd name="eiS" fmla="sqrt eiQ"/>
    <a:gd name="eiN" fmla="+- eiS 0 eiC"/>
    <a:gd name="eit" fmla="*/ eiN 100000 eiA"/>
    <a:gd name="eiex" fmla="*/ eit eux 100000"/>
    <a:gd name="eiey" fmla="*/ eit euy 100000"/>
    <a:gd name="eiX" fmla="+- ebx eiex 0"/>
    <a:gd name="eiY" fmla="+- eby eiey 0"/>
    <a:gd name="eiaX" fmla="abs eiX"/>
    <a:gd name="eiaY" fmla="abs eiY"/>
    <a:gd name="eimX" fmla="+- eiaX 0 1"/>
    <a:gd name="eimY" fmla="+- eiaY 0 1"/>
    <a:gd name="eisX" fmla="?: eimX eiX 0"/>
    <a:gd name="eisY" fmla="?: eimY eiY 0"/>
    <a:gd name="eiang" fmla="at2 eisX eisY"/>
    <a:gd name="ewoT" fmla="max ha eot"/>
    <a:gd name="ewiT" fmla="min negHa eit"/>
    <a:gd name="ewox" fmla="*/ ewoT eux 100000"/>
    <a:gd name="ewoy" fmla="*/ ewoT euy 100000"/>
    <a:gd name="ewix" fmla="*/ ewiT eux 100000"/>
    <a:gd name="ewiy" fmla="*/ ewiT euy 100000"/>
    <a:gd name="eWox" fmla="+- ebx ewox 0"/>
    <a:gd name="eWoy" fmla="+- eby ewoy 0"/>
    <a:gd name="eWix" fmla="+- ebx ewix 0"/>
    <a:gd name="eWiy" fmla="+- eby ewiy 0"/>
    <a:gd name="scB" fmla="cos ryM st"/>
    <a:gd name="ssB" fmla="sin rxM st"/>
    <a:gd name="sbx" fmla="cat2 rxM scB ssB"/>
    <a:gd name="sby" fmla="sat2 ryM scB ssB"/>
    <a:gd name="scT" fmla="cos ryM tq"/>
    <a:gd name="ssT" fmla="sin rxM tq"/>
    <a:gd name="stx" fmla="cat2 rxM scT ssT"/>
    <a:gd name="sty" fmla="sat2 ryM scT ssT"/>
    <a:gd name="sux" fmla="cos 100000 tq"/>
    <a:gd name="suy" fmla="sin 100000 tq"/>
    <a:gd name="sux2" fmla="*/ sux sux 1"/>
    <a:gd name="suy2" fmla="*/ suy suy 1"/>
    <a:gd name="sbxux" fmla="*/ sbx sux 1"/>
    <a:gd name="sbyuy" fmla="*/ sby suy 1"/>
    <a:gd name="sbx2" fmla="*/ sbx sbx 1"/>
    <a:gd name="sby2" fmla="*/ sby sby 1"/>
    <a:gd name="sox2" fmla="*/ rxO rxO 1"/>
    <a:gd name="soy2" fmla="*/ ryO ryO 1"/>
    <a:gd name="soA1" fmla="*/ sux2 soy2 1"/>
    <a:gd name="soA2" fmla="*/ suy2 sox2 1"/>
    <a:gd name="soA" fmla="+- soA1 soA2 0"/>
    <a:gd name="soC1" fmla="*/ sbxux soy2 1"/>
    <a:gd name="soC2" fmla="*/ sbyuy sox2 1"/>
    <a:gd name="soC" fmla="+- soC1 soC2 0"/>
    <a:gd name="soD1" fmla="*/ sbx2 soy2 1"/>
    <a:gd name="soD2" fmla="*/ sby2 sox2 1"/>
    <a:gd name="soD3" fmla="*/ sox2 soy2 1"/>
    <a:gd name="soD0" fmla="+- soD1 soD2 0"/>
    <a:gd name="soD" fmla="+- soD0 0 soD3"/>
    <a:gd name="soCC" fmla="*/ soC soC 1"/>
    <a:gd name="soAD" fmla="*/ soA soD 1"/>
    <a:gd name="soQ" fmla="+- soCC 0 soAD"/>
    <a:gd name="soS" fmla="sqrt soQ"/>
    <a:gd name="soN" fmla="+- soS 0 soC"/>
    <a:gd name="sot" fmla="*/ soN 100000 soA"/>
    <a:gd name="soex" fmla="*/ sot sux 100000"/>
    <a:gd name="soey" fmla="*/ sot suy 100000"/>
    <a:gd name="soX" fmla="+- sbx soex 0"/>
    <a:gd name="soY" fmla="+- sby soey 0"/>
    <a:gd name="soaX" fmla="abs soX"/>
    <a:gd name="soaY" fmla="abs soY"/>
    <a:gd name="somX" fmla="+- soaX 0 1"/>
    <a:gd name="somY" fmla="+- soaY 0 1"/>
    <a:gd name="sosX" fmla="?: somX soX 0"/>
    <a:gd name="sosY" fmla="?: somY soY 0"/>
    <a:gd name="soang" fmla="at2 sosX sosY"/>
    <a:gd name="six2" fmla="*/ rxI rxI 1"/>
    <a:gd name="siy2" fmla="*/ ryI ryI 1"/>
    <a:gd name="siA1" fmla="*/ sux2 siy2 1"/>
    <a:gd name="siA2" fmla="*/ suy2 six2 1"/>
    <a:gd name="siA" fmla="+- siA1 siA2 0"/>
    <a:gd name="siC1" fmla="*/ sbxux siy2 1"/>
    <a:gd name="siC2" fmla="*/ sbyuy six2 1"/>
    <a:gd name="siC" fmla="+- siC1 siC2 0"/>
    <a:gd name="siD1" fmla="*/ sbx2 siy2 1"/>
    <a:gd name="siD2" fmla="*/ sby2 six2 1"/>
    <a:gd name="siD3" fmla="*/ six2 siy2 1"/>
    <a:gd name="siD0" fmla="+- siD1 siD2 0"/>
    <a:gd name="siD" fmla="+- siD0 0 siD3"/>
    <a:gd name="siCC" fmla="*/ siC siC 1"/>
    <a:gd name="siAD" fmla="*/ siA siD 1"/>
    <a:gd name="siQ" fmla="+- siCC 0 siAD"/>
    <a:gd name="siS" fmla="sqrt siQ"/>
    <a:gd name="siN" fmla="+- siS 0 siC"/>
    <a:gd name="sit" fmla="*/ siN 100000 siA"/>
    <a:gd name="siex" fmla="*/ sit sux 100000"/>
    <a:gd name="siey" fmla="*/ sit suy 100000"/>
    <a:gd name="siX" fmla="+- sbx siex 0"/>
    <a:gd name="siY" fmla="+- sby siey 0"/>
    <a:gd name="siaX" fmla="abs siX"/>
    <a:gd name="siaY" fmla="abs siY"/>
    <a:gd name="simX" fmla="+- siaX 0 1"/>
    <a:gd name="simY" fmla="+- siaY 0 1"/>
    <a:gd name="sisX" fmla="?: simX siX 0"/>
    <a:gd name="sisY" fmla="?: simY siY 0"/>
    <a:gd name="siang" fmla="at2 sisX sisY"/>
    <a:gd name="swoT" fmla="max ha sot"/>
    <a:gd name="swiT" fmla="min negHa sit"/>
    <a:gd name="swox" fmla="*/ swoT sux 100000"/>
    <a:gd name="swoy" fmla="*/ swoT suy 100000"/>
    <a:gd name="swix" fmla="*/ swiT sux 100000"/>
    <a:gd name="swiy" fmla="*/ swiT suy 100000"/>
    <a:gd name="sWox" fmla="+- sbx swox 0"/>
    <a:gd name="sWoy" fmla="+- sby swoy 0"/>
    <a:gd name="sWix" fmla="+- sbx swix 0"/>
    <a:gd name="sWiy" fmla="+- sby swiy 0"/>
    <a:gd name="swO0" fmla="+- eoang 0 soang"/>
    <a:gd name="swO1" fmla="+- swO0 21600000 0"/>
    <a:gd name="nO" fmla="+- 0 0 swO0"/>
    <a:gd name="swO" fmla="?: nO swO1 swO0"/>
    <a:gd name="swI0" fmla="+- eiang 0 siang"/>
    <a:gd name="swI1" fmla="+- swI0 21600000 0"/>
    <a:gd name="nI" fmla="+- 0 0 swI0"/>
    <a:gd name="swI2" fmla="?: nI swI1 swI0"/>
    <a:gd name="swI" fmla="+- 0 0 swI2"/>
    <a:gd name="Ex" fmla="+- hc eWox 0"/>
    <a:gd name="Ey" fmla="+- vc eWoy 0"/>
    <a:gd name="Fx" fmla="+- hc eWix 0"/>
    <a:gd name="Fy" fmla="+- vc eWiy 0"/>
    <a:gd name="Tx" fmla="+- hc etx 0"/>
    <a:gd name="Ty" fmla="+- vc ety 0"/>
    <a:gd name="Oex" fmla="+- hc eoX 0"/>
    <a:gd name="Oey" fmla="+- vc eoY 0"/>
    <a:gd name="Iex" fmla="+- hc eiX 0"/>
    <a:gd name="Iey" fmla="+- vc eiY 0"/>
    <a:gd name="Gx" fmla="+- hc sWox 0"/>
    <a:gd name="Gy" fmla="+- vc sWoy 0"/>
    <a:gd name="Hx" fmla="+- hc sWix 0"/>
    <a:gd name="Hy" fmla="+- vc sWiy 0"/>
    <a:gd name="Ux" fmla="+- hc stx 0"/>
    <a:gd name="Uy" fmla="+- vc sty 0"/>
    <a:gd name="Osx" fmla="+- hc soX 0"/>
    <a:gd name="Osy" fmla="+- vc soY 0"/>
    <a:gd name="Isx" fmla="+- hc siX 0"/>
    <a:gd name="Isy" fmla="+- vc siY 0"/>
  </a:gdLst>
  <a:pathLst>
  <a:path>
    <a:moveTo><a:pt x="Ux" y="Uy"/></a:moveTo>
    <a:lnTo><a:pt x="Gx" y="Gy"/></a:lnTo>
    <a:lnTo><a:pt x="Osx" y="Osy"/></a:lnTo>
    <a:arcTo wR="rxOa" hR="ryOa" stAng="soang" swAng="swO"/>
    <a:lnTo><a:pt x="Ex" y="Ey"/></a:lnTo>
    <a:lnTo><a:pt x="Tx" y="Ty"/></a:lnTo>
    <a:lnTo><a:pt x="Fx" y="Fy"/></a:lnTo>
    <a:lnTo><a:pt x="Iex" y="Iey"/></a:lnTo>
    <a:arcTo wR="rxIa" hR="ryIa" stAng="eiang" swAng="swI"/>
    <a:lnTo><a:pt x="Hx" y="Hy"/></a:lnTo>
    <a:close/>
  </a:path>
  </a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// The presets of this batch `(name, definition)`.
pub(super) fn entries() -> Vec<(&'static str, ShapeDef)> {
    [
        ("notchedRightArrow", notched_right_arrow()),
        ("bentUpArrow", bent_up_arrow()),
        ("leftRightArrow", left_right_arrow()),
        ("upDownArrow", up_down_arrow()),
        ("leftUpArrow", left_up_arrow()),
        ("leftRightUpArrow", left_right_up_arrow()),
        ("quadArrow", quad_arrow()),
        ("leftArrowCallout", left_arrow_callout()),
        ("rightArrowCallout", right_arrow_callout()),
        ("upArrowCallout", up_arrow_callout()),
        ("downArrowCallout", down_arrow_callout()),
        ("leftRightArrowCallout", left_right_arrow_callout()),
        ("upDownArrowCallout", up_down_arrow_callout()),
        ("quadArrowCallout", quad_arrow_callout()),
        ("bentArrow", bent_arrow()),
        ("uturnArrow", uturn_arrow()),
        ("curvedRightArrow", curved_right_arrow()),
        ("curvedLeftArrow", curved_left_arrow()),
        ("curvedDownArrow", curved_down_arrow()),
        ("curvedUpArrow", curved_up_arrow()),
        ("swooshArrow", swoosh_arrow()),
        ("circularArrow", circular_arrow()),
        ("leftCircularArrow", left_circular_arrow()),
        ("leftRightCircularArrow", left_right_circular_arrow()),
    ]
    .into_iter()
    .filter_map(|(n, d)| d.map(|d| (n, d)))
    .collect()
}
