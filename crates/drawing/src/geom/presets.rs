//! The preset table, rebuilt clean-room from measurement of Word's own output.
//!
//! Method (docs/architecture/drawing.md, "Clean-room preset method"): every entry below was written
//! from the ECMA-376 guide-language prose (operators and built-in names only) and from numbers read
//! out of Word's PDF export of probe documents (`tools/office/make-preset-probes.ps1`). The
//! measured paths sit next to the PDFs in `fixtures/reference/drawing/presets/*.paths.json`, and
//! `tests/preset_probes.rs` compares each entry with them within 0.25 pt. No other project's preset
//! tables and no copy of Ecma's `presetShapeDefinitions.xml` were opened. Guide names are our own.
//!
//! Text rectangles are not measured yet (the probes carry no text), so the entries below leave them
//! out and text uses the whole box.
//!
//! Adjust handling common to the entries: Word clamps an adjust before use (out-of-range values in
//! the probes behave exactly like the nearest limit), so every adjust passes through `pin`.

use super::ShapeDef;
#[path = "presets_b01.rs"]
mod presets_b01;
#[path = "presets_b02.rs"]
mod presets_b02;
#[path = "presets_b03.rs"]
mod presets_b03;
#[path = "presets_b04.rs"]
mod presets_b04;
#[path = "presets_b05.rs"]
mod presets_b05;
#[path = "presets_b06.rs"]
mod presets_b06;
#[path = "presets_b07.rs"]
mod presets_b07;
#[path = "presets_b08.rs"]
mod presets_b08;
use crate::parse::cust_geom_from_str;

/// `rect`.
///
/// Measured from the pilot probe (Word 16, 144x144, 288x96, 96x288 and 216x144 pt): four points
/// (left top, right top, right bottom, left bottom), closed, exactly on the box at every size.
/// Word reports no adjust handles (`Shape.Adjustments.Count` = 0).
fn rect() -> ShapeDef {
    ShapeDef::rect()
}

/// `roundRect`.
///
/// Measured: one adjust, default 0.16667 (Word's object model). The corner radius is
/// `min(w,h) * adj / 100000` at all four sizes (24 pt at 144 and 216x144, 16 pt at 288x96 and
/// 96x288). With `adj` 30000 the radius on 216x144 is 43.2 pt, with 100000 and 400000 it stops at
/// 72 pt (half the short side), and with 0 and -50000 the outline is the plain rectangle, so the
/// adjust is clamped to 0..50000. Each corner is a quarter ellipse (cubic control offset 0.5523 r).
fn round_rect() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 16667"/></a:avLst>
  <a:gdLst>
    <a:gd name="aPin" fmla="pin 0 adj 50000"/>
    <a:gd name="rad" fmla="*/ ss aPin 100000"/>
    <a:gd name="xr" fmla="+- r 0 rad"/>
    <a:gd name="yb" fmla="+- b 0 rad"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="rad"/></a:moveTo>
    <a:arcTo wR="rad" hR="rad" stAng="cd2" swAng="cd4"/>
    <a:lnTo><a:pt x="xr" y="t"/></a:lnTo>
    <a:arcTo wR="rad" hR="rad" stAng="3cd4" swAng="cd4"/>
    <a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
    <a:arcTo wR="rad" hR="rad" stAng="0" swAng="cd4"/>
    <a:lnTo><a:pt x="rad" y="b"/></a:lnTo>
    <a:arcTo wR="rad" hR="rad" stAng="cd4" swAng="cd4"/>
    <a:close/>
  </a:path></a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `ellipse`.
///
/// Measured: no adjusts. Four cubic arcs starting at the left middle point and running clockwise
/// (left, top, right, bottom), through the four box edge midpoints at every size; control offsets
/// 0.5523 of the radii (32.24 and 48.35 pt on 216x144), so an exact ellipse inscribed in the box.
fn ellipse() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst/>
  <a:gdLst/>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="vc"/></a:moveTo>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd2" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="3cd4" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="0" swAng="cd4"/>
    <a:arcTo wR="wd2" hR="hd2" stAng="cd4" swAng="cd4"/>
    <a:close/>
  </a:path></a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `triangle`.
