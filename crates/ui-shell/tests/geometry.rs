//! Compares the shell's laid-out chrome with the measured reference (`fixtures/reference/ui`).
//!
//! The reference was measured at 125 % scale in a 1600 x 1000 px window, so that is the scale at
//! which the comparison is exact (every rectangle within 1 px). Other scales are derived, not
//! measured: the second test only checks that the layout scales consistently.

use open365_ui_shell::layout::{LayoutInput, PxRect, compute};
use open365_ui_shell::spec::{AppSpec, Kind};
use serde_json::Value;

fn reference(name: &str) -> Value {
    let path = format!(
        "{}/../../fixtures/reference/ui/{name}-ribbon.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("reference file")).expect("json")
}

fn rect(v: &Value) -> PxRect {
    let a: Vec<i32> = v["rect"]
        .as_array()
        .expect("rect")
        .iter()
        .map(|n| n.as_i64().expect("int") as i32)
        .collect();
    PxRect {
        x: a[0],
        y: a[1],
        w: a[2],
        h: a[3],
    }
}

fn leaf_items<'a>(g: &'a Value, out: &mut Vec<&'a Value>) {
    for c in g["children"].as_array().into_iter().flatten() {
        if c["type"] == "Group" {
            leaf_items(c, out);
        } else {
            out.push(c);
        }
    }
}

fn input(spec: &AppSpec, scale: f32, tab: usize) -> LayoutInput {
    let [px, py, _, _] = spec.pane;
    LayoutInput {
        scale,
        window: (spec.ref_window[0], spec.ref_window[1]),
        inset: (px, py, px, px),
        tab,
        ribbon_open: true,
        formula_expanded: true,
    }
}

#[derive(Default, Debug)]
struct Tally {
    total: usize,
    ok: usize,
    misses: Vec<String>,
}

impl Tally {
    fn check(&mut self, what: &str, ours: PxRect, theirs: PxRect) {
        self.total += 1;
        if ours.max_delta(&theirs) <= 1 {
            self.ok += 1;
        } else {
            self.misses
                .push(format!("{what}: ours {ours:?} reference {theirs:?}"));
        }
    }

    /// Like `check` for widgets whose reference extent differs by construction (spinners).
    fn check_origin(&mut self, what: &str, ours: PxRect, theirs: PxRect) {
        self.check(
            what,
            PxRect {
                w: theirs.w,
                ..ours
            },
            theirs,
        );
    }
}

fn run(app: &str, spec: AppSpec) -> (Tally, Tally, Tally) {
    let r = reference(app);
    let mut shell = Tally::default();
    let mut groups = Tally::default();
    let mut controls = Tally::default();
    let tabs = r["tabs"].as_array().expect("tabs");
    assert_eq!(tabs.len(), spec.tabs.len(), "tab count");
    for (ti, t) in tabs.iter().enumerate() {
        let layout = compute(&spec, &input(&spec, 1.25, ti));
        shell.check(&format!("tab {ti}"), layout.tab_rects[ti], rect(t));
        let ref_groups = t["lowerRibbon"]["children"][0]["children"]
            .as_array()
            .expect("groups");
        assert_eq!(ref_groups.len(), layout.groups.len(), "groups of tab {ti}");
        shell.check("lower ribbon", layout.lower, rect(&t["lowerRibbon"]));
        for (gi, (rg, pg)) in ref_groups.iter().zip(&layout.groups).enumerate() {
            let gname = rg["name"].as_str().unwrap_or_default();
            groups.check(
                &format!("{app} tab {ti} group {gi} {gname}"),
                pg.rect,
                rect(rg),
            );
            let mut items = Vec::new();
            leaf_items(rg, &mut items);
            for pc in &pg.controls {
                let name = pc.control.ref_name.as_deref().unwrap_or_default();
                let best = items
                    .iter()
                    .filter(|i| i["name"].as_str() == Some(name))
                    .min_by_key(|i| rect(i).max_delta(&pc.rect));
                let what = format!("{app} {} {}", gname, pc.control.id);
                match best {
                    Some(b) if pc.control.kind == Kind::Spinner => {
                        controls.check_origin(&what, pc.rect, rect(b));
                    }
                    Some(b) => controls.check(&what, pc.rect, rect(b)),
                    None => {
                        controls.total += 1;
                        controls.misses.push(format!("{what}: no reference item"));
                    }
                }
            }
        }
    }
    // Chrome common to every tab.
    let layout = compute(&spec, &input(&spec, 1.25, 0));
    let elements = r["chrome"]["elements"].as_array().expect("chrome");
    let find = |id: &str| elements.iter().find(|e| e["id"] == id);
    if let (Some(e), Some(f)) = (find("FileTabButton"), layout.file_tab.as_ref()) {
        shell.check("file tab", f.rect, rect(e));
    }
    if let (Some(e), Some(f)) = (find("TellMeControlAnchor"), layout.search.as_ref()) {
        shell.check("search", f.rect, rect(e));
    }
    for c in &layout.title {
        let id = c.chrome.id.as_str();
        let name = c.chrome.label.as_str();
        let found = elements.iter().find(|e| {
            (id == "mecontrolwidget" && e["id"] == "MeControlWidget")
                || (id.starts_with("caption.") && e["cls"] == "NetUIAppFrameHelper" && {
                    let n = e["name"].as_str().unwrap_or_default().to_lowercase();
                    id.ends_with(&n.replace(' ', "-"))
                })
        });
        match found {
            Some(e) => shell.check(&format!("title {name}"), c.rect, rect(e)),
            None => {
                shell.total += 1;
                shell.misses.push(format!("title {id}: no reference"));
            }
        }
    }
    let status = elements.len(); // keep the borrow checker simple
    let _ = status;
    // Status bar items: reference tree under panes.
    let mut status_items = Vec::new();
    fn walk<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
        if v["type"] == "StatusBar" {
            for c in v["children"].as_array().into_iter().flatten() {
                out.push(c);
            }
        }
        for c in v["children"].as_array().into_iter().flatten() {
            walk(c, out);
        }
    }
    for p in r["panes"].as_array().expect("panes") {
        walk(p, &mut status_items);
    }
    // The 1 px chrome sweep shows a size box (14 px) at the right end of Word's status bar that the
    // 1600 px reference run did not have; the spec places the right-hand group for the sweep.
    let reference_has_size_box = status_items
        .iter()
        .any(|s| s["name"].as_str() == Some("Size box"));
    let rules = spec.rules.as_ref().map(|r| &r.status);
    let right_from = rules.and_then(|r| {
        layout
            .status_items
            .iter()
            .position(|c| c.chrome.id == r.right_from)
    });
    let grip = rules.map_or(0, |r| r.grip);
    for (i, c) in layout.status_items.iter().enumerate() {
        let label = c.chrome.id.as_str();
        let want = c.chrome.ref_name.as_deref().unwrap_or_default();
        let mut ours = c.rect;
        if !reference_has_size_box && right_from.is_some_and(|f| i >= f) {
            ours.x += grip;
        }
        let best = status_items
            .iter()
            .filter(|s| s["name"].as_str() == Some(want))
            .min_by_key(|s| rect(s).max_delta(&ours));
        match best {
            Some(b) => shell.check(&format!("status {label}"), ours, rect(b)),
            None => {
                shell.total += 1;
                shell.misses.push(format!("status {label}: no reference"));
            }
        }
    }
    (shell, groups, controls)
}

