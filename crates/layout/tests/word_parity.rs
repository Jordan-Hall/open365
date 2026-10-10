//! Compares the layout engine with Microsoft Word, line by line.
//!
//! Reference: `fixtures/reference/layout/<name>.json`, written by
//! `tools/office/word-layout-oracle.ps1` from the real Word. Each reference line must be
//! reproduced with the same page, the same first-character offset (so identical line breaks),
//! the same vertical position (baseline from the PDF when the oracle has it, else the screen
//! line top) within 0.5 pt, and the same x of the first drawn glyph within 0.5 pt (from the PDF
//! when available: the screen `left`/`lastX` values are unreliable for aligned text).
//!
//! The expected number of matching lines per fixture lives in
//! `fixtures/reference/layout/parity-floor.toml`. The floor only goes up: a lower result fails
//! the test; a higher one prints a note, and `UPDATE_PARITY_FLOOR=1` rewrites the file.
//! `PARITY_DETAIL=<fixture>` prints every reference line with the engine's answer.
//!
//! Two font modes. The default uses only the bundled fonts (Calibri -> Carlito, Cambria ->
//! Caladea), so results do not depend on the machine; a fixture that asks for a family the
//! bundle cannot stand in for (Calibri Light, Arial, Aptos ...) is reported as skipped, never as
//! passed. Caladea is not metric-compatible with the Cambria Word used (its glyphs are up to
//! 10 % narrower), so Cambria text only approximates. With `OPEN365_PARITY_SYSTEM_FONTS=1` the
//! test loads the real faces from the machine (Windows font folders and Office's cloud font
//! cache for Aptos) and scores against the `[system.*]` floors; a fixture whose families the
//! machine lacks is skipped. The references were produced on a machine with those fonts.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_text::{FontLibrary, FontMatch};
use serde_json::Value;

const TOLERANCE: f64 = 0.5;
/// Table cell rectangles are compared much tighter than text: the PDF writes five significant
/// digits and Word snaps edges to its 600 dpi device grid (0.12 pt), so 0.1 pt is one device pixel
/// less the rounding of the numbers.
const CELL_TOLERANCE_X: f64 = 0.25;
/// Vertical edges follow the 0.24 pt grid the engine models, so only the PDF's rounding is left.
const CELL_TOLERANCE_Y: f64 = 0.05;
/// Word's PDF places glyph origins 0.024 pt (1/3000 in) right of the layout position.
const PDF_SCALE: f64 = 1.0003;
const PDF_X_OFFSET: f64 = 0.024;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct RefLine {
    /// Word's `WdStoryType` of the line's story (1, the main story, unless the oracle says).
    story: u8,
    page: u64,
    start: u64,
    end: u64,
    top: f64,
    left: f64,
    /// Origin x of the first glyph drawn on the line, from Word's 600 dpi PDF.
    ink_left: Option<f64>,
    last_x: Option<f64>,
    baseline: Option<f64>,
    text: String,
}

