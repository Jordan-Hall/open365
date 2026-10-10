//! Compares the layout engine with Microsoft Word, line by line.
//!
//! Reference: `fixtures/reference/layout/<name>.json`, written by
//! `tools/office/word-layout-oracle.ps1` from the real Word. Each reference line must be
//! reproduced with the same page, the same first-character offset (so identical line breaks),
//! the same vertical position (baseline from the PDF when the oracle has it, else the screen
//! line top) within 0.5 pt, and the same x of the first character within 0.5 pt.
//!
//! The expected number of matching lines per fixture lives in
//! `fixtures/reference/layout/parity-floor.toml`. The floor only goes up: a lower result fails
//! the test; a higher one prints a note, and `UPDATE_PARITY_FLOOR=1` rewrites the file.
//! `PARITY_DETAIL=<fixture>` prints every reference line with the engine's answer.
//!
//! Only the bundled fonts are used (Calibri -> Carlito, Cambria -> Caladea), so results do not
//! depend on the machine.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_text::FontLibrary;
use serde_json::Value;

const TOLERANCE: f64 = 0.5;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct RefLine {
    page: u64,
    start: u64,
    end: u64,
    top: f64,
    left: f64,
    last_x: Option<f64>,
    baseline: Option<f64>,
    text: String,
}

fn load_reference(path: &Path) -> Vec<RefLine> {
    let text = fs::read_to_string(path).expect("read reference json");
    let v: Value = serde_json::from_str(&text).expect("parse reference json");
    v["lines"]
        .as_array()
        .expect("lines array")
        .iter()
        .map(|l| RefLine {
            page: l["page"].as_u64().expect("page"),
            start: l["start"].as_u64().expect("start"),
            end: l["end"].as_u64().expect("end"),
            top: l["top"].as_f64().expect("top"),
            left: l["left"].as_f64().expect("left"),
            last_x: l["lastX"].as_f64(),
            baseline: l["baseline"].as_f64(),
            text: l["text"].as_str().unwrap_or_default().to_owned(),
        })
        .collect()
}

#[derive(Default)]
struct Score {
    total: usize,
    matched: usize,
    breaks: usize,
    page: usize,
    y: usize,
    x: usize,
    end: usize,
    last_x: usize,
}

fn show(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_control() { '~' } else { c })
        .collect();
    t.chars().take(40).collect()
}

fn score(name: &str, detail: bool) -> Score {
    let root = root();
    let reference = load_reference(&root.join(format!("fixtures/reference/layout/{name}.json")));
    let bytes = fs::read(root.join(format!("fixtures/docx/{name}.docx"))).expect("read docx");
    let doc = Document::open(&bytes, &Limits::default()).expect("open docx");
    let fonts = FontLibrary::bundled();
    let layout = layout_document(&doc, &fonts, LayoutOptions::default());

    let engine: BTreeMap<usize, (usize, &open365_layout::Line)> =
        layout.lines().map(|(p, l)| (l.start, (p, l))).collect();
    let mut s = Score {
        total: reference.len(),
        ..Score::default()
    };
    let mut report = String::new();
    for r in &reference {
        let found = engine.get(&(r.start as usize));
        let mut ok = false;
        if let Some((page, l)) = found {
            let page_ok = *page as u64 == r.page;
            let ry = r.baseline.unwrap_or(r.top);
            let ey = if r.baseline.is_some() {
                l.baseline
            } else {
                l.top
            };
            let y_ok = (ey - ry).abs() <= TOLERANCE;
            let x_ok = (l.left - r.left).abs() <= TOLERANCE;
            let end_ok = l.end as u64 == r.end;
            let lx_ok = r.last_x.is_none_or(|lx| (l.last_x - lx).abs() <= TOLERANCE);
            s.breaks += 1;
            s.page += usize::from(page_ok);
            s.y += usize::from(page_ok && y_ok);
            s.x += usize::from(x_ok);
            s.end += usize::from(end_ok);
            s.last_x += usize::from(lx_ok);
            ok = page_ok && y_ok && x_ok;
            if detail {
                let _ = writeln!(
                    report,
                    "{} p{}/{} @{:>5} y {:>8.2}/{:>8.2} ({:+.2}) x {:>7.2}/{:>7.2} end {}/{} lastX {:>7.2}/{} {}",
                    if ok { "ok  " } else { "FAIL" },
                    page,
                    r.page,
                    r.start,
                    ey,
                    ry,
                    ey - ry,
                    l.left,
                    r.left,
                    l.end,
                    r.end,
                    l.last_x,
                    r.last_x.map_or("-".to_owned(), |v| format!("{v:.2}")),
                    show(&r.text),
                );
            }
        } else if detail {
            let _ = writeln!(
                report,
                "MISS  p{} @{:>5} (ref end {}) {}",
                r.page,
                r.start,
                r.end,
                show(&r.text)
            );
        }
        s.matched += usize::from(ok);
    }
    if detail {
        println!("--- {name}\n{report}");
        for note in &layout.notes {
            println!("note: {note}");
        }
    }
    s
}

