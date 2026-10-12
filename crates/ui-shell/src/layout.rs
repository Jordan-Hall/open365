//! Pure layout: where every piece of chrome goes, in physical pixels.
//!
//! Sizes and gaps come from the spec (measured, in `u` = pixels at 125 %); positions are
//! computed here: tabs, groups, blocks, rows and items flow from the origin, rows sit on the
//! ribbon's row grid, and right-anchored widgets keep their distance from the right edge. At the
//! reference scale the result equals the reference; at other scales every edge is `round(u *
//! scale / 1.25)`, so adjacent controls keep abutting.

use crate::chrome;
use crate::spec::{
    AppSpec, Block, Chrome, Control, Group, Kind, REF_SCALE, Rect4, Scale, Size, StateRef, Tab,
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
/// Sheet-tab strip (Excel, measured by the 1 px sweep): it ends at `SPLIT_FRACTION * pane width - SPLIT_MARGIN_U`,
/// the split handle is `SPLIT_W_U` wide and the horizontal scroll bar starts right after it.
pub const SPLIT_FRACTION: f32 = 0.6;
pub const SPLIT_MARGIN_U: f32 = 15.0;
pub const SPLIT_W_U: i32 = 40;
/// Width of the vertical scroll bar the horizontal one stops before (right-anchored, 23 u from the pane edge).
pub const VSCROLL_RIGHT_U: i32 = 23;
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
    /// The icon-only form of a button that gave up its label (Comments, Editing, Share).
    pub compact: bool,
    /// Behind the tab row's scroll button (Office reports such items as off screen).
    pub offscreen: bool,
}

/// The scroll button of an overflowing ribbon and how far the groups can scroll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RibbonScroll {
    /// The "Column right" button at the right end of the ribbon body.
    pub button: PxRect,
    /// How far the groups can move left, in `u`.
    pub max: i32,
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
    /// The search box is its 58 u button form.
    pub search_button: bool,
    pub tab_rects: Vec<PxRect>,
    /// Tabs behind the tab row's scroll button.
    pub tab_offscreen: Vec<bool>,
    /// The scroll button of the tab row (shown when the tabs and buttons do not fit).
    pub tab_scroll: Option<PxRect>,
    pub tab_row_right: Vec<PlacedChrome<'a>>,
    /// Set when the groups do not fit the ribbon body even in their narrowest states.
    pub ribbon_scroll: Option<RibbonScroll>,
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
    /// Spreadsheet: the split handle between the sheet tabs and the horizontal scroll bar (Excel's
    /// `XLCTL`). The tab strip ends here.
    pub sheet_split: Option<PxRect>,
    /// Spreadsheet: the horizontal scroll bar, from 40 u right of the split to the vertical scroll bar.
    pub hscroll: Option<PxRect>,
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

fn placed(c: &Chrome, rect: PxRect) -> PlacedChrome<'_> {
    PlacedChrome {
        chrome: c,
        rect,
        compact: false,
        offscreen: false,
    }
}

fn place<'a>(conv: &Conv, c: &'a Chrome, pane_w_u: i32, dy: i32) -> PlacedChrome<'a> {
    placed(c, conv.rect(anchored(c, pane_w_u), c.y + dy, c.w, c.h))
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

/// Width of the ribbon's scroll button (in `u`); the groups end where it begins.
pub const SCROLL_BUTTON_U: i32 = 30;

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

