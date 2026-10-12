//! Preset batch b06 (cloud, ribbons, scrolls, waves, plus, flowchart shapes), rebuilt clean-room
//! from Word measurements (docs/architecture/drawing.md).
//!
//! Source: `fixtures/reference/drawing/presets/b06.{docx,pdf,probes.json,handles.json,paths.json}`
//! and the filled variant `b06.fill.*` (same probes with a grey fill, which shows the fill-only
//! paths and their relative shade), made by `tools/office/make-preset-probes.ps1`. Every formula
//! below was derived from those measured paths and handle defaults only; no other project's
//! preset table and no copy of Ecma's `presetShapeDefinitions.xml` was opened. Guide names are
//! our own.
//!
//! Findings common to the batch:
//! - Word emits one fill operation and one stroke operation per preset path. The shaded parts of
//!   ribbons and scrolls are separate fill-only paths painted at 0.604 / 0.753 = 0.80 of the base
//!   grey, which is `darkenLess`; they have no outline of their own (`stroke="0"`).
//! - Several shapes are fixed outlines scaled to the box (cloud, the flowchart document family):
//!   Word's output at four box sizes is an exact affine copy, so those are stored as the measured
//!   control points in a path coordinate space (21600 grid or a 1e6 grid for the cloud).
//! - Adjusts that are sizes are clamped before use (limits per entry); a negative or oversized
//!   value shows the nearest limit in Word's output.
//! - Text rectangles and connection sites are not measurable with these probes and are left out.

use super::ShapeDef;
use crate::parse::cust_geom_from_str;

fn parse(xml: &str) -> Option<ShapeDef> {
    cust_geom_from_str(xml).ok()
}

