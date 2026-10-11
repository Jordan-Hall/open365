//! Word's rulers as pure geometry: no egui types, every length in `u` (one physical pixel at 125 % display
//! scale, 0.8 egui points), so another renderer can reuse it. Measured from Word through the oracle
//! (`fixtures/reference/ui/word-ruler.json`, dark mode, 100 % zoom, A4, 72 pt margins), see
//! `docs/architecture/app-shell.md`.
//!
//! Coordinates: `x` is measured from the left outline of the page the ruler belongs to, `y` from the top of the
//! horizontal band (the vertical ruler uses the same numbers across: `y` of the band is `x` of the vertical band
//! and `x` along the page is `y` down the page).

/// Horizontal ruler band height (the page moves down by this much when the ruler is shown).
pub const BAND_U: f64 = 30.0;
/// Vertical ruler width (the page moves right by half of it, because it stays centred in what is left).
pub const VBAND_U: f64 = 30.0;
/// Space between the top of the document area (or the bottom of the ruler band) and the page outline.
pub const PAGE_TOP_U: f64 = 21.0;
/// `u` per document point at 100 % zoom (96 dpi, 125 % reference scale): 120 u for a 72 pt margin.
pub const U_PER_PT: f64 = 96.0 / 72.0 / 0.8;
/// Points per centimetre.
pub const PT_PER_CM: f64 = 72.0 / 2.54;
/// Tick zero lies this far left of the margin edge (426.2 for a margin at 428).
pub const ZERO_SHIFT_U: f64 = -1.8;
/// The white track starts 1 u before the left margin and ends 2 u before the right margin.
pub const TRACK_LEFT_U: f64 = -1.0;
pub const TRACK_RIGHT_U: f64 = -2.0;
/// Rows of the track inside the band.
pub const TRACK_Y: (f64, f64) = (4.0, 22.0);
/// Tall (half centimetre) tick, quarter-centimetre tick and digit rows.
pub const TICK_TALL_Y: (f64, f64) = (11.0, 17.0);
pub const TICK_SMALL_Y: (f64, f64) = (13.0, 15.0);
pub const DIGIT_Y: (f64, f64) = (9.0, 18.0);
/// Marker width.
pub const MARKER_W: f64 = 14.0;
/// Dragging snaps to a quarter of a centimetre, the finest tick.
pub const SNAP_PT: f64 = PT_PER_CM / 4.0;
/// Default tab stop when the document sets none (720 twips).
pub const DEFAULT_TAB_PT: f64 = 36.0;

/// A rectangle `[x0, x1) x [y0, y1)` in `u`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct URect {
    pub x0: f64,
    pub x1: f64,
    pub y0: f64,
    pub y1: f64,
}

impl URect {
    #[must_use]
    pub const fn new(x0: f64, x1: f64, y0: f64, y1: f64) -> Self {
        Self { x0, x1, y0, y1 }
    }

    #[must_use]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

/// Tab stop kinds the ruler shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RulerTab {
    #[default]
    Left,
    Center,
    Right,
    Decimal,
    Bar,
}

impl RulerTab {
    /// The corner button cycles through these.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Left => Self::Center,
            Self::Center => Self::Right,
            Self::Right => Self::Decimal,
            Self::Decimal | Self::Bar => Self::Left,
        }
    }
}

/// One custom tab stop, position in points from the left margin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RulerTabStop {
    pub pos_pt: f64,
    pub kind: RulerTab,
}

/// What the ruler shows: the page of the paragraph holding the caret and that paragraph's indents and tabs.
#[derive(Debug, Clone, PartialEq)]
pub struct RulerInfo {
    /// Paragraph index the info belongs to (the caret's paragraph).
    pub para: usize,
    pub page: usize,
    pub page_width_pt: f64,
    pub page_height_pt: f64,
    pub margin_left_pt: f64,
    pub margin_right_pt: f64,
    pub margin_top_pt: f64,
    pub margin_bottom_pt: f64,
    pub indent_left_pt: f64,
    /// First-line indent; negative for a hanging indent.
    pub first_line_pt: f64,
    pub indent_right_pt: f64,
    pub tabs: Vec<RulerTabStop>,
}

