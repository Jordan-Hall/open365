//! Shared scoring of the layout engine against Word reference JSON (see `word_parity.rs`).
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_opc::Limits;
use open365_text::{FontLibrary, FontMatch};
use serde_json::Value;

pub const TOLERANCE: f64 = 0.5;
/// Table cell rectangles are compared much tighter than text. Word snaps every cell edge to
/// its 0.24 pt grid (the engine does the same), so what is left is the rounding of the PDF's
/// numbers and the scale error of `PDF_SCALE_X`.
const CELL_TOLERANCE_X: f64 = 0.05;
/// Horizontal PDF scale of cell edges: Word's 600 dpi device pixel is 0.120048 pt in the PDF
/// (edges at 600 px = 72.024, 4500 px = 540.21), 1.0004 times the nominal 0.12 pt.
const PDF_SCALE_X: f64 = 1.0004;
/// Text box clip rectangles: the same grid as cells, a little looser while the rule is young.
const FRAME_TOLERANCE: f64 = 0.6;
const CELL_TOLERANCE_Y: f64 = 0.05;
/// Pictures: Word places them on its device grid and the PDF rounds the numbers; a picture must
/// land within this of Word's, edge by edge.
const IMAGE_TOLERANCE: f64 = 0.6;
/// Word's PDF places glyph origins 0.024 pt (1/3000 in) right of the layout position.
const PDF_SCALE: f64 = 1.0003;
const PDF_X_OFFSET: f64 = 0.024;

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub struct RefLine {
    /// Word's `WdStoryType` of the line's story (1, the main story, unless the oracle says).
    story: u8,
    /// The text box the line is in (`frame` in the reference), 0 for other stories.
    frame: u64,
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
pub struct RefCell {
    page: u64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    /// The text box whose text this clip rectangle bounds.
    frame: Option<u64>,
}

pub fn load_cells(path: &Path) -> Vec<RefCell> {
    load_rects(path, "cells")
}

/// Picture rectangles Word drew (PDF image matrix), page coordinates.
pub fn load_images(path: &Path) -> Vec<RefCell> {
    load_rects(path, "images")
}

pub fn load_rects(path: &Path, key: &str) -> Vec<RefCell> {
    let text = fs::read_to_string(path).expect("read reference json");
    let v: Value = serde_json::from_str(&text).expect("parse reference json");
    v[key]
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
                    frame: c["frame"].as_u64(),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn load_reference(path: &Path) -> Vec<RefLine> {
    let text = fs::read_to_string(path).expect("read reference json");
    let v: Value = serde_json::from_str(&text).expect("parse reference json");
    v["lines"]
        .as_array()
        .expect("lines array")
        .iter()
        .map(|l| RefLine {
            story: l["story"].as_u64().map_or(1, |v| v as u8),
            frame: l["frame"].as_u64().unwrap_or(0),
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
pub struct Score {
    pub total: usize,
    pub matched: usize,
    pub breaks: usize,
    pub page: usize,
    pub y: usize,
    pub x: usize,
    pub end: usize,
    pub last_x: usize,
}

pub fn show(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if c.is_control() { '~' } else { c })
        .collect();
    t.chars().take(40).collect()
}

/// `Err` carries the families that keep the fixture from being scored in this mode.
pub fn score(name: &str, detail: bool, mode: Mode, fonts: &FontLibrary) -> Result<Score, String> {
    let root = root();
    let ref_path = root.join(format!("fixtures/reference/layout/{name}.json"));
    let bytes = fs::read(root.join(format!("fixtures/docx/{name}.docx"))).expect("read docx");
    let doc = Document::open(&bytes, &Limits::default()).expect("open docx");
    score_doc(name, &ref_path, &doc, detail, mode, fonts, true)
}

/// Scores `doc` against the reference at `ref_path`. With `strict_fonts`, a document whose
/// fonts the mode cannot stand in for is returned as `Err` instead of being scored.
pub fn score_doc(
    name: &str,
    ref_path: &Path,
    doc: &Document,
    detail: bool,
    mode: Mode,
    fonts: &FontLibrary,
    strict_fonts: bool,
) -> Result<Score, String> {
    let reference = load_reference(ref_path);
    let ref_cells = load_cells(ref_path);
    let ref_images = load_images(ref_path);
    let layout = layout_document(doc, fonts, LayoutOptions::default());
    let unusable: Vec<String> = layout
        .fonts
        .iter()
        .filter(|f| match mode {
            Mode::Bundled => f.matched == FontMatch::Fallback,
            Mode::System => f.matched != FontMatch::Exact,
        })
        .map(|f| f.requested.clone())
        .collect();
    if strict_fonts && !unusable.is_empty() {
        return Err(unusable.join(", "));
    }

    // Lines by first-character offset; a repeated table header row shares its offset with the
    // original, so there can be several per key.
    type Key = (u8, usize, usize);
    let mut engine: BTreeMap<Key, Vec<(usize, &open365_layout::Line)>> = BTreeMap::new();
    for (p, l) in layout.all_lines() {
        engine
            .entry((l.story.word_code(), l.frame.unwrap_or(0), l.start))
            .or_default()
            .push((p, l));
    }
    let mut s = Score {
        total: reference.len() + ref_cells.len() + ref_images.len(),
        ..Score::default()
    };
    let mut report = String::new();
    for r in &reference {
        let found = engine
            .get(&(r.story, r.frame as usize, r.start as usize))
            .and_then(|c| {
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
        if let Some(f) = c.frame {
            let ok = layout
                .pages
                .iter()
                .filter(|p| p.number as u64 == c.page)
                .flat_map(|p| p.frames.iter())
                .any(|b| {
                    b.id as u64 == f
                        && (b.x - c.x / PDF_SCALE_X).abs() <= FRAME_TOLERANCE
                        && (b.y - c.y / PDF_SCALE).abs() <= FRAME_TOLERANCE
                        && (b.width - c.w / PDF_SCALE_X).abs() <= FRAME_TOLERANCE
                        && (b.height - c.h / PDF_SCALE).abs() <= FRAME_TOLERANCE
                });
            s.matched += usize::from(ok);
            if detail {
                let _ = writeln!(
                    report,
                    "{} frame {} p{} x {:>7.2} y {:>7.2} w {:>7.2} h {:>7.2}",
                    if ok { "ok  " } else { "FAIL" },
                    f,
                    c.page,
                    c.x,
                    c.y,
                    c.w,
                    c.h
                );
            }
            continue;
        }
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
                    && (b.x - c.x / PDF_SCALE_X).abs() <= CELL_TOLERANCE_X
                    && (b.y - c.y / PDF_SCALE).abs() <= CELL_TOLERANCE_Y
                    && (b.width - c.w / PDF_SCALE_X).abs() <= CELL_TOLERANCE_X
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
    // Pictures: each reference rectangle needs an unused engine picture that agrees.
    let mut used_images: Vec<(usize, usize)> = Vec::new();
    for im in &ref_images {
        let page = layout.pages.iter().find(|p| p.number as u64 == im.page);
        let hit = page.and_then(|p| {
            p.images.iter().enumerate().find(|(i, b)| {
                !used_images.contains(&(p.number, *i))
                    && (b.x - im.x).abs() <= IMAGE_TOLERANCE
                    && (b.y - im.y).abs() <= IMAGE_TOLERANCE
                    && (b.width - im.w).abs() <= IMAGE_TOLERANCE
                    && (b.height - im.h).abs() <= IMAGE_TOLERANCE
            })
        });
        if let (Some(p), Some((i, _))) = (page, hit) {
            used_images.push((p.number, i));
            s.matched += 1;
        }
        if detail {
            let near = page.and_then(|p| {
                p.images.iter().min_by(|a, b| {
                    let d = |q: &open365_layout::ImagePlacement| {
                        (q.x - im.x).abs()
                            + (q.y - im.y).abs()
                            + (q.width - im.w).abs()
                            + (q.height - im.h).abs()
                    };
                    d(a).total_cmp(&d(b))
                })
            });
            let _ = writeln!(
                report,
                "{} image p{} x {:>7.2} y {:>7.2} w {:>7.2} h {:>7.2}   engine {}",
                if hit.is_some() { "ok  " } else { "FAIL" },
                im.page,
                im.x,
                im.y,
                im.w,
                im.h,
                near.map_or("-".to_owned(), |b| format!(
                    "x {:.2} y {:.2} w {:.2} h {:.2}",
                    b.x, b.y, b.width, b.height
                )),
            );
        }
    }
    if detail {
        println!("--- {name}\n{report}");
        let used: Vec<String> = layout
            .fonts
            .iter()
            .map(|f| format!("{} ({:?})", f.requested, f.matched))
            .collect();
        println!("fonts: {}", used.join(", "));
        for note in &layout.notes {
            println!("note: {note}");
        }
    }
    Ok(s)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Bundled,
    System,
}

impl Mode {
    pub fn from_env() -> Self {
        if std::env::var("OPEN365_PARITY_SYSTEM_FONTS").is_ok_and(|v| v != "0" && !v.is_empty()) {
            Self::System
        } else {
            Self::Bundled
        }
    }
}

/// Families the system mode loads from the machine.
pub const SYSTEM_FAMILIES: [&str; 15] = [
    "Calibri",
    "Calibri Light",
    "Cambria",
    "Aptos",
    "Aptos Display",
    "Aptos Narrow",
    "Arial",
    "Times New Roman",
    "Courier New",
    "Segoe UI",
    "Symbol",
    "Cascadia Mono",
    "Bahnschrift",
    "Gabriola",
    "Aptos Light",
];

pub fn library(mode: Mode) -> FontLibrary {
    library_with(mode, &[])
}

/// Like [`library`], and in system mode also loads the `extra` families.
pub fn library_with(mode: Mode, extra: &[String]) -> FontLibrary {
    let mut lib = FontLibrary::bundled();
    if mode == Mode::System {
        let mut dirs = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let cloud = PathBuf::from(local).join("Microsoft/FontCache/4/CloudFonts");
            if cloud.is_dir() {
                dirs.push(cloud);
            }
        }
        let mut families: Vec<&str> = SYSTEM_FAMILIES.to_vec();
        families.extend(extra.iter().map(String::as_str));
        lib.add_system_families(&families, &dirs);
    }
    lib
}