fn report(app: &str, t: &(Tally, Tally, Tally)) {
    println!(
        "{app}: shell {}/{}, groups {}/{}, controls {}/{} within 1 px",
        t.0.ok, t.0.total, t.1.ok, t.1.total, t.2.ok, t.2.total
    );
    for m in
        t.0.misses
            .iter()
            .chain(&t.1.misses)
            .chain(&t.2.misses)
            .take(40)
    {
        println!("  miss: {m}");
    }
}

#[test]
fn writer_matches_reference_at_125_percent() {
    let t = run("word", AppSpec::writer());
    report("writer", &t);
    assert_eq!(t.0.ok, t.0.total, "shell chrome");
    assert_eq!(t.1.ok, t.1.total, "groups");
    assert_eq!(t.2.ok, t.2.total, "controls");
}

#[test]
fn sheets_matches_reference_at_125_percent() {
    let t = run("excel", AppSpec::sheets());
    report("sheets", &t);
    assert_eq!(t.0.ok, t.0.total, "shell chrome");
    assert_eq!(t.1.ok, t.1.total, "groups");
    assert_eq!(t.2.ok, t.2.total, "controls");
}

/// Other scales are derived: edges are `round(u * scale / 1.25)`, so a 100 % layout is the
/// reference scaled by 0.8 to within a pixel.
#[test]
fn other_scales_follow_the_reference() {
    for (app, spec) in [("word", AppSpec::writer()), ("excel", AppSpec::sheets())] {
        let r = reference(app);
        for scale in [1.0_f32, 1.5, 2.0] {
            let k = scale / 1.25;
            let mut inp = input(&spec, scale, 0);
            inp.window = (
                (spec.ref_window[0] as f32 * k).round() as i32,
                (spec.ref_window[1] as f32 * k).round() as i32,
            );
            inp.inset = (
                (spec.pane[0] as f32 * k).round() as i32,
                (spec.pane[1] as f32 * k).round() as i32,
                (spec.pane[0] as f32 * k).round() as i32,
                (spec.pane[0] as f32 * k).round() as i32,
            );
            let layout = compute(&spec, &inp);
            for (ti, t) in r["tabs"].as_array().expect("tabs").iter().enumerate() {
                let want = rect(t);
                let got = layout.tab_rects[ti];
                let scaled = |v: i32| (v as f32 * k).round() as i32;
                // The window inset is rounded separately from the tab offset, so allow 2 px.
                assert!(
                    (got.x - scaled(want.x)).abs() <= 2 && (got.w - scaled(want.w)).abs() <= 2,
                    "{app} tab {ti} at {scale}: {got:?} vs {want:?}"
                );
            }
        }
    }
}
