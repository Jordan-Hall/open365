//! Pure layout: where every piece of chrome goes, in physical pixels.
//!
//! Sizes and gaps come from the spec (measured, in `u` = pixels at 125 %); positions are
//! computed here: tabs, groups, blocks, rows and items flow from the origin, rows sit on the
//! ribbon's row grid, and right-anchored widgets keep their distance from the right edge. At the
//! reference scale the result equals the reference; at other scales every edge is `round(u *
//! scale / 1.25)`, so adjacent controls keep abutting.

use crate::spec::{
    AppSpec, Chrome, Control, Group, Kind, REF_SCALE, Rect4, Scale, Size, StateRef, Tab,
};

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl PxRect {
    #[must_use]
    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    #[must_use]
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    /// Largest per-edge difference between two rectangles.
    #[must_use]
    pub fn max_delta(&self, other: &Self) -> i32 {
        [
            self.x - other.x,
            self.y - other.y,
            self.w - other.w,
            self.h - other.h,
        ]
        .into_iter()
        .map(i32::abs)
        .max()
        .unwrap_or(0)
    }
}

/// What the layout is computed for.
#[derive(Debug, Clone, Copy)]
pub struct LayoutInput {
    /// Physical pixels per device-independent pixel (egui `pixels_per_point`).
    pub scale: f32,
    /// Window size in physical pixels.
    pub window: (i32, i32),
    /// Transparent resize frame around the window: left, top, right, bottom (physical pixels).
    pub inset: (i32, i32, i32, i32),
    /// Index of the selected ribbon tab.
    pub tab: usize,
    /// The ribbon is pinned open (false: tabs only).
    pub ribbon_open: bool,
    /// Spreadsheet formula bar expanded (the height measured on the reference machine, three
    /// lines) or collapsed to one line (derived: the same margins around the Name Box height).
    pub formula_expanded: bool,
}

/// Formula strip pieces measured from the reference by pixel scans of its window (`metrics.scanRowY248`,
/// `scanColX60`, `scanColX900` in `excel-ribbon.json`): `x, y, w, h` in units (pixels at 125 %),
/// relative to the ribbon pane, borders included. Name Box, grip dots (centre x), cancel/enter/fx
/// group, formula field.
pub const NAME_BOX_U: [i32; 4] = [10, 232, 127, 30];
pub const GRIP_X_U: i32 = 149;
pub const FX_GROUP_U: [i32; 4] = [162, 232, 107, 30];
/// The formula field, expanded. Its right edge sits 10 units from the pane's right edge.
pub const FIELD_U: [i32; 4] = [274, 232, 1298, 78];
pub const FIELD_RIGHT_U: i32 = 10;
/// Strip height (formula_left / formula_bar panes): expanded measured, collapsed derived.
pub const STRIP_H_EXPANDED_U: i32 = 96;
pub const STRIP_H_COLLAPSED_U: i32 = 48;
/// Distance from the pane's right edge to the ribbon card's right edge (pixel scan of the oracle window).
pub const CARD_RIGHT_U: i32 = 11;

/// A control in place.
#[derive(Debug, Clone)]
pub struct PlacedControl<'a> {
    pub control: &'a Control,
    pub rect: PxRect,
    /// The part that runs the command (a split button's face).
    pub main: PxRect,
    /// A split button's drop-down part (empty otherwise).
    pub arrow: PxRect,
}

/// A group in place.
#[derive(Debug, Clone)]
pub struct PlacedGroup<'a> {
    pub group: &'a Group,
    /// Index of the state the group is shown in (0 is the widest).
    pub state: usize,
    pub scale: Scale,
    pub rect: PxRect,
    /// The group button of a collapsed group (opens the popup); empty otherwise.
    pub button: Option<PxRect>,
    pub label: PxRect,
    pub launcher: Option<PxRect>,
    pub controls: Vec<PlacedControl<'a>>,
}

