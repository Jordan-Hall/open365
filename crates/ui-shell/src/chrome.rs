//! Narrow-window behaviour of the title row, tab row and status bar, in `u` (pixels at 125 %).
//!
//! The rules are derived from a 1 px sweep of Office's chrome over every client width from 1882 down
//! to 389 `u` (`fixtures/reference/ui/*-chrome-responsive.json`); `tests/chrome.rs` replays the sweep.
//! Thresholds that Office decides by its own arithmetic (the Quick Access Toolbar stages, the stages of
//! the right-hand tab-row buttons) are data in `spec/chrome-*.json`; everything else is computed.

use crate::spec::{Chrome, ClusterStage, QatStage, StatusRules, TabRules, TitleRules};

/// What the title row shows at one client width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitlePlan {
    /// Index of the Quick Access Toolbar stage in use.
    pub qat_stage: usize,
    /// Left edge of the document title; the title runs up to `title_x + title_w`.
    pub title_x: i32,
    pub title_w: i32,
    /// Width the right-anchored widgets (user widget, caption buttons) are laid out for.
    pub anchor_w: i32,
    pub show_user: bool,
    pub search: Option<SearchPlan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchPlan {
    pub x: i32,
    pub w: i32,
    /// The 58 u button form (false: the box).
    pub button: bool,
}

/// Width of the user widget and of the three caption buttons together, from the right edge.
#[derive(Debug, Clone, Copy)]
pub struct RightEdge {
    /// Distance of the user widget's left edge from the pane's right edge.
    pub user_right: i32,
    /// Distance of the first caption button's left edge from the pane's right edge.
    pub captions_right: i32,
}

#[must_use]
pub fn qat_stage(rules: &[QatStage], cw: i32) -> usize {
    rules
        .iter()
        .position(|s| cw - s.width >= s.min_rest)
        .unwrap_or(rules.len().saturating_sub(1))
}

/// Plans the title row for a client width of `cw`. `qat_x` is the toolbar's left edge.
#[must_use]
pub fn title_plan(rules: &TitleRules, qat_x: i32, right: &RightEdge, cw: i32) -> TitlePlan {
    let qat_stage = qat_stage(&rules.qat, cw);
    let stage = &rules.qat[qat_stage];
    let anchor_w = cw.max(stage.freeze.unwrap_or(0));
    let title_x = qat_x + stage.width + 1;
    let s = &rules.search;
    let user_x = anchor_w - right.user_right;
    let captions_x = anchor_w - right.captions_right;
    if anchor_w >= s.box_min_cw {
        let w = (anchor_w * s.ratio_pct / 100).min(s.max_w);
        let x = (anchor_w / 2 + 10 - w / 2)
            .max(s.pin_x)
            .min(user_x - s.gap - w);
        return TitlePlan {
            qat_stage,
            title_x,
            title_w: x - title_x,
            anchor_w,
            show_user: true,
            search: Some(SearchPlan {
                x,
                w,
                button: false,
            }),
        };
    }
    // Button form: the title keeps `min_w`; the user widget leaves first, then the button.
    let button = |edge: i32| edge - s.gap - s.button_w;
    if button(user_x) - title_x >= rules.min_w {
        let x = button(user_x);
        return TitlePlan {
            qat_stage,
            title_x,
            title_w: x - title_x,
            anchor_w,
            show_user: true,
            search: Some(SearchPlan {
                x,
                w: s.button_w,
                button: true,
            }),
        };
    }
    let x = button(captions_x);
    if x - title_x >= rules.min_w {
        return TitlePlan {
            qat_stage,
            title_x,
            title_w: x - title_x,
            anchor_w,
            show_user: false,
            search: Some(SearchPlan {
                x,
                w: s.button_w,
                button: true,
            }),
        };
    }
    TitlePlan {
        qat_stage,
        title_x,
        title_w: captions_x - title_x,
        anchor_w,
        show_user: false,
        search: None,
    }
}

/// Widths of the tabs when `deficit` `u` do not fit, in two phases.
///
/// 1. Padding: for `rounds` rounds every tab, right to left, loses 1 `u`.
/// 2. Text: the widest tabs lose 1 `u` each per round. A tab that joins the widest ones is cut
///    first in the following rounds, newer joiners before older ones (higher index first when
///    several join together).
///
/// No tab goes below `min_w`.
#[must_use]
pub fn shrink_tabs(natural: &[i32], mut deficit: i32, rounds: i32, min_w: i32) -> Vec<i32> {
    let n = natural.len();
    let mut w = natural.to_vec();
    for _ in 0..rounds {
        for i in (0..n).rev() {
            if deficit > 0 && w[i] > min_w {
                w[i] -= 1;
                deficit -= 1;
            }
        }
    }
    let mut joined: Vec<Option<usize>> = vec![None; n];
    let mut round = 0;
    while deficit > 0 {
        let Some(level) = w.iter().copied().filter(|&x| x > min_w).max() else {
            break;
        };
        let mut members: Vec<usize> = (0..n).filter(|&i| w[i] == level).collect();
        for &i in &members {
            joined[i].get_or_insert(round);
        }
        members.sort_by_key(|&i| (std::cmp::Reverse(joined[i]), std::cmp::Reverse(i)));
        for i in members {
            if deficit == 0 {
                break;
            }
            w[i] -= 1;
            deficit -= 1;
        }
        round += 1;
    }
    w
}