/// Tick sizes along a ruler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickKind {
    /// Half centimetre (and whole centimetres inside the margins).
    Tall,
    /// Quarter centimetre.
    Small,
}

/// One tick: position along the ruler.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    pub at: f64,
    pub kind: TickKind,
}

/// A centimetre number centred on `at`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Digit {
    pub at: f64,
    pub number: u32,
}

/// Ticks and digits along one axis. `zero` is the tick for 0 cm, `text` the text area (digits only inside it),
/// `extent` the page length. Ticks run edge to edge.
#[must_use]
pub fn ticks_along(zero: f64, text: (f64, f64), extent: f64, k: f64) -> (Vec<Tick>, Vec<Digit>) {
    let quarter = PT_PER_CM * k / 4.0;
    let mut ticks = Vec::new();
    let mut digits = Vec::new();
    let lo = ((0.0 - zero) / quarter).ceil() as i64;
    let hi = ((extent - zero) / quarter).floor() as i64;
    for i in lo..=hi {
        if i == 0 {
            continue;
        }
        let at = zero + i as f64 * quarter;
        let cm = i.rem_euclid(4) == 0;
        if cm && at >= text.0 && at <= text.1 {
            digits.push(Digit {
                at,
                number: (i / 4).unsigned_abs() as u32,
            });
        } else if cm || i.rem_euclid(2) == 0 {
            ticks.push(Tick {
                at,
                kind: TickKind::Tall,
            });
        } else {
            ticks.push(Tick {
                at,
                kind: TickKind::Small,
            });
        }
    }
    (ticks, digits)
}

/// Which marker of the horizontal ruler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    FirstLine,
    Hanging,
    LeftBox,
    Right,
}

/// A marker: centre x and the polygon outline (x, y pairs, `u`, band-relative) plus an optional box under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    pub kind: MarkerKind,
    pub cx: f64,
    pub outline: Vec<(f64, f64)>,
    /// The rectangle under the hanging marker that moves both indents.
    pub box_rect: Option<URect>,
}

impl Marker {
    fn first_line(cx: f64) -> Self {
        let h = MARKER_W / 2.0;
        Self {
            kind: MarkerKind::FirstLine,
            cx,
            outline: vec![
                (cx - h, 2.0),
                (cx + h, 2.0),
                (cx + h, 8.0),
                (cx, 13.0),
                (cx - h, 8.0),
            ],
            box_rect: None,
        }
    }

    fn up(kind: MarkerKind, cx: f64, with_box: bool) -> Self {
        let h = MARKER_W / 2.0;
        Self {
            kind,
            cx,
            outline: vec![
                (cx, 13.0),
                (cx + h, 19.0),
                (cx + h, 23.0),
                (cx - h, 23.0),
                (cx - h, 19.0),
            ],
            box_rect: with_box.then(|| URect::new(cx - h, cx + h, 23.0, 29.0)),
        }
    }

    /// Hit rectangle.
    #[must_use]
    pub fn bounds(&self) -> URect {
        let h = MARKER_W / 2.0;
        match self.kind {
            MarkerKind::FirstLine => URect::new(self.cx - h, self.cx + h, 2.0, 13.0),
            MarkerKind::Hanging | MarkerKind::Right => {
                URect::new(self.cx - h, self.cx + h, 13.0, 23.0)
            }
            MarkerKind::LeftBox => URect::new(self.cx - h, self.cx + h, 23.0, 29.0),
        }
    }
}

/// Rectangles of one tab glyph (stem and foot), plus the decimal dot.
#[derive(Debug, Clone, PartialEq)]
pub struct TabGlyph {
    pub kind: RulerTab,
    pub at: f64,
    pub stem: URect,
    pub foot: URect,
    pub dot: Option<URect>,
}

/// The measured glyph of the ruler (2 u strokes): stem rows 16..22 of the band, foot rows 22..24.
fn ruler_glyph(kind: RulerTab, p: f64) -> TabGlyph {
    let stem = URect::new(p - 1.0, p + 1.0, 16.0, 22.0);
    let fy = (22.0, 24.0);
    let foot = match kind {
        RulerTab::Left | RulerTab::Bar => URect::new(p - 1.0, p + 7.0, fy.0, fy.1),
        RulerTab::Right => URect::new(p - 7.0, p + 1.0, fy.0, fy.1),
        RulerTab::Center | RulerTab::Decimal => URect::new(p - 5.0, p + 5.0, fy.0, fy.1),
    };
    let dot = (kind == RulerTab::Decimal).then(|| URect::new(p + 3.0, p + 5.0, 19.0, 21.0));
    TabGlyph {
        kind,
        at: p,
        stem,
        foot,
        dot,
    }
}