/// A fixed widget in place.
#[derive(Debug, Clone)]
pub struct PlacedChrome<'a> {
    pub chrome: &'a Chrome,
    pub rect: PxRect,
}

/// Everything placed for one frame.
#[derive(Debug, Clone, Default)]
pub struct Layout<'a> {
    pub window: PxRect,
    pub pane: PxRect,
    pub title: Vec<PlacedChrome<'a>>,
    pub qat: Vec<PlacedChrome<'a>>,
    pub qat_rect: PxRect,
    pub title_text: Option<PlacedChrome<'a>>,
    pub search: Option<PlacedChrome<'a>>,
    pub file_tab: Option<PlacedChrome<'a>>,
    pub tab_rects: Vec<PxRect>,
    pub tab_row_right: Vec<PlacedChrome<'a>>,
    /// The ribbon body (empty when the ribbon is collapsed).
    pub lower: PxRect,
    /// The painted ribbon card: the body widened to the card's right margin.
    pub card: PxRect,
    pub groups: Vec<PlacedGroup<'a>>,
    pub status: PxRect,
    pub status_items: Vec<PlacedChrome<'a>>,
    pub name_box: Option<PxRect>,
    /// Name Box including its border, the grip dots' column, the cancel/enter/fx group and the
    /// formula field (spreadsheet only).
    pub name_box_outer: Option<PxRect>,
    pub grip: Option<PxRect>,
    pub fx_group: Option<PxRect>,
    pub formula_field: Option<PxRect>,
    pub formula_left: Option<PxRect>,
    pub formula_bar: Option<PxRect>,
    pub sheet_bar: Option<PxRect>,
    pub sheet_items: Vec<PlacedChrome<'a>>,
    /// The area left for the document.
    pub content: PxRect,
}

struct Conv {
    scale: f32,
    ox: i32,
    oy: i32,
}

impl Conv {
    fn px(&self, u: i32) -> i32 {
        (u as f32 * self.scale / REF_SCALE).round() as i32
    }

    fn rect(&self, x: i32, y: i32, w: i32, h: i32) -> PxRect {
        let x0 = self.px(x);
        let y0 = self.px(y);
        PxRect {
            x: self.ox + x0,
            y: self.oy + y0,
            w: self.px(x + w) - x0,
            h: self.px(y + h) - y0,
        }
    }

    fn rect4(&self, r: &Rect4) -> PxRect {
        self.rect(r.x, r.y, r.w, r.h)
    }
}

fn anchored(c: &Chrome, pane_w_u: i32) -> i32 {
    if c.x * 2 + c.w > pane_w_u {
        pane_w_u - c.right - c.w
    } else {
        c.x
    }
}

fn place<'a>(conv: &Conv, c: &'a Chrome, pane_w_u: i32, dy: i32) -> PlacedChrome<'a> {
    PlacedChrome {
        chrome: c,
        rect: conv.rect(anchored(c, pane_w_u), c.y + dy, c.w, c.h),
    }
}

/// Walks the controls of one state left to right from `start_x`, calling `f(control, x, y, h)`
/// in `u`. Returns the right edge of the last block.
fn walk_controls<'a>(
    st: &StateRef<'a>,
    start_x: i32,
    gy: i32,
    mut f: impl FnMut(&'a Control, i32, i32, i32),
) -> i32 {
    let mut prev_right = start_x;
    for (bi, block) in st.blocks.iter().enumerate() {
        let bx = if bi == 0 {
            start_x
        } else {
            prev_right + block.gap.unwrap_or(2)
        };
        let mut block_right = bx;
        for row in &block.rows {
            let mut ix = bx + row.dx;
            let mut item_right = ix;
            for (ii, it) in row.items.iter().enumerate() {
                if ii > 0 {
                    ix = item_right + it.gap.unwrap_or(2);
                }
                let h = it.height();
                let dy = if h < 30 { (30 - h) / 2 } else { 0 };
                f(it, ix, gy + row.top() + dy, h);
                item_right = ix + it.w;
                block_right = block_right.max(item_right);
            }
        }
        prev_right = block_right;
    }
    prev_right
}

