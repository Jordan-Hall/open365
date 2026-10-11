//! Responsive chrome: for every client width recorded in `fixtures/reference/ui/*-chrome-responsive.json`
//! (a 1 px sweep of Office's title row, tab row and status bar at 125 %), the layout must show the same
//! items and put every one within 1 px of the recorded rectangle.

use open365_ui_shell::layout::{Layout, LayoutInput, PxRect, compute};
use open365_ui_shell::spec::{AppSpec, Kind};
use serde_json::Value;

fn load(app: &str) -> Value {
    let path = format!(
        "{}/../../fixtures/reference/ui/{app}-chrome-responsive.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("reference file")).expect("json")
}

/// One recorded item: name, automation id, rectangle (pane-relative) and the off-screen flag.
struct Rec<'a> {
    name: &'a str,
    id: &'a str,
    r: PxRect,
    off: bool,
}

fn recs(list: &Value) -> Vec<Rec<'_>> {
    list.as_array()
        .expect("list")
        .iter()
        .map(|v| {
            let n = |i: usize| i32::try_from(v[i].as_i64().expect("int")).expect("small");
            Rec {
                name: v[0].as_str().unwrap_or_default(),
                id: v[1].as_str().unwrap_or_default(),
                r: PxRect {
                    x: n(2),
                    y: n(3),
                    w: n(4),
                    h: n(5),
                },
                off: n(6) != 0,
            }
        })
        .collect()
}

#[derive(Default)]
struct Tally {
    checked: usize,
    misses: Vec<String>,
}

impl Tally {
    fn rect(&mut self, what: &str, got: PxRect, want: PxRect) {
        self.checked += 1;
        if got.max_delta(&want) > 1 {
            self.misses
                .push(format!("{what}: {got:?}, reference {want:?}"));
        }
    }

    fn x_w(&mut self, what: &str, got: PxRect, want: PxRect) {
        self.checked += 1;
        if (got.x - want.x).abs() > 1 || (got.w - want.w).abs() > 1 {
            self.misses
                .push(format!("{what}: {got:?}, reference {want:?}"));
        }
    }

    fn flag(&mut self, what: &str, got: bool, want: bool) {
        self.checked += 1;
        if got != want {
            self.misses.push(format!("{what}: {got}, reference {want}"));
        }
    }
}

/// Moves a window-relative rectangle into the ribbon pane's frame.
fn pane(r: PxRect, inset: i32, top: i32) -> PxRect {
    PxRect {
        x: r.x - inset,
        y: r.y - top,
        ..r
    }
}