/// The tab selector in the corner (band-relative to the corner's top left): the measured left glyph, 3 u strokes.
#[must_use]
pub fn corner_glyph(kind: RulerTab) -> TabGlyph {
    let p = 11.0;
    let stem = URect::new(p - 1.0, p + 2.0, 12.0, 20.0);
    let fy = (17.0, 20.0);
    let foot = match kind {
        RulerTab::Left | RulerTab::Bar => URect::new(p - 1.0, p + 7.0, fy.0, fy.1),
        RulerTab::Right => URect::new(p - 6.0, p + 2.0, fy.0, fy.1),
        RulerTab::Center | RulerTab::Decimal => URect::new(p - 4.0, p + 6.0, fy.0, fy.1),
    };
    let dot = (kind == RulerTab::Decimal).then(|| URect::new(p + 4.0, p + 6.0, 12.0, 14.0));
    TabGlyph {
        kind,
        at: p,
        stem,
        foot,
        dot,
    }
}

/// Everything the horizontal ruler draws, `x` from the page outline.
#[derive(Debug, Clone, PartialEq)]
pub struct HRuler {
    /// Page width in `u`.
    pub page_w: f64,
    /// White track (the text area) from x0 to x1.
    pub track: URect,
    pub ticks: Vec<Tick>,
    pub digits: Vec<Digit>,
    pub first_line: Marker,
    pub hanging: Marker,
    pub left_box: Marker,
    pub right: Marker,
    pub tabs: Vec<TabGlyph>,
}

/// Geometry of the horizontal ruler for `info` at `k` `u` per document point.
#[must_use]
pub fn horizontal(info: &RulerInfo, k: f64) -> HRuler {
    let page_w = info.page_width_pt * k;
    let ml = info.margin_left_pt * k;
    let mr_edge = page_w - info.margin_right_pt * k;
    let zero = ml + ZERO_SHIFT_U;
    let (ticks, digits) = ticks_along(zero, (zero, mr_edge), page_w, k);
    let left = ml + info.indent_left_pt * k;
    let first = left + info.first_line_pt * k;
    let right = mr_edge - info.indent_right_pt * k;
    let hang = Marker::up(MarkerKind::Hanging, left, false);
    let boxed = Marker::up(MarkerKind::LeftBox, left, true);
    HRuler {
        page_w,
        track: URect::new(
            ml + TRACK_LEFT_U,
            mr_edge + TRACK_RIGHT_U,
            TRACK_Y.0,
            TRACK_Y.1,
        ),
        ticks,
        digits,
        first_line: Marker::first_line(first),
        hanging: hang,
        left_box: boxed,
        right: Marker::up(MarkerKind::Right, right, false),
        tabs: info
            .tabs
            .iter()
            .map(|t| ruler_glyph(t.kind, ml + t.pos_pt * k))
            .collect(),
    }
}

/// Geometry of the vertical ruler of one page: `y` from the page outline's top, `x` across the 30 u band.
#[derive(Debug, Clone, PartialEq)]
pub struct VRuler {
    pub page_h: f64,
    pub track: URect,
    pub ticks: Vec<Tick>,
    pub digits: Vec<Digit>,
}

#[must_use]
pub fn vertical(info: &RulerInfo, k: f64) -> VRuler {
    let page_h = info.page_height_pt * k;
    let mt = info.margin_top_pt * k;
    let bottom_edge = page_h - info.margin_bottom_pt * k;
    let zero = mt + ZERO_SHIFT_U;
    let (ticks, digits) = ticks_along(zero, (zero, bottom_edge), page_h, k);
    VRuler {
        page_h,
        track: URect::new(
            TRACK_Y.0,
            TRACK_Y.1,
            mt + TRACK_LEFT_U,
            bottom_edge + TRACK_RIGHT_U,
        ),
        ticks,
        digits,
    }
}