/// Places one state of a group, left to right. `width` is the width a squeezable group is given;
/// other groups take their content width. Returns the placed group and its width in `u`.
#[allow(clippy::too_many_arguments)]
fn place_group<'a>(
    conv: &Conv,
    spec: &AppSpec,
    g: &'a Group,
    state: usize,
    gx: i32,
    gy: i32,
    width: i32,
) -> (PlacedGroup<'a>, i32) {
    let st = g.state(state);
    let elastic = st.min_w.is_some();
    let mut controls = Vec::new();
    let pad_left = if st.center {
        let content = walk_controls(&st, 0, 0, |_, _, _, _| {});
        (width - content).div_euclid(2)
    } else {
        st.pad_left
    };
    let prev_right = walk_controls(&st, gx + pad_left, gy, |it, ix, y, h| {
        let rect = conv.rect(ix, y, it.w, h);
        let (main, arrow) = split_parts(conv, it, ix, y, h);
        controls.push(PlacedControl {
            control: it,
            rect,
            main,
            arrow,
        });
    });
    // Each state's `pad_right` was measured, so the content width already includes any minimum
    // a long group label forces.
    let w = if st.blocks.is_empty() {
        st.w
    } else if elastic {
        width
    } else {
        prev_right - gx + st.pad_right
    };
    let rect = conv.rect(gx, gy, w, spec.group_h);
    let collapsed = st.scale == Scale::Collapsed;
    let launcher = (st.launcher && !collapsed).then(|| conv.rect(gx + w - 20, gy + 100, 20, 20));
    let label = conv.rect(gx, gy + 98, w - if st.launcher { 20 } else { 0 }, 22);
    (
        PlacedGroup {
            group: g,
            state,
            scale: st.scale,
            rect,
            button: collapsed.then_some(rect),
            label,
            launcher,
            controls,
        },
        w,
    )
}

/// The state and width (in `u`) each group of `tab` takes when the groups have `avail` `u` of room.
///
/// Office shrinks a tab by an ordered list of steps (`Tab::steps`, measured): the next step is
/// taken while the groups do not fit. A group whose label is wider than its content may first
/// squeeze down to its `min_w`; what is still missing after the last step is squeezed too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupFit {
    pub state: usize,
    pub w: i32,
}

/// Room Office keeps free at the right end of the ribbon (in `u`).
pub const RIBBON_SPARE_U: i32 = 2;

#[must_use]
pub fn fit_groups(tab: &Tab, avail: i32) -> Vec<GroupFit> {
    let mut states = vec![0_usize; tab.groups.len()];
    let totals = |states: &[usize]| {
        let (mut nat, mut cap) = (0, 0);
        for (g, &s) in tab.groups.iter().zip(states) {
            let st = g.state(s);
            nat += st.w;
            cap += st.min_w.map_or(0, |m| st.w - m);
        }
        (nat, cap)
    };
    for step in &tab.steps {
        let (nat, cap) = totals(&states);
        if nat - cap <= avail {
            break;
        }
        for &[g, s] in step {
            if let Some(slot) = states.get_mut(g) {
                *slot = s;
            }
        }
    }
    let (nat, _) = totals(&states);
    let mut squeeze = (nat - avail).max(0);
    let mut fit: Vec<GroupFit> = tab
        .groups
        .iter()
        .zip(&states)
        .map(|(g, &state)| GroupFit {
            state,
            w: g.state(state).w,
        })
        .collect();
    for (f, g) in fit.iter_mut().zip(&tab.groups).rev() {
        if squeeze == 0 {
            break;
        }
        if let Some(min) = g.state(f.state).min_w {
            let take = squeeze.min(f.w - min);
            f.w -= take;
            squeeze -= take;
        }
    }
    fit
}