fn floor_path() -> PathBuf {
    root().join("fixtures/reference/layout/parity-floor.toml")
}

fn read_floor() -> BTreeMap<String, (usize, usize)> {
    let text = fs::read_to_string(floor_path()).expect("parity-floor.toml");
    let table: toml::Table = text.parse().expect("parse parity-floor.toml");
    let mut out = BTreeMap::new();
    if let Some(toml::Value::Table(fixtures)) = table.get("fixtures") {
        for (name, v) in fixtures {
            let m = v["matched"].as_integer().expect("matched") as usize;
            let t = v["total"].as_integer().expect("total") as usize;
            out.insert(name.clone(), (m, t));
        }
    }
    out
}

fn reference_names() -> Vec<String> {
    let dir = root().join("fixtures/reference/layout");
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("reference dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    names.sort();
    names
}

#[test]
fn layout_matches_word_at_least_as_well_as_the_floor() {
    let detail = std::env::var("PARITY_DETAIL").ok();
    let update = std::env::var_os("UPDATE_PARITY_FLOOR").is_some();
    let floor = read_floor();
    let mut failures = Vec::new();
    let mut summary = String::from(
        "\nWord layout parity (matched / total lines; stage columns count lines whose break is found)\n",
    );
    let _ = writeln!(
        summary,
        "{:<24} {:>9}  {:>6} {:>5} {:>5} {:>5} {:>5} {:>6}",
        "fixture", "matched", "breaks", "page", "y", "x", "end", "lastX"
    );
    let mut new_floor = String::from(
        "# Ratchet for crates/layout/tests/word_parity.rs: lines that must match Word.\n\
         # Raise `matched` when the engine improves (UPDATE_PARITY_FLOOR=1 cargo test -p open365-layout\n\
         # --test word_parity); never lower it. `total` is the number of reference lines.\n",
    );
    let (mut all_matched, mut all_total) = (0, 0);
    for name in reference_names() {
        let want = detail.as_deref().is_some_and(|d| d == name || d == "all");
        let s = score(&name, want);
        all_matched += s.matched;
        all_total += s.total;
        let _ = writeln!(
            summary,
            "{:<24} {:>4}/{:<4}  {:>6} {:>5} {:>5} {:>5} {:>5} {:>6}",
            name, s.matched, s.total, s.breaks, s.page, s.y, s.x, s.end, s.last_x
        );
        match floor.get(&name) {
            None => failures.push(format!(
                "{name}: no entry in parity-floor.toml (matched {}/{})",
                s.matched, s.total
            )),
            Some(&(fm, ft)) => {
                if ft != s.total {
                    failures.push(format!(
                        "{name}: reference has {} lines, floor file says {ft}",
                        s.total
                    ));
                }
                if s.matched < fm {
                    failures.push(format!(
                        "{name}: matched {} lines, floor is {fm}",
                        s.matched
                    ));
                } else if s.matched > fm {
                    let _ = writeln!(
                        summary,
                        "  note: {name} improved to {} (floor {fm}); raise the floor",
                        s.matched
                    );
                }
            }
        }
        let keep = if update {
            s.matched
        } else {
            floor.get(&name).map_or(0, |f| f.0)
        };
        let _ = write!(
            new_floor,
            "\n[fixtures.{name}]\nmatched = {keep}\ntotal = {}\n",
            s.total
        );
    }
    let _ = writeln!(summary, "{:<24} {:>4}/{:<4}", "all", all_matched, all_total);
    println!("{summary}");
    if update {
        fs::write(floor_path(), new_floor).expect("write floor");
    }
    assert!(
        update || failures.is_empty(),
        "parity regressions:\n{}",
        failures.join("\n")
    );
}