fn check_width(spec: &AppSpec, width: i32, w: &Value, t: &mut Tally) {
    let [px, py, _, _] = spec.pane;
    let lay: Layout<'_> = compute(
        spec,
        &LayoutInput {
            scale: 1.25,
            window: (width, spec.ref_window[1]),
            inset: (px, py, px, px),
            tab: 0,
            ribbon_open: true,
            formula_expanded: true,
        },
    );
    let top = recs(&w["top"]);
    let status = recs(&w["status"]);
    let tag = |s: &str| format!("{width} {s}");

    // Quick Access Toolbar: same items, same places (the overflow button is "More controls").
    let qat_names: Vec<&str> = spec
        .qat
        .items
        .iter()
        .filter_map(|c| c.ref_name.as_deref())
        .chain(["More controls"])
        .collect();
    let want_qat: Vec<&Rec<'_>> = top
        .iter()
        .filter(|r| qat_names.contains(&r.name) && r.r.y < 60)
        .collect();
    t.flag(
        &tag("qat item count"),
        lay.qat.len() == want_qat.len(),
        true,
    );
    for (p, r) in lay.qat.iter().zip(&want_qat) {
        t.rect(&tag(&format!("qat {}", r.name)), pane(p.rect, px, py), r.r);
    }
    if let Some(q) = top.iter().find(|r| r.name == "Quick Access Toolbar") {
        t.x_w(&tag("qat container"), pane(lay.qat_rect, px, py), q.r);
    }

    // Title, search, user widget, caption buttons.
    if let (Some(tt), Some(r)) = (
        &lay.title_text,
        top.iter().find(|r| r.name.starts_with("<document>")),
    ) {
        t.rect(&tag("title"), pane(tt.rect, px, py), r.r);
    }
    let want_search = top
        .iter()
        .find(|r| r.id == "TellMeControlAnchor" || r.id == "SearchToggleButton");
    match (&lay.search, want_search) {
        (Some(s), Some(r)) => {
            t.flag(
                &tag("search is button"),
                lay.search_button,
                r.id != "TellMeControlAnchor",
            );
            t.rect(&tag("search"), pane(s.rect, px, py), r.r);
        }
        (None, None) => {}
        (got, want) => t.flag(&tag("search present"), got.is_some(), want.is_some()),
    }
    let want_user = top.iter().find(|r| r.id == "MeControlWidget");
    let got_user = lay.title.iter().find(|p| p.chrome.kind == Kind::Avatar);
    match (got_user, want_user) {
        (Some(p), Some(r)) => t.rect(&tag("user"), pane(p.rect, px, py), r.r),
        (None, None) => {}
        (got, want) => t.flag(&tag("user present"), got.is_some(), want.is_some()),
    }
    let mut want_caps: Vec<&Rec<'_>> = top
        .iter()
        .filter(|r| matches!(r.name, "Minimise" | "Maximise" | "Restore Down" | "Close"))
        .collect();
    want_caps.sort_by_key(|r| r.r.x);
    let mut got_caps: Vec<&PxRect> = lay
        .title
        .iter()
        .filter(|p| p.chrome.kind == Kind::Caption)
        .map(|p| &p.rect)
        .collect();
    got_caps.sort_by_key(|r| r.x);
    for (g, r) in got_caps.iter().zip(&want_caps) {
        t.rect(&tag(&format!("caption {}", r.name)), pane(**g, px, py), r.r);
    }

    // Tabs, Comments / Editing / Share, display options, scroll button.
    for (i, tab) in spec.tabs.iter().enumerate() {
        let id = tab.reference.as_deref().unwrap_or_default();
        let Some(r) = top.iter().find(|r| r.id == id) else {
            continue;
        };
        t.rect(
            &tag(&format!("tab {}", tab.label)),
            pane(lay.tab_rects[i], px, py),
            r.r,
        );
        t.flag(
            &tag(&format!("tab {} off screen", tab.label)),
            lay.tab_offscreen[i],
            r.off,
        );
    }
    for p in &lay.tab_row_right {
        let name = p.chrome.ref_name.as_deref().unwrap_or(&p.chrome.label);
        let Some(r) = top.iter().find(|r| r.name == name && r.r.y >= 60) else {
            t.flag(&tag(&format!("{name} in the reference")), false, true);
            continue;
        };
        t.rect(&tag(name), pane(p.rect, px, py), r.r);
        if !p.chrome.id.contains("display-options") {
            t.flag(&tag(&format!("{name} off screen")), p.offscreen, r.off);
        }
    }
    let want_scroll = top.iter().find(|r| r.name == "Column right");
    match (&lay.tab_scroll, want_scroll) {
        (Some(s), Some(r)) => t.rect(&tag("tab scroll"), pane(*s, px, py), r.r),
        (None, None) => {}
        (got, want) => t.flag(&tag("tab scroll present"), got.is_some(), want.is_some()),
    }

    // Status bar: the items the spec knows must be shown exactly when Office shows them.
    for c in &spec.status.items {
        let name = c.ref_name.as_deref().unwrap_or_default();
        let want = status.iter().find(|r| r.name == name && r.r.h > 0);
        let got = lay.status_items.iter().find(|p| p.chrome.id == c.id);
        match (got, want) {
            (Some(p), Some(r)) => t.x_w(&tag(&format!("status {name}")), pane(p.rect, px, py), r.r),
            (None, None) => {}
            (got, want) => t.flag(
                &tag(&format!("status {name} shown")),
                got.is_some(),
                want.is_some(),
            ),
        }
    }
}

fn run(app: &str, spec: &AppSpec) {
    let d = load(app);
    if d["schema"].as_i64() != Some(2) {
        // The committed file is still the old 25 px sweep (schema 1); re-run the oracle with
        // `-ChromeWidths 1900..407:1` to replace it.
        println!("{app}: NOT RUN, {app}-chrome-responsive.json is not a schema 2 sweep");
        return;
    }
    let widths = d["widths"].as_array().expect("widths");
    let mut t = Tally::default();
    for w in widths {
        let width = i32::try_from(w["w"].as_i64().expect("w")).expect("small");
        check_width(spec, width, w, &mut t);
    }
    println!(
        "{app}: {} widths, {} checks, {} misses",
        widths.len(),
        t.checked,
        t.misses.len()
    );
    for m in t.misses.iter().take(60) {
        println!("  miss: {m}");
    }
    assert!(t.misses.is_empty(), "{} misses", t.misses.len());
}

#[test]
fn writer_chrome_follows_the_sweep() {
    run("word", &AppSpec::writer());
}

#[test]
fn sheets_chrome_follows_the_sweep() {
    run("excel", &AppSpec::sheets());
}