/// The full group (state 0) as the popup of a collapsed group shows it, with its top-left corner at
/// the origin.
#[must_use]
pub fn popup_group<'a>(spec: &AppSpec, group: &'a Group, scale: f32) -> PlacedGroup<'a> {
    let conv = Conv {
        scale,
        ox: 0,
        oy: 0,
    };
    place_group(&conv, spec, group, 0, 0, 0, group.w).0
}

fn split_parts(conv: &Conv, it: &Control, x: i32, y: i32, h: i32) -> (PxRect, PxRect) {
    let whole = conv.rect(x, y, it.w, h);
    if it.kind != Kind::Split {
        return (whole, PxRect::default());
    }
    if it.size == Size::Large {
        let ah = it.arrow_h.unwrap_or(h / 2);
        (
            conv.rect(x, y, it.w, h - ah),
            conv.rect(x, y + h - ah, it.w, ah),
        )
    } else {
        let aw = it.arrow_w.unwrap_or(19);
        (
            conv.rect(x, y, it.w - aw, h),
            conv.rect(x + it.w - aw, y, aw, h),
        )
    }
}

/// Computes the layout of `spec` for `input`.
#[must_use]
pub fn compute<'a>(spec: &'a AppSpec, input: &LayoutInput) -> Layout<'a> {
    let (ww, wh) = input.window;
    let (il, it, ir, ib) = input.inset;
    let conv = Conv {
        scale: input.scale,
        ox: il,
        oy: it,
    };
    let pane_w_px = ww - il - ir;
    let pane_w_u = (pane_w_px as f32 * REF_SCALE / input.scale).round() as i32;
    let collapse = if input.ribbon_open { 0 } else { spec.lower.h };
    let mut out = Layout {
        window: PxRect {
            x: 0,
            y: 0,
            w: ww,
            h: wh,
        },
        ..Layout::default()
    };

    // Title row.
    for c in &spec.title {
        out.title.push(place(&conv, c, pane_w_u, 0));
    }
    out.qat_rect = conv.rect(spec.qat.x, spec.qat.y, spec.qat.w, spec.qat.h);
    for c in &spec.qat.items {
        out.qat.push(place(&conv, c, pane_w_u, 0));
    }
    out.title_text = Some(place(&conv, &spec.title_text, pane_w_u, 0));
    out.search = Some(place(&conv, &spec.search, pane_w_u, 0));

    // Tab row.
    out.file_tab = Some(place(&conv, &spec.file_tab, pane_w_u, 0));
    let mut tx = spec.tab_strip.x;
    for t in &spec.tabs {
        out.tab_rects
            .push(conv.rect(tx, spec.tab_strip.y, t.w, spec.tab_strip.h));
        tx += t.w + 1;
    }
    for c in &spec.tab_row_right {
        out.tab_row_right.push(place(&conv, c, pane_w_u, 0));
    }

    // Ribbon body.
    let pane_h_u = spec.lower.y + if input.ribbon_open { spec.lower.h } else { 0 };
    out.pane = conv.rect(0, 0, pane_w_u, pane_h_u);
    out.pane.w = pane_w_px;
    if input.ribbon_open {
        out.lower = conv.rect(
            spec.lower.x,
            spec.lower.y,
            pane_w_u - spec.lower.x - (spec.pane[2] - spec.lower.x - spec.lower.w),
            spec.lower.h,
        );
        out.card = conv.rect(
            spec.lower.x,
            spec.lower.y,
            pane_w_u - spec.lower.x - CARD_RIGHT_U,
            spec.lower.h,
        );
        if let Some(tab) = spec.tabs.get(input.tab) {
            let lower_w = pane_w_u - spec.pane[2] + spec.lower.w;
            let avail = lower_w - (spec.groups_x - spec.lower.x) - RIBBON_SPARE_U;
            let mut gx = spec.groups_x;
            for (g, fit) in tab.groups.iter().zip(fit_groups(tab, avail)) {
                let (placed, width) =
                    place_group(&conv, spec, g, fit.state, gx, spec.group_y, fit.w);
                out.groups.push(placed);
                gx += width;
            }
        }
    }

    // Status bar.
    let status_h = conv.px(spec.status.h);
    out.status = PxRect {
        x: il,
        y: wh - ib - status_h,
        w: pane_w_px,
        h: status_h,
    };
    let status_conv = Conv {
        scale: input.scale,
        ox: il,
        oy: out.status.y,
    };
    for c in &spec.status.items {
        // Status items are stored relative to the bar (y is already bar-relative).
        out.status_items.push(PlacedChrome {
            chrome: c,
            rect: status_conv.rect(anchored(c, pane_w_u), c.y, c.w, c.h),
        });
    }

    // Formula bar and sheet tabs (spreadsheet only).
    let mut content_top = conv.px(pane_h_u);
    if let Some(r) = &spec.formula_left {
        let rr = Rect4 {
            y: r.y - collapse,
            ..*r
        };
        out.formula_left = Some(conv.rect4(&rr));
        content_top = content_top.max(conv.px(rr.y + rr.h));
    }
    if let Some(r) = &spec.name_box {
        let rr = Rect4 {
            y: r.y - collapse,
            ..*r
        };
        out.name_box = Some(conv.rect4(&rr));
    }
    if let Some(f) = &spec.formula_bar {
        let w = pane_w_u - f.x - f.right;
        out.formula_bar = Some(conv.rect(f.x, f.y - collapse, w, f.h));
        // Pieces of the strip, anchored like Excel's: left pieces to the left, field to the right.
        let shrink = if input.formula_expanded {
            0
        } else {
            STRIP_H_EXPANDED_U - STRIP_H_COLLAPSED_U
        };
        let strip_h = STRIP_H_EXPANDED_U - shrink;
        if shrink > 0 {
            if let Some(l) = out.formula_left.as_mut() {
                l.h = conv.px(f.y - collapse + strip_h) - conv.px(f.y - collapse);
            }
            if let Some(b) = out.formula_bar.as_mut() {
                b.h = conv.px(f.y - collapse + strip_h) - conv.px(f.y - collapse);
            }
            content_top = content_top.min(conv.px(f.y - collapse + strip_h));
        }
        let dy = -collapse;
        let [x, y, w0, h0] = NAME_BOX_U;
        out.name_box_outer = Some(conv.rect(x, y + dy, w0, h0));
        out.grip = Some(conv.rect(GRIP_X_U - 4, y + dy, 8, h0));
        let [x, y, w0, h0] = FX_GROUP_U;
        out.fx_group = Some(conv.rect(x, y + dy, w0, h0));
        let [x, y, _, h0] = FIELD_U;
        let fh = if input.formula_expanded {
            h0
        } else {
            NAME_BOX_U[3]
        };
        out.formula_field = Some(conv.rect(x, y + dy, pane_w_u - FIELD_RIGHT_U - x, fh));
    }
    let mut content_bottom = out.status.y;
    if let Some(s) = &spec.sheet_tabs {
        let bar = PxRect {
            x: il,
            y: out.status.y - conv.px(s.h),
            w: pane_w_px,
            h: conv.px(s.h),
        };
        out.sheet_bar = Some(bar);
        let sc = Conv {
            scale: input.scale,
            ox: bar.x + conv.px(s.x),
            oy: bar.y,
        };
        for c in &s.items {
            out.sheet_items.push(PlacedChrome {
                chrome: c,
                rect: sc.rect(c.x, c.y, c.w, c.h),
            });
        }
        content_bottom = bar.y;
    }
    out.content = PxRect {
        x: il,
        y: content_top + it,
        w: pane_w_px,
        h: (content_bottom - content_top - it).max(0),
    };
    out
}