/// What the pointer is over in the horizontal band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RulerHit {
    Marker(MarkerKind),
    /// A tab glyph, by index into [`RulerInfo::tabs`].
    Tab(usize),
    /// The corner selector.
    Corner,
    /// Empty track: a click adds a tab here (`x` from the page outline).
    Track(f64),
}

/// Width of the corner selector.
pub const CORNER_U: f64 = 30.0;

/// Hit test at `(x, y)`: `x` from the page outline, `y` from the band top. Markers win over tabs over the track.
#[must_use]
pub fn hit_test(h: &HRuler, x: f64, y: f64) -> Option<RulerHit> {
    for m in [&h.first_line, &h.hanging, &h.left_box, &h.right] {
        if m.bounds().contains(x, y) {
            return Some(RulerHit::Marker(m.kind));
        }
    }
    for (i, t) in h.tabs.iter().enumerate() {
        if URect::new(t.at - 5.0, t.at + 5.0, 14.0, 26.0).contains(x, y) {
            return Some(RulerHit::Tab(i));
        }
    }
    (x >= h.track.x0 && x < h.track.x1 && (TRACK_Y.0..TRACK_Y.1).contains(&y))
        .then_some(RulerHit::Track(x))
}

/// An edit the ruler asks the command layer for.
#[derive(Debug, Clone, PartialEq)]
pub enum RulerEdit {
    /// Left indent, first-line indent (negative: hanging) and right indent of one paragraph, in points.
    SetIndents {
        para: usize,
        left_pt: f64,
        first_line_pt: f64,
        right_pt: f64,
    },
    AddTab {
        para: usize,
        pos_pt: f64,
        kind: RulerTab,
    },
    MoveTab {
        para: usize,
        from_pt: f64,
        to_pt: f64,
    },
    ClearTab {
        para: usize,
        pos_pt: f64,
    },
}

/// Snaps a length in points to the quarter-centimetre grid.
#[must_use]
pub fn snap_pt(pt: f64) -> f64 {
    (pt / SNAP_PT).round() * SNAP_PT
}

/// The edit a drag of `hit` produces when released at `x` (from the page outline) and `y` (from the band top).
/// `None` when nothing changes.
#[must_use]
pub fn drag_edit(info: &RulerInfo, k: f64, hit: RulerHit, x: f64, y: f64) -> Option<RulerEdit> {
    let ml = info.margin_left_pt * k;
    let text_w_pt = info.page_width_pt - info.margin_left_pt - info.margin_right_pt;
    let rel_pt = snap_pt((x - ml) / k);
    let first_abs = info.indent_left_pt + info.first_line_pt;
    let edit = match hit {
        RulerHit::Marker(MarkerKind::FirstLine) => {
            let abs = rel_pt.clamp(-info.margin_left_pt, text_w_pt);
            RulerEdit::SetIndents {
                para: info.para,
                left_pt: info.indent_left_pt,
                first_line_pt: abs - info.indent_left_pt,
                right_pt: info.indent_right_pt,
            }
        }
        RulerHit::Marker(MarkerKind::Hanging) => {
            let left = rel_pt.clamp(-info.margin_left_pt, text_w_pt);
            RulerEdit::SetIndents {
                para: info.para,
                left_pt: left,
                first_line_pt: first_abs - left,
                right_pt: info.indent_right_pt,
            }
        }
        RulerHit::Marker(MarkerKind::LeftBox) => RulerEdit::SetIndents {
            para: info.para,
            left_pt: rel_pt.clamp(-info.margin_left_pt, text_w_pt),
            first_line_pt: info.first_line_pt,
            right_pt: info.indent_right_pt,
        },
        RulerHit::Marker(MarkerKind::Right) => {
            let from_right = snap_pt(text_w_pt - (x - ml) / k);
            RulerEdit::SetIndents {
                para: info.para,
                left_pt: info.indent_left_pt,
                first_line_pt: info.first_line_pt,
                right_pt: from_right.clamp(-info.margin_right_pt, text_w_pt),
            }
        }
        RulerHit::Tab(i) => {
            let from = info.tabs.get(i)?.pos_pt;
            // Dragging a tab off the band removes it.
            if !(-6.0..BAND_U + 6.0).contains(&y) {
                RulerEdit::ClearTab {
                    para: info.para,
                    pos_pt: from,
                }
            } else {
                let to = rel_pt.clamp(0.0, text_w_pt);
                if (to - from).abs() < 1e-6 {
                    return None;
                }
                RulerEdit::MoveTab {
                    para: info.para,
                    from_pt: from,
                    to_pt: to,
                }
            }
        }
        RulerHit::Corner | RulerHit::Track(_) => return None,
    };
    match &edit {
        RulerEdit::SetIndents {
            left_pt,
            first_line_pt,
            right_pt,
            ..
        } if (*left_pt - info.indent_left_pt).abs() < 1e-6
            && (*first_line_pt - info.first_line_pt).abs() < 1e-6
            && (*right_pt - info.indent_right_pt).abs() < 1e-6 =>
        {
            None
        }
        _ => Some(edit),
    }
}

