//! Responsive ribbon: for every window width recorded in `fixtures/reference/ui/*-responsive.json`
//! (Office's ribbon at 1900..500 px, 125 % scale), every group of every tab must be in the same
//! state (large, medium, small or collapsed) and sit within 1 px of the recorded x and width.

use open365_ui_shell::layout::{LayoutInput, PxRect, compute};
use open365_ui_shell::spec::{AppSpec, Kind, Scale};
use serde_json::Value;

fn load(name: &str) -> Value {
    let path = format!(
        "{}/../../fixtures/reference/ui/{name}-responsive.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("reference file")).expect("json")
}

fn rect(v: &Value) -> PxRect {
    let a: Vec<i32> = v["rect"]
        .as_array()
        .expect("rect")
        .iter()
        .map(|n| i32::try_from(n.as_i64().expect("int")).expect("small"))
        .collect();
    PxRect {
        x: a[0],
        y: a[1],
        w: a[2],
        h: a[3],
    }
}

fn leaves<'a>(g: &'a Value, out: &mut Vec<&'a Value>) {
    for c in g["children"].as_array().into_iter().flatten() {
        if c["type"] == "Group" {
            leaves(c, out);
        } else {
            out.push(c);
        }
    }
}

/// The controls of a group node without the dialog launcher and zero-width items.
fn controls(g: &Value) -> Vec<&Value> {
    let gr = rect(g);
    let mut all = Vec::new();
    leaves(g, &mut all);
    all.into_iter()
        .filter(|c| {
            let r = rect(c);
            r.w > 0 && !(r.w == 20 && r.h == 20 && r.right() == gr.right())
        })
        .collect()
}

/// A group that is one button filling the whole chunk.
fn spans(g: &Value) -> bool {
    let gr = rect(g);
    let items = controls(g);
    items.len() == 1 && rect(items[0]).w >= gr.w - 1 && rect(items[0]).h >= gr.h - 6
}

/// The state a recorded group is in, judged from its geometry alone. `full_spans` tells whether the
/// group already fills its chunk at the widest window (a single menu button, not a collapse).
fn classify(g: &Value, full_spans: bool) -> Scale {
    if spans(g) && !full_spans {
        return Scale::Collapsed;
    }
    let items = controls(g);
    if items.iter().any(|c| rect(c).h >= 80) {
        return Scale::Large;
    }
    let labelled = items.iter().any(|c| {
        matches!(
            c["type"].as_str(),
            Some("Button" | "SplitButton" | "MenuItem")
        ) && rect(c).w > 60
    });
    if labelled {
        Scale::Medium
    } else {
        Scale::Small
    }
}

struct Outcome {
    checked: usize,
    controls: usize,
    misses: Vec<String>,
}

fn check_app(app: &str, spec: &AppSpec) -> Outcome {
    let d = load(app);
    let widths = d["widths"].as_array().expect("widths");
    let widest = widths
        .iter()
        .max_by_key(|w| w["width"].as_i64())
        .expect("a width");
    let mut out = Outcome {
        checked: 0,
        controls: 0,
        misses: Vec::new(),
    };
    for w in widths {
        let width = i32::try_from(w["width"].as_i64().expect("width")).expect("small");
        for (ti, t) in w["tabs"].as_array().expect("tabs").iter().enumerate() {
            let Some(groups) = t["lowerRibbon"]["children"][0]["children"].as_array() else {
                continue;
            };
            let inset = spec.pane[0];
            let layout = compute(
                spec,
                &LayoutInput {
                    scale: 1.25,
                    window: (width, spec.ref_window[1]),
                    inset: (inset, spec.pane[1], inset, inset),
                    tab: ti,
                    ribbon_open: true,
                    formula_expanded: true,
                },
            );
            let tname = t["name"].as_str().unwrap_or_default();
            if layout.groups.len() != groups.len() {
                out.misses.push(format!(
                    "{app} {width} {tname}: {} groups, reference {}",
                    layout.groups.len(),
                    groups.len()
                ));
                continue;
            }
            let wide_groups = widest["tabs"][ti]["lowerRibbon"]["children"][0]["children"]
                .as_array()
                .expect("widest groups");
            for (gi, (rg, pg)) in groups.iter().zip(&layout.groups).enumerate() {
                let gname = rg["name"].as_str().unwrap_or_default();
                let want = classify(rg, spans(&wide_groups[gi]));
                let r = rect(rg);
                out.checked += 1;
                let what = format!("{app} {width} {tname}/{gname}");
                if pg.scale != want {
                    out.misses.push(format!(
                        "{what}: state {:?} (#{}), reference {want:?}",
                        pg.scale, pg.state
                    ));
                }
                // Every control of the group sits where the reference has it (matched by name).
                let mut items = Vec::new();
                leaves(rg, &mut items);
                for pc in &pg.controls {
                    let name = pc.control.ref_name.as_deref().unwrap_or_default();
                    let best = items
                        .iter()
                        .filter(|i| i["name"].as_str() == Some(name))
                        .min_by_key(|i| rect(i).max_delta(&pc.rect));
                    out.controls += 1;
                    match best {
                        Some(b) if pc.control.kind == Kind::Spinner => {
                            let want = rect(b);
                            if (pc.rect.x - want.x).abs() > 1 || (pc.rect.y - want.y).abs() > 1 {
                                out.misses
                                    .push(format!("{what}/{name}: {:?} vs {want:?}", pc.rect));
                            }
                        }
                        Some(b) if rect(b).max_delta(&pc.rect) > 1 => {
                            out.misses.push(format!(
                                "{what}/{name}: {:?} vs {:?}",
                                pc.rect,
                                rect(b)
                            ));
                        }
                        Some(_) => {}
                        None => out
                            .misses
                            .push(format!("{what}/{name}: not in the reference")),
                    }
                }
                if (pg.rect.x - r.x).abs() > 1 || (pg.rect.w - r.w).abs() > 1 {
                    out.misses.push(format!(
                        "{what}: x {} w {} (state #{}), reference x {} w {}",
                        pg.rect.x, pg.rect.w, pg.state, r.x, r.w
                    ));
                }
            }
        }
    }
    out
}

fn report(app: &str, o: &Outcome) {
    println!(
        "{app}: {} group placements and {} controls checked, {} misses",
        o.checked,
        o.controls,
        o.misses.len()
    );
    for m in o.misses.iter().take(60) {
        println!("  miss: {m}");
    }
}

#[test]
fn writer_ribbon_collapses_like_the_reference() {
    let o = check_app("word", &AppSpec::writer());
    report("writer", &o);
    assert!(o.misses.is_empty(), "{} misses", o.misses.len());
}

#[test]
fn sheets_ribbon_collapses_like_the_reference() {
    let o = check_app("excel", &AppSpec::sheets());
    report("sheets", &o);
    assert!(o.misses.is_empty(), "{} misses", o.misses.len());
}