/// The tab row at one client width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabPlan {
    /// `(x, w)` of every tab.
    pub tabs: Vec<(i32, i32)>,
    /// `(x, w)` of every right-hand button, in the order of the spec (display options excluded).
    pub right: Vec<(i32, i32)>,
    /// Left edge of the scroll button, when the row does not fit even at the narrowest tabs.
    pub scroll_x: Option<i32>,
}

/// Plans the tab row. `tab_x` is the left edge of the first tab, `natural` the full tab widths.
#[must_use]
pub fn tab_plan(rules: &TabRules, tab_x: i32, natural: &[i32], cw: i32) -> TabPlan {
    let stage: &ClusterStage = rules
        .clusters
        .iter()
        .find(|c| cw >= c.min_cw)
        .or(rules.clusters.last())
        .expect("a cluster stage");
    let n = i32::try_from(natural.len()).unwrap_or(0);
    let cluster_w = |c: &ClusterStage| {
        c.widths.iter().sum::<i32>()
            + rules.item_gap * i32::try_from(c.widths.len().saturating_sub(1)).unwrap_or(0)
    };
    let narrowest = rules.clusters.last().unwrap_or(stage);
    // The tabs never need less than `min_w` each; the buttons stop moving left there.
    let pinned_left = tab_x + n * rules.min_w + (n - 1) * rules.gap + rules.right_gap;
    let right_edge = (cw - rules.right_margin).max(pinned_left + cluster_w(narrowest));
    let cluster_left = right_edge - cluster_w(stage);
    let natural_end = tab_x + natural.iter().sum::<i32>() + (n - 1) * rules.gap;
    let deficit = (natural_end - (cluster_left - rules.right_gap)).max(0);
    let widths = shrink_tabs(natural, deficit, rules.rounds, rules.min_w);
    let mut x = tab_x;
    let mut tabs = Vec::new();
    for w in widths {
        tabs.push((x, w));
        x += w + rules.gap;
    }
    let mut right = Vec::new();
    let mut x = cluster_left;
    for &w in &stage.widths {
        right.push((x, w));
        x += w + rules.item_gap;
    }
    let scroll_x =
        (cw - rules.right_margin < right_edge).then(|| cw - rules.right_margin - rules.scroll_w);
    TabPlan {
        tabs,
        right,
        scroll_x,
    }
}

/// Where the status bar items go at client width `cw`: `(index into items, x)` of every item that
/// is shown.
///
/// The right-hand group (from `rules.right_from`) is anchored to the right edge. When its first
/// item would start left of the bar's left edge plus the minimum gap, the next item group of
/// `rules.drop` leaves and the rest moves right by its width. Left-hand items stay while the gap to
/// the full right-hand group is at least `rules.min_gap`.
#[must_use]
pub fn status_plan(rules: &StatusRules, items: &[Chrome], cw: i32) -> Vec<(usize, i32)> {
    let first = items
        .iter()
        .position(|c| c.id == rules.right_from)
        .unwrap_or(items.len());
    let mut dropped = vec![false; items.len()];
    let pos = |i: usize, dropped: &[bool]| {
        let c = &items[i];
        let shift: i32 = (i + 1..items.len())
            .filter(|&j| dropped[j])
            .map(|j| items[j].w)
            .sum();
        cw - (c.right + rules.grip) - c.w + shift
    };
    let mut stage = 0;
    while first < items.len()
        && pos(first, &dropped) < rules.left_edge + rules.min_gap
        && stage < rules.drop.len()
    {
        for (j, c) in items.iter().enumerate().skip(first) {
            if rules.drop[stage].contains(&c.id) {
                dropped[j] = true;
            }
        }
        stage += 1;
    }
    // Left-hand items do not come back when a right-hand item leaves: they are judged against the
    // group at its full width.
    let group_x = if first < items.len() {
        pos(first, &vec![false; items.len()])
    } else {
        cw
    };
    let mut out = Vec::new();
    for (i, c) in items.iter().enumerate() {
        if i < first {
            if group_x - (c.x + c.w) >= rules.min_gap {
                out.push((i, c.x));
            }
        } else if !dropped[i] {
            out.push((i, pos(i, &dropped)));
        }
    }
    out
}
