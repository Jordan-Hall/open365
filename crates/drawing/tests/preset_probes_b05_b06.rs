//! Batches b05 and b06 against Word, with fills and page culling (see `common/b05_b06.rs`).
//!
//! Reads `b05`, `b06` (stroke only) and the filled variants `b05.fill`, `b06.fill` from
//! `fixtures/reference/drawing/presets`. For every probe page the stroke operations and, in the
//! filled batches, the fill operations of our preset are compared with Word's: the same 0.25 pt
//! limit as the shared check, and the shade of each fill operation.
//!
//! `cargo test -p open365-drawing --test preset_probes_b05_b06 -- --nocapture` prints the report.

#[path = "common/b05_b06.rs"]
mod b05_b06;
mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use common::pdf::Pdf;
use open365_drawing::geom::presets;
use serde_json::Value;

const TOL_PT: f64 = 0.25;
const BATCHES: [&str; 4] = ["b05", "b06", "b05.fill", "b06.fill"];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/reference/drawing/presets")
}

fn adjust_overrides(page: &Value) -> Vec<(String, String)> {
    page["adj"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), format!("val {}", v.as_i64().unwrap_or(0))))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Default)]
struct Row {
    pages: usize,
    bad: Vec<String>,
    worst_stroke: f64,
    worst_fill: f64,
}

#[test]
fn b05_b06_match_word_with_fills() {
    let table: BTreeMap<&str, _> = presets::table().into_iter().collect();
    let mut rows: BTreeMap<String, Row> = BTreeMap::new();
    for name in BATCHES {
        let (Ok(probes), Ok(pdf)) = (
            fs::read(dir().join(format!("{name}.probes.json"))),
            fs::read(dir().join(format!("{name}.pdf"))),
        ) else {
            continue;
        };
        let probes: Value = serde_json::from_slice(&probes).expect("probes.json is JSON");
        let pdf = Pdf::parse(&pdf);
        let ids = pdf.page_ids();
        let painted = pdf.page_paths();
        let pages = probes["pages"].as_array().expect("pages");
        assert_eq!(painted.len(), pages.len(), "{name}: page count");
        let with_fill = probes["fill"].as_bool().unwrap_or(false);
        for (i, pg) in pages.iter().enumerate() {
            let preset = pg["preset"].as_str().expect("preset");
            let Some(def) = table.get(preset) else {
                continue;
            };
            let (w, h) = (pg["w_emu"].as_f64().unwrap(), pg["h_emu"].as_f64().unwrap());
            let (ws, wf) = b05_b06::word_ops(&painted[i], &pdf.page_content(ids[i]));
            let (os, of) = b05_b06::our_ops(def, &adjust_overrides(pg), w, h);
            let row = rows.entry(format!("{preset} [{name}]")).or_default();
            row.pages += 1;
            let ds = b05_b06::stroke_deviation(&os, &ws);
            row.worst_stroke = row.worst_stroke.max(ds);
            let kind = pg["kind"].as_str().unwrap_or("?");
            if ds > TOL_PT || ds.is_nan() {
                row.bad.push(format!("{kind} stroke {ds:.3}"));
            }
            if with_fill {
                let (df, shade) = b05_b06::fill_deviation(&of, &wf);
                row.worst_fill = row.worst_fill.max(df);
                if df > TOL_PT || df.is_nan() {
                    row.bad.push(format!("{kind} fill {df:.3}"));
                }
                if let Some(s) = shade {
                    row.bad.push(format!("{kind} {s}"));
                }
            }
        }
    }
    let mut failures = 0;
    println!("preset [batch]                          pages  worst stroke  worst fill  problems");
    for (k, r) in &rows {
        println!(
            "{k:<38} {:>5}  {:>12.3}  {:>10.3}  {}",
            r.pages,
            r.worst_stroke,
            r.worst_fill,
            if r.bad.is_empty() {
                "ok".to_owned()
            } else {
                r.bad.join("; ")
            }
        );
        failures += usize::from(!r.bad.is_empty());
    }
    assert!(!rows.is_empty(), "no b05/b06 batches found");
    assert_eq!(failures, 0, "presets off Word's output");
}