/// The title row and tab row with the responsive rules of the reference applications.
fn place_top_rows<'a>(
    spec: &'a AppSpec,
    rules: &crate::spec::ChromeRules,
    conv: &Conv,
    cw: i32,
    out: &mut Layout<'a>,
) {
    let user = spec.title.iter().find(|c| c.kind == Kind::Avatar);
    let captions_right = spec
        .title
        .iter()
        .filter(|c| c.kind == Kind::Caption)
        .map(|c| c.right + c.w)
        .max()
        .unwrap_or(0);
    let right = chrome::RightEdge {
        user_right: user.map_or(0, |u| u.right + u.w),
        captions_right,
    };
    let plan = chrome::title_plan(&rules.title, spec.qat.x, &right, cw);
    for c in &spec.title {
        if c.kind == Kind::Avatar && !plan.show_user {
            continue;
        }
        let x = plan.anchor_w - c.right - c.w;
        out.title.push(placed(c, conv.rect(x, c.y, c.w, c.h)));
    }
    let stage = &rules.title.qat[plan.qat_stage];
    out.qat_rect = conv.rect(spec.qat.x, spec.qat.y, stage.width, spec.qat.h);
    // The leading items, then the last item (the overflow button) in place of the rest.
    let (more, lead) = spec.qat.items.split_last().expect("qat items");
    let mut x = spec.qat.x + i32::from(stage.keep == 0);
    for c in lead.iter().take(stage.keep).chain(std::iter::once(more)) {
        out.qat.push(placed(c, conv.rect(x, c.y, c.w, c.h)));
        x += c.w + 1;
    }
    let t = &spec.title_text;
    out.title_text = Some(placed(t, conv.rect(plan.title_x, t.y, plan.title_w, t.h)));
    if let Some(sp) = plan.search {
        let s = &spec.search;
        let (y, h) = if sp.button {
            (rules.title.search.button_y, rules.title.search.button_h)
        } else {
            (s.y, s.h)
        };
        out.search = Some(placed(s, conv.rect(sp.x, y, sp.w, h)));
        out.search_button = sp.button;
    }

    // Tab row.
    out.file_tab = Some(place(conv, &spec.file_tab, cw, 0));
    let natural: Vec<i32> = spec.tabs.iter().map(|t| t.w).collect();
    let tp = chrome::tab_plan(&rules.tabs, spec.tab_strip.x, &natural, cw);
    for &(x, w) in &tp.tabs {
        out.tab_rects
            .push(conv.rect(x, spec.tab_strip.y, w, spec.tab_strip.h));
    }
    let scroll_x = tp.scroll_x;
    out.tab_offscreen = tp
        .tabs
        .iter()
        .map(|&(x, _)| scroll_x.is_some_and(|s| x >= s))
        .collect();
    out.tab_scroll =
        scroll_x.map(|x| conv.rect(x, spec.tab_strip.y, rules.tabs.scroll_w, spec.tab_strip.h));
    let mut cluster = tp.right.iter();
    for c in &spec.tab_row_right {
        if c.id.contains("display-options") {
            out.tab_row_right.push(place(conv, c, cw, 0));
            continue;
        }
        if let Some(&(x, w)) = cluster.next() {
            let mut p = placed(c, conv.rect(x, c.y, w, c.h));
            p.compact = w < c.w;
            p.offscreen = scroll_x.is_some_and(|s| x >= s);
            out.tab_row_right.push(p);
        }
    }
}

/// The title row and tab row at their reference positions (no responsive rules).
fn place_top_rows_fixed<'a>(spec: &'a AppSpec, conv: &Conv, cw: i32, out: &mut Layout<'a>) {
    for c in &spec.title {
        out.title.push(place(conv, c, cw, 0));
    }
    out.qat_rect = conv.rect(spec.qat.x, spec.qat.y, spec.qat.w, spec.qat.h);
    for c in &spec.qat.items {
        out.qat.push(place(conv, c, cw, 0));
    }
    out.title_text = Some(place(conv, &spec.title_text, cw, 0));
    out.search = Some(place(conv, &spec.search, cw, 0));
    out.file_tab = Some(place(conv, &spec.file_tab, cw, 0));
    let mut tx = spec.tab_strip.x;
    for t in &spec.tabs {
        out.tab_rects
            .push(conv.rect(tx, spec.tab_strip.y, t.w, spec.tab_strip.h));
        out.tab_offscreen.push(false);
        tx += t.w + 1;
    }
    for c in &spec.tab_row_right {
        out.tab_row_right.push(place(conv, c, cw, 0));
    }
}

/// Computes the layout of `spec` for `input`.
#[must_use]
pub fn compute<'a>(spec: &'a AppSpec, input: &LayoutInput) -> Layout<'a> {
    compute_scrolled(spec, input, 0)
}

/// Moves a placed group `dx` physical pixels (the ribbon scrolled).
fn shift_group(g: &mut PlacedGroup<'_>, dx: i32) {
    g.rect.x += dx;
    g.label.x += dx;
    if let Some(b) = g.button.as_mut() {
        b.x += dx;
    }
    if let Some(l) = g.launcher.as_mut() {
        l.x += dx;
    }
    for c in &mut g.controls {
        c.rect.x += dx;
        c.main.x += dx;
        c.arrow.x += dx;
    }
}