/// A table cell rectangle Word drew (PDF clip rectangle), page coordinates.
struct RefCell {
    page: u64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

fn load_cells(path: &Path) -> Vec<RefCell> {
    let text = fs::read_to_string(path).expect("read reference json");
    let v: Value = serde_json::from_str(&text).expect("parse reference json");
    v["cells"]
        .as_array()
        .map(|cells| {
            cells
                .iter()
                .map(|c| RefCell {
                    page: c["page"].as_u64().expect("cell page"),
                    x: c["x"].as_f64().expect("cell x"),
                    y: c["y"].as_f64().expect("cell y"),
                    w: c["w"].as_f64().expect("cell w"),
                    h: c["h"].as_f64().expect("cell h"),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn load_reference(path: &Path) -> Vec<RefLine> {
    let text = fs::read_to_string(path).expect("read reference json");
    let v: Value = serde_json::from_str(&text).expect("parse reference json");
    v["lines"]
        .as_array()
        .expect("lines array")
        .iter()
        .map(|l| RefLine {
            story: l["story"].as_u64().map_or(1, |v| v as u8),
            page: l["page"].as_u64().expect("page"),
            start: l["start"].as_u64().expect("start"),
            end: l["end"].as_u64().expect("end"),
            top: l["top"].as_f64().expect("top"),
            left: l["left"].as_f64().expect("left"),
            ink_left: l["inkLeft"].as_f64(),
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

/// `Err` carries the families that keep the fixture from being scored in this mode.
fn score(name: &str, detail: bool, mode: Mode, fonts: &FontLibrary) -> Result<Score, String> {
    let root = root();
    let ref_path = root.join(format!("fixtures/reference/layout/{name}.json"));
    let reference = load_reference(&ref_path);
    let ref_cells = load_cells(&ref_path);
    let bytes = fs::read(root.join(format!("fixtures/docx/{name}.docx"))).expect("read docx");
    let doc = Document::open(&bytes, &Limits::default()).expect("open docx");
    let layout = layout_document(&doc, fonts, LayoutOptions::default());
    let unusable: Vec<String> = layout
        .fonts
        .iter()
        .filter(|f| match mode {
            Mode::Bundled => f.matched == FontMatch::Fallback,
            Mode::System => f.matched != FontMatch::Exact,
        })
        .map(|f| f.requested.clone())
        .collect();
    if !unusable.is_empty() {
        return Err(unusable.join(", "));
    }

    // Lines by first-character offset; a repeated table header row shares its offset with the
    // original, so there can be several per key.
    let mut engine: BTreeMap<(u8, usize), Vec<(usize, &open365_layout::Line)>> = BTreeMap::new();
    for (p, l) in layout.all_lines() {
        engine
            .entry((l.story.word_code(), l.start))
            .or_default()
            .push((p, l));
    }
    let mut s = Score {
        total: reference.len() + ref_cells.len(),
        ..Score::default()
    };
    let mut report = String::new();
    for r in &reference {
        let found = engine.get(&(r.story, r.start as usize)).and_then(|c| {
            c.iter()
                .find(|(p, _)| *p as u64 == r.page)
                .or_else(|| c.first())
        });
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
            let (ex, rx) = match r.ink_left {
                Some(ink) => (l.first_glyph_x(), ink - PDF_X_OFFSET),
                None => (l.left, r.left),
            };
            let x_ok = (ex - rx).abs() <= TOLERANCE;
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
                    ex,
                    rx,
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
    // Table cell rectangles: each reference rectangle needs an unused engine rectangle that agrees.
    let mut used: Vec<(usize, usize)> = Vec::new();
    for c in &ref_cells {
        let hit = layout
            .pages
            .iter()
            .filter(|p| p.number as u64 == c.page)
            .flat_map(|p| {
                p.cells
                    .iter()
                    .enumerate()
                    .map(move |(i, b)| (p.number, i, b))
            })
            .find(|(pn, i, b)| {
                !used.contains(&(*pn, *i))
                    && (b.x - c.x / PDF_SCALE).abs() <= CELL_TOLERANCE_X
                    && (b.y - c.y / PDF_SCALE).abs() <= CELL_TOLERANCE_Y
                    && (b.width - c.w / PDF_SCALE).abs() <= CELL_TOLERANCE_X
                    && (b.height - c.h / PDF_SCALE).abs() <= CELL_TOLERANCE_Y
            });
        if let Some((pn, i, _)) = hit {
            used.push((pn, i));
            s.matched += 1;
        }
        if detail {
            let near = layout
                .pages
                .iter()
                .filter(|p| p.number as u64 == c.page)
                .flat_map(|p| p.cells.iter())
                .min_by(|a, b| {
                    let d = |q: &open365_layout::CellBox| {
                        (q.x - c.x).abs()
                            + (q.y - c.y).abs()
                            + (q.width - c.w).abs()
                            + (q.height - c.h).abs()
                    };
                    d(a).total_cmp(&d(b))
                });
            let _ = writeln!(
                report,
                "{} cell p{} x {:>7.2} y {:>7.2} w {:>7.2} h {:>7.2}   engine {}",
                if hit.is_some() { "ok  " } else { "FAIL" },
                c.page,
                c.x - PDF_X_OFFSET,
                c.y,
                c.w,
                c.h,
                near.map_or("-".to_owned(), |b| format!(
                    "x {:.2} y {:.2} w {:.2} h {:.2}",
                    b.x, b.y, b.width, b.height
                )),
            );
        }
    }
    if detail {
        println!("--- {name}\n{report}");
        for note in &layout.notes {
            println!("note: {note}");
        }
    }
    Ok(s)
}

fn floor_path() -> PathBuf {
    root().join("fixtures/reference/layout/parity-floor.toml")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Bundled,
    System,
}

impl Mode {
    fn from_env() -> Self {
        if std::env::var("OPEN365_PARITY_SYSTEM_FONTS").is_ok_and(|v| v != "0" && !v.is_empty()) {
            Self::System
        } else {
            Self::Bundled
        }
    }
}

type Floors = BTreeMap<String, (usize, usize)>;

fn read_floors() -> (Floors, Floors) {
    let text = fs::read_to_string(floor_path()).expect("parity-floor.toml");
    let table: toml::Table = text.parse().expect("parse parity-floor.toml");
    let section = |name: &str| {
        let mut out = Floors::new();
        if let Some(toml::Value::Table(fixtures)) = table.get(name) {
            for (fixture, v) in fixtures {
                let m = v["matched"].as_integer().expect("matched") as usize;
                let t = v["total"].as_integer().expect("total") as usize;
                out.insert(fixture.clone(), (m, t));
            }
        }
        out
    };
    (section("fixtures"), section("system"))
}

fn write_floors(bundled: &Floors, system: &Floors) {
    let mut out = String::from(concat!(
        "# Ratchet for crates/layout/tests/word_parity.rs: lines that must match Word.
",
        "# [fixtures.*] is the default bundled-font mode, [system.*] the
",
        "# OPEN365_PARITY_SYSTEM_FONTS=1 mode. Raise `matched` when the engine improves
",
        "# (UPDATE_PARITY_FLOOR=1 cargo test -p open365-layout --test word_parity); never
",
        "# lower it. `total` is the number of reference lines.
",
    ));
    for (section, floors) in [("fixtures", bundled), ("system", system)] {
        for (name, (matched, total)) in floors {
            let _ = write!(
                out,
                "
[{section}.{name}]
matched = {matched}
total = {total}
"
            );
        }
    }
    fs::write(floor_path(), out).expect("write floor");
}

/// Families the system mode loads from the machine.
const SYSTEM_FAMILIES: [&str; 10] = [
    "Calibri",
    "Calibri Light",
    "Cambria",
    "Aptos",
    "Aptos Display",
    "Aptos Narrow",
    "Arial",
    "Times New Roman",
    "Segoe UI",
    "Symbol",
];

fn library(mode: Mode) -> FontLibrary {
    let mut lib = FontLibrary::bundled();
    if mode == Mode::System {
        let mut dirs = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let cloud = PathBuf::from(local).join("Microsoft/FontCache/4/CloudFonts");
            if cloud.is_dir() {
                dirs.push(cloud);
            }
        }
        lib.add_system_families(&SYSTEM_FAMILIES, &dirs);
    }
    lib
}

fn reference_names() -> Vec<String> {
    let dir = root().join("fixtures/reference/layout");
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("reference dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        // `<name>.lists.json` holds Word's list labels, not line positions.
        .filter(|n| !n.ends_with(".lists"))
        .collect();
    names.sort();
    names
}

#[test]
fn layout_matches_word_at_least_as_well_as_the_floor() {
    let detail = std::env::var("PARITY_DETAIL").ok();
    let update = std::env::var_os("UPDATE_PARITY_FLOOR").is_some();
    let mode = Mode::from_env();
    let (mut bundled_floor, mut system_floor) = read_floors();
    let floor = match mode {
        Mode::Bundled => &mut bundled_floor,
        Mode::System => &mut system_floor,
    };
    let fonts = library(mode);
    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    let mut summary = format!(
        "
Word layout parity, {} fonts (matched / total lines; stage columns count lines whose break is found)
",
        match mode {
            Mode::Bundled => "bundled",
            Mode::System => "system",
        }
    );
    let _ = writeln!(
        summary,
        "{:<24} {:>9}  {:>6} {:>5} {:>5} {:>5} {:>5} {:>6}",
        "fixture", "matched", "breaks", "page", "y", "x", "end", "lastX"
    );
    let (mut all_matched, mut all_total) = (0, 0);
    for name in reference_names() {
        let want = detail.as_deref().is_some_and(|d| d == name || d == "all");
        let s = match score(&name, want, mode, &fonts) {
            Ok(s) => s,
            Err(missing) => {
                skipped.push(format!("{name} (needs {missing})"));
                continue;
            }
        };
        all_matched += s.matched;
        all_total += s.total;
        let _ = writeln!(
            summary,
            "{:<24} {:>4}/{:<4}  {:>6} {:>5} {:>5} {:>5} {:>5} {:>6}",
            name, s.matched, s.total, s.breaks, s.page, s.y, s.x, s.end, s.last_x
        );
        match floor.get(&name).copied() {
            None => {
                if !update {
                    failures.push(format!(
                        "{name}: no entry in parity-floor.toml (matched {}/{})",
                        s.matched, s.total
                    ));
                }
            }
            Some((fm, ft)) => {
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
        if update {
            let keep = floor.get(&name).map_or(0, |f| f.0).max(s.matched);
            floor.insert(name, (keep, s.total));
        }
    }
    let _ = writeln!(
        summary,
        "{:<24} {:>4}/{:<4}",
        "all scored", all_matched, all_total
    );
    for item in &skipped {
        let _ = writeln!(summary, "skipped: {item}");
    }
    println!("{summary}");
    if update {
        write_floors(&bundled_floor, &system_floor);
    }
    assert!(
        failures.is_empty(),
        "parity regressions:
{}",
        failures.join(
            "
"
        )
    );
}