/// One `a:arcTo` element.
fn arc(wr: &str, hr: &str, st: &str, sw: &str) -> String {
    format!(r#"<a:arcTo wR="{wr}" hR="{hr}" stAng="{st}" swAng="{sw}"/>"#)
}

/// `a:moveTo` and `a:lnTo` with named or literal coordinates.
fn mv(x: &str, y: &str) -> String {
    format!(r#"<a:moveTo><a:pt x="{x}" y="{y}"/></a:moveTo>"#)
}

fn ln(x: &str, y: &str) -> String {
    format!(r#"<a:lnTo><a:pt x="{x}" y="{y}"/></a:lnTo>"#)
}

fn quad(cx: &str, cy: &str, x: &str, y: &str) -> String {
    format!(r#"<a:quadBezTo><a:pt x="{cx}" y="{cy}"/><a:pt x="{x}" y="{y}"/></a:quadBezTo>"#)
}

fn gd(name: &str, fmla: &str) -> String {
    format!(r#"<a:gd name="{name}" fmla="{fmla}"/>"#)
}

/// Common angle guides for signed sweeps.
const ANGLES: &str =
    r#"<a:gd name="ncd2" fmla="val -10800000"/><a:gd name="ncd4" fmla="val -5400000"/>"#;

// ---- cloud and cloud callout ---------------------------------------------------------------

/// Path coordinate space of the cloud data.
const CLOUD_GRID: u32 = 1_000_000;

/// `cloud`: no adjusts. Measured at four sizes: the outline is the same affine copy at every size
/// (largest difference 0.01 pt), so it is stored as measured control points on a 1e6 grid:
/// one closed outline (filled and stroked) of 22 cubics plus ten short stroke-only arcs that
/// mark the folds between the puffs.
fn cloud_paths() -> String {
    format!(
        r#"<a:path w="{g}" h="{g}"><a:moveTo><a:pt x="90278" y="332639"/></a:moveTo><a:cubicBezTo><a:pt x="78644" y="216250"/><a:pt x="138565" y="109028"/><a:pt x="224120" y="93194"/></a:cubicBezTo><a:cubicBezTo><a:pt x="258750" y="86736"/><a:pt x="294028" y="96319"/><a:pt x="324213" y="120417"/></a:cubicBezTo><a:cubicBezTo><a:pt x="356157" y="38403"/><a:pt x="430926" y="7222"/><a:pt x="491111" y="50764"/></a:cubicBezTo><a:cubicBezTo><a:pt x="501667" y="58403"/><a:pt x="511296" y="68056"/><a:pt x="519815" y="79444"/></a:cubicBezTo><a:cubicBezTo><a:pt x="544722" y="11458"/><a:pt x="605417" y="-16181"/><a:pt x="655324" y="17708"/></a:cubicBezTo><a:cubicBezTo><a:pt x="669167" y="27153"/><a:pt x="681204" y="40694"/><a:pt x="690556" y="57431"/></a:cubicBezTo><a:cubicBezTo><a:pt x="730741" y="-6875"/><a:pt x="801667" y="-14861"/><a:pt x="849028" y="39653"/></a:cubicBezTo><a:cubicBezTo><a:pt x="868935" y="62569"/><a:pt x="882361" y="94167"/><a:pt x="886991" y="129097"/></a:cubicBezTo><a:cubicBezTo><a:pt x="952778" y="153472"/><a:pt x="991574" y="246042"/><a:pt x="973657" y="335764"/></a:cubicBezTo><a:cubicBezTo><a:pt x="972176" y="343264"/><a:pt x="970278" y="350694"/><a:pt x="968009" y="357847"/></a:cubicBezTo><a:cubicBezTo><a:pt x="1020787" y="451319"/><a:pt x="1007870" y="585347"/><a:pt x="939167" y="657153"/></a:cubicBezTo><a:cubicBezTo><a:pt x="917778" y="679514"/><a:pt x="892546" y="693958"/><a:pt x="865833" y="699167"/></a:cubicBezTo><a:cubicBezTo><a:pt x="865231" y="799722"/><a:pt x="804815" y="880625"/><a:pt x="730880" y="879792"/></a:cubicBezTo><a:cubicBezTo><a:pt x="706157" y="879514"/><a:pt x="681991" y="869931"/><a:pt x="661019" y="852153"/></a:cubicBezTo><a:cubicBezTo><a:pt x="636019" y="964931"/><a:pt x="548750" y="1028611"/><a:pt x="466157" y="994514"/></a:cubicBezTo><a:cubicBezTo><a:pt x="431528" y="980208"/><a:pt x="401620" y="949931"/><a:pt x="381481" y="908889"/></a:cubicBezTo><a:cubicBezTo><a:pt x="296898" y="978333"/><a:pt x="187130" y="940972"/><a:pt x="136250" y="825486"/></a:cubicBezTo><a:cubicBezTo><a:pt x="135602" y="824028"/><a:pt x="135000" y="822569"/><a:pt x="134352" y="821042"/></a:cubicBezTo><a:cubicBezTo><a:pt x="79014" y="829861"/><a:pt x="28870" y="776111"/><a:pt x="22370" y="700972"/></a:cubicBezTo><a:cubicBezTo><a:pt x="18907" y="660903"/><a:pt x="28620" y="620833"/><a:pt x="48926" y="591389"/></a:cubicBezTo><a:cubicBezTo><a:pt x="991" y="552986"/><a:pt x="-15111" y="468681"/><a:pt x="12958" y="403125"/></a:cubicBezTo><a:cubicBezTo><a:pt x="29153" y="365278"/><a:pt x="57560" y="340278"/><a:pt x="89435" y="335764"/></a:cubicBezTo><a:close/></a:path><a:path w="{g}" h="{g}" fill="none"><a:moveTo><a:pt x="108634" y="605972"/></a:moveTo><a:cubicBezTo><a:pt x="88171" y="608125"/><a:pt x="67708" y="601667"/><a:pt x="49995" y="587500"/></a:cubicBezTo><a:moveTo><a:pt x="160370" y="807847"/></a:moveTo><a:cubicBezTo><a:pt x="152130" y="812292"/><a:pt x="143519" y="815278"/><a:pt x="134722" y="816667"/></a:cubicBezTo><a:moveTo><a:pt x="381435" y="904861"/></a:moveTo><a:cubicBezTo><a:pt x="375231" y="892222"/><a:pt x="370046" y="878750"/><a:pt x="365972" y="864583"/></a:cubicBezTo><a:moveTo><a:pt x="667269" y="804444"/></a:moveTo><a:cubicBezTo><a:pt x="666389" y="819375"/><a:pt x="664306" y="834236"/><a:pt x="661111" y="848611"/></a:cubicBezTo><a:moveTo><a:pt x="790000" y="531319"/></a:moveTo><a:cubicBezTo><a:pt x="836435" y="562083"/><a:pt x="865694" y="626319"/><a:pt x="865278" y="696528"/></a:cubicBezTo><a:moveTo><a:pt x="967546" y="355417"/></a:moveTo><a:cubicBezTo><a:pt x="960046" y="379306"/><a:pt x="948565" y="400486"/><a:pt x="934028" y="417361"/></a:cubicBezTo><a:moveTo><a:pt x="887130" y="125625"/></a:moveTo><a:cubicBezTo><a:pt x="888426" y="135278"/><a:pt x="888981" y="145069"/><a:pt x="888889" y="154861"/></a:cubicBezTo><a:moveTo><a:pt x="673102" y="91458"/></a:moveTo><a:cubicBezTo><a:pt x="677454" y="77986"/><a:pt x="683241" y="65417"/><a:pt x="690278" y="54167"/></a:cubicBezTo><a:moveTo><a:pt x="512500" y="109236"/></a:moveTo><a:cubicBezTo><a:pt x="514306" y="98125"/><a:pt x="517083" y="87292"/><a:pt x="520833" y="77083"/></a:cubicBezTo><a:moveTo><a:pt x="324074" y="120208"/></a:moveTo><a:cubicBezTo><a:pt x="335000" y="128889"/><a:pt x="345093" y="139375"/><a:pt x="354167" y="151389"/></a:cubicBezTo><a:moveTo><a:pt x="95532" y="365486"/></a:moveTo><a:cubicBezTo><a:pt x="93148" y="354722"/><a:pt x="91394" y="343750"/><a:pt x="90282" y="332639"/></a:cubicBezTo></a:path>"#,
        g = CLOUD_GRID
    )
}

fn cloud() -> Option<ShapeDef> {
    parse(&format!(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst>{}</a:pathLst></a:custGeom>"#,
        cloud_paths()
    ))
}

/// `cloudCallout`: two adjusts, defaults -20833 and 62500. The cloud outline of `cloud`, plus three
/// filled and stroked circles of radius `ss/36`, `ss/18` and `ss/12`.
///
/// Measured: the first circle is centred on the tip `T = (hc + w*adj1/100000, vc + h*adj2/100000)`
/// (not clamped, may be inside the cloud). Let `B` be the point of the inscribed ellipse in the
/// direction of the tip (parametric angle `at2(dy * wd2, dx * hd2)`), `m = |B - T|` and
/// `k = ss / 54`. The other circles are centred at `T + (B - T) * (1/3 - k/m)` and
/// `T + (B - T) * (2/3 + k/m)`: symmetric about the middle of `T..B`, checked on all eleven
/// probe pages (fit error below 0.02 pt). No clamp was seen on the adjusts.
fn cloud_callout() -> Option<ShapeDef> {
    parse(&format!(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val -20833"/><a:gd name="adj2" fmla="val 62500"/></a:avLst>
    <a:gdLst>
      <a:gd name="dxp" fmla="*/ w adj1 100000"/>
      <a:gd name="dyp" fmla="*/ h adj2 100000"/>
      <a:gd name="tx" fmla="+- hc dxp 0"/>
      <a:gd name="ty" fmla="+- vc dyp 0"/>
      <a:gd name="gx" fmla="*/ dxp hd2 1"/>
      <a:gd name="gy" fmla="*/ dyp wd2 1"/>
      <a:gd name="th" fmla="at2 gx gy"/>
      <a:gd name="bx0" fmla="cos wd2 th"/>
      <a:gd name="by0" fmla="sin hd2 th"/>
      <a:gd name="bx" fmla="+- hc bx0 0"/>
      <a:gd name="by" fmla="+- vc by0 0"/>
      <a:gd name="ux" fmla="+- bx 0 tx"/>
      <a:gd name="uy" fmla="+- by 0 ty"/>
      <a:gd name="m" fmla="mod ux uy 0"/>
      <a:gd name="k" fmla="*/ ss 1 54"/>
      <a:gd name="kr" fmla="*/ k 100000 m"/>
      <a:gd name="f2" fmla="+- 33333.3333 0 kr"/>
      <a:gd name="f3" fmla="+- 66666.6667 kr 0"/>
      <a:gd name="c2x" fmla="*/ ux f2 100000"/>
      <a:gd name="c2y" fmla="*/ uy f2 100000"/>
      <a:gd name="c3x" fmla="*/ ux f3 100000"/>
      <a:gd name="c3y" fmla="*/ uy f3 100000"/>
      <a:gd name="x2" fmla="+- tx c2x 0"/>
      <a:gd name="y2" fmla="+- ty c2y 0"/>
      <a:gd name="x3" fmla="+- tx c3x 0"/>
      <a:gd name="y3" fmla="+- ty c3y 0"/>
      <a:gd name="r1" fmla="*/ ss 1 36"/>
      <a:gd name="r2" fmla="*/ ss 1 18"/>
      <a:gd name="r3" fmla="*/ ss 1 12"/>
      <a:gd name="tx1" fmla="+- tx r1 0"/>
      <a:gd name="tx2" fmla="+- x2 r2 0"/>
      <a:gd name="tx3" fmla="+- x3 r3 0"/>
    </a:gdLst>
    <a:pathLst>{cloud}
      <a:path><a:moveTo><a:pt x="tx1" y="ty"/></a:moveTo>{a1}<a:close/></a:path>
      <a:path><a:moveTo><a:pt x="tx2" y="y2"/></a:moveTo>{a2}<a:close/></a:path>
      <a:path><a:moveTo><a:pt x="tx3" y="y3"/></a:moveTo>{a3}<a:close/></a:path>
    </a:pathLst></a:custGeom>"#,
        cloud = cloud_paths(),
        a1 = arc("r1", "r1", "0", "21600000"),
        a2 = arc("r2", "r2", "0", "21600000"),
        a3 = arc("r3", "r3", "0", "21600000"),
    ))
}

// ---- ribbons -------------------------------------------------------------------------------

/// Maps a y guide name for the vertical mirror used by `ribbon2` and `ellipseRibbon2`.
struct Mirror(bool);

impl Mirror {
    fn y(&self, n: &str) -> String {
        match (self.0, n) {
            (false, _) => n.to_owned(),
            (true, "t") => "b".to_owned(),
            (true, "b") => "t".to_owned(),
            (true, _) => format!("m_{n}"),
        }
    }

    /// Start angle under the mirror.
    fn st(&self, a: &'static str) -> &'static str {
        match (self.0, a) {
            (true, "3cd4") => "cd4",
            (true, "cd4") => "3cd4",
            _ => a,
        }
    }

    /// Sweep under the mirror.
    fn sw(&self, a: &'static str) -> &'static str {
        match (self.0, a) {
            (true, "cd2") => "ncd2",
            (true, "ncd2") => "cd2",
            (true, "cd4") => "ncd4",
            (true, "ncd4") => "cd4",
            _ => a,
        }
    }

    /// Guides defining the mirrored copies of the named y values.
    fn guides(&self, names: &[&str]) -> String {
        if !self.0 {
            return String::new();
        }
        names
            .iter()
            .map(|n| gd(&format!("m_{n}"), &format!("+- b 0 {n}")))
            .collect()
    }
}

/// `ribbon` and `ribbon2`: two adjusts, defaults 16667 and 50000.
///
/// Measured (`ribbon`, a banner whose folded tails hang down; `ribbon2` is the same outline mirrored
/// top to bottom, checked on the default page). `adj1` is the fold height: `a1 = h * adj1 / 100000`,
/// clamped to 0..33333 (100000 gives h/3). `adj2` is the width of the centre band, `cw = w * adj2
/// / 100000`, clamped to 25000..75000 (0 gives w/4, 100000 gives 3w/4). The fold edges sit at
/// `xL = hc - cw/2`, `xl2 = xL + w/8` and their mirrors; each fold edge ends in a half ellipse of
/// radii `w/32` and `a1/4`. The tails are notched to `w/8` from the sides, at half of `h - a1`.
/// Fill: the outline (normal), the two shaded fold pockets (`darkenLess`); strokes: the outline and
/// four vertical fold lines.
fn ribbon(flip: bool) -> Option<ShapeDef> {
    let m = Mirror(flip);
    let y = |n: &str| m.y(n);
    let ys = ["yq", "yh", "a1", "y3q", "yWB", "yWM", "yBq"];
    let a_top = |sw: &'static str, st: &'static str| arc("wd32", "yq", m.st(st), m.sw(sw));
    // Outline, starting at the top left corner.
    let outline = |close: bool| {
        let mut s = String::new();
        s += &mv("l", &y("t"));
        s += &ln("xl2m", &y("t"));
        s += &a_top("cd2", "3cd4");
        s += &ln("xLp", &y("yh"));
        s += &a_top("ncd2", "3cd4");
        s += &ln("xRm", &y("a1"));
        s += &a_top("ncd2", "cd4");
        s += &ln("xr2p", &y("yh"));
        s += &a_top("cd2", "cd4");
        s += &ln("r", &y("t"));
        s += &ln("wR8", &y("yWM"));
        s += &ln("r", &y("yWB"));
        s += &ln("xR", &y("yWB"));
        s += &ln("xR", &y("yBq"));
        s += &a_top("cd4", "0");
        s += &ln("xLp", &y("b"));
        s += &a_top("cd4", "cd4");
        s += &ln("xL", &y("yWB"));
        s += &ln("l", &y("yWB"));
        s += &ln("wd8", &y("yWM"));
        if close {
            s += "<a:close/>";
        }
        s
    };
    let shade = {
        let mut s = String::new();
        s += &mv("xl2", &y("yq"));
        s += &arc("wd32", "yq", m.st("0"), m.sw("cd4"));
        s += &ln("xLp", &y("yh"));
        s += &a_top("ncd2", "3cd4");
        s += &ln("xl2", &y("a1"));
        s += "<a:close/>";
        s += &mv("xr2", &y("yq"));
        s += &arc("wd32", "yq", "cd2", m.sw("ncd4"));
        s += &ln("xRm", &y("yh"));
        s += &a_top("cd2", "3cd4");
        s += &ln("xr2", &y("a1"));
        s += "<a:close/>";
        s
    };
    let lines = format!(
        "{}{}{}{}{}{}{}{}",
        mv("xl2", &y("yq")),
        ln("xl2", &y("a1")),
        mv("xr2", &y("a1")),
        ln("xr2", &y("yq")),
        mv("xL", &y("yWB")),
        ln("xL", &y("y3q")),
        mv("xR", &y("y3q")),
        ln("xR", &y("yWB"))
    );
    parse(&format!(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 16667"/><a:gd name="adj2" fmla="val 50000"/></a:avLst>
    <a:gdLst>{ANGLES}
      <a:gd name="a1p" fmla="pin 0 adj1 33333"/>
      <a:gd name="a1" fmla="*/ h a1p 100000"/>
      <a:gd name="cwp" fmla="pin 25000 adj2 75000"/>
      <a:gd name="half" fmla="*/ w cwp 200000"/>
      <a:gd name="xL" fmla="+- hc 0 half"/>
      <a:gd name="xR" fmla="+- hc half 0"/>
      <a:gd name="xl2" fmla="+- xL wd8 0"/>
      <a:gd name="xr2" fmla="+- xR 0 wd8"/>
      <a:gd name="xl2m" fmla="+- xl2 0 wd32"/>
      <a:gd name="xLp" fmla="+- xL wd32 0"/>
      <a:gd name="xRm" fmla="+- xR 0 wd32"/>
      <a:gd name="xr2p" fmla="+- xr2 wd32 0"/>
      <a:gd name="wR8" fmla="+- r 0 wd8"/>
      <a:gd name="yq" fmla="*/ a1 1 4"/>
      <a:gd name="yh" fmla="*/ a1 1 2"/>
      <a:gd name="y3q" fmla="*/ a1 3 4"/>
      <a:gd name="yWB" fmla="+- b 0 a1"/>
      <a:gd name="yWM" fmla="*/ yWB 1 2"/>
      <a:gd name="yBq" fmla="+- b 0 yq"/>
      {mirror}
    </a:gdLst>
    <a:pathLst>
      <a:path stroke="0">{fill}</a:path>
      <a:path fill="darkenLess" stroke="0">{shade}</a:path>
      <a:path fill="none">{stroke}{lines}</a:path>
    </a:pathLst></a:custGeom>"#,
        mirror = m.guides(&ys),
        fill = outline(true),
        stroke = outline(true),
    ))
}

/// `ellipseRibbon` and `ellipseRibbon2`: three adjusts, defaults 25000, 50000, 12500.
///
/// Measured. `a1 = h * adj1 / 100000` (clamped 0..100000), `a3 = h * adj3 / 100000` (clamped to
/// 0..adj1), `adj2` is the centre band width as in `ribbon` (clamped 25000..75000, band edges
/// `xL`, `xR` and fold edges `xl2 = xL + w/8`, `xr2 = xR - w/8`). The curved edges are parabolas
/// drawn as quadratic Beziers: `f(x) = a3 * (1 - (1 - x/hc)^2)` on the left half, mirrored on the
/// right. Top wing: `f` from 0 to `xl2`; top centre arc: `(a1 - a3) + f` from `xL` to `xR` (through
/// `(hc, a1)`); bottom wings: `(h - a1) + f`; bottom centre arc `(h - a3) + f` (it ends flush with the box bottom at the middle). The tail notch
/// is at `w/8` and `(h - a1)/2 + f(w/8)`. `ellipseRibbon2` is the outline mirrored top to bottom.
/// Fill: outline normal; the shaded band under the top fold in `darkenLess`; strokes: outline and
/// four vertical fold lines.
fn ellipse_ribbon(flip: bool) -> Option<ShapeDef> {
    let m = Mirror(flip);
    let y = |n: &str| m.y(n);
    let ys = [
        "fl2", "cL", "cl2", "ctrlW", "ctrlC", "yWB", "lwL", "lcL", "ymid", "ctrlLW", "ctrlLC",
        "cdk",
    ];
    let outline = {
        let mut s = String::new();
        s += &mv("l", &y("t"));
        s += &quad("xl2h", &y("ctrlW"), "xl2", &y("fl2"));
        s += &ln("xL", &y("cL"));
        s += &quad("hc", &y("ctrlC"), "xR", &y("cL"));
        s += &ln("xr2", &y("fl2"));
        s += &quad("xr2cc", &y("ctrlW"), "r", &y("t"));
        s += &ln("wR8", &y("ymid"));
        s += &ln("r", &y("yWB"));
        s += &quad("xRc", &y("ctrlLW"), "xR", &y("lwL"));
        s += &ln("xR", &y("lcL"));
        s += &quad("hc", &y("ctrlLC"), "xL", &y("lcL"));
        s += &ln("xL", &y("lwL"));
        s += &quad("xLc", &y("ctrlLW"), "l", &y("yWB"));
        s += &ln("wd8", &y("ymid"));
        s += "<a:close/>";
        s
    };
    let shade = {
        let mut s = String::new();
        s += &mv("xl2", &y("cl2"));
        s += &ln("xl2", &y("fl2"));
        s += &ln("xL", &y("cL"));
        s += &quad("hc", &y("ctrlC"), "xR", &y("cL"));
        s += &ln("xr2", &y("fl2"));
        s += &ln("xr2", &y("cl2"));
        s += &quad("hc", &y("cdk"), "xl2", &y("cl2"));
        s += "<a:close/>";
        s
    };
    let lines = format!(
        "{}{}{}{}{}{}{}{}",
        mv("xL", &y("lwL")),
        ln("xL", &y("cL")),
        mv("xR", &y("cL")),
        ln("xR", &y("lwL")),
        mv("xl2", &y("fl2")),
        ln("xl2", &y("cl2")),
        mv("xr2", &y("cl2")),
        ln("xr2", &y("fl2"))
    );
    parse(&format!(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 25000"/><a:gd name="adj2" fmla="val 50000"/><a:gd name="adj3" fmla="val 12500"/></a:avLst>
    <a:gdLst>
      <a:gd name="a1p" fmla="pin 0 adj1 100000"/>
      <a:gd name="a1" fmla="*/ h a1p 100000"/>
      <a:gd name="a3p" fmla="pin 0 adj3 a1p"/>
      <a:gd name="a3" fmla="*/ h a3p 100000"/>
      <a:gd name="cwp" fmla="pin 25000 adj2 75000"/>
      <a:gd name="half" fmla="*/ w cwp 200000"/>
      <a:gd name="xL" fmla="+- hc 0 half"/>
      <a:gd name="xR" fmla="+- hc half 0"/>
      <a:gd name="xl2" fmla="+- xL wd8 0"/>
      <a:gd name="xr2" fmla="+- xR 0 wd8"/>
      <a:gd name="wR8" fmla="+- r 0 wd8"/>
      <a:gd name="xl2h" fmla="*/ xl2 1 2"/>
      <a:gd name="xr2c" fmla="+- xr2 r 0"/>
      <a:gd name="xr2cc" fmla="*/ xr2c 1 2"/>
      <a:gd name="xLc" fmla="*/ xL 1 2"/>
      <a:gd name="xRc0" fmla="+- xR r 0"/>
      <a:gd name="xRc" fmla="*/ xRc0 1 2"/>
      <a:gd name="hc2" fmla="*/ hc hc 1"/>
      <a:gd name="e1" fmla="+- hc 0 xl2"/>
      <a:gd name="e1s" fmla="*/ e1 e1 1"/>
      <a:gd name="n1" fmla="+- hc2 0 e1s"/>
      <a:gd name="fl2" fmla="*/ a3 n1 hc2"/>
      <a:gd name="e2" fmla="+- hc 0 xL"/>
      <a:gd name="e2s" fmla="*/ e2 e2 1"/>
      <a:gd name="n2" fmla="+- hc2 0 e2s"/>
      <a:gd name="fL" fmla="*/ a3 n2 hc2"/>
      <a:gd name="a13" fmla="+- a1 0 a3"/>
      <a:gd name="cL" fmla="+- a13 fL 0"/>
      <a:gd name="cl2" fmla="+- a13 fl2 0"/>
      <a:gd name="ctrlW" fmla="*/ xl2 a3 hc"/>
      <a:gd name="k2" fmla="*/ a3 e2s hc2"/>
      <a:gd name="k2d" fmla="*/ a3 e1s hc2"/>
      <a:gd name="k22" fmla="*/ k2 2 1"/>
      <a:gd name="k2d2" fmla="*/ k2d 2 1"/>
      <a:gd name="ctrlC" fmla="+- cL k22 0"/>
      <a:gd name="cdk" fmla="+- cl2 k2d2 0"/>
      <a:gd name="yWB" fmla="+- b 0 a1"/>
      <a:gd name="lwL" fmla="+- yWB fL 0"/>
      <a:gd name="lc0" fmla="+- yWB a13 0"/>
      <a:gd name="lcL" fmla="+- lc0 fL 0"/>
      <a:gd name="ctrlLW0" fmla="*/ xL a3 hc"/>
      <a:gd name="ctrlLW" fmla="+- yWB ctrlLW0 0"/>
      <a:gd name="ctrlLC" fmla="+- lcL k22 0"/>
      <a:gd name="e3" fmla="+- hc 0 wd8"/>
      <a:gd name="e3s" fmla="*/ e3 e3 1"/>
      <a:gd name="n3" fmla="+- hc2 0 e3s"/>
      <a:gd name="f8" fmla="*/ a3 n3 hc2"/>
      <a:gd name="yh2" fmla="*/ yWB 1 2"/>
      <a:gd name="ymid" fmla="+- yh2 f8 0"/>
      {mirror}
    </a:gdLst>
    <a:pathLst>
      <a:path stroke="0">{outline}</a:path>
      <a:path fill="darkenLess" stroke="0">{shade}</a:path>
      <a:path fill="none">{outline}{lines}</a:path>
    </a:pathLst></a:custGeom>"#,
        mirror = m.guides(&ys),
    ))
}

/// `leftRightRibbon`: three adjusts, defaults 50000, 50000, 16667.
///
/// Measured. `a3 = h * adj3 / 100000` is the band thickness (clamped 0..33333); `x1 = ss * adj2 /
/// 100000` is the width of each arrow head (clamped to 0..`hc - w/32`: 100000 shows 101.25 pt on
/// 216x144); `adj1` places the band: its top edge is `yA = vc - a3/2 - h * adj1 / 200000`, with adj1
/// clamped so that `yA >= 0` (and to 0 at the low end). The left head is the triangle `(0, vc -
/// a3/2)`, `(x1, 0)`, `(x1, h - a3)`, the right head `(w, vc + a3/2)`, `(w - x1, a3)`, `(w - x1,
/// h)`. The two bands cross at the centre with half-ellipse curls of radii `w/32` and `a3/4`; the
/// lower band is the upper one mirrored about the vertical centre. Fill: outline normal; the lower
/// half of the curl in `darkenLess`; strokes: outline and two vertical edges of the curl.
fn left_right_ribbon() -> Option<ShapeDef> {
    let outline = {
        let mut s = String::new();
        s += &mv("l", "ytip");
        s += &ln("x1", "t");
        s += &ln("x1", "yA");
        s += &ln("hc", "yA");
        s += &arc("wd32", "q3", "3cd4", "cd2");
        s += &arc("wd32", "q3", "3cd4", "ncd2");
        s += &ln("x1r", "yAa");
        s += &ln("x1r", "a3");
        s += &ln("r", "ytip2");
        s += &ln("x1r", "b");
        s += &ln("x1r", "yBot");
        s += &ln("hc", "yBot");
        s += &arc("wd32", "q3", "cd4", "cd4");
        s += &ln("xcl", "yL1");
        s += &ln("x1", "yL1");
        s += &ln("x1", "yHb");
        s += "<a:close/>";
        s
    };
    let shade = format!(
        "{}{}{}{}<a:close/>",
        mv("xcr", "yc1"),
        arc("wd32", "q3", "0", "cd4"),
        arc("wd32", "q3", "3cd4", "ncd2"),
        ln("xcr", "yAa"),
    );
    let lines = format!(
        "{}{}{}{}",
        mv("xcr", "yc1"),
        ln("xcr", "yAa"),
        mv("xcl", "yc3"),
        ln("xcl", "yL1")
    );
    parse(&format!(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val 50000"/><a:gd name="adj2" fmla="val 50000"/><a:gd name="adj3" fmla="val 16667"/></a:avLst>
    <a:gdLst>{ANGLES}
      <a:gd name="a3p" fmla="pin 0 adj3 33333"/>
      <a:gd name="a3" fmla="*/ h a3p 100000"/>
      <a:gd name="x1max" fmla="+- hc 0 wd32"/>
      <a:gd name="x1raw" fmla="*/ ss adj2 100000"/>
      <a:gd name="x1a" fmla="max 0 x1raw"/>
      <a:gd name="x1" fmla="min x1a x1max"/>
      <a:gd name="x1r" fmla="+- r 0 x1"/>
      <a:gd name="a3h" fmla="*/ a3 1 2"/>
      <a:gd name="q3" fmla="*/ ss a3p 400000"/>
      <a:gd name="ytip" fmla="+- vc 0 a3h"/>
      <a:gd name="ytip2" fmla="+- vc a3h 0"/>
      <a:gd name="adj1m" fmla="*/ ytip 200000 h"/>
      <a:gd name="adj1p" fmla="pin 0 adj1 adj1m"/>
      <a:gd name="dsh" fmla="*/ h adj1p 200000"/>
      <a:gd name="yA" fmla="+- ytip 0 dsh"/>
      <a:gd name="yAa" fmla="+- yA a3 0"/>
      <a:gd name="yc1" fmla="+- yA q3 0"/>
      <a:gd name="yc3" fmla="+- yAa 0 q3"/>
      <a:gd name="yBot" fmla="+- b 0 yA"/>
      <a:gd name="yL1" fmla="+- yBot 0 a3"/>
      <a:gd name="yHb" fmla="+- b 0 a3"/>
      <a:gd name="xcr" fmla="+- hc wd32 0"/>
      <a:gd name="xcl" fmla="+- hc 0 wd32"/>
    </a:gdLst>
    <a:pathLst>
      <a:path stroke="0">{outline}</a:path>
      <a:path fill="darkenLess" stroke="0">{shade}</a:path>
      <a:path fill="none">{outline}{lines}</a:path>
    </a:pathLst></a:custGeom>"#,
    ))
}

// ---- scrolls -------------------------------------------------------------------------------

/// `verticalScroll`: one adjust, default 12500: the curl radius `rr = ss * adj / 100000`, clamped
/// to 0..25000 (30000 and 100000 both give `ss / 4`; -50000 gives the plain rectangle). Measured
/// outline: a sheet inset `rr` from the left and right, with a rolled top at the right (quarter
/// ellipses of radius `rr/2`) and a rolled bottom at the left, and inner spiral lines of radius
/// `rr/4`. Fill: outline and top curl disc (normal), the disc and the bottom roll (`darkenLess`);
/// strokes: outline, the top spiral, the line under the top roll, the bottom spiral and the bottom
/// roll edge.
fn vertical_scroll() -> Option<ShapeDef> {
    let outline_body = {
        let mut s = String::new();
        s += &ln("rr", "rh");
        s += &arc("rh", "rh", "cd2", "cd4");
        s += &ln("xrh", "t");
        s += &arc("rh", "rh", "3cd4", "cd4");
        s += &arc("rh", "rh", "0", "cd4");
        s += &ln("xr", "rr");
        s += &ln("xr", "ybh");
        s += &arc("rh", "rh", "0", "cd4");
        s
    };
    let outline = format!(
        "{}{}{}{}{}<a:close/>",
        mv("rr", "yb"),
        outline_body,
        ln("rh", "b"),
        arc("rh", "rh", "cd4", "cd4"),
        arc("rh", "rh", "cd2", "cd4"),
    );
    let disc = format!(
        "{}{}{}<a:close/>",
        mv("r2", "rh"),
        arc("rh", "rh", "0", "cd4"),
        arc("rq", "rq", "cd4", "cd2")
    );
    let fill_main = format!(
        "{}{}{}{}{}{}<a:close/>{}",
        mv("rh", "b"),
        arc("rh", "rh", "cd4", "ncd4"),
        ln("rh", "ybh"),
        arc("rq", "rq", "cd4", "ncd2"),
        ln("rr", "yb"),
        outline_body,
        disc
    );
    let roll = format!(
        "{}{}{}{}<a:close/>",
        mv("rr", "ybh"),
        arc("rh", "rh", "0", "cd2"),
        arc("rh", "rh", "cd2", "cd4"),
        arc("rq", "rq", "3cd4", "cd2"),
    );
    let strokes = format!(
        "{}{}{}{}{}{}{}{}{}{}{}{}",
        mv("r15", "t"),
        arc("rh", "rh", "3cd4", "cd2"),
        arc("rq", "rq", "cd4", "cd2"),
        ln("r2", "rh"),
        mv("xr", "rr"),
        ln("r15", "rr"),
        mv("rh", "yb"),
        arc("rq", "rq", "3cd4", "cd2"),
        ln("rr", "ybh"),
        mv("rh", "b"),
        arc("rh", "rh", "cd4", "ncd4"),
        ln("rr", "yb"),
    );
    parse(&format!(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 12500"/></a:avLst>
    <a:gdLst>{ANGLES}
      <a:gd name="ap" fmla="pin 0 adj 25000"/>
      <a:gd name="rr" fmla="*/ ss ap 100000"/>
      <a:gd name="rh" fmla="*/ rr 1 2"/>
      <a:gd name="rq" fmla="*/ rr 1 4"/>
      <a:gd name="r2" fmla="*/ rr 2 1"/>
      <a:gd name="r15" fmla="+- rr rh 0"/>
      <a:gd name="xr" fmla="+- r 0 rr"/>
      <a:gd name="xrh" fmla="+- r 0 rh"/>
      <a:gd name="yb" fmla="+- b 0 rr"/>
      <a:gd name="ybh" fmla="+- b 0 rh"/>
    </a:gdLst>
    <a:pathLst>
      <a:path stroke="0">{fill_main}</a:path>
      <a:path fill="darkenLess" stroke="0">{disc}{roll}</a:path>
      <a:path fill="none">{outline}{strokes}</a:path>
    </a:pathLst></a:custGeom>"#,
    ))
}

/// `horizontalScroll`: one adjust, default 12500, `rr = ss * adj / 100000` clamped to 0..25000 as
/// in `verticalScroll`. Measured outline: a sheet inset `rr` from the top and bottom, rolled at
/// the top right and bottom left, with spiral lines of radius `rr/4`. Fill: outline and the left
/// curl disc (normal); the disc and the right roll (`darkenLess`); strokes: outline, the line
/// under the right roll, the right spiral, the left spiral and the line right of the left roll.
fn horizontal_scroll() -> Option<ShapeDef> {
    let outline = format!(
        "{}{}{}{}{}{}{}{}{}{}{}{}{}<a:close/>",
        mv("l", "r15"),
        arc("rh", "rh", "cd2", "cd4"),
        ln("xr", "rr"),
        ln("xr", "rh"),
        arc("rh", "rh", "cd2", "cd4"),
        arc("rh", "rh", "3cd4", "cd4"),
        ln("r", "ybr"),
        arc("rh", "rh", "0", "cd4"),
        ln("rr", "yb"),
        ln("rr", "ybh"),
        arc("rh", "rh", "0", "cd4"),
        arc("rh", "rh", "cd4", "cd4"),
        ""
    );
    let disc_l = format!(
        "{}{}{}<a:close/>",
        mv("rh", "r2"),
        arc("rh", "rh", "cd4", "ncd4"),
        arc("rq", "rq", "0", "ncd2")
    );
    let disc_r = format!(
        "{}{}{}{}{}<a:close/>",
        mv("xrh", "rr"),
        arc("rh", "rh", "cd4", "ncd4"),
        arc("rh", "rh", "0", "ncd4"),
        arc("rh", "rh", "3cd4", "ncd4"),
        arc("rq", "rq", "cd2", "ncd2")
    );
    let fill_main = format!(
        "{}{}{}{}{}{}{}{}{}{}{}{}{}<a:close/>{}",
        mv("r", "rh"),
        arc("rh", "rh", "0", "cd4"),
        ln("xrh", "rh"),
        arc("rq", "rq", "0", "cd2"),
        ln("xr", "rr"),
        ln("rh", "rr"),
        arc("rh", "rh", "3cd4", "ncd4"),
        ln("l", "ybh"),
        arc("rh", "rh", "cd2", "ncd4"),
        arc("rh", "rh", "cd4", "ncd4"),
        ln("rr", "yb"),
        ln("xrh", "yb"),
        arc("rh", "rh", "cd4", "ncd4"),
        disc_l
    );
    let strokes = format!(
        "{}{}{}{}{}{}{}{}{}{}{}{}{}",
        mv("xr", "rr"),
        ln("xrh", "rr"),
        arc("rh", "rh", "cd4", "ncd4"),
        mv("xrh", "rr"),
        ln("xrh", "rh"),
        arc("rq", "rq", "0", "cd2"),
        mv("rh", "r2"),
        ln("rh", "r15"),
        arc("rq", "rq", "cd2", "cd2"),
        arc("rh", "rh", "0", "cd2"),
        mv("rr", "r15"),
        ln("rr", "yb"),
        ""
    );
    parse(&format!(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 12500"/></a:avLst>
    <a:gdLst>{ANGLES}
      <a:gd name="ap" fmla="pin 0 adj 25000"/>
      <a:gd name="rr" fmla="*/ ss ap 100000"/>
      <a:gd name="rh" fmla="*/ rr 1 2"/>
      <a:gd name="rq" fmla="*/ rr 1 4"/>
      <a:gd name="r2" fmla="*/ rr 2 1"/>
      <a:gd name="r15" fmla="+- rr rh 0"/>
      <a:gd name="xr" fmla="+- r 0 rr"/>
      <a:gd name="xrh" fmla="+- r 0 rh"/>
      <a:gd name="yb" fmla="+- b 0 rr"/>
      <a:gd name="ybh" fmla="+- b 0 rh"/>
      <a:gd name="ybr" fmla="+- b 0 r15"/>
    </a:gdLst>
    <a:pathLst>
      <a:path stroke="0">{fill_main}</a:path>
      <a:path fill="darkenLess" stroke="0">{disc_l}{disc_r}</a:path>
      <a:path fill="none">{outline}{strokes}</a:path>
    </a:pathLst></a:custGeom>"#,
    ))
}

// ---- waves ---------------------------------------------------------------------------------

/// `wave` and `doubleWave`: adjusts `adj1` (amplitude, defaults 12500 / 6250) and `adj2` (horizontal
/// shift, default 0). Measured: `a1 = h * adj1 / 100000` clamped to 0..20000 (wave) or 0..12500
/// (doubleWave); `dx = w * adj2 / 100000` clamped to -20000..20000. The top edge runs from
/// `(max(0,-dx), a1)` to `(min(w, w-dx), a1)`, the bottom edge back from `(min(w, w+dx), h - a1)`
/// to `(max(0, dx), h - a1)`; the sides are straight lines joining the ends. Each wave is one
/// cubic (two for `doubleWave`, one per half) with controls at thirds (sixths and thirds of each
/// half for the double wave), their y at `a1 - 10/3 a1` and `a1 + 10/3 a1` on the top edge and
/// mirrored on the bottom.
fn wave(double: bool) -> Option<ShapeDef> {
    let (a1_max, a1_def) = if double {
        (12_500, 6_250)
    } else {
        (20_000, 12_500)
    };
    let cubic = |x1: &str, y1: &str, x2: &str, y2: &str, x: &str, y: &str| {
        format!(
            r#"<a:cubicBezTo><a:pt x="{x1}" y="{y1}"/><a:pt x="{x2}" y="{y2}"/><a:pt x="{x}" y="{y}"/></a:cubicBezTo>"#
        )
    };
    let (top, bottom, gd_x) = if double {
        (
            format!(
                "{}{}",
                cubic("tc1", "yUp", "tc2", "yDn", "tmid", "a1"),
                cubic("tc3", "yUp", "tc4", "yDn", "xt1", "a1")
            ),
            format!(
                "{}{}",
                cubic("bc1", "yBDn", "bc2", "yBUp", "bmid", "yb"),
                cubic("bc3", "yBDn", "bc4", "yBUp", "xb0", "yb")
            ),
            [
                gd("tl6", "*/ tlen 1 6"),
                gd("tl3", "*/ tlen 1 3"),
                gd("bl6", "*/ blen 1 6"),
                gd("bl3", "*/ blen 1 3"),
                gd("tmid", "+/ xt0 xt1 2"),
                gd("tc1", "+- xt0 tl6 0"),
                gd("tc2", "+- xt0 tl3 0"),
                gd("tc3", "+- tmid tl6 0"),
                gd("tc4", "+- tmid tl3 0"),
                gd("bmid", "+/ xb0 xb1 2"),
                gd("bc1", "+- xb1 0 bl6"),
                gd("bc2", "+- xb1 0 bl3"),
                gd("bc3", "+- bmid 0 bl6"),
                gd("bc4", "+- bmid 0 bl3"),
            ]
            .concat(),
        )
    } else {
        (
            cubic("tc1", "yUp", "tc2", "yDn", "xt1", "a1"),
            cubic("bc1", "yBDn", "bc2", "yBUp", "xb0", "yb"),
            [
                gd("tl3", "*/ tlen 1 3"),
                gd("tl23", "*/ tlen 2 3"),
                gd("bl3", "*/ blen 1 3"),
                gd("bl23", "*/ blen 2 3"),
                gd("tc1", "+- xt0 tl3 0"),
                gd("tc2", "+- xt0 tl23 0"),
                gd("bc1", "+- xb1 0 bl3"),
                gd("bc2", "+- xb1 0 bl23"),
            ]
            .concat(),
        )
    };
    parse(&format!(
        r#"<a:custGeom><a:avLst><a:gd name="adj1" fmla="val {a1_def}"/><a:gd name="adj2" fmla="val 0"/></a:avLst>
    <a:gdLst>
      <a:gd name="a1p" fmla="pin 0 adj1 {a1_max}"/>
      <a:gd name="a1" fmla="*/ h a1p 100000"/>
      <a:gd name="dxp" fmla="pin -20000 adj2 20000"/>
      <a:gd name="dx" fmla="*/ w dxp 100000"/>
      <a:gd name="ndx" fmla="+- 0 0 dx"/>
      <a:gd name="xt0" fmla="max 0 ndx"/>
      <a:gd name="wmdx" fmla="+- w 0 dx"/>
      <a:gd name="xt1" fmla="min w wmdx"/>
      <a:gd name="xb0" fmla="max 0 dx"/>
      <a:gd name="wpdx" fmla="+- w dx 0"/>
      <a:gd name="xb1" fmla="min w wpdx"/>
      <a:gd name="tlen" fmla="+- xt1 0 xt0"/>
      <a:gd name="blen" fmla="+- xb1 0 xb0"/>
      <a:gd name="yb" fmla="+- b 0 a1"/>
      <a:gd name="yUp" fmla="*/ a1 -7 3"/>
      <a:gd name="yDn" fmla="*/ a1 13 3"/>
      <a:gd name="yBDn" fmla="+- b 0 yUp"/>
      <a:gd name="yBUp" fmla="+- b 0 yDn"/>
      {gd_x}
    </a:gdLst>
    <a:pathLst><a:path>
      <a:moveTo><a:pt x="xt0" y="a1"/></a:moveTo>{top}
      <a:lnTo><a:pt x="xb1" y="yb"/></a:lnTo>{bottom}
      <a:close/>
    </a:path></a:pathLst></a:custGeom>"#
    ))
}

// ---- plus and flowchart shapes ------------------------------------------------------------

/// `plus`: one adjust, default 25000. Measured: a twelve-point cross, `i = ss * adj / 100000` from
/// each corner, adj clamped to 0..50000 (100000 gives the half-size cross, 0 the full rectangle).
fn plus() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst><a:gd name="adj" fmla="val 25000"/></a:avLst>
    <a:gdLst>
      <a:gd name="ap" fmla="pin 0 adj 50000"/>
      <a:gd name="i" fmla="*/ ss ap 100000"/>
      <a:gd name="xr" fmla="+- r 0 i"/>
      <a:gd name="yb" fmla="+- b 0 i"/>
    </a:gdLst>
    <a:pathLst><a:path>
      <a:moveTo><a:pt x="l" y="i"/></a:moveTo><a:lnTo><a:pt x="i" y="i"/></a:lnTo><a:lnTo><a:pt x="i" y="t"/></a:lnTo>
      <a:lnTo><a:pt x="xr" y="t"/></a:lnTo><a:lnTo><a:pt x="xr" y="i"/></a:lnTo><a:lnTo><a:pt x="r" y="i"/></a:lnTo>
      <a:lnTo><a:pt x="r" y="yb"/></a:lnTo><a:lnTo><a:pt x="xr" y="yb"/></a:lnTo><a:lnTo><a:pt x="xr" y="b"/></a:lnTo>
      <a:lnTo><a:pt x="i" y="b"/></a:lnTo><a:lnTo><a:pt x="i" y="yb"/></a:lnTo><a:lnTo><a:pt x="l" y="yb"/></a:lnTo>
      <a:close/>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// A closed polygon on the 21600 grid as a `custGeom` with no adjusts.
fn grid_poly(points: &[(f64, f64)]) -> Option<ShapeDef> {
    let mut s = String::new();
    for (i, (x, y)) in points.iter().enumerate() {
        s += &if i == 0 {
            mv(&x.to_string(), &y.to_string())
        } else {
            ln(&x.to_string(), &y.to_string())
        };
    }
    parse(&format!(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path w="21600" h="21600">{s}<a:close/></a:path></a:pathLst></a:custGeom>"#
    ))
}

/// `flowChartProcess`: no adjusts; measured as the plain box.
fn flow_process() -> Option<ShapeDef> {
    grid_poly(&[
        (0.0, 0.0),
        (21600.0, 0.0),
        (21600.0, 21600.0),
        (0.0, 21600.0),
    ])
}

/// `flowChartAlternateProcess`: no adjusts; a rounded rectangle with corner radius `ss / 6`
/// (24 pt on 144, 16 pt on 96), quarter ellipses.
fn flow_alternate_process() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst>
      <a:gd name="rad" fmla="*/ ss 1 6"/><a:gd name="xr" fmla="+- r 0 rad"/><a:gd name="yb" fmla="+- b 0 rad"/>
    </a:gdLst><a:pathLst><a:path>
      <a:moveTo><a:pt x="l" y="rad"/></a:moveTo>
      <a:arcTo wR="rad" hR="rad" stAng="cd2" swAng="cd4"/><a:lnTo><a:pt x="xr" y="t"/></a:lnTo>
      <a:arcTo wR="rad" hR="rad" stAng="3cd4" swAng="cd4"/><a:lnTo><a:pt x="r" y="yb"/></a:lnTo>
      <a:arcTo wR="rad" hR="rad" stAng="0" swAng="cd4"/><a:lnTo><a:pt x="rad" y="b"/></a:lnTo>
      <a:arcTo wR="rad" hR="rad" stAng="cd4" swAng="cd4"/><a:close/>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartDecision`: no adjusts; a diamond through the edge midpoints.
fn flow_decision() -> Option<ShapeDef> {
    grid_poly(&[
        (0.0, 10800.0),
        (10800.0, 0.0),
        (21600.0, 10800.0),
        (10800.0, 21600.0),
    ])
}

/// `flowChartInputOutput`: no adjusts; a parallelogram, the top edge from `w/5` to the right
/// corner, the bottom edge from the left corner to `4w/5`.
fn flow_input_output() -> Option<ShapeDef> {
    grid_poly(&[
        (0.0, 21600.0),
        (4320.0, 0.0),
        (21600.0, 0.0),
        (17280.0, 21600.0),
    ])
}

/// `flowChartPredefinedProcess`: no adjusts; the box with vertical lines at `w/8` and `7w/8`.
fn flow_predefined() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst>
      <a:path w="21600" h="21600" stroke="0"><a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="21600" y="0"/></a:lnTo><a:lnTo><a:pt x="21600" y="21600"/></a:lnTo><a:lnTo><a:pt x="0" y="21600"/></a:lnTo><a:close/></a:path>
      <a:path w="21600" h="21600" fill="none">
        <a:moveTo><a:pt x="2700" y="0"/></a:moveTo><a:lnTo><a:pt x="2700" y="21600"/></a:lnTo>
        <a:moveTo><a:pt x="18900" y="0"/></a:moveTo><a:lnTo><a:pt x="18900" y="21600"/></a:lnTo>
      </a:path>
      <a:path w="21600" h="21600" fill="none"><a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="21600" y="0"/></a:lnTo><a:lnTo><a:pt x="21600" y="21600"/></a:lnTo><a:lnTo><a:pt x="0" y="21600"/></a:lnTo><a:close/></a:path>
    </a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartInternalStorage`: no adjusts; the box with a vertical line at `w/8` and a horizontal
/// line at `h/8`.
fn flow_internal_storage() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst>
      <a:path w="21600" h="21600" stroke="0"><a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="21600" y="0"/></a:lnTo><a:lnTo><a:pt x="21600" y="21600"/></a:lnTo><a:lnTo><a:pt x="0" y="21600"/></a:lnTo><a:close/></a:path>
      <a:path w="21600" h="21600" fill="none">
        <a:moveTo><a:pt x="2700" y="0"/></a:moveTo><a:lnTo><a:pt x="2700" y="21600"/></a:lnTo>
        <a:moveTo><a:pt x="0" y="2700"/></a:moveTo><a:lnTo><a:pt x="21600" y="2700"/></a:lnTo>
      </a:path>
      <a:path w="21600" h="21600" fill="none"><a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="21600" y="0"/></a:lnTo><a:lnTo><a:pt x="21600" y="21600"/></a:lnTo><a:lnTo><a:pt x="0" y="21600"/></a:lnTo><a:close/></a:path>
    </a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartDocument`: no adjusts; a box whose bottom edge is one S-shaped cubic: the right
/// side stops at 17322/21600 of the height, the cubic has controls `(w/2, 17322)` and
/// `(w/2, 23922)` and ends at `(0, 20172)` (21600 grid; fractions measured at four sizes).
fn flow_document() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path w="21600" h="21600">
      <a:moveTo><a:pt x="0" y="0"/></a:moveTo><a:lnTo><a:pt x="21600" y="0"/></a:lnTo><a:lnTo><a:pt x="21600" y="17322"/></a:lnTo>
      <a:cubicBezTo><a:pt x="10800" y="17322"/><a:pt x="10800" y="23922"/><a:pt x="0" y="20172"/></a:cubicBezTo>
      <a:close/>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartMultidocument`: no adjusts; three stacked documents on the 21600 grid. Front page:
/// `(0,3675)`, `(18595,3675)`, `(18595,18022.5)`, a cubic with controls `(9298,18022.5)`,
/// `(9298,23542.5)` to `(0,20782.5)`. The two pages behind are stroked as open polylines (their
/// hidden edges are not drawn) and filled closed.
fn flow_multidocument() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst>
      <a:path w="21600" h="21600" stroke="0">
        <a:moveTo><a:pt x="0" y="3675"/></a:moveTo><a:lnTo><a:pt x="18595" y="3675"/></a:lnTo><a:lnTo><a:pt x="18595" y="18022.5"/></a:lnTo>
        <a:cubicBezTo><a:pt x="9298" y="18022.5"/><a:pt x="9298" y="23542.5"/><a:pt x="0" y="20782.5"/></a:cubicBezTo><a:close/>
        <a:moveTo><a:pt x="1532" y="3675"/></a:moveTo><a:lnTo><a:pt x="1532" y="1815"/></a:lnTo><a:lnTo><a:pt x="20000" y="1815"/></a:lnTo><a:lnTo><a:pt x="20000" y="16252.5"/></a:lnTo>
        <a:cubicBezTo><a:pt x="19298" y="16252.5"/><a:pt x="18595" y="16351.5"/><a:pt x="18595" y="16351.5"/></a:cubicBezTo><a:lnTo><a:pt x="18595" y="3675"/></a:lnTo><a:close/>
        <a:moveTo><a:pt x="2972" y="1815"/></a:moveTo><a:lnTo><a:pt x="2972" y="0"/></a:lnTo><a:lnTo><a:pt x="21600" y="0"/></a:lnTo><a:lnTo><a:pt x="21600" y="14392.5"/></a:lnTo>
        <a:cubicBezTo><a:pt x="20800" y="14392.5"/><a:pt x="20000" y="14467.5"/><a:pt x="20000" y="14467.5"/></a:cubicBezTo><a:lnTo><a:pt x="20000" y="1815"/></a:lnTo><a:close/>
      </a:path>
      <a:path w="21600" h="21600" fill="none">
        <a:moveTo><a:pt x="0" y="3675"/></a:moveTo><a:lnTo><a:pt x="18595" y="3675"/></a:lnTo><a:lnTo><a:pt x="18595" y="18022.5"/></a:lnTo>
        <a:cubicBezTo><a:pt x="9298" y="18022.5"/><a:pt x="9298" y="23542.5"/><a:pt x="0" y="20782.5"/></a:cubicBezTo><a:close/>
        <a:moveTo><a:pt x="1532" y="3675"/></a:moveTo><a:lnTo><a:pt x="1532" y="1815"/></a:lnTo><a:lnTo><a:pt x="20000" y="1815"/></a:lnTo><a:lnTo><a:pt x="20000" y="16252.5"/></a:lnTo>
        <a:cubicBezTo><a:pt x="19298" y="16252.5"/><a:pt x="18595" y="16351.5"/><a:pt x="18595" y="16351.5"/></a:cubicBezTo>
        <a:moveTo><a:pt x="2972" y="1815"/></a:moveTo><a:lnTo><a:pt x="2972" y="0"/></a:lnTo><a:lnTo><a:pt x="21600" y="0"/></a:lnTo><a:lnTo><a:pt x="21600" y="14392.5"/></a:lnTo>
        <a:cubicBezTo><a:pt x="20800" y="14392.5"/><a:pt x="20000" y="14467.5"/><a:pt x="20000" y="14467.5"/></a:cubicBezTo>
      </a:path>
    </a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartTerminator`: no adjusts; a stadium: top and bottom edges from `3475/21600 w` to
/// `18125/21600 w`, the ends are half ellipses of horizontal radius `3475/21600 w` and vertical
/// radius `h/2`.
fn flow_terminator() -> Option<ShapeDef> {
    parse(
        r#"<a:custGeom><a:avLst/><a:gdLst/><a:pathLst><a:path w="21600" h="21600">
      <a:moveTo><a:pt x="3475" y="0"/></a:moveTo><a:lnTo><a:pt x="18125" y="0"/></a:lnTo>
      <a:arcTo wR="3475" hR="10800" stAng="3cd4" swAng="cd2"/>
      <a:lnTo><a:pt x="3475" y="21600"/></a:lnTo>
      <a:arcTo wR="3475" hR="10800" stAng="cd4" swAng="cd2"/>
      <a:close/>
    </a:path></a:pathLst></a:custGeom>"#,
    )
}

/// `flowChartPreparation`: no adjusts; a hexagon with the left and right points at mid height and
/// the top and bottom corners inset `w/5`.
fn flow_preparation() -> Option<ShapeDef> {
    grid_poly(&[
        (0.0, 10800.0),
        (4320.0, 0.0),
        (17280.0, 0.0),
        (21600.0, 10800.0),
        (17280.0, 21600.0),
        (4320.0, 21600.0),
    ])
}

/// `flowChartManualInput`: no adjusts; a box whose top edge slopes from `h/5` on the left to 0 on
/// the right.
fn flow_manual_input() -> Option<ShapeDef> {
    grid_poly(&[
        (0.0, 4320.0),
        (21600.0, 0.0),
        (21600.0, 21600.0),
        (0.0, 21600.0),
    ])
}

/// The b06 entries.
pub fn entries() -> Vec<(&'static str, ShapeDef)> {
    let all = [
        ("cloudCallout", cloud_callout()),
        ("cloud", cloud()),
        ("ribbon", ribbon(false)),
        ("ribbon2", ribbon(true)),
        ("ellipseRibbon", ellipse_ribbon(false)),
        ("ellipseRibbon2", ellipse_ribbon(true)),
        ("leftRightRibbon", left_right_ribbon()),
        ("verticalScroll", vertical_scroll()),
        ("horizontalScroll", horizontal_scroll()),
        ("wave", wave(false)),
        ("doubleWave", wave(true)),
        ("plus", plus()),
        ("flowChartProcess", flow_process()),
        ("flowChartAlternateProcess", flow_alternate_process()),
        ("flowChartDecision", flow_decision()),
        ("flowChartInputOutput", flow_input_output()),
        ("flowChartPredefinedProcess", flow_predefined()),
        ("flowChartInternalStorage", flow_internal_storage()),
        ("flowChartDocument", flow_document()),
        ("flowChartMultidocument", flow_multidocument()),
        ("flowChartTerminator", flow_terminator()),
        ("flowChartPreparation", flow_preparation()),
        ("flowChartManualInput", flow_manual_input()),
    ];
    all.into_iter()
        .filter_map(|(n, d)| d.map(|d| (n, d)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_b06_entries_parse() {
        assert_eq!(entries().len(), 23);
    }
}