///
/// Measured: one adjust, default 0.5. Three points: left bottom, apex on the top edge, right
/// bottom. The apex x is `w * adj / 100000` at all sizes (108 pt on 216 wide; 64.8 pt with
/// 30000). The adjust is clamped to 0..100000: 400000 puts the apex at the right edge and
/// 0 and -50000 at the left edge.
fn triangle() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst><a:gd name="adj" fmla="val 50000"/></a:avLst>
  <a:gdLst>
    <a:gd name="aPin" fmla="pin 0 adj 100000"/>
    <a:gd name="xApex" fmla="*/ w aPin 100000"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="b"/></a:moveTo>
    <a:lnTo><a:pt x="xApex" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="b"/></a:lnTo>
    <a:close/>
  </a:path></a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// `rightArrow`.
///
/// Measured: two adjusts, both default 0.5 (Word's object model). Seven points: shaft top left,
/// shaft top at the head base, head top corner, tip on the right edge middle, head bottom corner,
/// shaft bottom at the head base, shaft bottom left. `adj1` is the shaft thickness as a fraction
/// of the height (50000 gives 72 of 144 pt; 30000 gives 43.2), clamped to 0..100000 (100000 and
/// 400000 both give a full-height shaft). `adj2` is the head length as a fraction of the short
/// side (48 pt on 96 pt short side at the default, at all three sizes), clamped to
/// 0..`100000 * w / ss`: with 400000 on 216x144 the head is 216 pt long, the full width, and
/// with 100000 it is 144 pt (so the limit is not 100000 when the box is wider than tall).
fn right_arrow() -> Option<ShapeDef> {
    cust_geom_from_str(
        r#"<a:custGeom>
  <a:avLst>
    <a:gd name="adj1" fmla="val 50000"/>
    <a:gd name="adj2" fmla="val 50000"/>
  </a:avLst>
  <a:gdLst>
    <a:gd name="shaftPin" fmla="pin 0 adj1 100000"/>
    <a:gd name="headMax" fmla="*/ 100000 w ss"/>
    <a:gd name="headPin" fmla="pin 0 adj2 headMax"/>
    <a:gd name="headLen" fmla="*/ ss headPin 100000"/>
    <a:gd name="xHead" fmla="+- r 0 headLen"/>
    <a:gd name="halfShaft" fmla="*/ h shaftPin 200000"/>
    <a:gd name="yTop" fmla="+- vc 0 halfShaft"/>
    <a:gd name="yBot" fmla="+- vc halfShaft 0"/>
  </a:gdLst>
  <a:pathLst><a:path>
    <a:moveTo><a:pt x="l" y="yTop"/></a:moveTo>
    <a:lnTo><a:pt x="xHead" y="yTop"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="t"/></a:lnTo>
    <a:lnTo><a:pt x="r" y="vc"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="b"/></a:lnTo>
    <a:lnTo><a:pt x="xHead" y="yBot"/></a:lnTo>
    <a:lnTo><a:pt x="l" y="yBot"/></a:lnTo>
    <a:close/>
  </a:path></a:pathLst>
</a:custGeom>"#,
    )
    .ok()
}

/// Presets authored so far `(name, definition)`, each checked against Word by
/// `tests/preset_probes.rs`.
pub fn table() -> Vec<(&'static str, ShapeDef)> {
    let mut v = vec![("rect", rect())];
    let others = [
        ("roundRect", round_rect()),
        ("ellipse", ellipse()),
        ("triangle", triangle()),
        ("rightArrow", right_arrow()),
    ];
    v.extend(others.into_iter().filter_map(|(n, d)| d.map(|d| (n, d))));
    v.extend(presets_b01::entries());
    v.extend(presets_b02::entries());
    v.extend(presets_b03::entries());
    v.extend(presets_b04::entries());
    v.extend(presets_b05::entries());
    v.extend(presets_b06::entries());
    v.extend(presets_b07::entries());
    v.extend(presets_b08::entries());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_parses() {
        let t = table();
        assert!(t.len() >= 5);
        let mut names = std::collections::BTreeSet::new();
        for (n, def) in &t {
            assert!(names.insert(*n), "{n} registered twice");
            let ev = super::super::evaluate(def, &[], 1_828_800.0, 914_400.0);
            assert!(
                !ev.paths.is_empty(),
                "{n} renders no path at its default adjusts"
            );
        }
        for n in ["rect", "roundRect", "ellipse", "triangle", "rightArrow"] {
            assert!(names.contains(n), "{n} missing");
        }
    }
}