/// A click on empty track adds a tab of the corner's kind there (unless one already sits at that position).
#[must_use]
pub fn click_edit(info: &RulerInfo, k: f64, x: f64, kind: RulerTab) -> Option<RulerEdit> {
    let text_w_pt = info.page_width_pt - info.margin_left_pt - info.margin_right_pt;
    let pos = snap_pt((x - info.margin_left_pt * k) / k).clamp(0.0, text_w_pt);
    if info.tabs.iter().any(|t| (t.pos_pt - pos).abs() < 0.5) {
        return None;
    }
    Some(RulerEdit::AddTab {
        para: info.para,
        pos_pt: pos,
        kind,
    })
}

/// `info` as it looks with `edit` applied (the live preview while a marker is dragged).
#[must_use]
pub fn preview(info: &RulerInfo, edit: &RulerEdit) -> RulerInfo {
    let mut out = info.clone();
    match *edit {
        RulerEdit::SetIndents {
            left_pt,
            first_line_pt,
            right_pt,
            ..
        } => {
            out.indent_left_pt = left_pt;
            out.first_line_pt = first_line_pt;
            out.indent_right_pt = right_pt;
        }
        RulerEdit::AddTab { pos_pt, kind, .. } => {
            out.tabs.push(RulerTabStop { pos_pt, kind });
        }
        RulerEdit::MoveTab { from_pt, to_pt, .. } => {
            for t in &mut out.tabs {
                if (t.pos_pt - from_pt).abs() < 1e-6 {
                    t.pos_pt = to_pt;
                }
            }
        }
        RulerEdit::ClearTab { pos_pt, .. } => {
            out.tabs.retain(|t| (t.pos_pt - pos_pt).abs() >= 1e-6);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn fixture() -> Value {
        let path = format!(
            "{}/../../fixtures/reference/ui/word-ruler.json",
            env!("CARGO_MANIFEST_DIR")
        );
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
    }

    /// Brightness (first channel) per x of window row `y` in state `state`.
    fn row(f: &Value, state: usize, y: i32) -> std::collections::HashMap<i32, u8> {
        let mut out = std::collections::HashMap::new();
        for r in f["states"][state]["rows"][format!("y{y}")]
            .as_array()
            .expect("row")
        {
            let (s, n) = (r[0].as_i64().unwrap() as i32, r[1].as_i64().unwrap() as i32);
            let v = u8::from_str_radix(&r[2].as_str().unwrap()[..2], 16).unwrap();
            for x in s..s + n {
                out.insert(x, v);
            }
        }
        out
    }

    const BAND_TOP: i32 = 222;
    /// Page outline (left) in the fixture window with the ruler on, and the margin edge.
    const PAGE_LEFT: f64 = 308.0;

    fn info(left: f64, first: f64, right: f64, tabs: Vec<RulerTabStop>) -> RulerInfo {
        RulerInfo {
            para: 0,
            page: 0,
            page_width_pt: 595.3,
            page_height_pt: 841.9,
            margin_left_pt: 72.0,
            margin_right_pt: 72.0,
            margin_top_pt: 72.0,
            margin_bottom_pt: 72.0,
            indent_left_pt: left,
            first_line_pt: first,
            indent_right_pt: right,
            tabs,
        }
    }

    /// Bright run `[a, b)` of pixels at least `min` bright in row `y` between `lo` and `hi`.
    fn extent(f: &Value, state: usize, y: i32, lo: i32, hi: i32, min: u8) -> Option<(i32, i32)> {
        let r = row(f, state, y);
        let xs: Vec<i32> = (lo..hi)
            .filter(|x| r.get(x).copied().unwrap_or(0) >= min)
            .collect();
        Some((*xs.first()?, *xs.last()? + 1))
    }

    fn close(what: &str, ours: f64, theirs: i32) {
        assert!(
            (ours - f64::from(theirs)).abs() <= 1.0,
            "{what}: ours {ours} reference {theirs}"
        );
    }

    #[test]
    fn plain_paragraph_matches_word() {
        let f = fixture();
        let h = horizontal(&info(0.0, 0.0, 0.0, vec![]), U_PER_PT);
        // Track fill: row 230 holds 0x29 from the first marker to the right margin.
        let r = row(&f, 0, 227);
        let right_end = (1100..1300)
            .rev()
            .find(|x| r.get(x).copied() == Some(0x26))
            .expect("track end")
            + 1;
        close("track right", PAGE_LEFT + h.track.x1, right_end);
        let m = row(&f, 1, 227);
        let left_start = (300..520)
            .find(|x| m.get(x).copied() == Some(0x26))
            .expect("track start");
        close("track left", PAGE_LEFT + h.track.x0, left_start);
        // Markers: first-line 421..435, right 1173..1187.
        let e = extent(&f, 0, 227, 400, 460, 0x40).expect("first");
        close("first-line left", PAGE_LEFT + h.first_line.cx - 7.0, e.0);
        close("first-line right", PAGE_LEFT + h.first_line.cx + 7.0, e.1);
        let e = extent(&f, 0, 241, 1150, 1210, 0x40).expect("right");
        close("right left", PAGE_LEFT + h.right.cx - 7.0, e.0);
        close("right right", PAGE_LEFT + h.right.cx + 7.0, e.1);
        // Band rows: the track spans rows 226..244, the hanging box rows 246..251.
        assert_eq!(TRACK_Y, (4.0, 22.0));
        assert_eq!(f64::from(BAND_TOP) + TRACK_Y.0, 226.0);
        assert_eq!(h.left_box.box_rect.expect("box").y1, 29.0);
    }

    #[test]
    fn ticks_match_word() {
        let f = fixture();
        let h = horizontal(&info(0.0, 0.0, 0.0, vec![]), U_PER_PT);
        let tall = row(&f, 0, 233);
        let small = row(&f, 0, 235);
        let mut checked = 0;
        for t in &h.ticks {
            let x = PAGE_LEFT + t.at;
            // Skip what the markers cover.
            if !(300.0..415.0).contains(&x) && !(442.0..1165.0).contains(&x) {
                continue;
            }
            let rows = if t.kind == TickKind::Tall {
                &tall
            } else {
                &small
            };
            let near = (-1..=1).any(|d| rows.get(&(x.round() as i32 + d)).copied() == Some(0xff));
            assert!(near, "tick at {x} ({:?}) has no white pixel", t.kind);
            checked += 1;
        }
        assert!(checked > 50, "{checked}");
        // And every white pixel of the tall row (the digits sit above row 233 only at whole centimetres).
        let ours: Vec<f64> = h
            .ticks
            .iter()
            .filter(|t| t.kind == TickKind::Tall)
            .map(|t| PAGE_LEFT + t.at)
            .collect();
        for (&x, &v) in &tall {
            if v == 0xff && (300..415).contains(&x) {
                assert!(
                    ours.iter().any(|o| (o - f64::from(x)).abs() <= 1.5),
                    "reference tick at {x} missing"
                );
            }
        }
        // Whole centimetres in the text area carry digits, none in the margins.
        assert_eq!(h.digits.first().map(|d| d.number), Some(1));
        assert_eq!(h.digits.last().map(|d| d.number), Some(15));
        // Digit rows.
        let digit_row = (DIGIT_Y.0 as i32 + BAND_TOP..DIGIT_Y.1 as i32 + BAND_TOP)
            .any(|y| extent(&f, 0, y, 1128, 1143, 0xe0).is_some());
        assert!(digit_row, "digit 15 pixels at the expected rows");
    }

    #[test]
    fn indent_markers_and_tabs_match_word() {
        let f = fixture();
        let tabs = vec![
            RulerTabStop {
                pos_pt: 144.0,
                kind: RulerTab::Left,
            },
            RulerTabStop {
                pos_pt: 216.0,
                kind: RulerTab::Center,
            },
            RulerTabStop {
                pos_pt: 288.0,
                kind: RulerTab::Right,
            },
            RulerTabStop {
                pos_pt: 360.0,
                kind: RulerTab::Decimal,
            },
        ];
        let h = horizontal(&info(36.0, 18.0, 36.0, tabs), U_PER_PT);
        let e = extent(&f, 1, 227, 480, 560, 0x40).expect("first");
        close("first-line", PAGE_LEFT + h.first_line.cx, (e.0 + e.1) / 2);
        let e = extent(&f, 1, 243, 460, 520, 0x40).expect("hanging");
        close("hanging", PAGE_LEFT + h.hanging.cx, (e.0 + e.1) / 2);
        let e = extent(&f, 1, 248, 460, 520, 0x40).expect("box");
        close("box left", PAGE_LEFT + h.left_box.cx - 7.0, e.0);
        let e = extent(&f, 1, 241, 1060, 1130, 0x40).expect("right");
        close("right", PAGE_LEFT + h.right.cx, (e.0 + e.1) / 2);
        // Glyphs: bright pixels of rows 238..246 around each stop.
        for g in &h.tabs {
            let p = PAGE_LEFT + g.at;
            let lo = (p - 14.0) as i32;
            let hi = (p + 14.0) as i32;
            let stem = extent(&f, 1, BAND_TOP + 18, lo, hi, 0x70).expect("stem");
            close("stem left", PAGE_LEFT + g.stem.x0, stem.0);
            close("stem right", PAGE_LEFT + g.stem.x1, stem.1);
            let foot = extent(&f, 1, BAND_TOP + 23, lo, hi, 0x70).expect("foot");
            close("foot left", PAGE_LEFT + g.foot.x0, foot.0);
            close("foot right", PAGE_LEFT + g.foot.x1, foot.1);
            assert_eq!(g.stem.y0, 16.0);
            assert_eq!(g.foot.y1, 24.0);
            if g.kind == RulerTab::Decimal {
                let dot = g.dot.expect("dot");
                let d = extent(&f, 1, BAND_TOP + 19, lo, hi, 0x70).expect("dot row");
                assert!(
                    PAGE_LEFT + dot.x0 >= f64::from(d.0) - 1.0
                        && PAGE_LEFT + dot.x1 <= f64::from(d.1) + 1.0,
                    "dot {dot:?} vs {d:?}"
                );
            }
        }
    }

    #[test]
    fn corner_shows_the_left_glyph() {
        let f = fixture();
        let g = corner_glyph(RulerTab::Left);
        // Window x of the corner is 9; stem x 19..22 at rows 234..242, foot 20..27 at rows 239..242.
        let stem = extent(&f, 0, 236, 9, 39, 0xc0).expect("stem");
        close("corner stem", 9.0 + g.stem.x0, stem.0);
        close("corner stem right", 9.0 + g.stem.x1, stem.1);
        let foot = extent(&f, 0, 240, 9, 39, 0xc0).expect("foot");
        close("corner foot", 9.0 + g.foot.x1, foot.1);
        assert_eq!((g.stem.y0, g.foot.y1), (12.0, 20.0));
    }

    #[test]
    fn dragging_moves_markers_like_word() {
        let i = info(36.0, 18.0, 36.0, vec![]);
        let k = U_PER_PT;
        let ml = 72.0 * k;
        // First-line marker to 1 cm from the margin.
        let x = ml + PT_PER_CM * k;
        let e = drag_edit(&i, k, RulerHit::Marker(MarkerKind::FirstLine), x, 8.0).expect("edit");
        let RulerEdit::SetIndents {
            left_pt,
            first_line_pt,
            ..
        } = e
        else {
            panic!()
        };
        assert!((left_pt - 36.0).abs() < 1e-9);
        assert!((first_line_pt - (PT_CM_SNAP(1.0) - 36.0)).abs() < 1e-6);
        // Hanging marker keeps the first line where it is.
        let e = drag_edit(
            &i,
            k,
            RulerHit::Marker(MarkerKind::Hanging),
            ml + 100.0 * k,
            18.0,
        )
        .unwrap();
        let RulerEdit::SetIndents {
            left_pt,
            first_line_pt,
            ..
        } = e
        else {
            panic!()
        };
        assert!(
            (left_pt + first_line_pt - 54.0).abs() < 1e-6,
            "first line stays at 54 pt"
        );
        // The box moves both.
        let e = drag_edit(
            &i,
            k,
            RulerHit::Marker(MarkerKind::LeftBox),
            ml + 100.0 * k,
            26.0,
        )
        .unwrap();
        let RulerEdit::SetIndents { first_line_pt, .. } = e else {
            panic!()
        };
        assert!((first_line_pt - 18.0).abs() < 1e-9);
        // Right marker.
        let right_edge = (595.3 - 72.0) * k;
        let e = drag_edit(
            &i,
            k,
            RulerHit::Marker(MarkerKind::Right),
            right_edge - PT_PER_CM * k,
            18.0,
        )
        .unwrap();
        let RulerEdit::SetIndents { right_pt, .. } = e else {
            panic!()
        };
        assert!((right_pt - PT_CM_SNAP(1.0)).abs() < 1e-6);
        // Released where it started: no edit.
        let flat = info(0.0, 0.0, 0.0, vec![]);
        let h = horizontal(&flat, k);
        assert!(
            drag_edit(
                &flat,
                k,
                RulerHit::Marker(MarkerKind::FirstLine),
                h.first_line.cx,
                8.0
            )
            .is_none()
        );
    }

    #[allow(non_snake_case)]
    fn PT_CM_SNAP(cm: f64) -> f64 {
        snap_pt(cm * PT_PER_CM)
    }

    #[test]
    fn tabs_are_added_moved_and_dragged_off() {
        let i = info(
            0.0,
            0.0,
            0.0,
            vec![RulerTabStop {
                pos_pt: 72.0,
                kind: RulerTab::Left,
            }],
        );
        let k = U_PER_PT;
        let ml = 72.0 * k;
        let h = horizontal(&i, k);
        // Click on empty track adds a tab of the selected kind.
        let x = ml + 150.0 * k;
        assert_eq!(hit_test(&h, x, 12.0), Some(RulerHit::Track(x)));
        let e = click_edit(&i, k, x, RulerTab::Right).expect("add");
        let RulerEdit::AddTab { pos_pt, kind, .. } = e else {
            panic!()
        };
        assert_eq!(kind, RulerTab::Right);
        assert!((pos_pt - snap_pt(150.0)).abs() < 1e-9);
        // Clicking the existing glyph picks it up.
        assert_eq!(hit_test(&h, ml + 72.0 * k, 20.0), Some(RulerHit::Tab(0)));
        let e = drag_edit(&i, k, RulerHit::Tab(0), ml + 144.0 * k, 20.0).expect("move");
        assert!(matches!(e, RulerEdit::MoveTab { from_pt, .. } if (from_pt - 72.0).abs() < 1e-9));
        let e = drag_edit(&i, k, RulerHit::Tab(0), ml + 72.0 * k, 90.0).expect("clear");
        assert!(matches!(e, RulerEdit::ClearTab { .. }));
        // Markers beat the track.
        assert_eq!(
            hit_test(&h, h.first_line.cx, 8.0),
            Some(RulerHit::Marker(MarkerKind::FirstLine))
        );
    }

    #[test]
    fn vertical_ruler_mirrors_the_horizontal_one() {
        let i = info(0.0, 0.0, 0.0, vec![]);
        let v = vertical(&i, U_PER_PT);
        assert_eq!((v.track.x0, v.track.x1), (4.0, 22.0));
        assert!((v.track.y0 - 119.0).abs() < 1e-9);
        assert_eq!(v.digits.first().map(|d| d.number), Some(1));
        assert_eq!(v.digits.last().map(|d| d.number), Some(24));
    }
}