/// [`compute`] with the ribbon groups scrolled left by `scroll` `u` (clamped to what overflows).
#[must_use]
pub fn compute_scrolled<'a>(spec: &'a AppSpec, input: &LayoutInput, scroll: i32) -> Layout<'a> {
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

    // Title row and tab row.
    match spec.rules.as_ref() {
        Some(rules) => place_top_rows(spec, rules, &conv, pane_w_u, &mut out),
        None => place_top_rows_fixed(spec, &conv, pane_w_u, &mut out),
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
            // Groups that overflow the body scroll behind a 30 u button at its right end; what lies
            // right of the button's left edge is off screen.
            let body_w = lower_w - (spec.groups_x - spec.lower.x);
            let content_w = gx - spec.groups_x;
            if content_w > body_w {
                let max = content_w - body_w + SCROLL_BUTTON_U;
                let offset = scroll.clamp(0, max);
                out.ribbon_scroll = Some(RibbonScroll {
                    button: conv.rect(
                        spec.lower.x + lower_w - SCROLL_BUTTON_U,
                        spec.lower.y,
                        SCROLL_BUTTON_U,
                        spec.lower.h,
                    ),
                    max,
                });
                let dx = conv.px(spec.groups_x) - conv.px(spec.groups_x + offset);
                for g in &mut out.groups {
                    shift_group(g, dx);
                }
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
    match spec.rules.as_ref() {
        Some(r) => {
            for (i, x) in chrome::status_plan(&r.status, &spec.status.items, pane_w_u) {
                let c = &spec.status.items[i];
                out.status_items
                    .push(placed(c, status_conv.rect(x, c.y, c.w, c.h)));
            }
        }
        None => {
            for c in &spec.status.items {
                // Status items are stored relative to the bar (y is already bar-relative).
                out.status_items.push(placed(
                    c,
                    status_conv.rect(anchored(c, pane_w_u), c.y, c.w, c.h),
                ));
            }
        }
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
        // Excel's formula bar is `win - 292` wide at every width: it always runs to the pane edge.
        let w = pane_w_u - f.x;
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
        let split_u = split_x_u(pane_w_u);
        let tabs_fit = tab_strip_fits(s, split_u);
        for c in &s.items {
            let name = c.ref_name.as_deref().unwrap_or_default();
            if name == "Sheet1" && !tabs_fit {
                continue;
            }
            // The first tab is clipped by the strip's end, so its card narrows as the window does.
            let w = if name == "Sheet1" {
                c.w.min(split_u - c.x - TAB_STRIP_RESERVE_U)
            } else {
                c.w
            };
            let x = if name == "Add Sheet" && !tabs_fit {
                hidden_add_x(s)
            } else {
                c.x
            };
            out.sheet_items.push(placed(c, sc.rect(x, c.y, w, c.h)));
        }
        let bar_conv = Conv {
            scale: input.scale,
            ox: bar.x,
            oy: bar.y,
        };
        out.sheet_split = Some(bar_conv.rect(split_u, 1, SPLIT_W_U, s.h - 1));
        let hs_x = split_u + SPLIT_W_U;
        let hs_end = pane_w_u - VSCROLL_RIGHT_U;
        out.hscroll = Some(bar_conv.rect(hs_x, 9, (hs_end - hs_x).max(0), 21));
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

/// Where the sheet-tab strip ends (the split handle's x) at a pane width, in `u`.
#[must_use]
pub fn split_x_u(pane_w_u: i32) -> i32 {
    (SPLIT_FRACTION * pane_w_u as f32 - SPLIT_MARGIN_U).floor() as i32
}

/// The first tab stops `TAB_STRIP_RESERVE_U` before the split handle (its card is the tab minus 11 u).
const TAB_STRIP_RESERVE_U: i32 = 74;
/// The tab is dropped once its clipped card (the tab minus 11 u) is no wider than this.
const TAB_MIN_CARD_U: i32 = 20;

/// The first sheet tab is shown while its clipped card is wider than `TAB_MIN_CARD_U`.
fn tab_strip_fits(s: &crate::spec::SheetTabs, split_u: i32) -> bool {
    s.items
        .iter()
        .filter(|c| c.ref_name.as_deref() == Some("Sheet1"))
        .all(|c| c.w.min(split_u - c.x - TAB_STRIP_RESERVE_U) - 11 > TAB_MIN_CARD_U)
}

/// Add Sheet moves next to the scroll buttons when no tab is shown.
fn hidden_add_x(s: &crate::spec::SheetTabs) -> i32 {
    s.items
        .iter()
        .filter(|c| c.ref_name.as_deref() == Some("Sheet1"))
        .map(|c| c.x)
        .min()
        .unwrap_or(0)
}

/// Breaks a label into at most two centred lines. A multi-word label of more than `min_len` bytes
/// breaks at the word gap that makes the wider line narrowest (Office: "Ink to / Shape", not "Ink /
/// to Shape"). When the widest line is still wider than `max_w`, a slash is a break too ("Win/ /
/// Loss"), so a word is never cut. `text_w` measures a string.
pub fn wrap_label(
    label: &str,
    min_len: usize,
    max_w: f32,
    text_w: impl Fn(&str) -> f32,
) -> Vec<String> {
    let widest = |lines: &[String]| lines.iter().map(|l| text_w(l)).fold(0.0, f32::max);
    let best = |breaks: &[char]| -> Vec<String> {
        let tokens: Vec<&str> = label.split_inclusive(breaks).collect();
        (1..tokens.len())
            .map(|k| {
                vec![
                    tokens[..k].concat().trim_end().to_owned(),
                    tokens[k..].concat().trim_end().to_owned(),
                ]
            })
            .min_by(|a, b| widest(a).total_cmp(&widest(b)))
            .unwrap_or_else(|| vec![label.to_owned()])
    };
    let one = vec![label.to_owned()];
    let lines = if label.len() > min_len {
        best(&[' '])
    } else {
        one.clone()
    };
    if widest(&lines) <= max_w {
        return lines;
    }
    let slashed = best(&[' ', '/']);
    if widest(&slashed) < widest(&lines) {
        slashed
    } else {
        lines
    }
}

/// The text a ribbon button shows: Office keeps the "..." of a command that opens a dialog in its
/// accessible name only, never on the face of the button.
pub fn ribbon_label(label: &str) -> &str {
    label.strip_suffix("...").unwrap_or(label)
}

/// Room (in `u`) a large button keeps around its label: Office sizes a large ribbon button to its
/// label plus this padding instead of truncating or renaming the label.
pub const LARGE_LABEL_PAD_U: i32 = 4;

fn blocks_extent(blocks: &[Block]) -> i32 {
    let mut right = 0;
    for (bi, block) in blocks.iter().enumerate() {
        let bx = if bi == 0 {
            0
        } else {
            right + block.gap.unwrap_or(2)
        };
        let mut block_right = bx;
        for row in &block.rows {
            let mut x = bx + row.dx;
            for (ii, it) in row.items.iter().enumerate() {
                if ii > 0 {
                    x += it.gap.unwrap_or(2);
                }
                x += it.w;
            }
            block_right = block_right.max(x);
        }
        right = block_right;
    }
    right
}

/// Grows `blocks` so every large button fits its label; returns how much wider the content got.
fn fit_blocks(blocks: &mut [Block], text_w: &impl Fn(&str) -> f32, scale: f32) -> i32 {
    let before = blocks_extent(blocks);
    for it in blocks
        .iter_mut()
        .flat_map(|b| &mut b.rows)
        .flat_map(|r| &mut r.items)
    {
        if it.size != Size::Large || it.label.is_empty() {
            continue;
        }
        if !matches!(it.kind, Kind::Button | Kind::Split | Kind::Dropdown) {
            continue;
        }
        let room = it.w as f32 / scale;
        let lines = wrap_label(ribbon_label(&it.label), 7, room, text_w);
        let need = lines.iter().map(|l| text_w(l)).fold(0.0, f32::max);
        let want = (need * scale).ceil() as i32 + LARGE_LABEL_PAD_U;
        it.w = it.w.max(want);
    }
    blocks_extent(blocks) - before
}

/// Widens every large ribbon button whose label (plus [`LARGE_LABEL_PAD_U`]) is wider than its
/// measured width, and the groups that hold it, in every state. Office sizes a large button to its
/// label; the spec widths were measured for the reference labels only. `text_w` measures a label in
/// points at the ribbon label size; `scale` is `u` per point (the reference scale).
pub fn fit_large_labels(spec: &mut AppSpec, text_w: impl Fn(&str) -> f32, scale: f32) {
    for tab in &mut spec.tabs {
        for g in &mut tab.groups {
            let d = fit_blocks(&mut g.blocks, &text_w, scale);
            g.w += d;
            if let Some(m) = &mut g.min_w {
                *m += d;
            }
            for st in &mut g.states {
                let d = fit_blocks(&mut st.blocks, &text_w, scale);
                st.w += d;
                if let Some(m) = &mut st.min_w {
                    *m += d;
                }
            }
        }
    }
}
